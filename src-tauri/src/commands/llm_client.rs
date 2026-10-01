//! OpenAI 兼容的 Chat Completions 客户端：标签精修、标签排序、LLM 打标共用。
//!
//! 只有一种协议：POST `{endpoint}/chat/completions`，Bearer 鉴权，取第一个 choice。
//! 审核拒绝（finish_reason 为 content_filter / safety）一律是错误；截断（finish_reason ==
//! "length"）能不能接受由调用方决定——标签列表被截断就是残缺的，写盘会丢标签。
//! 这里还放着几个功能共用的回复处理与日志小工具（拒绝语判定、取标签行、耗时与增删摘要）。

use base64::Engine;
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// 发给 VLM 的图片最长边，`image_data_url` 收到 0 时用它
pub(crate) const DEFAULT_IMAGE_MAX_SIDE: u32 = 1024;

/// 节流等待期间检查取消标志的间隔
const THROTTLE_POLL: Duration = Duration::from_millis(200);

/// LLM 请求用的 HTTP 客户端：仅当开启 llm_proxy 时走应用内代理，超时见 build_http_client_for_llm
pub(crate) fn llm_http_client() -> Result<reqwest::Client, String> {
    super::proxy_config::build_http_client_for_llm()
        .build()
        .map_err(|e| format!("创建 HTTP 客户端失败: {}", e))
}

/// 拼接 `{endpoint}/{path}`；endpoint 末尾有没有 `/` 都行
pub(crate) fn api_url(endpoint: &str, path: &str) -> String {
    if endpoint.ends_with('/') {
        format!("{}{}", endpoint, path)
    } else {
        format!("{}/{}", endpoint, path)
    }
}

/// 一次请求的连接与采样参数，字段直接取自各功能的选项
#[derive(Debug, Clone, Copy)]
pub(crate) struct ChatParams<'a> {
    /// OpenAI 兼容基址，例如 `https://api.openai.com/v1`
    pub endpoint: &'a str,
    /// 为空时不发 Authorization 头
    pub api_key: &'a str,
    pub model: &'a str,
    pub temperature: f32,
    /// <= 0 时不发送，由服务商决定输出上限
    pub max_tokens: i32,
    /// 只在 (0, 1] 内发送
    pub top_p: f64,
}

/// 一条对话消息
#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct ChatMessage {
    pub role: &'static str,
    pub content: serde_json::Value,
}

impl ChatMessage {
    pub(crate) fn system(text: impl Into<String>) -> Self {
        ChatMessage {
            role: "system",
            content: serde_json::Value::String(text.into()),
        }
    }

    /// content 可以是纯文本（String / &str），也可以是 `vision_user_content` 拼出的图文数组
    pub(crate) fn user(content: impl Into<serde_json::Value>) -> Self {
        ChatMessage {
            role: "user",
            content: content.into(),
        }
    }
}

/// 一次成功的回复
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ChatReply {
    /// 去掉首尾空白的回复正文，一定非空。content 为空时取 reasoning_content（Qwen3 等思维模型）
    pub text: String,
    pub finish_reason: Option<String>,
}

impl ChatReply {
    /// 回复因输出长度上限被截断（finish_reason == "length"），正文是残缺的
    pub(crate) fn is_truncated(&self) -> bool {
        self.finish_reason.as_deref() == Some("length")
    }
}

/// `chat_completion` 的失败原因；Display 即给用户看的文案
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ChatError {
    /// 节流等待期间取消标志置位，请求没有发出
    Cancelled,
    /// finish_reason 为 content_filter / safety：服务商的内容安全审核拦下了这次请求。
    /// 默认文案不提具体对象，需要"该图片"之类措辞的调用方自己 match 这一项
    ContentFilter,
    /// 网络、HTTP 状态、响应解析、空回复等
    Other(String),
}

impl std::fmt::Display for ChatError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ChatError::Cancelled => f.write_str("已取消"),
            ChatError::ContentFilter => f.write_str("LLM 内容安全审核拒绝了该请求"),
            ChatError::Other(message) => f.write_str(message),
        }
    }
}

impl std::error::Error for ChatError {}

