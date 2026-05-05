use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;
use tauri::Emitter;

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
    pub max_tokens: i32,
    /// 请求间隔（毫秒），<= 0 表示无间隔
    pub request_interval_ms: i64,
    /// 并发线程数，<= 0 或 1 表示单线程
    pub concurrency: u32,
}

#[derive(Serialize)]
struct ChatMessage {
    role: String,
    content: String,
}

#[derive(Serialize)]
struct ChatRequest {
    model: String,
    messages: Vec<ChatMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
    temperature: f32,
}

#[derive(Deserialize)]
struct ChatResponse {
    choices: Vec<ChatChoice>,
}

#[derive(Deserialize)]
struct ChatChoice {
    message: ChatChoiceMessage,
}

#[derive(Deserialize)]
struct ChatChoiceMessage {
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    reasoning_content: Option<String>,
}

#[tauri::command]
pub fn cancel_tag_sorting() {
    TAG_SORT_CANCELLED.store(true, Ordering::SeqCst);
}

/// 收集目录中的 .txt 标签文件
fn collect_txt_files(dir: &Path) -> Result<Vec<PathBuf>, String> {
    if !dir.exists() || !dir.is_dir() {
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

/// 处理单个文件的结果
enum FileResult {
    Success {
        filename: String,
        original_count: usize,
        sorted_count: usize,
        changed: bool,
        /// 异常描述，为空表示正常
        warnings: Vec<String>,
        /// 耗时（毫秒）
        elapsed_ms: u128,
    },
    Skipped { filename: String },
    Error { filename: String, message: String },
}

#[tauri::command]
pub async fn start_tag_sorting(
    app: tauri::AppHandle,
    options: TagSortOptions,
) -> Result<ProcessResult, String> {
    TAG_SORT_CANCELLED.store(false, Ordering::SeqCst);

    let input_dir = Path::new(&options.input_path);
    let output_dir_path = PathBuf::from(&options.output_path);

    let files = collect_txt_files(input_dir)?;
    let total = files.len() as u32;

    if total == 0 {
        return Err("输入目录中没有找到 .txt 标签文件".to_string());
    }

    std::fs::create_dir_all(&output_dir_path)
        .map_err(|e| format!("创建输出目录失败: {}", e))?;

    let client = super::proxy_config::build_http_client_for_llm()
        .build()
        .map_err(|e| format!("创建 HTTP 客户端失败: {}", e))?;

    let concurrency = std::cmp::max(1, options.concurrency) as usize;

    let _ = app.emit("tag-sort-progress", ProgressEvent {
        current: 0, total,
        filename: String::new(),
        status: "info".to_string(),
        message: format!("找到 {} 个标签文件，{} 线程开始排序...", total, concurrency),
    });

    let success_count = Arc::new(AtomicU32::new(0));
    let fail_count = Arc::new(AtomicU32::new(0));
    let processed = Arc::new(AtomicU32::new(0));
    let errors: Arc<tokio::sync::Mutex<Vec<String>>> = Arc::new(tokio::sync::Mutex::new(Vec::new()));
    let cancelled = Arc::new(AtomicBool::new(false));

    let semaphore = Arc::new(tokio::sync::Semaphore::new(concurrency));

    let mut handles = Vec::new();

    for file_path in files.iter() {
        // 主循环中检查取消
        if TAG_SORT_CANCELLED.load(Ordering::SeqCst) {
            cancelled.store(true, Ordering::SeqCst);
            break;
        }

        let sem = semaphore.clone();
        let client = client.clone();
        let options = options.clone();
        let app = app.clone();
        let output_dir = output_dir_path.clone();
        let file_path = file_path.clone();
        let success_count = success_count.clone();
        let fail_count = fail_count.clone();
        let processed = processed.clone();
        let errors = errors.clone();
        let cancelled = cancelled.clone();

        let handle = tokio::spawn(async move {
            // 等待信号量前检查取消
            if TAG_SORT_CANCELLED.load(Ordering::SeqCst) {
                cancelled.store(true, Ordering::SeqCst);
                return;
            }

            let _permit = match sem.acquire().await {
                Ok(p) => p,
                Err(_) => return,
            };

            // 获取信号量后再次检查
            if TAG_SORT_CANCELLED.load(Ordering::SeqCst) {
                cancelled.store(true, Ordering::SeqCst);
                return;
            }

            // 请求间隔
            if options.request_interval_ms > 0 {
                let cur = processed.load(Ordering::SeqCst);
                if cur > 0 {
                    // 间隔期间也检查取消
                    let interval = options.request_interval_ms as u64;
                    let step = 200u64;
                    let mut waited = 0u64;
                    while waited < interval {
                        if TAG_SORT_CANCELLED.load(Ordering::SeqCst) {
                            cancelled.store(true, Ordering::SeqCst);
                            return;
                        }
                        let sleep_ms = std::cmp::min(step, interval - waited);
                        tokio::time::sleep(std::time::Duration::from_millis(sleep_ms)).await;
                        waited += sleep_ms;
                    }
                }
            }

            // 再次检查
            if TAG_SORT_CANCELLED.load(Ordering::SeqCst) {
                cancelled.store(true, Ordering::SeqCst);
                return;
            }

            // 使用 select! 让取消可以立即中断处理
            let result = tokio::select! {
                r = process_single_file(&client, &file_path, &output_dir, &options) => r,
                _ = async {
                    loop {
                        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                        if TAG_SORT_CANCELLED.load(Ordering::SeqCst) { break; }
                    }
                } => {
                    cancelled.store(true, Ordering::SeqCst);
                    return;
                }
            };

            // select 完成后检查取消
            if TAG_SORT_CANCELLED.load(Ordering::SeqCst) {
                cancelled.store(true, Ordering::SeqCst);
                return;
            }

            let cur = processed.fetch_add(1, Ordering::SeqCst) + 1;

            match result {
                FileResult::Success { filename, original_count, sorted_count, changed, warnings, elapsed_ms } => {
                    success_count.fetch_add(1, Ordering::SeqCst);
                    let has_warn = !warnings.is_empty();
                    let elapsed_str = if elapsed_ms >= 1000 {
                        format!("{:.1}s", elapsed_ms as f64 / 1000.0)
                    } else {
                        format!("{}ms", elapsed_ms)
                    };
                    let warn_str = if has_warn {
                        format!(" ⚠ {}", warnings.join("; "))
                    } else {
                        String::new()
                    };
                    let _ = app.emit("tag-sort-progress", ProgressEvent {
                        current: cur, total,
                        filename: filename.clone(),
                        status: "success".to_string(),
                        message: format!("[完成] {} | 原TAG数 {} → 排序后TAG数 {} | {}{}{}",
                            filename, original_count, sorted_count, elapsed_str, warn_str,
                            if !changed && !has_warn { " (顺序未变)" } else { "" }),
                    });
                }
                FileResult::Skipped { filename } => {
                    success_count.fetch_add(1, Ordering::SeqCst);
                    let _ = app.emit("tag-sort-progress", ProgressEvent {
                        current: cur, total,
                        filename: filename.clone(),
                        status: "success".to_string(),
                        message: format!("[跳过] {} (空文件)", filename),
                    });
                }
                FileResult::Error { filename, message } => {
                    fail_count.fetch_add(1, Ordering::SeqCst);
                    errors.lock().await.push(format!("{}: {}", filename, message));
                    let _ = app.emit("tag-sort-progress", ProgressEvent {
                        current: cur, total,
                        filename: filename.clone(),
                        status: "error".to_string(),
                        message: format!("[错误] {}: {}", filename, message),
                    });
                }
            }
        });

        handles.push(handle);
    }

    // 等待所有已启动的任务完成
    for handle in handles {
        let _ = handle.await;
    }

    let sc = success_count.load(Ordering::SeqCst);
    let fc = fail_count.load(Ordering::SeqCst);
    let errs = errors.lock().await.clone();
    let was_cancelled = cancelled.load(Ordering::SeqCst) || TAG_SORT_CANCELLED.load(Ordering::SeqCst);

    let _ = app.emit("tag-sort-progress", ProgressEvent {
        current: total, total,
        filename: String::new(),
        status: "done".to_string(),
        message: if was_cancelled {
            format!("已取消: 成功 {}, 失败 {}, 共处理 {}/{}", sc, fc, sc + fc, total)
        } else {
            format!("标签排序完成: 成功 {}, 失败 {}, 共 {}", sc, fc, total)
        },
    });

    Ok(ProcessResult { success_count: sc, fail_count: fc, total, errors: errs })
}

/// 处理单个文件
async fn process_single_file(
    client: &reqwest::Client,
    file_path: &Path,
    output_dir: &Path,
    options: &TagSortOptions,
) -> FileResult {
    let start = std::time::Instant::now();
    let filename = file_path.file_name().unwrap_or_default().to_string_lossy().to_string();

    let content = match std::fs::read_to_string(file_path) {
        Ok(c) => c.trim().to_string(),
        Err(e) => return FileResult::Error { filename, message: format!("读取失败: {}", e) },
    };

    if content.is_empty() {
        return FileResult::Skipped { filename };
    }

    let original_tags: Vec<String> = content.split(',')
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
        .collect();

    if original_tags.is_empty() {
        return FileResult::Skipped { filename };
    }

    match sort_tags_with_llm(client, &original_tags, options).await {
        Ok(sorted_tags) => {
            let elapsed_ms = start.elapsed().as_millis();
            // 完整标签对比
            let original_count = original_tags.len();
            let sorted_count = sorted_tags.len();
            let changed = sorted_tags != original_tags;
            let mut warnings: Vec<String> = Vec::new();

            // 用 HashSet 比对内容
            use std::collections::HashSet;
            let orig_set: HashSet<&str> = original_tags.iter().map(|s| s.as_str()).collect();
            let sort_set: HashSet<&str> = sorted_tags.iter().map(|s| s.as_str()).collect();

            // 缺失的标签（原始有但排序后没有）
            let missing: Vec<&str> = orig_set.difference(&sort_set).copied().collect();
            // 新增的标签（排序后有但原始没有）
            let added: Vec<&str> = sort_set.difference(&orig_set).copied().collect();

            if original_count != sorted_count {
                warnings.push(format!("数量变化: {}→{}", original_count, sorted_count));
            }
            if !missing.is_empty() {
                let display: Vec<&str> = missing.iter().take(5).copied().collect();
                let suffix = if missing.len() > 5 { format!("等{}个", missing.len()) } else { String::new() };
                warnings.push(format!("缺失: {}{}", display.join(", "), suffix));
            }
            if !added.is_empty() {
                let display: Vec<&str> = added.iter().take(5).copied().collect();
                let suffix = if added.len() > 5 { format!("等{}个", added.len()) } else { String::new() };
                warnings.push(format!("新增: {}{}", display.join(", "), suffix));
            }

            let output_path = output_dir.join(&filename);
            let output_content = sorted_tags.join(", ");
            match std::fs::write(&output_path, &output_content) {
                Ok(_) => {
                    FileResult::Success { filename, original_count, sorted_count, changed, warnings, elapsed_ms }
                }
                Err(e) => FileResult::Error { filename, message: format!("写入失败: {}", e) },
            }
        }
        Err(e) => FileResult::Error { filename, message: e },
    }
}

/// 调用 LLM 对标签进行排序
async fn sort_tags_with_llm(
    client: &reqwest::Client,
    tags: &[String],
    options: &TagSortOptions,
) -> Result<Vec<String>, String> {
    let tag_list = tags.join(", ");

    let user_content = if options.prompt.contains("{tags}") {
        options.prompt.replace("{tags}", &tag_list)
    } else {
        format!("{}\n\n需要排序的tags: {}\n\n排序后的tags:", options.prompt, tag_list)
    };

    let messages = vec![
        ChatMessage {
            role: "user".to_string(),
            content: user_content,
        },
    ];

    let request_body = ChatRequest {
        model: options.model_name.clone(),
        messages,
        max_tokens: if options.max_tokens > 0 { Some(options.max_tokens as u32) } else { None },
        temperature: options.temperature,
    };

    let endpoint = if options.api_endpoint.ends_with('/') {
        format!("{}chat/completions", options.api_endpoint)
    } else {
        format!("{}/chat/completions", options.api_endpoint)
    };

    let mut req = client.post(&endpoint)
        .header("Content-Type", "application/json")
        .json(&request_body);

    if !options.api_key.is_empty() {
        req = req.header("Authorization", format!("Bearer {}", options.api_key));
    }

    let response = req.send().await
        .map_err(|e| format!("API 请求失败: {}", e))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(format!("API 错误 ({}): {}", status, body));
    }

    let chat_resp: ChatResponse = response.json().await
        .map_err(|e| format!("解析响应失败: {}", e))?;

    let choice = chat_resp.choices.first()
        .ok_or_else(|| "API 未返回任何结果".to_string())?;

    let content = choice.message.content.as_deref().unwrap_or("").trim().to_string();
    let reasoning = choice.message.reasoning_content.as_deref().unwrap_or("").trim().to_string();

    let final_content = if !content.is_empty() {
        content
    } else if !reasoning.is_empty() {
        reasoning
    } else {
        return Err("API 返回空内容".to_string());
    };

    let cleaned = if final_content.contains('\n') {
        final_content.lines()
            .filter(|l| l.contains(','))
            .max_by_key(|l| l.len())
            .unwrap_or(&final_content)
            .to_string()
    } else {
        final_content
    };

    let sorted_tags: Vec<String> = cleaned.split(',')
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
        .collect();

    if sorted_tags.is_empty() {
        return Err("AI 返回的排序结果为空".to_string());
    }

    Ok(sorted_tags)
}
