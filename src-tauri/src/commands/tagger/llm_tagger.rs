use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use super::{ProcessResult, ProgressEvent};
use crate::commands::llm_batch::{self, ItemOutcome};
use crate::commands::llm_client::{self, ChatMessage, ChatParams, RequestThrottle};
use crate::commands::{collect_image_files_with_recursive, file_name_lossy, ProblemArchive};

const EVENT: &str = "llm-tagger-progress";

static LLM_CANCELLED: AtomicBool = AtomicBool::new(false);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmTaggerOptions {
    pub input_path: String,
    pub api_endpoint: String,
    pub api_key: String,
    pub model_name: String,
    pub system_prompt: String,
    pub user_prompt: String,
    pub temperature: f32,
    pub max_tokens: i32,
    /// 发送图片的最大边长（默认 1024，超出会等比缩放）
    #[serde(default = "default_image_size")]
    pub image_size: u32,
    /// Top P 采样参数（0~1，为 0 或负数时不发送）
    #[serde(default)]
    pub top_p: f64,
    /// 是否跳过已有 .txt/.json 描述文件的图片
    #[serde(default)]
    pub skip_existing: bool,
    /// 输出格式: "txt" 或 "json"
    #[serde(default = "default_llm_output_format")]
    pub output_format: String,
    #[serde(default)]
    pub json_simplified: bool,
    /// 请求间隔 (毫秒), -1 表示无间隔
    #[serde(default = "default_interval")]
    pub request_interval_ms: i64,
    /// 并发线程数
    #[serde(default = "default_concurrency")]
    pub concurrency: u32,
    /// 是否递归扫描子文件夹
    #[serde(default)]
    pub recursive: bool,
    /// OpenAI Vision 的 image_url.detail：low / high / original / auto。
    /// 空则整个字段不发送——不认识它的端点会因未知字段报错
    #[serde(default)]
    pub image_detail: String,
}

fn default_image_size() -> u32 {
    1024
}
fn default_llm_output_format() -> String {
    "txt".into()
}
fn default_interval() -> i64 {
    -1
}
fn default_concurrency() -> u32 {
    1
}

#[tauri::command]
pub fn cancel_llm_tagging() {
    LLM_CANCELLED.store(true, Ordering::SeqCst);
}

#[tauri::command]
pub async fn start_llm_tagging(
    app: tauri::AppHandle,
    options: LlmTaggerOptions,
) -> Result<ProcessResult, String> {
    static LLM_RUNNING: AtomicBool = AtomicBool::new(false);
    let _busy = crate::commands::BusyGuard::acquire(&LLM_RUNNING, "LLM 打标")?;
    LLM_CANCELLED.store(false, Ordering::SeqCst);
    crate::commands::begin_run(EVENT);
    let client = llm_client::llm_http_client()?;
    run_llm_tagging(&app, options, client).await
}

async fn run_llm_tagging<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    options: LlmTaggerOptions,
    client: reqwest::Client,
) -> Result<ProcessResult, String> {
    let input_dir = PathBuf::from(&options.input_path);
    let files = collect_image_files_with_recursive(&input_dir, options.recursive)?;
    let total = files.len() as u32;
    ProgressEvent::new("info", format!("读取到 {} 张图片", total))
        .at(0, total)
        .emit(app, EVENT);

    // 开始前一次定下要跳过的图片：本轮写出的标签不影响其他图片的判断
    let skipped: Arc<HashSet<PathBuf>> = Arc::new(if options.skip_existing {
        files
            .iter()
            .filter(|path| super::has_labels(path))
            .cloned()
            .collect()
    } else {
        HashSet::new()
    });
    let recursive = options.recursive;
    let concurrency = options.concurrency.max(1) as usize;
    let throttle = Arc::new(RequestThrottle::new(options.request_interval_ms));
    let options = Arc::new(options);
    let outcome = llm_batch::run_file_batch(
        app,
        EVENT,
        &files,
        concurrency,
        &LLM_CANCELLED,
        move |path| {
            let (client, options, throttle, skipped) = (
                client.clone(),
                options.clone(),
                throttle.clone(),
                skipped.clone(),
            );
            async move {
                if skipped.contains(&path) {
                    return ItemOutcome::Done {
                        message: format!("[跳过] {} (已有描述)", file_name_lossy(&path)),
                        warning: false,
                    };
                }
                tag_file(&client, &path, &options, &throttle).await
            }
        },
    )
    .await;
    Ok(outcome.finish(
        app,
        EVENT,
        "LLM 打标完成",
        &ProblemArchive::new(&input_dir, &input_dir, recursive),
    ))
}