impl From<ChatError> for String {
    fn from(e: ChatError) -> String {
        e.to_string()
    }
}

/// 全局请求起点节流：一个批任务的所有并发 worker 共享一个（放进 Arc）。
/// 第一次请求立即放行；之后相邻两次请求的发出时刻至少间隔 interval_ms，<= 0 不节流。
/// 等待期间持有内部锁，其他 worker 依次排在后面。
pub(crate) struct RequestThrottle {
    interval_ms: i64,
    last: tokio::sync::Mutex<Option<Instant>>,
}

impl RequestThrottle {
    pub(crate) fn new(interval_ms: i64) -> Self {
        RequestThrottle {
            interval_ms,
            last: tokio::sync::Mutex::new(None),
        }
    }

    /// 等到可以发下一个请求并占下这个时刻；等待期间每 200ms 检查一次取消，取消时返回 false
    pub(crate) async fn wait(&self, cancel: &AtomicBool) -> bool {
        if self.interval_ms <= 0 {
            return true;
        }

        let mut last = self.last.lock().await;
        if let Some(previous) = *last {
            let interval = Duration::from_millis(self.interval_ms as u64);
            let mut remaining = interval.saturating_sub(previous.elapsed());
            while !remaining.is_zero() {
                if cancel.load(Ordering::SeqCst) {
                    return false;
                }
                let step = remaining.min(THROTTLE_POLL);
                tokio::time::sleep(step).await;
                remaining -= step;
            }
        }

        if cancel.load(Ordering::SeqCst) {
            return false;
        }
        *last = Some(Instant::now());
        true
    }
}

#[derive(Serialize)]
struct ChatRequest<'a> {
    model: &'a str,
    messages: &'a [ChatMessage],
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
    temperature: f32,
    #[serde(skip_serializing_if = "Option::is_none")]
    top_p: Option<f64>,
}

#[derive(Deserialize)]
struct ChatResponse {
    choices: Vec<ChatChoice>,
}

#[derive(Deserialize)]
struct ChatChoice {
    message: ChatChoiceMessage,
    #[serde(default)]
    finish_reason: Option<String>,
}

#[derive(Deserialize)]
struct ChatChoiceMessage {
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    reasoning_content: Option<String>,
}

/// 发一次 Chat Completions 请求。
///
/// 依次：节流等待（取消返回 `Cancelled`）→ POST `{endpoint}/chat/completions` →
/// 非 2xx 带上响应体报错 → 取第一个 choice → 审核拒绝返回 `ContentFilter` →
/// content 为空时取 reasoning_content，两者都为空报错。
///
/// 请求发出后不再观察取消：要中途放弃，调用方用 `select!` 包住整个单项任务（各批任务现在就是这样做的）。
/// 截断不在这里判定，见 `ChatReply::is_truncated`；拒绝语（"I'm sorry…"）也不在这里判定，见 `reject_refusal`。
pub(crate) async fn chat_completion(
    client: &reqwest::Client,
    params: &ChatParams<'_>,
    messages: &[ChatMessage],
    throttle: &RequestThrottle,
    cancel: &AtomicBool,
) -> Result<ChatReply, ChatError> {
    if !throttle.wait(cancel).await {
        return Err(ChatError::Cancelled);
    }

    let body = ChatRequest {
        model: params.model,
        messages,
        max_tokens: (params.max_tokens > 0).then_some(params.max_tokens as u32),
        temperature: params.temperature,
        top_p: (params.top_p > 0.0 && params.top_p <= 1.0).then_some(params.top_p),
    };
    let mut request = client
        .post(api_url(params.endpoint, "chat/completions"))
        .json(&body);
    if !params.api_key.is_empty() {
        request = request.bearer_auth(params.api_key);
    }

    let response = request
        .send()
        .await
        .map_err(|e| ChatError::Other(format!("API 请求失败: {}", e)))?;
    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        return Err(ChatError::Other(format!("API 错误 ({}): {}", status, body)));
    }
    let parsed: ChatResponse = response
        .json()
        .await
        .map_err(|e| ChatError::Other(format!("解析响应失败: {}", e)))?;
    let choice = parsed
        .choices
        .into_iter()
        .next()
        .ok_or_else(|| ChatError::Other("API 未返回任何结果".to_string()))?;

    if matches!(
        choice.finish_reason.as_deref(),
        Some("content_filter") | Some("safety")
    ) {
        return Err(ChatError::ContentFilter);
    }

    let non_empty = |s: Option<String>| s.map(|t| t.trim().to_string()).filter(|t| !t.is_empty());
    let text =
        non_empty(choice.message.content).or_else(|| non_empty(choice.message.reasoning_content));
    let Some(text) = text else {
        // 思维模型把输出上限用在推理上时，正文和 reasoning_content 可能都是空的
        let message = if choice.finish_reason.as_deref() == Some("length") {
            "API 返回空内容（输出达到长度上限被截断）"
        } else {
            "API 返回空内容"
        };
        return Err(ChatError::Other(message.to_string()));
    };

    Ok(ChatReply {
        text,
        finish_reason: choice.finish_reason,
    })
}

