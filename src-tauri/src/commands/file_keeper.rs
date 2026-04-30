use serde::{Deserialize, Serialize};
use std::path::Path;
use tauri::Emitter;

use super::{ProcessResult, ProgressEvent};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileKeeperOptions {
    pub folder_path: String,
    pub keep_extensions: Vec<String>,
}

#[tauri::command]
pub async fn keep_specified_files(app: tauri::AppHandle, options: FileKeeperOptions) -> Result<ProcessResult, String> {
    tokio::task::spawn_blocking(move || {
        keep_files_sync(&app, &options)
    })
    .await
    .map_err(|e| format!("任务执行失败: {}", e))?
}

fn keep_files_sync(app: &tauri::AppHandle, options: &FileKeeperOptions) -> Result<ProcessResult, String> {
    let folder = Path::new(&options.folder_path);
    if !folder.exists() || !folder.is_dir() {
        return Err(format!("文件夹不存在: {}", options.folder_path));
    }

    let mut all_files = Vec::new();
    for entry in walkdir::WalkDir::new(folder)
        .max_depth(1)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        let p = entry.path();
        if p.is_file() {
            all_files.push(p.to_path_buf());
        }
    }

    all_files.sort_by(|a, b| a.file_name().cmp(&b.file_name()));

    let total = all_files.len() as u32;
    let mut success_count = 0u32;
    let mut fail_count = 0u32;
    let mut kept_count = 0u32;
    let mut errors = Vec::new();

    let keep_exts: Vec<String> = options.keep_extensions.iter().map(|e| e.to_lowercase()).collect();

    let _ = app.emit("keeper-progress", ProgressEvent {
        current: 0,
        total,
        filename: String::new(),
        status: "processing".to_string(),
        message: format!("开始处理: 共 {} 个文件, 保留后缀: {}", total, keep_exts.join(", ")),
    });

    for (i, file_path) in all_files.iter().enumerate() {
        let filename = file_path.file_name().unwrap_or_default().to_string_lossy().to_string();
        let ext = file_path.extension()
            .map(|e| e.to_string_lossy().to_lowercase())
            .unwrap_or_default();

        let should_keep = keep_exts.contains(&ext);

        if should_keep {
            kept_count += 1;
            let _ = app.emit("keeper-progress", ProgressEvent {
                current: i as u32 + 1,
                total,
                filename: filename.clone(),
                status: "success".to_string(),
                message: format!("[保留] {} (.{})", filename, ext),
            });
        } else {
            match std::fs::remove_file(file_path) {
                Ok(_) => {
                    success_count += 1;
                    let _ = app.emit("keeper-progress", ProgressEvent {
                        current: i as u32 + 1,
                        total,
                        filename: filename.clone(),
                        status: "success".to_string(),
                        message: format!("[删除] {} (.{})", filename, ext),
                    });
                }
                Err(e) => {
                    fail_count += 1;
                    let err_msg = format!("{}: {}", filename, e);
                    errors.push(err_msg.clone());
                    let _ = app.emit("keeper-progress", ProgressEvent {
                        current: i as u32 + 1,
                        total,
                        filename: filename.clone(),
                        status: "error".to_string(),
                        message: format!("[错误] {}", err_msg),
                    });
                }
            }
        }
    }

    let _ = app.emit("keeper-progress", ProgressEvent {
        current: total,
        total,
        filename: String::new(),
        status: "done".to_string(),
        message: format!("完成: 保留 {} 个, 删除 {} 个, 失败 {} 个, 共 {} 个文件", kept_count, success_count, fail_count, total),
    });

    Ok(ProcessResult { success_count, fail_count, total, errors })
}
