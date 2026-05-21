use base64::Engine;
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::Emitter;

use super::{ProcessResult, ProgressEvent};
use crate::commands::collect_image_files;

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
}

fn default_image_size() -> u32 { 1024 }
fn default_llm_output_format() -> String { "txt".into() }
fn default_interval() -> i64 { -1 }
fn default_concurrency() -> u32 { 1 }

#[derive(Serialize)]
struct ChatMessage {
    role: String,
    content: serde_json::Value,
}

#[derive(Serialize)]
struct ChatRequest {
    model: String,
    messages: Vec<ChatMessage>,
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
}

#[derive(Deserialize)]
struct ChatChoiceMessage {
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    reasoning_content: Option<String>,
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
    LLM_CANCELLED.store(false, Ordering::SeqCst);
    let input_path_owned = options.input_path.clone();
    let input_dir = Path::new(&input_path_owned);
    let files = collect_image_files(input_dir)?;
    let total = files.len() as u32;
    let mut success_count = 0u32;
    let fail_count = 0u32;
    let errors: Vec<String> = Vec::new();
    let failed_files: Vec<std::path::PathBuf> = Vec::new();

    let client = crate::commands::proxy_config::build_http_client_for_llm()
        .build()
        .map_err(|e| format!("创建 HTTP 客户端失败: {}", e))?;

    let _ = app.emit("llm-tagger-progress", ProgressEvent {
        current: 0, total,
        filename: String::new(),
        status: "info".to_string(),
        message: format!("读取到 {} 张图片", total),
    });

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
                    let filename = file_path.file_name().unwrap_or_default().to_string_lossy().to_string();
                    let _ = app.emit("llm-tagger-progress", ProgressEvent {
                        current: i as u32 + 1, total,
                        filename: filename.clone(),
                        status: "success".to_string(),
                        message: format!("[跳过] {} (已有描述)", filename),
                    });
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
    let errors_arc = std::sync::Arc::new(tokio::sync::Mutex::new(errors));
    let failed_arc = std::sync::Arc::new(tokio::sync::Mutex::new(failed_files));

    let mut handles = Vec::new();

