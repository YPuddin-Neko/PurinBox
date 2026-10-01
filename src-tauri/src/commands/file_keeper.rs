use serde::{Deserialize, Serialize};
use std::path::Path;

use super::{ProcessResult, ProgressEvent};

use super::batch::{BatchJob, FileBatch, FileOutcome};

static JOB: BatchJob = BatchJob::new("文件保留");

#[cfg(test)]
mod tests {
    use super::*;
    use tauri::Listener;

    #[test]
    fn cancellation_after_last_file_emits_one_done() {
        let root = super::super::image_io::test_dir("keeper_cancel");
        std::fs::write(root.join("keep.txt"), b"keep").unwrap();
        let app = tauri::test::mock_app();
        let log = super::super::batch::capture_events(app.handle(), "keeper-progress");
        app.listen_any("keeper-progress", |event| {
            let payload: serde_json::Value = serde_json::from_str(event.payload()).unwrap();
            if payload["filename"] == "keep.txt" {
                JOB.cancel();
            }
        });
        let result = keep_files_sync(
            app.handle(),
            &FileKeeperOptions {
                folder_path: root.to_string_lossy().into_owned(),
                keep_extensions: vec!["txt".into()],
            },
        )
        .unwrap();
        JOB.cancel_flag()
            .store(false, std::sync::atomic::Ordering::SeqCst);
        assert_eq!(result.total, 1);
        let events = log.lock().unwrap();
        assert_eq!(events.iter().filter(|e| e["status"] == "done").count(), 1);
        assert_eq!(events.last().unwrap()["message"], "已取消: 已处理 1, 共 1");
        assert_eq!(std::fs::read(root.join("keep.txt")).unwrap(), b"keep");
        std::fs::remove_dir_all(root).unwrap();
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileKeeperOptions {
    pub folder_path: String,
    pub keep_extensions: Vec<String>,
}

#[tauri::command]
pub async fn keep_specified_files(
    app: tauri::AppHandle,
    options: FileKeeperOptions,
) -> Result<ProcessResult, String> {
    JOB.run(move || keep_files_sync(&app, &options)).await
}

#[tauri::command]
pub fn cancel_keeper() {
    JOB.cancel();
}

fn keep_files_sync<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    options: &FileKeeperOptions,
) -> Result<ProcessResult, String> {
    let folder = Path::new(&options.folder_path);
    if !folder.is_dir() {
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
    let keep_exts: Vec<String> = options
        .keep_extensions
        .iter()
        .map(|e| e.to_lowercase())
        .collect();
    ProgressEvent::new(
        "processing",
        format!(
            "开始处理: 共 {} 个文件, 保留后缀: {}",
            total,
            keep_exts.join(", ")
        ),
    )
    .at(0, total)
    .emit(app, "keeper-progress");
    Ok(FileBatch::new(app, "keeper-progress", JOB.cancel_flag())
        .no_processing()
        .error_prefix("[错误] ")
        .run(
            &all_files,
            |item| {
                let ext = item
                    .path
                    .extension()
                    .map(|e| e.to_string_lossy().to_lowercase())
                    .unwrap_or_default();
                if keep_exts.contains(&ext) {
                    return Ok(FileOutcome::skipped(format!(
                        "[保留] {} (.{})",
                        item.name, ext
                    )));
                }
                std::fs::remove_file(item.path).map_err(|e| e.to_string())?;
                Ok(FileOutcome::done(format!(
                    "[删除] {} (.{})",
                    item.name, ext
                )))
            },
            |c| {
                format!(
                    "完成: 保留 {} 个, 删除 {} 个, 失败 {} 个, 共 {} 个文件",
                    c.skipped, c.success, c.failed, c.total
                )
            },
        ))
}
