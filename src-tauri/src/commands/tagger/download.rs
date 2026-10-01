use std::sync::atomic::{AtomicBool, Ordering};
use tauri::Emitter;

use super::models::{basename, ModelDefinition};
use super::{get_model_dir, ProgressEvent};
use crate::commands::http_download::{self, DownloadError, DownloadProgress};

/// 下载进度事件（独立于打标进度）
const DOWNLOAD_EVENT: &str = "tagger-download";

/// 全局下载取消标志
static DOWNLOAD_CANCELLED: AtomicBool = AtomicBool::new(false);

/// 取消下载
pub fn cancel_download() {
    DOWNLOAD_CANCELLED.store(true, Ordering::SeqCst);
}

/// 从 HuggingFace 下载模型文件
pub async fn download_model(app: &tauri::AppHandle, model: &ModelDefinition) -> Result<(), String> {
    DOWNLOAD_CANCELLED.store(false, Ordering::SeqCst);

    let model_dir = get_model_dir(&model.id);
    if !model_dir.exists() {
        std::fs::create_dir_all(&model_dir).map_err(|e| format!("创建模型目录失败: {}", e))?;
    }

    let _ = app.emit(
        "tagger-progress",
        ProgressEvent::new("info", format!("开始下载模型: {}", model.name)),
    );

    let client = http_download::download_client()?;
    let hf_url = |file: &str| http_download::huggingface_url(&model.repo_id, file);

    // 权重固定存为 model.onnx，额外文件（例如 ONNX external data: model.onnx.data）
    // 按原文件名存放。每个文件下完后若发现已取消，连同该文件一起删除
    let weights = std::iter::once((model.model_filename.as_str(), "model.onnx".to_string()))
        .chain(model.extra_files.iter().map(|f| (f.as_str(), basename(f))));
    for (remote, local) in weights {
        let dest = model_dir.join(&local);
        download_file(app, &client, &hf_url(remote), &dest, &local).await?;
        if DOWNLOAD_CANCELLED.load(Ordering::SeqCst) {
            let _ = std::fs::remove_file(&dest);
            return Err(DownloadError::Cancelled.into());
        }
    }

    let tags_basename = model.tags_basename();
    let tags_dest = model_dir.join(&tags_basename);
    download_file(
        app,
        &client,
        &hf_url(&model.tags_filename),
        &tags_dest,
        &tags_basename,
    )
    .await?;

    let _ = app.emit(
        DOWNLOAD_EVENT,
        DownloadProgress::done(format!("模型 {} 下载完成", model.name)),
    );

    let _ = app.emit(
        "tagger-progress",
        ProgressEvent::new("success", format!("模型 {} 下载完成", model.name)),
    );

    Ok(())
}

/// 下载单个文件。取消时发 cancelled 事件，其余失败发 error 事件（前端据此记"下载失败"）
async fn download_file(
    app: &tauri::AppHandle,
    client: &reqwest::Client,
    url: &str,
    dest: &std::path::Path,
    label: &str,
) -> Result<(), String> {
    let _ = app.emit(DOWNLOAD_EVENT, DownloadProgress::starting(label));
    let request = crate::commands::huggingface_config::apply_huggingface_auth(client.get(url));
    let result =
        http_download::download_to_file(request, dest, label, &DOWNLOAD_CANCELLED, |progress| {
            let _ = app.emit(DOWNLOAD_EVENT, progress);
        })
        .await;

    let err = match result {
        Ok(_) => return Ok(()),
        Err(e) => e,
    };
    let message = match &err {
        DownloadError::Status { status, .. } if matches!(status.as_u16(), 401 | 403) => {
            "Hugging Face 访问被拒绝，请先在设置中保存 Access Token，并确认已在模型页面同意协议。"
                .to_string()
        }
        _ => err.to_string(),
    };
    let event = if matches!(err, DownloadError::Cancelled) {
        DownloadProgress::cancelled(message.clone())
    } else {
        DownloadProgress::error(message.clone())
    };
    let _ = app.emit(DOWNLOAD_EVENT, event);
    Err(message)
}
