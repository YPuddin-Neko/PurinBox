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

    // 下载 model.onnx
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

    // 下载 selected_tags.csv
    let tags_url = format!(
        "https://huggingface.co/{}/resolve/main/{}",
        model.repo_id, model.tags_filename
    );
    let tags_dest = model_dir.join("selected_tags.csv");

    let _ = app.emit("tagger-progress", ProgressEvent {
        current: 0, total: 0,
        filename: String::new(),
        status: "info".to_string(),
        message: "正在下载标签定义文件...".to_string(),
    });

    download_file(app, &tags_url, &tags_dest, "selected_tags.csv").await?;

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
    let client = reqwest::Client::new();
    let response = client
        .get(url)
        .send()
        .await
        .map_err(|e| format!("下载请求失败: {}", e))?;

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