    for (idx, (i, file_path)) in work_items.into_iter().enumerate() {
        if LLM_CANCELLED.load(Ordering::SeqCst) { break; }

        // 请求间隔（仅在非首个请求时）
        if idx > 0 && interval_ms > 0 {
            tokio::time::sleep(std::time::Duration::from_millis(interval_ms as u64)).await;
        }

        let permit = sem.clone().acquire_owned().await.unwrap();
        let app_c = app_arc.clone();
        let opts = options_arc.clone();
        let cli = client_arc.clone();
        let s_cnt = success_cnt.clone();
        let f_cnt = fail_cnt.clone();
        let errs = errors_arc.clone();
        let fails = failed_arc.clone();

        let handle = tokio::spawn(async move {
            let _permit = permit;
            if LLM_CANCELLED.load(Ordering::SeqCst) { return; }

            let filename = file_path.file_name().unwrap_or_default().to_string_lossy().to_string();
            let _ = app_c.emit("llm-tagger-progress", ProgressEvent {
                current: i as u32 + 1, total,
                filename: filename.clone(),
                status: "processing".to_string(),
                message: format!("正在处理: {} ({}/{})", filename, i + 1, total),
            });

            let file_start = std::time::Instant::now();

            let tag_result = tokio::select! {
                result = tag_with_llm(&cli, &file_path, &opts) => result,
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
            let elapsed_str = if elapsed_ms >= 1000 {
                format!("{:.1}s", elapsed_ms as f64 / 1000.0)
            } else {
                format!("{}ms", elapsed_ms)
            };

            if LLM_CANCELLED.load(Ordering::SeqCst) { return; }

            match tag_result {
                Ok(tag_text) => {
                    let stem = file_path.file_stem().unwrap_or_default().to_string_lossy().to_string();
                    let parent = file_path.parent().unwrap_or(Path::new("."));
                    let out_path = if opts.output_format == "json" {
                        parent.join(format!("{}.json", stem))
                    } else {
                        parent.join(format!("{}.txt", stem))
                    };
                    let content = if opts.output_format == "json" {
                        let cleaned = tag_text.trim();
                        let cleaned = if cleaned.starts_with("```json") {
                            cleaned.strip_prefix("```json").unwrap_or(cleaned)
                                .strip_suffix("```").unwrap_or(cleaned)
                                .trim()
                        } else if cleaned.starts_with("```") {
                            cleaned.strip_prefix("```").unwrap_or(cleaned)
                                .strip_suffix("```").unwrap_or(cleaned)
                                .trim()
                        } else {
                            cleaned
                        };
                        if let Ok(json_val) = serde_json::from_str::<serde_json::Value>(cleaned) {
                            serde_json::to_string_pretty(&json_val).unwrap_or_default()
                        } else if opts.json_simplified {
                                serde_json::to_string_pretty(&serde_json::json!({ "nl": tag_text })).unwrap_or_default()
                            } else {
                                serde_json::to_string_pretty(&serde_json::json!({ "ai_output": { "nl": tag_text } })).unwrap_or_default()
                            }
                    } else {
                        tag_text
                    };
                    match std::fs::write(&out_path, &content) {
                        Ok(_) => {
                            s_cnt.fetch_add(1, Ordering::SeqCst);
                            let _ = app_c.emit("llm-tagger-progress", ProgressEvent {
                                current: i as u32 + 1, total,
                                filename: filename.clone(),
                                status: "success".to_string(),
                                message: format!("[完成] {} ({})", filename, elapsed_str),
                            });
                        }
                        Err(e) => {
                            f_cnt.fetch_add(1, Ordering::SeqCst);
                            let err_msg = format!("{}: 写入失败 {}", filename, e);
                            errs.lock().await.push(err_msg.clone());
                            fails.lock().await.push(file_path.clone());
                            let _ = app_c.emit("llm-tagger-progress", ProgressEvent {
                                current: i as u32 + 1, total,
                                filename: filename.clone(),
                                status: "error".to_string(),
                                message: format!("[错误] {} ({})", err_msg, elapsed_str),
                            });
                        }
                    }
                }
                Err(e) => {
                    f_cnt.fetch_add(1, Ordering::SeqCst);
                    let err_msg = format!("{}: {}", filename, e);
                    errs.lock().await.push(err_msg.clone());
                    fails.lock().await.push(file_path.clone());
                    let _ = app_c.emit("llm-tagger-progress", ProgressEvent {
                        current: i as u32 + 1, total,
                        filename: filename.clone(),
                        status: "error".to_string(),
                        message: format!("[错误] {} ({})", err_msg, elapsed_str),
                    });
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

    // 将失败的图片复制到 Fail 文件夹
    if !failed_files.is_empty() {
        let fail_dir = input_dir.join("Fail");
        if let Err(e) = std::fs::create_dir_all(&fail_dir) {
            let _ = app_arc.emit("llm-tagger-progress", ProgressEvent {
                current: total, total,
                filename: String::new(),
                status: "error".to_string(),
                message: format!("创建 Fail 文件夹失败: {}", e),
            });
        } else {
            let mut copy_count = 0u32;
            for f in &failed_files {
                let fname = f.file_name().unwrap_or_default();
                let dest = fail_dir.join(fname);
                if std::fs::copy(f, &dest).is_ok() {
                    copy_count += 1;
                }
            }
            let _ = app_arc.emit("llm-tagger-progress", ProgressEvent {
                current: total, total,
                filename: String::new(),
                status: "info".to_string(),
                message: format!("已将 {} 张失败图片复制到 Fail 文件夹", copy_count),
            });
        }
    }

    let _ = app_arc.emit("llm-tagger-progress", ProgressEvent {
        current: total, total,
        filename: String::new(),
        status: "done".to_string(),
        message: format!("LLM 打标完成: 成功 {}, 失败 {}, 共 {}", success_count, fail_count, total),
    });

    Ok(ProcessResult { success_count, fail_count, total, errors })
}

async fn tag_with_llm(
    client: &reqwest::Client,
    img_path: &Path,
    options: &LlmTaggerOptions,
) -> Result<String, String> {
    // 读取并缩放图片
    let max_side = if options.image_size > 0 { options.image_size } else { 1024 };
    let img = image::open(img_path)
        .map_err(|e| format!("读取图片失败: {}", e))?;

    let img = if img.width() > max_side || img.height() > max_side {
        img.resize(max_side, max_side, image::imageops::FilterType::Lanczos3)
    } else {
        img
    };

    // 编码为 JPEG base64（压缩更小、传输更快）
    let mut buf = std::io::Cursor::new(Vec::new());
    img.write_to(&mut buf, image::ImageFormat::Jpeg)
        .map_err(|e| format!("编码图片失败: {}", e))?;
    let b64 = base64::engine::general_purpose::STANDARD.encode(buf.get_ref());
    let data_url = format!("data:image/jpeg;base64,{}", b64);

    // Build OpenAI-compatible request
    let messages = vec![
        ChatMessage {
            role: "system".to_string(),
            content: serde_json::Value::String(options.system_prompt.clone()),
        },
        ChatMessage {
            role: "user".to_string(),
            content: serde_json::json!([
                { "type": "text", "text": options.user_prompt },
                { "type": "image_url", "image_url": { "url": data_url } }
            ]),
        },
    ];

    let request_body = ChatRequest {
        model: options.model_name.clone(),
        messages,
        max_tokens: if options.max_tokens > 0 { Some(options.max_tokens as u32) } else { None },
        temperature: options.temperature,
        top_p: if options.top_p > 0.0 && options.top_p <= 1.0 { Some(options.top_p) } else { None },
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

    // 优先使用 content，为空时 fallback 到 reasoning_content（支持 Qwen3 等思维模型）
    let content = choice.message.content.as_deref().unwrap_or("").trim().to_string();
    let reasoning = choice.message.reasoning_content.as_deref().unwrap_or("").trim().to_string();

    let final_content = if !content.is_empty() {
        content
    } else if !reasoning.is_empty() {
        reasoning
    } else {
        return Err("API 返回空内容".to_string());
    };

    Ok(final_content)
}

#[derive(Deserialize)]
struct ModelsResponse {
    data: Vec<ModelInfo>,
}

#[derive(Deserialize, Serialize, Clone)]
struct ModelInfo {
    id: String,
}

#[tauri::command]
pub async fn fetch_llm_models(
    api_endpoint: String,
    api_key: String,
) -> Result<Vec<String>, String> {
    let client = crate::commands::proxy_config::build_http_client_for_llm()
        .build()
        .map_err(|e| format!("创建 HTTP 客户端失败: {}", e))?;

    let endpoint = if api_endpoint.ends_with('/') {
        format!("{}models", api_endpoint)
    } else {
        format!("{}/models", api_endpoint)
    };

    let mut req = client.get(&endpoint)
        .header("Content-Type", "application/json");

    if !api_key.is_empty() {
        req = req.header("Authorization", format!("Bearer {}", api_key));
    }

    let response = req.send().await
        .map_err(|e| format!("请求模型列表失败: {:?}", e))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(format!("API 错误 ({}): {}", status, body));
    }

    let models_resp: ModelsResponse = response.json().await
        .map_err(|e| format!("解析模型列表失败: {}", e))?;

    let mut model_ids: Vec<String> = models_resp.data.iter().map(|m| m.id.clone()).collect();
    model_ids.sort();

    Ok(model_ids)
}
