use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use super::llm_batch::{self, ItemOutcome};
use super::llm_client::{
    self, fmt_elapsed, pick_tag_line, summarize_tags, ChatMessage, ChatParams, RequestThrottle,
};
use super::tag_text::{join_tags, split_tags};
use super::{ProblemArchive, ProcessResult, ProgressEvent};

const EVENT: &str = "tag-sort-progress";

static TAG_SORT_CANCELLED: AtomicBool = AtomicBool::new(false);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TagSortOptions {
    pub input_path: String,
    pub output_path: String,
    pub api_endpoint: String,
    pub api_key: String,
    pub model_name: String,
    pub prompt: String,
    pub temperature: f32,
    /// 请求间隔（毫秒），<= 0 表示无间隔
    pub request_interval_ms: i64,
    /// 并发线程数，<= 0 或 1 表示单线程
    pub concurrency: u32,
    /// Top P 采样参数（0~1，为 0 或负数时不发送）
    #[serde(default)]
    pub top_p: f64,
}

#[tauri::command]
pub fn cancel_tag_sorting() {
    TAG_SORT_CANCELLED.store(true, Ordering::SeqCst);
}

/// 收集目录中的 .txt 标签文件
fn collect_txt_files(dir: &Path) -> Result<Vec<PathBuf>, String> {
    if !dir.is_dir() {
        return Err(format!("目录不存在: {}", dir.display()));
    }
    let mut files = Vec::new();
    for entry in walkdir::WalkDir::new(dir)
        .max_depth(1)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        let p = entry.path();
        if p.is_file() {
            if let Some(ext) = p.extension() {
                if ext.to_string_lossy().to_lowercase() == "txt" {
                    files.push(p.to_path_buf());
                }
            }
        }
    }
    files.sort_by(|a, b| a.file_name().cmp(&b.file_name()));
    Ok(files)
}

#[derive(Debug)]
enum FileResult {
    Success {
        filename: String,
        original_count: usize,
        sorted_count: usize,
        changed: bool,
        warnings: Vec<String>,
        elapsed_ms: u128,
    },
    Skipped {
        filename: String,
    },
    Error {
        filename: String,
        message: String,
    },
}

#[tauri::command]
pub async fn start_tag_sorting(
    app: tauri::AppHandle,
    options: TagSortOptions,
) -> Result<ProcessResult, String> {
    // 互斥：全局取消标志不允许并发运行，后启动的任务会把前一个的取消标志复位
    static SORT_RUNNING: AtomicBool = AtomicBool::new(false);
    let _busy = super::BusyGuard::acquire(&SORT_RUNNING, "标签排序")?;

    TAG_SORT_CANCELLED.store(false, Ordering::SeqCst);
    let client = llm_client::llm_http_client()?;
    sort_dataset(&app, options, client).await
}

/// 排序输入目录里的全部标签文件：开始新一轮运行，逐个请求 LLM 写出结果，
/// 收尾时归集问题文件并发唯一的终态 done
async fn sort_dataset<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    options: TagSortOptions,
    client: reqwest::Client,
) -> Result<ProcessResult, String> {
    super::begin_run(EVENT);
    let input_dir = PathBuf::from(&options.input_path);
    let output_dir = PathBuf::from(&options.output_path);

    let files = collect_txt_files(&input_dir)?;
    let total = files.len() as u32;
    if total == 0 {
        return Err("输入目录中没有找到 .txt 标签文件".to_string());
    }
    std::fs::create_dir_all(&output_dir).map_err(|e| format!("创建输出目录失败: {}", e))?;

    let concurrency = std::cmp::max(1, options.concurrency) as usize;
    ProgressEvent::new(
        "info",
        format!("找到 {} 个标签文件，{} 线程开始排序...", total, concurrency),
    )
    .at(0, total)
    .emit(app, EVENT);

    let throttle = Arc::new(RequestThrottle::new(options.request_interval_ms));
    let work_output_dir = output_dir.clone();
    let outcome = llm_batch::run_file_batch(
        app,
        EVENT,
        &files,
        concurrency,
        &TAG_SORT_CANCELLED,
        move |file_path| {
            let (client, options, output_dir, throttle) = (
                client.clone(),
                options.clone(),
                work_output_dir.clone(),
                throttle.clone(),
            );
            async move {
                process_single_file(&client, &file_path, &output_dir, &options, &throttle)
                    .await
                    .into_outcome()
            }
        },
    )
    .await;
    Ok(outcome.finish(
        app,
        EVENT,
        "标签排序完成",
        &ProblemArchive::new(&input_dir, &output_dir, false),
    ))
}

