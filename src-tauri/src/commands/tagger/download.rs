use futures_util::StreamExt;
use tauri::Emitter;

use super::models::ModelDefinition;
use super::{get_model_dir, ProgressEvent};

/// 从 HuggingFace 下载模型文件
pub async fn download_model(
    app: &tauri::AppHandle,
    model: &ModelDefinition,
) -> Result<(), String> {
    let model_dir = get_model_dir(&model.id);
    if !model_dir.exists() {
        std::fs::create_dir_all(&model_dir)
            .map_err(|e| format!("创建模型目录失败: {}", e))?;
    }

    // 下载 model file
    let model_url = format!(
        "https://huggingface.co/{}/resolve/main/{}",
        model.repo_id, model.model_filename
    );
    let model_dest = model_dir.join("model.onnx");

    let _ = app.emit("tagger-progress", ProgressEvent {
        current: 0, total: 0,
        filename: String::new(),
        status: "info".to_string(),
        message: format!("正在下载模型文件: {}...", model.model_filename),
    });

    download_file(app, &model_url, &model_dest, "model.onnx").await?;

    // 下载 tags file (可能是 csv 或 json)
    let tags_url = format!(
        "https://huggingface.co/{}/resolve/main/{}",
        model.repo_id, model.tags_filename
    );
    // 保存为原始文件名（selected_tags.csv 或 tag_mapping.json）
    let tags_basename = std::path::Path::new(&model.tags_filename)
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();
    let tags_dest = model_dir.join(&tags_basename);

    let _ = app.emit("tagger-progress", ProgressEvent {
        current: 0, total: 0,
        filename: String::new(),
        status: "info".to_string(),
        message: format!("正在下载标签定义文件: {}...", tags_basename),
    });

    download_file(app, &tags_url, &tags_dest, &tags_basename).await?;

    let _ = app.emit("tagger-progress", ProgressEvent {
        current: 0, total: 0,
        filename: String::new(),
        status: "success".to_string(),
        message: format!("模型 {} 下载完成", model.name),
    });

    Ok(())
}

async fn download_file(
    app: &tauri::AppHandle,
    url: &str,
    dest: &std::path::Path,
    label: &str,
) -> Result<(), String> {
    let client = reqwest::Client::builder()
        .user_agent("AiTrainTools/0.1.0")
        .timeout(std::time::Duration::from_secs(600))
        .build()
        .map_err(|e| format!("创建 HTTP 客户端失败: {}", e))?;

    let response = client
        .get(url)
        .send()
        .await
        .map_err(|e| format!("下载请求失败 ({}): {}", url, e))?;

    if !response.status().is_success() {
        return Err(format!(
            "下载失败 (HTTP {}): {}",
            response.status(),
            url
        ));
    }

    let total_size = response.content_length().unwrap_or(0);
    let mut stream = response.bytes_stream();

    let mut file = tokio::fs::File::create(dest)
        .await
        .map_err(|e| format!("创建文件失败: {}", e))?;

    let mut downloaded: u64 = 0;
    let mut last_report: u64 = 0;

    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| format!("下载数据失败: {}", e))?;

        tokio::io::AsyncWriteExt::write_all(&mut file, &chunk)
            .await
            .map_err(|e| format!("写入文件失败: {}", e))?;

        downloaded += chunk.len() as u64;

        // 每 1MB 报告一次进度
        if downloaded - last_report > 1_048_576 || downloaded == total_size {
            last_report = downloaded;
            let progress_msg = if total_size > 0 {
                let mb_done = downloaded as f64 / 1_048_576.0;
                let mb_total = total_size as f64 / 1_048_576.0;
                let pct = (downloaded as f64 / total_size as f64 * 100.0) as u32;
                format!(
                    "[下载] {} : {:.1} / {:.1} MB ({}%)",
                    label, mb_done, mb_total, pct
                )
            } else {
                let mb_done = downloaded as f64 / 1_048_576.0;
                format!("[下载] {} : {:.1} MB", label, mb_done)
            };

            let _ = app.emit("tagger-progress", ProgressEvent {
                current: if total_size > 0 {
                    (downloaded as f64 / total_size as f64 * 100.0) as u32
                } else {
                    0
                },
                total: 100,
                filename: label.to_string(),
                status: "info".to_string(),
                message: progress_msg,
            });
        }
    }

    Ok(())
}
