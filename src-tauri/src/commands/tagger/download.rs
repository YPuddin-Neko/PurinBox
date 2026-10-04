use std::sync::atomic::{AtomicBool, Ordering};
use tauri::Emitter;

use super::models::{basename, ModelDefinition};
use super::{get_model_dir, ProgressEvent};
use crate::commands::http_download::{
    self, DownloadError, DownloadFile, DownloadFilesOptions, DownloadProgress,
};
use crate::commands::huggingface_config::apply_huggingface_auth;

/// 下载进度事件（独立于打标进度）
const DOWNLOAD_EVENT: &str = "tagger-download";

/// 全局下载取消标志
static DOWNLOAD_CANCELLED: AtomicBool = AtomicBool::new(false);

const HF_ACCESS_DENIED: &str =
    "Hugging Face 访问被拒绝，请先在设置中保存 Access Token，并确认已在模型页面同意协议。";

/// 取消下载
pub fn cancel_download() {
    DOWNLOAD_CANCELLED.store(true, Ordering::SeqCst);
}

/// 从 HuggingFace 依次下载模型文件：权重固定存为 model.onnx，额外文件（例如 ONNX external data:
/// model.onnx.data）和词表按原文件名存放。失败或取消时发一条终态下载事件并返回 Err，取消的文案以「已取消」开头
pub async fn download_model(app: &tauri::AppHandle, model: &ModelDefinition) -> Result<(), String> {
    DOWNLOAD_CANCELLED.store(false, Ordering::SeqCst);

    let model_dir = get_model_dir(&model.id);
    std::fs::create_dir_all(&model_dir).map_err(|e| format!("创建模型目录失败: {}", e))?;

    ProgressEvent::new("info", format!("开始下载模型: {}", model.name))
        .emit(app, super::inference::EVENT);

    let client = http_download::download_client()?;
    let files = std::iter::once((model.model_filename.as_str(), "model.onnx".to_string()))
        .chain(model.extra_files.iter().map(|f| (f.as_str(), basename(f))))
        .chain(std::iter::once((
            model.tags_filename.as_str(),
            model.tags_basename(),
        )))
        .map(|(remote, local)| {
            let url = http_download::huggingface_url(&model.repo_id, remote);
            DownloadFile::new(
                apply_huggingface_auth(client.get(url)),
                model_dir.join(&local),
                local,
            )
        })
        .collect();
    let downloaded = http_download::download_files(
        files,
        DownloadFilesOptions::default(),
        &DOWNLOAD_CANCELLED,
        |progress| {
            let _ = app.emit(DOWNLOAD_EVENT, progress);
        },
    )
    .await;
    if let Err(err) = downloaded {
        let event = failure_event(&err, &DOWNLOAD_CANCELLED);
        let message = event.message.clone();
        let _ = app.emit(DOWNLOAD_EVENT, event);
        return Err(message);
    }

    let _ = app.emit(
        DOWNLOAD_EVENT,
        DownloadProgress::done(format!("模型 {} 下载完成", model.name)),
    );
    ProgressEvent::new("success", format!("模型 {} 下载完成", model.name))
        .emit(app, super::inference::EVENT);
    Ok(())
}

/// 下载失败的终态事件（前端据此记「下载失败」或「已取消」）；HTTP 401/403 换成配置 Access Token 的提示
fn failure_event(err: &DownloadError, cancel: &AtomicBool) -> DownloadProgress {
    let event = DownloadProgress::from_error(err, cancel);
    match err {
        DownloadError::Status { status, .. }
            if event.status == "error" && matches!(status.as_u16(), 401 | 403) =>
        {
            event.with_message(HF_ACCESS_DENIED)
        }
        _ => event,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failures_map_to_cancelled_or_error_events() {
        let idle = AtomicBool::new(false);
        let denied = DownloadError::Status {
            status: reqwest::StatusCode::FORBIDDEN,
            url: "https://huggingface.co/x".into(),
        };
        let event = failure_event(&denied, &idle);
        assert_eq!(
            (event.status.as_str(), event.message.as_str()),
            ("error", HF_ACCESS_DENIED)
        );

        let missing = DownloadError::Status {
            status: reqwest::StatusCode::NOT_FOUND,
            url: "https://huggingface.co/x".into(),
        };
        let event = failure_event(&missing, &idle);
        assert_eq!(event.status, "error");
        assert!(event.message.contains("404"), "{}", event.message);

        let cancelled = failure_event(&DownloadError::Cancelled, &idle);
        assert_eq!(cancelled.status, "cancelled");
        assert!(cancelled.message.starts_with("已取消"));

        // 用户点过取消时，同时发生的网络错误也按取消处理，不提示去配 Token
        let cancel = AtomicBool::new(true);
        let event = failure_event(&denied, &cancel);
        assert_eq!(event.status, "cancelled");
        assert!(event.message.starts_with("已取消"));
    }
}