#[derive(Deserialize)]
struct ModelsResponse {
    data: Vec<ModelEntry>,
}

#[derive(Deserialize)]
struct ModelEntry {
    id: String,
}

/// GET `{endpoint}/models`，返回按字典序排好的模型 id
pub(crate) async fn list_models(
    client: &reqwest::Client,
    endpoint: &str,
    api_key: &str,
) -> Result<Vec<String>, String> {
    let mut request = client.get(api_url(endpoint, "models"));
    if !api_key.is_empty() {
        request = request.bearer_auth(api_key);
    }
    // Debug 格式带出底层原因（DNS、TLS、连接被拒），Display 只有一句 "error sending request"
    let response = request
        .send()
        .await
        .map_err(|e| format!("请求模型列表失败: {:?}", e))?;
    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        return Err(format!("API 错误 ({}): {}", status, body));
    }
    let models: ModelsResponse = response
        .json()
        .await
        .map_err(|e| format!("解析模型列表失败: {}", e))?;
    let mut ids: Vec<String> = models.data.into_iter().map(|m| m.id).collect();
    ids.sort();
    Ok(ids)
}

/// 读图并编码成发给 VLM 的 JPEG data URL。
///
/// 按文件内容识别格式（扩展名写错的图也能读）；最长边超过 max_side 时等比缩到 max_side
/// （0 按 `DEFAULT_IMAGE_MAX_SIDE`）；透明图按白底拍平——JPEG 不接受 alpha，直接丢掉会让透明区变成脏色。
/// 阻塞函数：解码、缩放、编码都是 CPU 密集操作，异步代码里用 `load_image_data_url`。
pub(crate) fn image_data_url(path: &Path, max_side: u32) -> Result<String, String> {
    let max_side = if max_side > 0 {
        max_side
    } else {
        DEFAULT_IMAGE_MAX_SIDE
    };
    let img = image::ImageReader::open(path)
        .map_err(|e| format!("读取图片失败: {}", e))?
        .with_guessed_format()
        .map_err(|e| format!("无法识别图片格式: {}", e))?
        .decode()
        .map_err(|e| format!("无法解码图片: {}", e))?;

    let img = if img.width() > max_side || img.height() > max_side {
        img.resize(max_side, max_side, image::imageops::FilterType::Lanczos3)
    } else {
        img
    };
    let img = super::flatten_to_rgb_white(img);

    let mut buf = std::io::Cursor::new(Vec::new());
    img.write_to(&mut buf, image::ImageFormat::Jpeg)
        .map_err(|e| format!("编码图片失败: {}", e))?;
    let b64 = base64::engine::general_purpose::STANDARD.encode(buf.get_ref());
    Ok(format!("data:image/jpeg;base64,{}", b64))
}

/// `image_data_url` 放进阻塞线程池执行，批量任务里不占异步执行器线程
pub(crate) async fn load_image_data_url(path: &Path, max_side: u32) -> Result<String, String> {
    let path = path.to_path_buf();
    tokio::task::spawn_blocking(move || image_data_url(&path, max_side))
        .await
        .map_err(|e| format!("图片处理任务失败: {}", e))?
}

