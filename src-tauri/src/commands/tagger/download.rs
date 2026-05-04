use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::Emitter;

use super::models::ModelDefinition;
use super::{get_model_dir, ProgressEvent};

/// 全局下载取消标志
static DOWNLOAD_CANCELLED: AtomicBool = AtomicBool::new(false);

/// 下载进度事件（独立于打标进度）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DownloadProgress {
    pub filename: String,
    pub downloaded: u64,
    pub total: u64,
    pub percent: f32,
    pub speed_mbps: f64,
    /// "downloading" | "done" | "error" | "cancelled"
    pub status: String,
    pub message: String,
}

/// 取消下载
pub fn cancel_download() {
    DOWNLOAD_CANCELLED.store(true, Ordering::SeqCst);
}

/// 从 HuggingFace 下载模型文件
pub async fn download_model(
    app: &tauri::AppHandle,
    model: &ModelDefinition,
) -> Result<(), String> {
    // 重置取消标志
    DOWNLOAD_CANCELLED.store(false, Ordering::SeqCst);

    let model_dir = get_model_dir(&model.id);
    if !model_dir.exists() {
        std::fs::create_dir_all(&model_dir)
            .map_err(|e| format!("创建模型目录失败: {}", e))?;
    }

    // 通知开始下载
    let _ = app.emit("tagger-progress", ProgressEvent {
        current: 0, total: 0,
        filename: String::new(),
        status: "info".to_string(),
        message: format!("开始下载模型: {}", model.name),
    });

    // 下载 model file
    let model_url = format!(
        "https://huggingface.co/{}/resolve/main/{}",
        model.repo_id, model.model_filename
    );
    let model_dest = model_dir.join("model.onnx");
    download_file(app, &model_url, &model_dest, "model.onnx").await?;

    // 检查取消
    if DOWNLOAD_CANCELLED.load(Ordering::SeqCst) {
        // 清理已下载的不完整文件
        let _ = std::fs::remove_file(&model_dest);
        return Err("下载已取消".into());
    }

    // 下载 tags file
    let tags_url = format!(
        "https://huggingface.co/{}/resolve/main/{}",
        model.repo_id, model.tags_filename
    );
    let tags_basename = std::path::Path::new(&model.tags_filename)
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();
    let tags_dest = model_dir.join(&tags_basename);
    download_file(app, &tags_url, &tags_dest, &tags_basename).await?;

    // 完成
    let _ = app.emit("tagger-download", DownloadProgress {
        filename: "all".into(),
        downloaded: 0, total: 0, percent: 100.0, speed_mbps: 0.0,
        status: "done".to_string(),
        message: format!("模型 {} 下载完成", model.name),
    });

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
        .user_agent("PurinBox/0.1.3")
        .timeout(std::time::Duration::from_secs(600))
        .build()
        .map_err(|e| format!("创建 HTTP 客户端失败: {}", e))?;

    let response = client
        .get(url)
        .send()
        .await
        .map_err(|e| format!("下载请求失败 ({}): {}", url, e))?;

    if !response.status().is_success() {
        let _ = app.emit("tagger-download", DownloadProgress {
            filename: label.into(),
            downloaded: 0, total: 0, percent: 0.0, speed_mbps: 0.0,
            status: "error".to_string(),
            message: format!("HTTP {}: {}", response.status(), url),
        });
        return Err(format!("下载失败 (HTTP {}): {}", response.status(), url));
    }

    let total_size = response.content_length().unwrap_or(0);
    let mut stream = response.bytes_stream();

    let mut file = tokio::fs::File::create(dest)
        .await
        .map_err(|e| format!("创建文件失败: {}", e))?;

    let mut downloaded: u64 = 0;
    let mut last_report_time = std::time::Instant::now();
    let mut last_report_bytes: u64 = 0;
    let start_time = std::time::Instant::now();

    // 初始进度
    let _ = app.emit("tagger-download", DownloadProgress {
        filename: label.into(),
        downloaded: 0, total: total_size, percent: 0.0, speed_mbps: 0.0,
        status: "downloading".to_string(),
        message: format!("正在下载 {}", label),
    });

    while let Some(chunk) = stream.next().await {
        // 检查取消
        if DOWNLOAD_CANCELLED.load(Ordering::SeqCst) {
            drop(file);
            let _ = tokio::fs::remove_file(dest).await;
            let _ = app.emit("tagger-download", DownloadProgress {
                filename: label.into(),
                downloaded, total: total_size, percent: 0.0, speed_mbps: 0.0,
                status: "cancelled".to_string(),
                message: "下载已取消".into(),
            });
            return Err("下载已取消".into());
        }

        let chunk = chunk.map_err(|e| format!("下载数据失败: {}", e))?;

        tokio::io::AsyncWriteExt::write_all(&mut file, &chunk)
            .await
            .map_err(|e| format!("写入文件失败: {}", e))?;

        downloaded += chunk.len() as u64;

        // 每 500ms 或完成时报告一次进度
        let now = std::time::Instant::now();
        let elapsed_since_report = now.duration_since(last_report_time).as_millis();
        if elapsed_since_report >= 500 || (total_size > 0 && downloaded >= total_size) {
            let speed = if elapsed_since_report > 0 {
                let bytes_delta = downloaded - last_report_bytes;
                bytes_delta as f64 / elapsed_since_report as f64 * 1000.0 / 1_048_576.0
            } else { 0.0 };

            last_report_time = now;
            last_report_bytes = downloaded;

            let percent = if total_size > 0 {
                (downloaded as f64 / total_size as f64 * 100.0) as f32
            } else { 0.0 };

            let elapsed_total = start_time.elapsed().as_secs_f64();
            let avg_speed = if elapsed_total > 0.0 { downloaded as f64 / elapsed_total / 1_048_576.0 } else { 0.0 };

            let mb_done = downloaded as f64 / 1_048_576.0;
            let message = if total_size > 0 {
                let mb_total = total_size as f64 / 1_048_576.0;
                format!("{} — {:.1}/{:.1} MB ({:.1} MB/s)", label, mb_done, mb_total, avg_speed)
            } else {
                format!("{} — {:.1} MB ({:.1} MB/s)", label, mb_done, avg_speed)
            };

            let _ = app.emit("tagger-download", DownloadProgress {
                filename: label.into(),
                downloaded,
                total: total_size,
                percent,
                speed_mbps: speed,
                status: "downloading".to_string(),
                message,
            });
        }
    }

    Ok(())
}
