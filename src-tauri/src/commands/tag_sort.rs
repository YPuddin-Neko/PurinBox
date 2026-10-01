use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use super::llm_batch::{self, ItemOutcome};
use super::llm_client::{
    self, fmt_elapsed, pick_tag_line, reject_refusal, summarize_tags, ChatMessage, ChatParams,
    RequestThrottle,
};
use super::{ProcessResult, ProgressEvent};

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

    let input_dir_path = PathBuf::from(&options.input_path);
    let input_dir = input_dir_path.as_path();
    let output_dir_path = PathBuf::from(&options.output_path);

    let files = collect_txt_files(input_dir)?;
    let total = files.len() as u32;

    if total == 0 {
        return Err("输入目录中没有找到 .txt 标签文件".to_string());
    }

    std::fs::create_dir_all(&output_dir_path).map_err(|e| format!("创建输出目录失败: {}", e))?;

    let client = llm_client::llm_http_client()?;

    let concurrency = std::cmp::max(1, options.concurrency) as usize;

    ProgressEvent::new(
        "info",
        format!("找到 {} 个标签文件，{} 线程开始排序...", total, concurrency),
    )
    .at(0, total)
    .emit(&app, "tag-sort-progress");

    let throttle = Arc::new(RequestThrottle::new(options.request_interval_ms));
    let output_dir = output_dir_path.clone();
    let outcome = llm_batch::run_file_batch(
        &app,
        "tag-sort-progress",
        &files,
        concurrency,
        &TAG_SORT_CANCELLED,
        move |file_path| {
            let (client, options, output_dir, throttle) = (
                client.clone(),
                options.clone(),
                output_dir.clone(),
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
    let extra =
        llm_batch::archive_problem_files(input_dir, &output_dir_path, false, &outcome, false);
    Ok(llm_batch::finish_batch(
        &app,
        "tag-sort-progress",
        "标签排序完成",
        total,
        outcome,
        &extra,
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
            } => {
                let warning = !warnings.is_empty();
                let warning_text = if warning {
                    format!(" ⚠ {}", warnings.join("; "))
                } else {
                    String::new()
                };
                ItemOutcome::Done {
                    message: format!(
                        "[完成] {} | 原TAG数 {} → 排序后TAG数 {} | {}{}{}",
                        filename,
                        original_count,
                        sorted_count,
                        fmt_elapsed(elapsed_ms),
                        warning_text,
                        if !changed && !warning {
                            " (顺序未变)"
                        } else {
                            ""
                        },
                    ),
                    warning,
                }
            }
            Self::Skipped { filename } => ItemOutcome::Done {
                message: format!("[跳过] {} (空文件)", filename),
                warning: false,
            },
            Self::Error { filename, message } => ItemOutcome::Failed { filename, message },
        }
    }
}

async fn process_single_file(
    client: &reqwest::Client,
    file_path: &Path,
    output_dir: &Path,
    options: &TagSortOptions,
    throttle: &RequestThrottle,
) -> FileResult {
    let start = std::time::Instant::now();
    let filename = file_path
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();

    let content = match std::fs::read_to_string(file_path) {
        Ok(c) => c.trim().to_string(),
        Err(e) => {
            return FileResult::Error {
                filename,
                message: format!("读取失败: {}", e),
            }
        }
    };

    if content.is_empty() {
        return FileResult::Skipped { filename };
    }

    let original_tags: Vec<String> = content
        .split(',')
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
        .collect();

    if original_tags.is_empty() {
        return FileResult::Skipped { filename };
    }

    match sort_tags_with_llm(client, &original_tags, options, throttle).await {
        Ok(sorted_tags) => {
            let elapsed_ms = start.elapsed().as_millis();
            let original_count = original_tags.len();
            let sorted_count = sorted_tags.len();
            let changed = sorted_tags != original_tags;
            let mut warnings: Vec<String> = Vec::new();

            let orig_set: HashSet<&str> = original_tags.iter().map(|s| s.as_str()).collect();
            let sort_set: HashSet<&str> = sorted_tags.iter().map(|s| s.as_str()).collect();
            let missing: Vec<&str> = orig_set.difference(&sort_set).copied().collect();
            let added: Vec<&str> = sort_set.difference(&orig_set).copied().collect();

            if original_count != sorted_count {
                warnings.push(format!("数量变化: {}→{}", original_count, sorted_count));
            }
            warnings.extend(summarize_tags("缺失", &missing));
            warnings.extend(summarize_tags("新增", &added));

            let output_path = output_dir.join(&filename);
            let output_content = sorted_tags.join(", ");
            match std::fs::write(&output_path, &output_content) {
                Ok(_) => FileResult::Success {
                    filename,
                    original_count,
                    sorted_count,
                    changed,
                    warnings,
                    elapsed_ms,
                },
                Err(e) => FileResult::Error {
                    filename,
                    message: format!("写入失败: {}", e),
                },
            }
        }
        Err(e) => FileResult::Error {
            filename,
            message: e,
        },
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
    .await?;

    // 截断的回复是残缺的标签列表，写盘会把没排到的标签全部丢掉
    if reply.is_truncated() {
        return Err("回复超出了服务商的输出长度上限，已丢弃".to_string());
    }
    // 拒绝语（"I'm sorry, I can't…"）会被当成标签拆开写盘
    reject_refusal(&reply.text, "该标签文件")?;

    let sorted_tags: Vec<String> = pick_tag_line(&reply.text)
        .split(',')
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
        .collect();

    if sorted_tags.is_empty() {
        return Err("AI 返回的排序结果为空".to_string());
    }

    Ok(sorted_tags)
}

/// 端到端：本地 mock 服务器 + 真实标签文件，跑 process_single_file 全链路（读 → 请求 → 校验 → 写盘）
#[cfg(test)]
mod e2e_tests {
    use super::*;
    use crate::commands::llm_client::test_support::{client, serve_chat_reply, TempDir};

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