/// 图文 user 消息的 content：`[{text}, {image_url}]`。
/// detail（low / high / original / auto）去掉空白后为空时整个字段不发——
/// Gemini 兼容层等不认识它的端点会因未知字段报错
pub(crate) fn vision_user_content(text: &str, data_url: &str, detail: &str) -> serde_json::Value {
    let mut image_url = serde_json::json!({ "url": data_url });
    let detail = detail.trim();
    if !detail.is_empty() {
        image_url["detail"] = serde_json::Value::String(detail.to_string());
    }
    serde_json::json!([
        { "type": "text", "text": text },
        { "type": "image_url", "image_url": image_url }
    ])
}

/// 像是模型拒绝了（疑似内容安全审核）就返回带摘录的错误：
/// "LLM 拒绝处理{subject}（疑似内容安全审核）: {前 80 字}"。subject 例如 "该图片"、"该标签文件"
pub(crate) fn reject_refusal(text: &str, subject: &str) -> Result<(), String> {
    if super::looks_like_refusal(text) {
        let excerpt: String = text.trim().chars().take(80).collect();
        return Err(format!(
            "LLM 拒绝处理{}（疑似内容安全审核）: {}",
            subject, excerpt
        ));
    }
    Ok(())
}

/// 无标记格式的回复：多行时取最长的含逗号行，没有含逗号的行就原样返回
pub(crate) fn pick_tag_line(text: &str) -> &str {
    if !text.contains('\n') {
        return text;
    }
    text.lines()
        .filter(|l| l.contains(','))
        .max_by_key(|l| l.len())
        .unwrap_or(text)
}

/// 日志里的耗时：不足 1 秒写毫秒，否则保留一位小数的秒
pub(crate) fn fmt_elapsed(ms: u128) -> String {
    if ms >= 1000 {
        format!("{:.1}s", ms as f64 / 1000.0)
    } else {
        format!("{}ms", ms)
    }
}

/// `{label}: a, b, c` 形式的增删摘要，最多列 5 个，超出时注明总数；没有标签时返回 None
pub(crate) fn summarize_tags(label: &str, tags: &[&str]) -> Option<String> {
    if tags.is_empty() {
        return None;
    }
    let shown: Vec<&str> = tags.iter().take(5).copied().collect();
    let suffix = if tags.len() > 5 {
        format!("等{}个", tags.len())
    } else {
        String::new()
    };
    Some(format!("{}: {}{}", label, shown.join(", "), suffix))
}

/// 测试用的 OpenAI 兼容 mock 服务器（tag_sort 等模块的测试也用它）。
/// 返回的 `TestServer` 由测试持有，drop 时关闭；`url` 已是 OpenAI 兼容基址（`…/v1`）
#[cfg(test)]
pub(crate) mod test_support {
    pub(crate) use crate::commands::http_download::test_support::TempDir;
    use crate::commands::http_download::test_support::{serve_once, write_response, TestServer};

    /// 直连本地 mock 的客户端：不读应用的代理配置，也不吃系统代理环境变量
    pub(crate) fn client() -> reqwest::Client {
        reqwest::Client::builder().no_proxy().build().unwrap()
    }

    /// 只答一次的服务器；`/chat/completions` 等路径由被测代码自己拼
    pub(crate) fn serve_status(
        status: &'static str,
        content_type: &'static str,
        body: Vec<u8>,
    ) -> TestServer {
        let mut server =
            serve_once(move |stream, _, _| write_response(stream, status, content_type, &body));
        server.url.push_str("/v1");
        server
    }

    /// 200 + 任意 JSON 响应体
    pub(crate) fn serve_json(body: serde_json::Value) -> TestServer {
        serve_status("200 OK", "application/json", body.to_string().into_bytes())
    }