impl FileResult {
    fn into_outcome(self) -> ItemOutcome {
        match self {
            Self::Success {
                filename,
                original_count,
                sorted_count,
                changed,
                warnings,
                elapsed_ms,
            } => ItemOutcome::completed(
                format!(
                    "[完成] {} | 原TAG数 {} → 排序后TAG数 {} | {}",
                    filename,
                    original_count,
                    sorted_count,
                    fmt_elapsed(elapsed_ms)
                ),
                &warnings,
                (!changed).then_some(" (顺序未变)"),
            ),
            Self::Skipped { filename } => ItemOutcome::Done {
                message: format!("[跳过] {} (空文件)", filename),
                warning: false,
            },
            Self::Error { filename, message } => ItemOutcome::Failed { filename, message },
        }
    }
}

/// `tags` 里不在 `other` 中的标签，按出现顺序去重
fn tags_absent_from<'a>(tags: &'a [String], other: &HashSet<&str>) -> Vec<&'a str> {
    let mut seen = HashSet::new();
    tags.iter()
        .map(String::as_str)
        .filter(|tag| !other.contains(tag) && seen.insert(*tag))
        .collect()
}

async fn process_single_file(
    client: &reqwest::Client,
    file_path: &Path,
    output_dir: &Path,
    options: &TagSortOptions,
    throttle: &RequestThrottle,
) -> FileResult {
    let start = std::time::Instant::now();
    let filename = super::file_name_lossy(file_path);

    let original_tags = match std::fs::read_to_string(file_path) {
        Ok(content) => split_tags(&content),
        Err(e) => {
            return FileResult::Error {
                filename,
                message: format!("读取失败: {}", e),
            }
        }
    };
    if original_tags.is_empty() {
        return FileResult::Skipped { filename };
    }

    let sorted_tags = match sort_tags_with_llm(client, &original_tags, options, throttle).await {
        Ok(tags) => tags,
        Err(message) => return FileResult::Error { filename, message },
    };
    let elapsed_ms = start.elapsed().as_millis();
    let original_set: HashSet<&str> = original_tags.iter().map(String::as_str).collect();
    let sorted_set: HashSet<&str> = sorted_tags.iter().map(String::as_str).collect();
    let missing = tags_absent_from(&original_tags, &sorted_set);
    let added = tags_absent_from(&sorted_tags, &original_set);

    // 排序只调整顺序：原标签缺了一半以上，回复多半是没认出来的拒绝语或答非所问，写盘会丢掉原标签
    if missing.len() * 2 > original_set.len() {
        let excerpt: String = sorted_tags.join(", ").chars().take(80).collect();
        return FileResult::Error {
            filename,
            message: format!(
                "排序结果缺少 {}/{} 个原标签，已丢弃: {}",
                missing.len(),
                original_set.len(),
                excerpt
            ),
        };
    }

    let mut warnings = Vec::new();
    if original_tags.len() != sorted_tags.len() {
        warnings.push(format!(
            "数量变化: {}→{}",
            original_tags.len(),
            sorted_tags.len()
        ));
    }
    warnings.extend(summarize_tags("缺失", &missing));
    warnings.extend(summarize_tags("新增", &added));

    // 就地排序时输出就是原文件：先写临时文件再替换，写入中途失败也不会截断原标签
    let output_path = output_dir.join(&filename);
    if let Err(e) =
        super::config_paths::write_file_atomic(&output_path, join_tags(&sorted_tags).as_bytes())
    {
        return FileResult::Error {
            filename,
            message: format!("写入失败: {}", e),
        };
    }
    FileResult::Success {
        filename,
        original_count: original_tags.len(),
        sorted_count: sorted_tags.len(),
        changed: sorted_tags != original_tags,
        warnings,
        elapsed_ms,
    }
}

async fn sort_tags_with_llm(
    client: &reqwest::Client,
    tags: &[String],
    options: &TagSortOptions,
    throttle: &RequestThrottle,
) -> Result<Vec<String>, String> {
    let tag_list = tags.join(", ");

    let user_content = if options.prompt.contains("{tags}") {
        options.prompt.replace("{tags}", &tag_list)
    } else {
        format!(
            "{}\n\n需要排序的tags: {}\n\n排序后的tags:",
            options.prompt, tag_list
        )
    };

    let params = ChatParams {
        endpoint: &options.api_endpoint,
        api_key: &options.api_key,
        model: &options.model_name,
        temperature: options.temperature,
        max_tokens: -1,
        top_p: options.top_p,
    };
    let reply = llm_client::chat_completion(
        client,
        &params,
        &[ChatMessage::user(user_content)],
        throttle,
        &TAG_SORT_CANCELLED,
    )
    .await;
    let sorted_tags = split_tags(pick_tag_line(llm_client::accept_reply(
        &reply,
        "该标签文件",
    )?));
    if sorted_tags.is_empty() {
        return Err("AI 返回的排序结果为空".to_string());
    }
    Ok(sorted_tags)
}