/// 一张图：请求 VLM、整理成标签文件写到图片旁
async fn tag_file(
    client: &reqwest::Client,
    path: &Path,
    options: &LlmTaggerOptions,
    throttle: &RequestThrottle,
) -> ItemOutcome {
    let filename = file_name_lossy(path);
    let start = std::time::Instant::now();
    let written = async {
        let text = tag_with_llm(client, path, options, throttle).await?;
        let content = format_output(&text, options)?;
        // 回复到达前用户点了取消：不再写盘
        if LLM_CANCELLED.load(Ordering::SeqCst) {
            return Err("已取消".to_string());
        }
        let output = path.with_extension(if options.output_format == "json" {
            "json"
        } else {
            "txt"
        });
        crate::commands::config_paths::write_file_atomic(&output, content.as_bytes())
            .map_err(|e| format!("写入失败 {}", e))
    }
    .await;
    let elapsed = llm_client::fmt_elapsed(start.elapsed().as_millis());
    match written {
        Ok(()) => ItemOutcome::Done {
            message: format!("[完成] {} ({})", filename, elapsed),
            warning: false,
        },
        Err(message) => ItemOutcome::Failed {
            filename,
            message: format!("{} ({})", message, elapsed),
        },
    }
}

/// VLM 回复 → 标签文件内容。txt 原样写；JSON 统一规范成恒定骨架（`tag_manager::normalize_tag_json`）：
/// 回复是 JSON 对象（可带 ``` 围栏）时补齐缺的段、画师逐位补 `@`、保留额外字段；
/// 不是 JSON 对象（自然语言、标签串、JSON 数组等）时原文整段放进 nl
fn format_output(text: &str, options: &LlmTaggerOptions) -> Result<String, String> {
    if options.output_format != "json" {
        return Ok(text.to_string());
    }
    let trimmed = text.trim();
    let cleaned = trimmed
        .strip_prefix("```json")
        .or_else(|| trimmed.strip_prefix("```"))
        .map(|s| s.trim_end().trim_end_matches("```"))
        .unwrap_or(trimmed)
        .trim();
    let value = match serde_json::from_str::<serde_json::Value>(cleaned) {
        Ok(value @ serde_json::Value::Object(_)) => value,
        _ => serde_json::Value::String(text.to_string()),
    };
    let normalized =
        crate::commands::tag_manager::normalize_tag_json(value, options.json_simplified);
    serde_json::to_string_pretty(&normalized).map_err(|e| format!("序列化标签失败: {}", e))
}

async fn tag_with_llm(
    client: &reqwest::Client,
    img_path: &Path,
    options: &LlmTaggerOptions,
    throttle: &RequestThrottle,
) -> Result<String, String> {
    let data_url = llm_client::load_image_data_url(img_path, options.image_size).await?;
    let params = ChatParams {
        endpoint: &options.api_endpoint,
        api_key: &options.api_key,
        model: &options.model_name,
        temperature: options.temperature,
        max_tokens: options.max_tokens,
        top_p: options.top_p,
    };
    let messages = [
        ChatMessage::system(&options.system_prompt),
        ChatMessage::user(llm_client::vision_user_content(
            &options.user_prompt,
            &data_url,
            &options.image_detail,
        )),
    ];
    let reply =
        llm_client::chat_completion(client, &params, &messages, throttle, &LLM_CANCELLED).await;
    llm_client::accept_reply(&reply, "该图片").map(str::to_owned)
}