    /// 200 + 单个 choice：`{message: {content}, finish_reason}`，content 为 None 时发 null
    pub(crate) fn serve_chat_reply(content: Option<&str>, finish_reason: &str) -> TestServer {
        serve_json(serde_json::json!({
            "choices": [{
                "message": {"role": "assistant", "content": content},
                "finish_reason": finish_reason
            }]
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::*;
    use super::*;
    use serde_json::json;

    const WAIT: Duration = Duration::from_secs(5);
    static NOT_CANCELLED: AtomicBool = AtomicBool::new(false);

    fn params(endpoint: &str) -> ChatParams<'_> {
        ChatParams {
            endpoint,
            api_key: "sk-test",
            model: "mock-model",
            temperature: 0.3,
            max_tokens: -1,
            top_p: 0.0,
        }
    }

    async fn ask(endpoint: &str) -> Result<ChatReply, ChatError> {
        chat_completion(
            &client(),
            &params(endpoint),
            &[ChatMessage::user("hello")],
            &RequestThrottle::new(-1),
            &NOT_CANCELLED,
        )
        .await
    }

    #[tokio::test]
    async fn posts_to_chat_completions_with_bearer_and_minimal_body() {
        let server = serve_chat_reply(Some("  1girl, solo  \n"), "stop");
        let reply = chat_completion(
            &client(),
            &params(&server.url),
            &[ChatMessage::system("sys"), ChatMessage::user("hello")],
            &RequestThrottle::new(-1),
            &NOT_CANCELLED,
        )
        .await
        .unwrap();
        assert_eq!(
            reply,
            ChatReply {
                text: "1girl, solo".into(),
                finish_reason: Some("stop".into())
            }
        );
        assert!(!reply.is_truncated());

        let req = server.requests.recv_timeout(WAIT).unwrap();
        assert_eq!(req.method, "POST");
        assert_eq!(req.path, "/v1/chat/completions");
        assert_eq!(req.header("authorization"), Some("Bearer sk-test"));
        assert_eq!(req.header("content-type"), Some("application/json"));
        // max_tokens <= 0、top_p 不在 (0, 1] 时不发送
        assert_eq!(
            req.json(),
            json!({
                "model": "mock-model",
                "messages": [
                    {"role": "system", "content": "sys"},
                    {"role": "user", "content": "hello"}
                ],
                "temperature": 0.3
            })
        );
    }

    #[tokio::test]
    async fn sends_limits_when_set_and_no_auth_without_key() {
        let server = serve_chat_reply(Some("ok"), "stop");
        let p = ChatParams {
            api_key: "",
            max_tokens: 512,
            top_p: 0.9,
            ..params(&server.url)
        };
        chat_completion(
            &client(),
            &p,
            &[ChatMessage::user("hi")],
            &RequestThrottle::new(-1),
            &NOT_CANCELLED,
        )
        .await
        .unwrap();
        let req = server.requests.recv_timeout(WAIT).unwrap();
        assert_eq!(req.header("authorization"), None);
        assert_eq!(req.json()["max_tokens"], 512);
        assert_eq!(req.json()["top_p"], 0.9);
    }

    #[tokio::test]
    async fn falls_back_to_reasoning_content() {
        let server = serve_json(json!({"choices": [{
            "message": {"content": "  ", "reasoning_content": " thought answer "},
            "finish_reason": "stop"
        }]}));
        assert_eq!(ask(&server.url).await.unwrap().text, "thought answer");
    }

    #[tokio::test]
    async fn content_filter_and_safety_are_errors_even_with_text() {
        for reason in ["content_filter", "safety"] {
            let server = serve_chat_reply(Some("partial text"), reason);
            assert_eq!(
                ask(&server.url).await,
                Err(ChatError::ContentFilter),
                "{reason}"
            );
        }
        assert_eq!(
            String::from(ChatError::ContentFilter),
            "LLM 内容安全审核拒绝了该请求"
        );
    }

    /// 截断不是错误，交给调用方判断
    #[tokio::test]
    async fn truncated_reply_is_returned_for_the_caller_to_judge() {
        let server = serve_chat_reply(Some("1girl, so"), "length");
        let reply = ask(&server.url).await.unwrap();
        assert!(reply.is_truncated());
        assert_eq!(reply.text, "1girl, so");
    }

    #[tokio::test]
    async fn empty_reply_is_an_error() {
        let server = serve_chat_reply(None, "stop");
        assert_eq!(
            ask(&server.url).await,
            Err(ChatError::Other("API 返回空内容".into()))
        );
        let server = serve_chat_reply(Some(""), "length");
        assert_eq!(
            ask(&server.url).await,
            Err(ChatError::Other(
                "API 返回空内容（输出达到长度上限被截断）".into()
            ))
        );
    }

    #[tokio::test]
    async fn http_and_protocol_errors_are_reported() {
        let server = serve_status("500 Internal Server Error", "text/plain", b"boom".to_vec());
        assert_eq!(
            ask(&server.url).await,
            Err(ChatError::Other(
                "API 错误 (500 Internal Server Error): boom".into()
            ))
        );

        let server = serve_json(json!({"choices": []}));
        assert_eq!(
            ask(&server.url).await,
            Err(ChatError::Other("API 未返回任何结果".into()))
        );

        let server = serve_status("200 OK", "application/json", b"not json".to_vec());
        match ask(&server.url).await {
            Err(ChatError::Other(m)) => assert!(m.starts_with("解析响应失败"), "{m}"),
            other => panic!("应解析失败: {other:?}"),
        }
    }

    #[tokio::test]
    async fn cancel_during_throttle_skips_the_request() {
        let throttle = RequestThrottle::new(60_000);
        let cancel = AtomicBool::new(false);
        assert!(throttle.wait(&cancel).await, "第一次请求立即放行");
        cancel.store(true, Ordering::SeqCst);
        let start = Instant::now();
        // 端口 9 不会有人应答：取消后根本不该发请求
        let result = chat_completion(
            &client(),
            &params("http://127.0.0.1:9/v1"),
            &[ChatMessage::user("hi")],
            &throttle,
            &cancel,
        )
        .await;
        assert_eq!(result, Err(ChatError::Cancelled));
        assert_eq!(String::from(ChatError::Cancelled), "已取消");
        assert!(start.elapsed() < Duration::from_secs(2));
    }

    #[tokio::test]
    async fn throttle_spaces_request_starts() {
        let cancel = AtomicBool::new(false);
        let throttle = RequestThrottle::new(150);
        let start = Instant::now();
        assert!(throttle.wait(&cancel).await);
        assert!(start.elapsed() < Duration::from_millis(100), "第一次不等");
        assert!(throttle.wait(&cancel).await);
        assert!(
            start.elapsed() >= Duration::from_millis(150),
            "第二次至少隔一个间隔"
        );

        let unthrottled = RequestThrottle::new(0);
        let start = Instant::now();
        for _ in 0..3 {
            assert!(unthrottled.wait(&cancel).await);
        }
        assert!(start.elapsed() < Duration::from_millis(100));
    }

    #[tokio::test]
    async fn list_models_returns_sorted_ids() {
        let server = serve_json(json!({"data": [{"id": "b-model"}, {"id": "a-model"}]}));
        let ids = list_models(&client(), &server.url, "sk-test")
            .await
            .unwrap();
        assert_eq!(ids, vec!["a-model", "b-model"]);
        let req = server.requests.recv_timeout(WAIT).unwrap();
        assert_eq!(
            (req.method.as_str(), req.path.as_str()),
            ("GET", "/v1/models")
        );
        assert_eq!(req.header("authorization"), Some("Bearer sk-test"));
    }

    #[test]
    fn api_url_joins_with_one_slash() {
        assert_eq!(api_url("https://x/v1", "models"), "https://x/v1/models");
        assert_eq!(
            api_url("https://x/v1/", "chat/completions"),
            "https://x/v1/chat/completions"
        );
    }

    fn decode_data_url(url: &str) -> image::DynamicImage {
        let b64 = url
            .strip_prefix("data:image/jpeg;base64,")
            .expect("应是 JPEG data URL");
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(b64)
            .unwrap();
        assert_eq!(
            image::guess_format(&bytes).unwrap(),
            image::ImageFormat::Jpeg
        );
        image::load_from_memory(&bytes).unwrap()
    }

    /// 扩展名写错的透明 PNG：按内容识别、等比缩小、透明区拍成白色
    #[tokio::test]
    async fn image_data_url_sniffs_format_resizes_and_flattens() {
        let dir = TempDir::new("llm_image");
        let path = dir.join("actually_png.jpg");
        image::RgbaImage::from_fn(64, 32, |x, _| {
            if x < 32 {
                image::Rgba([0, 0, 0, 0])
            } else {
                image::Rgba([200, 30, 30, 255])
            }
        })
        .save_with_format(&path, image::ImageFormat::Png)
        .unwrap();

        let img = decode_data_url(&image_data_url(&path, 16).unwrap());
        assert_eq!((img.width(), img.height()), (16, 8));
        let corner = img.to_rgb8().get_pixel(0, 0).0;
        assert!(
            corner.iter().all(|&c| c > 240),
            "透明区应是白色: {corner:?}"
        );

        // 0 按缺省最长边，小图不放大；异步版结果一致
        let small = decode_data_url(&load_image_data_url(&path, 0).await.unwrap());
        assert_eq!((small.width(), small.height()), (64, 32));
    }

    #[test]
    fn image_data_url_reports_unreadable_files() {
        let dir = TempDir::new("llm_bad_image");
        let missing = image_data_url(&dir.join("missing.png"), 512).unwrap_err();
        assert!(missing.starts_with("读取图片失败"), "{missing}");
        let garbage = dir.join("garbage.png");
        std::fs::write(&garbage, b"definitely not an image").unwrap();
        let err = image_data_url(&garbage, 512).unwrap_err();
        assert!(err.starts_with("无法解码图片"), "{err}");
    }

    #[test]
    fn vision_content_omits_blank_detail() {
        assert_eq!(
            vision_user_content("describe", "data:image/jpeg;base64,AA", " high "),
            json!([
                {"type": "text", "text": "describe"},
                {"type": "image_url", "image_url": {"url": "data:image/jpeg;base64,AA", "detail": "high"}}
            ])
        );
        let content = vision_user_content("describe", "data:x", "  ");
        assert!(content[1]["image_url"].get("detail").is_none());
        assert_eq!(
            ChatMessage::user(content.clone()),
            ChatMessage {
                role: "user",
                content
            }
        );
    }

    #[test]
    fn refusal_is_rejected_with_subject_and_excerpt() {
        assert_eq!(
            reject_refusal("  I'm sorry, I can't help with that.  ", "该图片"),
            Err(
                "LLM 拒绝处理该图片（疑似内容安全审核）: I'm sorry, I can't help with that.".into()
            )
        );
        let long = format!("抱歉{}", "很".repeat(100));
        let err = reject_refusal(&long, "该标签文件").unwrap_err();
        assert!(err.starts_with("LLM 拒绝处理该标签文件（疑似内容安全审核）: 抱歉"));
        assert_eq!(err.split(": ").nth(1).unwrap().chars().count(), 80);
        // 逗号多的是标签列表，含拒绝措辞也放行
        assert!(reject_refusal("1girl, solo, i can't believe it, smile", "该图片").is_ok());
    }

    /// 精修、排序、LLM 打标共用的日志片段格式
    #[test]
    fn log_helpers_keep_message_format() {
        assert_eq!(fmt_elapsed(999), "999ms");
        assert_eq!(fmt_elapsed(1500), "1.5s");
        assert_eq!(summarize_tags("移除", &[]), None);
        assert_eq!(
            summarize_tags("新增", &["a", "b"]).as_deref(),
            Some("新增: a, b")
        );
        assert_eq!(
            summarize_tags("缺失", &["a", "b", "c", "d", "e", "f"]).as_deref(),
            Some("缺失: a, b, c, d, e等6个")
        );
    }

    #[test]
    fn pick_tag_line_takes_longest_comma_line() {
        assert_eq!(pick_tag_line("1girl, solo"), "1girl, solo");
        assert_eq!(
            pick_tag_line("Here you go:\n1girl, solo\n1girl, solo, smile\nDone."),
            "1girl, solo, smile"
        );
        assert_eq!(pick_tag_line("no\ncommas"), "no\ncommas");
    }
}
