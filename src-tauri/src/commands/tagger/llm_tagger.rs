use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use tauri::Emitter;

use super::{ProcessResult, ProgressEvent};
use crate::commands::{collect_image_files_with_recursive, report_failed_copies};

use crate::commands::llm_client::{self, ChatError, ChatMessage, ChatParams, RequestThrottle};

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
    // 互斥：全局取消标志不允许并发运行
    static LLM_RUNNING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    let _busy = crate::commands::BusyGuard::acquire(&LLM_RUNNING, "LLM 打标")?;

    LLM_CANCELLED.store(false, Ordering::SeqCst);
    let input_path_owned = options.input_path.clone();
    let input_dir = Path::new(&input_path_owned);
    let files = collect_image_files_with_recursive(input_dir, options.recursive)?;
    let total = files.len() as u32;
    let mut success_count = 0u32;
    let mut processed_count = 0u32;
    let fail_count = 0u32;
    let errors: Vec<String> = Vec::new();
    let failed_files: Vec<std::path::PathBuf> = Vec::new();

    let client = llm_client::llm_http_client()?;

    let _ = app.emit(
        "llm-tagger-progress",
        ProgressEvent::new("info", format!("读取到 {} 张图片", total)).at(0, total),
    );

    let concurrency = options.concurrency.max(1) as usize;
    let interval_ms = options.request_interval_ms;

    // 过滤出需要处理的文件（处理 skip_existing）
    let mut work_items: Vec<(usize, std::path::PathBuf)> = Vec::new();
    for (i, file_path) in files.iter().enumerate() {
        if options.skip_existing {
            let stem = file_path.file_stem().unwrap_or_default().to_string_lossy();
            let parent = file_path.parent().unwrap_or(Path::new("."));
            let txt_path = parent.join(format!("{}.txt", stem));
            let json_path = parent.join(format!("{}.json", stem));
            let existing = if json_path.exists() {
                std::fs::read_to_string(&json_path).ok()
            } else if txt_path.exists() {
                std::fs::read_to_string(&txt_path).ok()
            } else {
                None
            };
            if let Some(content) = existing {
                if !content.trim().is_empty() {
                    success_count += 1;
                    processed_count += 1;
                    let filename = file_path
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .to_string();
                    let _ = app.emit(
                        "llm-tagger-progress",
                        ProgressEvent::new("success", format!("[跳过] {} (已有描述)", filename))
                            .at(processed_count, total)
                            .file(filename.clone()),
                    );
                    continue;
                }
            }
        }
        work_items.push((i, file_path.clone()));
    }

    // 使用 semaphore 控制并发
    let sem = std::sync::Arc::new(tokio::sync::Semaphore::new(concurrency));
    let app_arc = std::sync::Arc::new(app);
    let options_arc = std::sync::Arc::new(options);
    let client_arc = std::sync::Arc::new(client);
    let success_cnt = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(success_count));
    let fail_cnt = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(fail_count));
    let processed_cnt = std::sync::Arc::new(AtomicU32::new(processed_count));
    let errors_arc = std::sync::Arc::new(tokio::sync::Mutex::new(errors));
    let failed_arc = std::sync::Arc::new(tokio::sync::Mutex::new(failed_files));

    let throttle = std::sync::Arc::new(RequestThrottle::new(interval_ms));

    let mut handles = Vec::new();

    for (i, file_path) in work_items.into_iter() {
        if LLM_CANCELLED.load(Ordering::SeqCst) {
            break;
        }

        let permit = sem.clone().acquire_owned().await.unwrap();
        let app_c = app_arc.clone();
        let opts = options_arc.clone();
        let cli = client_arc.clone();
        let s_cnt = success_cnt.clone();
        let f_cnt = fail_cnt.clone();
        let p_cnt = processed_cnt.clone();
        let errs = errors_arc.clone();
        let fails = failed_arc.clone();
        let throttle = throttle.clone();

        let handle = tokio::spawn(async move {
            let _permit = permit;
            if LLM_CANCELLED.load(Ordering::SeqCst) {
                return;
            }

            let filename = file_path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();
            let _ = app_c.emit(
                "llm-tagger-progress",
                ProgressEvent::new(
                    "processing",
                    format!("正在处理: {} ({}/{})", filename, i + 1, total),
                )
                .at(i as u32 + 1, total)
                .file(filename.clone()),
            );

            let file_start = std::time::Instant::now();

            let tag_result = tokio::select! {
                result = tag_with_llm(&cli, &file_path, &opts, &throttle) => result,
                _ = async {
                    loop {
                        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                        if LLM_CANCELLED.load(Ordering::SeqCst) { break; }
                    }
                } => {
                    Err("已取消".to_string())
                }
            };

            let elapsed_ms = file_start.elapsed().as_millis();
            let elapsed_str = llm_client::fmt_elapsed(elapsed_ms);

            if LLM_CANCELLED.load(Ordering::SeqCst) {
                return;
            }

            let current = p_cnt.fetch_add(1, Ordering::SeqCst) + 1;
            let outcome = tag_result.and_then(|tag_text| {
                let stem = file_path
                    .file_stem()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string();
                let parent = file_path.parent().unwrap_or(Path::new("."));
                let out_path = if opts.output_format == "json" {
                    parent.join(format!("{}.json", stem))
                } else {
                    parent.join(format!("{}.txt", stem))
                };
                let content = format_output(&tag_text, &opts)?;
                crate::commands::config_paths::write_file_atomic(&out_path, content.as_bytes())
                    .map_err(|e| format!("写入失败 {}", e))
            });
            match outcome {
                Ok(()) => {
                    s_cnt.fetch_add(1, Ordering::SeqCst);
                    let _ = app_c.emit(
                        "llm-tagger-progress",
                        ProgressEvent::new(
                            "success",
                            format!("[完成] {} ({})", filename, elapsed_str),
                        )
                        .at(current, total)
                        .file(filename.clone()),
                    );
                }
                Err(e) => {
                    f_cnt.fetch_add(1, Ordering::SeqCst);
                    let err_msg = format!("{}: {}", filename, e);
                    errs.lock().await.push(err_msg.clone());
                    fails.lock().await.push(file_path.clone());
                    let _ = app_c.emit(
                        "llm-tagger-progress",
                        ProgressEvent::new(
                            "error",
                            format!("[错误] {} ({})", err_msg, elapsed_str),
                        )
                        .at(current, total)
                        .file(filename.clone()),
                    );
                }
            }
        });
        handles.push(handle);
    }

    // 等待所有任务完成
    for h in handles {
        let _ = h.await;
    }

    let success_count = success_cnt.load(Ordering::SeqCst);
    let fail_count = fail_cnt.load(Ordering::SeqCst);
    let errors = errors_arc.lock().await.clone();
    let failed_files = failed_arc.lock().await.clone();

    report_failed_copies(
        &app_arc,
        "llm-tagger-progress",
        input_dir,
        &failed_files,
        options_arc.recursive,
        total,
    );

    let was_cancelled = LLM_CANCELLED.load(Ordering::SeqCst);
    let _ = app_arc.emit(
        "llm-tagger-progress",
        ProgressEvent::new(
            "done",
            if was_cancelled {
                format!(
                    "已取消: 已完成 {}/{}, 成功 {}, 失败 {}",
                    success_count + fail_count,
                    total,
                    success_count,
                    fail_count
                )
            } else {
                format!(
                    "LLM 打标完成: 成功 {}, 失败 {}, 共 {}",
                    success_count, fail_count, total
                )
            },
        )
        .at(
            if was_cancelled {
                success_count + fail_count
            } else {
                total
            },
            total,
        ),
    );

    Ok(ProcessResult {
        success_count,
        fail_count,
        total,
        errors,
    })
}

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
    let value = serde_json::from_str::<serde_json::Value>(cleaned).unwrap_or_else(|_| {
        if options.json_simplified {
            serde_json::json!({
                "quality": "", "series": "", "artist": "", "character": "", "count": "",
                "appearance": [], "tags": [], "environment": [], "nl": text,
            })
        } else {
            serde_json::json!({
                "fixed": {"quality": "", "series": "", "artist": ""},
                "character": {"name": "", "variant": ""},
                "from_path": {"appearance": []},
                "ai_output": {"count": "", "appearance": [], "tags": [], "environment": [], "nl": text},
            })
        }
    });
    serde_json::to_string_pretty(&value).map_err(|e| format!("序列化标签失败: {}", e))
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
    let reply = llm_client::chat_completion(client, &params, &messages, throttle, &LLM_CANCELLED)
        .await
        .map_err(|e| match e {
            ChatError::ContentFilter => "LLM 内容安全审核拒绝了该图片".to_string(),
            other => other.into(),
        })?;
    if reply.is_truncated() {
        return Err("响应因 max_tokens 被截断，已丢弃（请调大 max_tokens）".into());
    }
    llm_client::reject_refusal(&reply.text, "该图片")?;
    Ok(reply.text)
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
    use crate::commands::llm_client::test_support::{client, serve_chat_reply, TempDir};
    use serde_json::json;

    fn options() -> LlmTaggerOptions {
        serde_json::from_value(json!({
            "input_path": "", "api_endpoint": "", "api_key": "", "model_name": "mock",
            "system_prompt": "describe", "user_prompt": "image", "temperature": 0.5,
            "max_tokens": 20, "output_format": "json",
        }))
        .unwrap()
    }

    #[test]
    fn non_json_responses_keep_the_full_schema() {
        let mut opts = options();
        let full: serde_json::Value =
            serde_json::from_str(&format_output("description", &opts).unwrap()).unwrap();
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
        let simple: serde_json::Value =
            serde_json::from_str(&format_output("description", &opts).unwrap()).unwrap();
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
            let parsed: serde_json::Value =
                serde_json::from_str(&format_output(text, &options()).unwrap()).unwrap();
            assert_eq!(parsed, json!({"tags": ["solo"]}));
        }
    }

    #[tokio::test]
    async fn rejects_truncated_filtered_and_refused_responses() {
        let temp = TempDir::new("llm_tagger_responses");
        let path = temp.join("image.wrong_extension");
        image::RgbImage::new(2, 2)
            .save_with_format(&path, image::ImageFormat::Png)
            .unwrap();
        for (text, finish, expected) in [
            ("partial response", "length", "截断"),
            ("", "content_filter", "内容安全审核"),
            ("I cannot assist with this request", "stop", "拒绝"),
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
}