#[tauri::command]
pub async fn fetch_llm_models(
    api_endpoint: String,
    api_key: String,
) -> Result<Vec<String>, String> {
    llm_client::list_models(&llm_client::llm_http_client()?, &api_endpoint, &api_key).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::batch::capture_events;
    use crate::commands::llm_client::test_support::{client, serve_chat_reply};
    use crate::commands::test_support::TempDir;
    use serde_json::json;

    fn options() -> LlmTaggerOptions {
        serde_json::from_value(json!({
            "input_path": "", "api_endpoint": "", "api_key": "", "model_name": "mock",
            "system_prompt": "describe", "user_prompt": "image", "temperature": 0.5,
            "max_tokens": 20, "output_format": "json",
        }))
        .unwrap()
    }

    fn parse(text: &str, opts: &LlmTaggerOptions) -> serde_json::Value {
        serde_json::from_str(&format_output(text, opts).unwrap()).unwrap()
    }

    #[test]
    fn non_json_responses_keep_the_full_schema() {
        let mut opts = options();
        let full = parse("description", &opts);
        assert_eq!(
            full,
            json!({
                "fixed": {"quality": "", "series": "", "artist": ""},
                "character": {"name": "", "variant": ""},
                "from_path": {"appearance": []},
                "ai_output": {"count": "", "appearance": [], "tags": [], "environment": [], "nl": "description"},
            })
        );
        opts.json_simplified = true;
        let simple = parse("description", &opts);
        assert_eq!(simple.as_object().unwrap().len(), 9);
        assert_eq!(simple["nl"], "description");
        assert_eq!(simple["tags"], json!([]));
        assert_eq!(simple["artist"], "");
    }

    #[test]
    fn opening_only_fences_do_not_discard_json() {
        for text in [
            "```json\n{\"tags\": [\"solo\"]}\n```",
            "```json\n{\"tags\": [\"solo\"]}",
            "```\n{\"tags\": [\"solo\"]}",
        ] {
            let parsed = parse(text, &options());
            assert_eq!(parsed["ai_output"]["tags"], json!(["solo"]), "{text}");
            assert_eq!(parsed["ai_output"]["nl"], "", "{text}");
            assert_eq!(
                parsed.as_object().unwrap().keys().collect::<Vec<_>>(),
                ["fixed", "character", "from_path", "ai_output"]
            );
        }
    }

    /// JSON 回复也补齐骨架：默认完整格式提示词只要求 ai_output 一段；画师逐位补 @，额外字段保留
    #[test]
    fn json_replies_are_normalized_to_the_complete_schema() {
        let mut opts = options();
        let reply = r#"{"ai_output": {"count": "1girl", "tags": "smile, standing", "nl": "A girl."},
                        "fixed": {"artist": "foo, @bar"}, "rating": "safe"}"#;
        let full = parse(reply, &opts);
        assert_eq!(
            full["fixed"],
            json!({"quality": "", "series": "", "artist": "@foo, @bar"})
        );
        assert_eq!(full["character"], json!({"name": "", "variant": ""}));
        assert_eq!(full["from_path"], json!({"appearance": []}));
        assert_eq!(full["ai_output"]["tags"], json!(["smile", "standing"]));
        assert_eq!(full["ai_output"]["nl"], "A girl.");
        assert_eq!(full["rating"], "safe");

        opts.json_simplified = true;
        let simple = parse(
            r#"{"artist": ["foo", "bar"], "tags": ["solo"], "mood": "calm"}"#,
            &opts,
        );
        assert_eq!(simple["artist"], "@foo, @bar");
        assert_eq!(simple["tags"], json!(["solo"]));
        assert_eq!(simple["mood"], "calm");
        assert_eq!(simple["character"], "");
    }

    /// 合法 JSON 但不是对象：原文整段放进 nl，不丢内容
    #[test]
    fn json_that_is_not_an_object_goes_into_nl_verbatim() {
        for text in ["[\"solo\", \"smile\"]", "42", "\"just a string\"", "null"] {
            let parsed = parse(text, &options());
            assert_eq!(parsed["ai_output"]["nl"], text, "{text}");
            assert_eq!(parsed["ai_output"]["tags"], json!([]), "{text}");
        }
    }

    #[test]
    fn txt_output_is_written_verbatim() {
        let mut opts = options();
        opts.output_format = "txt".into();
        assert_eq!(
            format_output("{\"tags\": []}", &opts).unwrap(),
            "{\"tags\": []}"
        );
    }

    /// 「已有标签」按文件存在且非空判断，txt 与 json 各自判断，不读内容
    #[test]
    fn existing_labels_are_judged_by_size_per_format() {
        use crate::commands::tagger::has_labels;
        let root = TempDir::new("llm_tagger_existing");
        let image = root.join("a.png");
        assert!(!has_labels(&image));
        std::fs::write(
            image.with_extension("txt"),
            b"\xd2\xbb\xb8\xf6\xc5\xae\xba\xa2",
        )
        .unwrap();
        assert!(has_labels(&image), "GBK 编码的旧 txt 也算已有标签");
        std::fs::write(image.with_extension("txt"), "").unwrap();
        std::fs::write(image.with_extension("json"), " \n").unwrap();
        assert!(has_labels(&image));
        std::fs::write(image.with_extension("json"), "").unwrap();
        assert!(!has_labels(&image));
        std::fs::write(image.with_extension("txt"), "solo").unwrap();
        assert!(has_labels(&image), "空 json 不挡住对 txt 的判断");
    }

    #[tokio::test]
    async fn rejects_truncated_filtered_and_refused_responses() {
        let temp = TempDir::new("llm_tagger_responses");
        let path = temp.join("image.wrong_extension");
        image::RgbImage::new(2, 2)
            .save_with_format(&path, image::ImageFormat::Png)
            .unwrap();
        for (text, finish, expected) in [
            ("partial response", "length", "max_tokens 上限（20）被截断"),
            ("", "content_filter", "LLM 内容安全审核拒绝了该图片"),
            (
                "I cannot assist with this request",
                "stop",
                "拒绝处理该图片",
            ),
        ] {
            let server = serve_chat_reply(Some(text), finish);
            let mut opts = options();
            opts.api_endpoint = server.url.clone();
            let error = tag_with_llm(&client(), &path, &opts, &RequestThrottle::new(-1))
                .await
                .unwrap_err();
            assert!(error.contains(expected), "{error}");
            assert!(!path.with_extension("json").exists());
        }
        let server = serve_chat_reply(Some("a complete description"), "stop");
        let mut opts = options();
        opts.api_endpoint = server.url.clone();
        assert_eq!(
            tag_with_llm(&client(), &path, &opts, &RequestThrottle::new(-1))
                .await
                .unwrap(),
            "a complete description"
        );
    }

    /// 一轮：跳过已有标签的、成功的写出规范化 JSON、失败的进 Fail/，终态 done 只有一条
    #[tokio::test]
    async fn run_skips_labelled_images_writes_json_and_archives_failures() {
        LLM_CANCELLED.store(false, Ordering::SeqCst);
        let root = TempDir::new("llm_tagger_run");
        let good = root.join("a_good.png");
        image::RgbImage::new(2, 2).save(&good).unwrap();
        let skipped = root.join("b_skipped.png");
        image::RgbImage::new(2, 2).save(&skipped).unwrap();
        std::fs::write(skipped.with_extension("txt"), b"\xc4\xe3\xba\xc3").unwrap();
        let broken = root.join("c_broken.png");
        std::fs::write(&broken, "not an image").unwrap();
        let empty_label = root.join("d_empty.png");
        std::fs::write(&empty_label, "not an image").unwrap();
        std::fs::write(empty_label.with_extension("txt"), "").unwrap();

        let server = serve_chat_reply(Some(r#"{"ai_output": {"tags": ["solo"]}}"#), "stop");
        let mut opts = options();
        opts.input_path = root.to_string_lossy().into_owned();
        opts.api_endpoint = server.url.clone();
        opts.skip_existing = true;
        let app = tauri::test::mock_app();
        let events = capture_events(app.handle(), EVENT);
        let result = run_llm_tagging(app.handle(), opts, client()).await.unwrap();

        assert_eq!(
            (result.success_count, result.fail_count, result.total),
            (2, 2, 4)
        );
        let written: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(good.with_extension("json")).unwrap())
                .unwrap();
        assert_eq!(written["ai_output"]["tags"], json!(["solo"]));
        assert_eq!(written["fixed"]["artist"], "");
        assert!(!skipped.with_extension("json").exists());
        assert!(root.join("Fail/c_broken.png").exists());
        assert!(
            root.join("Fail/d_empty.png").exists(),
            "空 txt 不算已有标签"
        );
        assert!(!root.join("Fail/a_good.png").exists());
        let events = events.lock().unwrap();
        let done: Vec<_> = events.iter().filter(|e| e["status"] == "done").collect();
        assert_eq!(done.len(), 1);
        assert_eq!(done[0]["message"], "LLM 打标完成: 成功 2, 失败 2, 共 4");
        assert!(events
            .iter()
            .any(|e| e["message"] == "[跳过] b_skipped.png (已有描述)"));
        assert!(events
            .iter()
            .filter(|e| e["status"] == "success" || e["status"] == "error")
            .all(|e| e["total"] == 4));
    }
}
