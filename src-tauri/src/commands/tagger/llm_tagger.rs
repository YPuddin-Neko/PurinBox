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
}

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
    let input_dir = Path::new(&options.input_path);
    let files = collect_image_files(input_dir)?;
    let total = files.len() as u32;
    let mut success_count = 0u32;
    let mut fail_count = 0u32;
    let mut errors = Vec::new();

    let client = reqwest::Client::builder()
        .danger_accept_invalid_certs(true)
        .build()
        .map_err(|e| format!("创建 HTTP 客户端失败: {}", e))?;

    let _ = app.emit("llm-tagger-progress", ProgressEvent {
        current: 0, total,
        filename: String::new(),
        status: "info".to_string(),
        message: format!("读取到 {} 张图片", total),
    });

    for (i, file_path) in files.iter().enumerate() {
        // 检查取消
        if LLM_CANCELLED.load(Ordering::SeqCst) {
            let _ = app.emit("llm-tagger-progress", ProgressEvent {
                current: i as u32, total,
                filename: String::new(),
                status: "done".to_string(),
                message: format!("已取消 LLM 打标: 成功 {}, 失败 {}, 共处理 {}/{}", success_count, fail_count, i, total),
            });
            return Ok(ProcessResult { success_count, fail_count, total, errors });
        }

        let filename = file_path.file_name().unwrap_or_default().to_string_lossy().to_string();

        let _ = app.emit("llm-tagger-progress", ProgressEvent {
            current: i as u32 + 1, total,
            filename: filename.clone(),
            status: "processing".to_string(),
            message: format!("正在处理: {} ({}/{})", filename, i + 1, total),
        });

        // 使用 select! 让取消可以立即中断 HTTP 请求
        let tag_result = tokio::select! {
            result = tag_with_llm(&client, file_path, &options) => result,
            _ = async {
                loop {
                    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                    if LLM_CANCELLED.load(Ordering::SeqCst) { break; }
                }
            } => {
                Err("已取消".to_string())
            }
        };

        // 取消后立即退出循环
        if LLM_CANCELLED.load(Ordering::SeqCst) {
            let _ = app.emit("llm-tagger-progress", ProgressEvent {
                current: i as u32 + 1, total,
                filename: String::new(),
                status: "done".to_string(),
                message: format!("已取消 LLM 打标: 成功 {}, 失败 {}, 共处理 {}/{}", success_count, fail_count, i, total),
            });
            return Ok(ProcessResult { success_count, fail_count, total, errors });
        }

        match tag_result {
            Ok(tag_text) => {
                let stem = file_path.file_stem().unwrap_or_default().to_string_lossy();
                let parent = file_path.parent().unwrap_or(Path::new("."));
                let txt_path = parent.join(format!("{}.txt", stem));
                match std::fs::write(&txt_path, &tag_text) {
                    Ok(_) => {
                        success_count += 1;
                        let tag_count = tag_text.split(',').map(|s| s.trim()).filter(|s| !s.is_empty()).count();
                        let _ = app.emit("llm-tagger-progress", ProgressEvent {
                            current: i as u32 + 1, total,
                            filename: filename.clone(),
                            status: "success".to_string(),
                            message: format!("[完成] {} → {} 个标签", filename, tag_count),
                        });
                    }
                    Err(e) => {
                        fail_count += 1;
                        let err_msg = format!("{}: 写入失败 {}", filename, e);
                        errors.push(err_msg.clone());
                        let _ = app.emit("llm-tagger-progress", ProgressEvent {
                            current: i as u32 + 1, total,
                            filename: filename.clone(),
                            status: "error".to_string(),
                            message: format!("[错误] {}", err_msg),
                        });
                    }
                }
            }
            Err(e) => {
                fail_count += 1;
                let err_msg = format!("{}: {}", filename, e);
                errors.push(err_msg.clone());
                let _ = app.emit("llm-tagger-progress", ProgressEvent {
                    current: i as u32 + 1, total,
                    filename: filename.clone(),
                    status: "error".to_string(),
                    message: format!("[错误] {}", err_msg),
                });
            }
        }
    }

    let _ = app.emit("llm-tagger-progress", ProgressEvent {
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
    // Read image and encode to base64
    let img_bytes = std::fs::read(img_path)
        .map_err(|e| format!("读取图片失败: {}", e))?;

    let ext = img_path.extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_else(|| "png".into());
    let mime = match ext.as_str() {
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        "webp" => "image/webp",
        "gif" => "image/gif",
        "bmp" => "image/bmp",
        _ => "image/png",
    };

    let b64 = base64::engine::general_purpose::STANDARD.encode(&img_bytes);
    let data_url = format!("data:{};base64,{}", mime, b64);

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
    let client = reqwest::Client::builder()
        .danger_accept_invalid_certs(true)
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