/// 端到端：本地 mock 服务器 + 真实标签文件，跑 process_single_file 全链路（读 → 请求 → 校验 → 写盘）
#[cfg(test)]
mod e2e_tests {
    use super::*;
    use crate::commands::batch::capture_raw_events;
    use crate::commands::llm_client::test_support::{client, serve_chat_reply};
    use crate::commands::test_support::TempDir;

    fn options(endpoint: String) -> TagSortOptions {
        TagSortOptions {
            input_path: String::new(),
            output_path: String::new(),
            api_endpoint: endpoint,
            api_key: "test-key".into(),
            model_name: "mock-llm".into(),
            prompt: "请排序: {tags}".into(),
            temperature: 0.2,
            request_interval_ms: -1,
            concurrency: 1,
            top_p: 0.0,
        }
    }

    #[test]
    fn old_max_tokens_is_ignored_and_not_serialized() {
        let mut old = serde_json::to_value(options("local".into())).unwrap();
        old["max_tokens"] = serde_json::json!(-1);
        let parsed: TagSortOptions = serde_json::from_value(old).unwrap();
        assert!(serde_json::to_value(parsed)
            .unwrap()
            .get("max_tokens")
            .is_none());
    }

    #[test]
    fn outcome_messages_keep_existing_completion_and_skip_text() {
        let result = FileResult::Success {
            filename: "a b.txt".into(),
            original_count: 3,
            sorted_count: 3,
            changed: false,
            warnings: Vec::new(),
            elapsed_ms: 200,
        }
        .into_outcome();
        match result {
            ItemOutcome::Done { message, warning } => {
                assert_eq!(
                    message,
                    "[完成] a b.txt | 原TAG数 3 → 排序后TAG数 3 | 200ms (顺序未变)"
                );
                assert!(!warning);
            }
            _ => panic!("success expected"),
        }
        match (FileResult::Skipped {
            filename: "empty.txt".into(),
        })
        .into_outcome()
        {
            ItemOutcome::Done { message, warning } => {
                assert_eq!(message, "[跳过] empty.txt (空文件)");
                assert!(!warning);
            }
            _ => panic!("skip expected"),
        }
    }

    #[tokio::test]
    async fn changed_set_is_successful_with_warning() {
        let (result, written) = run(Some("1girl, smile, standing"), "stop", "warning").await;
        assert_eq!(written.as_deref(), Some("1girl, smile, standing"));
        match result.into_outcome() {
            ItemOutcome::Done { message, warning } => {
                assert!(warning);
                assert!(message.contains("缺失: solo"), "{message}");
                assert!(message.contains("新增: standing"), "{message}");
            }
            _ => panic!("successful warning expected"),
        }
    }

    /// 让 mock 回复 content / finish_reason，跑一遍单文件处理；
    /// 返回处理结果和写出的标签文件内容（没写出时为 None）
    async fn run(
        content: Option<&str>,
        finish_reason: &str,
        tag: &str,
    ) -> (FileResult, Option<String>) {
        let server = serve_chat_reply(content, finish_reason);
        let dir = TempDir::new(&format!("sort_e2e_{}", tag));
        let input = dir.join("a.txt");
        std::fs::write(&input, "smile, 1girl, solo").unwrap();
        let out = dir.join("out");
        std::fs::create_dir_all(&out).unwrap();

        let throttle = RequestThrottle::new(-1);
        let result = process_single_file(
            &client(),
            &input,
            &out,
            &options(server.url.clone()),
            &throttle,
        )
        .await;
        let sent = server
            .requests
            .recv_timeout(std::time::Duration::from_secs(5))
            .expect("mock 服务器应收到请求");
        assert_eq!(sent.path, "/v1/chat/completions");
        assert!(sent.json().get("max_tokens").is_none());
        assert_eq!(
            sent.json()["messages"][0]["content"],
            "请排序: smile, 1girl, solo"
        );
        (result, std::fs::read_to_string(out.join("a.txt")).ok())
    }

    #[tokio::test]
    async fn sorted_reply_is_written() {
        let (result, written) = run(Some("Sorted:\n1girl, solo, smile"), "stop", "ok").await;
        assert!(
            matches!(result, FileResult::Success { changed: true, .. }),
            "{result:?}"
        );
        assert_eq!(written.as_deref(), Some("1girl, solo, smile"));
    }

    /// 被截断的回复是残缺的标签列表，不能写盘
    #[tokio::test]
    async fn truncated_reply_is_discarded() {
        let (result, written) = run(Some("1girl, so"), "length", "truncated").await;
        match &result {
            FileResult::Error { message, .. } => {
                assert_eq!(message, "回复超出了服务商的输出长度上限，已丢弃");
            }
            other => panic!("截断应判失败: {other:?}"),
        }
        assert_eq!(written, None, "截断时不应写出文件");
    }

