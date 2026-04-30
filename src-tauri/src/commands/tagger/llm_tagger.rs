use base64::Engine;
use serde::{Deserialize, Serialize};
use std::path::Path;
use tauri::Emitter;

use super::{ProcessResult, ProgressEvent};
use crate::commands::collect_image_files;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmTaggerOptions {
    pub input_path: String,
    pub api_endpoint: String,
    pub api_key: String,
    pub model_name: String,
    pub system_prompt: String,
    pub user_prompt: String,
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
    max_tokens: u32,
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
    content: String,
}

#[tauri::command]
pub async fn start_llm_tagging(
    app: tauri::AppHandle,
    options: LlmTaggerOptions,
) -> Result<ProcessResult, String> {
    let input_dir = Path::new(&options.input_path);
    let files = collect_image_files(input_dir)?;
    let total = files.len() as u32;
    let mut success_count = 0u32;
    let mut fail_count = 0u32;
    let mut errors = Vec::new();

    let client = reqwest::Client::new();

    let _ = app.emit("llm-tagger-progress", ProgressEvent {
        current: 0, total,
        filename: String::new(),
        status: "info".to_string(),
        message: format!("开始 LLM 打标 | 模型: {} | 共 {} 张图片", options.model_name, total),
    });

    for (i, file_path) in files.iter().enumerate() {
        let filename = file_path.file_name().unwrap_or_default().to_string_lossy().to_string();

        let _ = app.emit("llm-tagger-progress", ProgressEvent {
            current: i as u32 + 1, total,
            filename: filename.clone(),
            status: "processing".to_string(),
            message: format!("正在处理: {} ({}/{})", filename, i + 1, total),
        });

        match tag_with_llm(&client, file_path, &options).await {
            Ok(tag_text) => {
                let stem = file_path.file_stem().unwrap_or_default().to_string_lossy();
                let parent = file_path.parent().unwrap_or(Path::new("."));
                let txt_path = parent.join(format!("{}.txt", stem));
                match std::fs::write(&txt_path, &tag_text) {
                    Ok(_) => {
                        success_count += 1;
                        let preview = if tag_text.len() > 80 {
                            format!("{}...", &tag_text[..80])
                        } else {
                            tag_text.clone()
                        };
                        let _ = app.emit("llm-tagger-progress", ProgressEvent {
                            current: i as u32 + 1, total,
                            filename: filename.clone(),
                            status: "success".to_string(),
                            message: format!("[完成] {} → {}", filename, preview),
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
        max_tokens: 1024,
        temperature: 0.1,
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

    let content = chat_resp.choices.first()
        .map(|c| c.message.content.trim().to_string())
        .unwrap_or_default();

    if content.is_empty() {
        return Err("API 返回空内容".to_string());
    }

    Ok(content)
}
