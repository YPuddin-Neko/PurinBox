use serde::{Deserialize, Serialize};
use std::path::Path;

use super::{ProcessResult, ProgressEvent};

use super::batch::{BatchJob, FileBatch, FileOutcome, RunEvents};

static JOB: BatchJob = BatchJob::new("文件保留");

const EVENT: &str = "keeper-progress";

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

/// 失败的只有删不掉的文件，它们原样留在文件夹里，所以不复制进 Fail/
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
    let run = RunEvents::begin(app, EVENT);
    run.emit(
        ProgressEvent::new(
            "processing",
            format!(
                "开始处理: 共 {} 个文件, 保留后缀: {}",
                total,
                keep_exts.join(", ")
            ),
        )
        .at(0, total),
    );
    Ok(FileBatch::new(app, EVENT, JOB.cancel_flag())
        .in_run(run.run_id())
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::batch::capture_raw_events;
    use crate::commands::test_support::TempDir;
    use tauri::Listener;

    fn options(root: &Path, keep: &str) -> FileKeeperOptions {
        FileKeeperOptions {
            folder_path: root.to_string_lossy().into_owned(),
            keep_extensions: vec![keep.into()],
        }
    }

    /// 两个场景共用 `JOB` 的取消标志，放在同一个测试里依次跑，免得并行时互相取消
    #[test]
    fn start_event_shares_the_run_and_cancel_after_last_file_emits_one_done() {
        // 「开始处理」与后面的逐文件事件、done 属于同一轮
        let root = TempDir::new("keeper_run");
        std::fs::write(root.join("a.txt"), b"keep").unwrap();
        std::fs::write(root.join("b.tmp"), b"drop").unwrap();
        let app = tauri::test::mock_app();
        let log = capture_raw_events(app.handle(), EVENT);
        let result = keep_files_sync(app.handle(), &options(&root, "TXT")).unwrap();
        assert_eq!((result.success_count, result.fail_count), (1, 0));
        assert!(root.join("a.txt").exists() && !root.join("b.tmp").exists());
        {
            let events = log.lock().unwrap();
            let run_id = events[0]["run_id"].as_u64().unwrap();
            assert!(events[0]["message"]
                .as_str()
                .unwrap()
                .starts_with("开始处理"));
            assert!(events.iter().all(|e| e["run_id"] == run_id));
            assert_eq!(
                events.last().unwrap()["message"],
                "完成: 保留 1 个, 删除 1 个, 失败 0 个, 共 2 个文件"
            );
        }

        // 处理最后一个文件时点取消：恰好一条带 cancelled 的 done
        let root = TempDir::new("keeper_cancel");
        std::fs::write(root.join("keep.txt"), b"keep").unwrap();
        let app = tauri::test::mock_app();
        let log = capture_raw_events(app.handle(), EVENT);
        app.listen_any(EVENT, |event| {
            let payload: serde_json::Value = serde_json::from_str(event.payload()).unwrap();
            if payload["filename"] == "keep.txt" {
                JOB.cancel();
            }
        });
        let result = keep_files_sync(app.handle(), &options(&root, "txt")).unwrap();
        JOB.cancel_flag()
            .store(false, std::sync::atomic::Ordering::SeqCst);
        assert_eq!(result.total, 1);
        let events = log.lock().unwrap();
        assert_eq!(events.iter().filter(|e| e["status"] == "done").count(), 1);
        let done = events.last().unwrap();
        assert_eq!(done["message"], "已取消: 已处理 1/1, 成功 0, 失败 0");
        assert_eq!(done["cancelled"], true);
        assert_eq!(std::fs::read(root.join("keep.txt")).unwrap(), b"keep");
    }
}