    /// 拒绝语不能被拆成标签写盘
    #[tokio::test]
    async fn refusal_is_not_written_as_tags() {
        let (result, written) = run(
            Some("I'm sorry, I can't help with that."),
            "stop",
            "refusal",
        )
        .await;
        match &result {
            FileResult::Error { message, .. } => {
                assert!(message.starts_with("LLM 拒绝处理该标签文件"), "{message}")
            }
            other => panic!("拒绝语应判失败: {other:?}"),
        }
        assert_eq!(written, None, "拒绝时不应写出文件");
    }

    /// 排序是重排：缺了一半以上的原标签就判失败不写盘，正好一半仍按警告写出
    #[tokio::test]
    async fn more_than_half_missing_is_discarded() {
        for (reply, written_ok) in [
            // 3 个原标签缺 2 个
            ("1girl", false),
            // 换成了别的语言，原标签全缺
            ("微笑, 女孩, 单人", false),
            ("smile, 1girl", true),
        ] {
            let (result, written) = run(Some(reply), "stop", "half").await;
            if written_ok {
                assert_eq!(written.as_deref(), Some(reply));
                assert!(
                    matches!(result, FileResult::Success { .. }),
                    "{reply}: {result:?}"
                );
                continue;
            }
            match &result {
                FileResult::Error { message, .. } => {
                    assert!(message.starts_with("排序结果缺少"), "{message}");
                    assert!(message.ends_with(reply), "{message}");
                }
                other => panic!("{reply}: 应判失败 {other:?}"),
            }
            assert_eq!(written, None);
        }
    }

    /// 以前没认出来的拒绝语：就地排序时原文件一字不改，原样复制进 Fail/，计为失败
    #[tokio::test]
    async fn unrecognized_refusals_never_touch_the_original_file() {
        const ORIGINAL: &str = "smile, 1girl, solo";
        for refusal in [
            "I'm sorry, but I can't, as an AI model, sort these tags.",
            "I'm not able to help with sorting these tags.",
            "I apologize, but I won't be able to help with that request.",
        ] {
            let dir = TempDir::new("sort_refusal_in_place");
            std::fs::write(dir.join("a.txt"), ORIGINAL).unwrap();
            let server = serve_chat_reply(Some(refusal), "stop");
            let app = tauri::test::mock_app();
            let events = capture_raw_events(app.handle(), EVENT);
            let mut opts = options(server.url.clone());
            opts.input_path = dir.to_string_lossy().into_owned();
            opts.output_path = opts.input_path.clone();
            let before = crate::commands::begin_run("tag-sort-test-marker");
            let result = sort_dataset(app.handle(), opts, client()).await.unwrap();

            assert_eq!(
                (result.success_count, result.fail_count, result.total),
                (0, 1, 1),
                "{refusal}"
            );
            assert_eq!(
                std::fs::read_to_string(dir.join("a.txt")).unwrap(),
                ORIGINAL
            );
            assert_eq!(
                std::fs::read_to_string(dir.join("Fail/a.txt")).unwrap(),
                ORIGINAL
            );
            let events = events.lock().unwrap();
            let run_id = events[0]["run_id"].as_u64().unwrap();
            assert!(run_id > before);
            assert!(events.iter().all(|e| e["run_id"] == run_id), "{events:?}");
            let summary: Vec<(&str, &str)> = events
                .iter()
                .map(|e| {
                    (
                        e["status"].as_str().unwrap(),
                        e["message"].as_str().unwrap(),
                    )
                })
                .collect();
            assert_eq!(summary.len(), 4, "{summary:?}");
            assert_eq!(summary[0], ("info", "找到 1 个标签文件，1 线程开始排序..."));
            assert_eq!(summary[1].0, "error");
            assert!(
                summary[1]
                    .1
                    .starts_with("[错误] a.txt: 排序结果缺少 3/3 个原标签，已丢弃"),
                "{}",
                summary[1].1
            );
            assert_eq!(summary[2], ("info", "已将 1 个失败文件复制到 Fail/ 文件夹"));
            assert_eq!(summary[3], ("done", "标签排序完成: 成功 0, 失败 1, 共 1"));
        }
    }

    #[tokio::test]
    async fn content_filter_is_an_error() {
        let (result, written) = run(None, "content_filter", "filtered").await;
        match &result {
            FileResult::Error { message, .. } => {
                assert!(message.contains("内容安全审核"), "{message}")
            }
            other => panic!("审核拒绝应判失败: {other:?}"),
        }
        assert_eq!(written, None);
    }
}
