use futures_util::{stream::FuturesUnordered, StreamExt};
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use super::{ProcessResult, ProgressEvent, FAIL_DIR_NAME, WARN_DIR_NAME};

pub(crate) enum ItemOutcome {
    Done { message: String, warning: bool },
    Failed { filename: String, message: String },
}

#[derive(Default)]
pub(crate) struct BatchOutcome {
    pub success: u32,
    pub fail: u32,
    pub errors: Vec<String>,
    pub error_files: Vec<PathBuf>,
    pub warning_files: Vec<PathBuf>,
    pub cancelled: bool,
}

pub(crate) async fn until_cancelled(cancel: &AtomicBool) {
    while !cancel.load(Ordering::SeqCst) {
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    }
}

pub(crate) async fn run_file_batch<R, F, Fut>(
    app: &tauri::AppHandle<R>,
    event: &str,
    files: &[PathBuf],
    concurrency: usize,
    cancel: &'static AtomicBool,
    work: F,
) -> BatchOutcome
where
    R: tauri::Runtime,
    F: Fn(PathBuf) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = ItemOutcome> + Send + 'static,
{
    let semaphore = Arc::new(tokio::sync::Semaphore::new(concurrency.max(1)));
    let work = Arc::new(work);
    let mut tasks = FuturesUnordered::new();
    for path in files {
        if cancel.load(Ordering::SeqCst) {
            break;
        }
        let (sem, work, file) = (semaphore.clone(), work.clone(), path.clone());
        let handle = tokio::spawn(async move {
            let _permit = sem.acquire().await.ok()?;
            if cancel.load(Ordering::SeqCst) {
                return None;
            }
            let result = tokio::select! {
                result = work(file) => result,
                _ = until_cancelled(cancel) => return None,
            };
            // 节流被取消时也可能返回错误，不能计入失败或复制进 Fail/。
            // 在 worker 完成时判断，之后的取消不丢弃已经完成的条目。
            if cancel.load(Ordering::SeqCst) {
                None
            } else {
                Some(result)
            }
        });
        tasks.push(async move { (path, handle.await) });
    }

    let mut out = BatchOutcome::default();
    while let Some((path, result)) = tasks.next().await {
        let result = match result {
            Ok(Some(result)) => result,
            Ok(None) => continue,
            Err(_) if cancel.load(Ordering::SeqCst) => continue,
            Err(error) => ItemOutcome::Failed {
                filename: super::file_name_lossy(path),
                message: format!("任务执行失败: {}", error),
            },
        };
        let filename = super::file_name_lossy(path);
        let (status, message) = match result {
            ItemOutcome::Done { message, warning } => {
                out.success += 1;
                if warning {
                    out.warning_files.push(path.clone());
                }
                (if warning { "warning" } else { "success" }, message)
            }
            ItemOutcome::Failed { filename, message } => {
                out.fail += 1;
                out.error_files.push(path.clone());
                out.errors.push(format!("{}: {}", filename, message));
                ("error", format!("[错误] {}: {}", filename, message))
            }
        };
        ProgressEvent::new(status, message)
            .at(out.success + out.fail, files.len() as u32)
            .file(filename)
            .emit(app, event);
    }
    out.cancelled = cancel.load(Ordering::SeqCst);
    out
}

pub(crate) fn archive_problem_files(
    input: &Path,
    output: &Path,
    recursive: bool,
    out: &BatchOutcome,
    skip_warnings: bool,
) -> String {
    let mut message = String::new();
    for (files, directory, label, skip) in [
        (&out.error_files, FAIL_DIR_NAME, "错误", false),
        (&out.warning_files, WARN_DIR_NAME, "警告", skip_warnings),
    ] {
        if files.is_empty() {
            continue;
        }
        if skip {
            message.push_str(&format!("，输出与输入目录相同，已跳过 {}/ 复制", directory));
            continue;
        }
        match super::copy_files_into_artifact_dir(input, output, files, directory, recursive) {
            Ok(copied) => message.push_str(&format!(
                "，{} 个{}文件已复制到 {}/",
                copied, label, directory
            )),
            Err(error) => message.push_str(&format!("，{}", error)),
        }
    }
    message
}

pub(crate) fn finish_batch<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    event: &str,
    label: &str,
    total: u32,
    out: BatchOutcome,
    extra: &str,
) -> ProcessResult {
    let message = if out.cancelled {
        format!(
            "已取消: 成功 {}, 失败 {}, 共处理 {}/{}{}",
            out.success,
            out.fail,
            out.success + out.fail,
            total,
            extra
        )
    } else {
        format!(
            "{}: 成功 {}, 失败 {}, 共 {}{}",
            label, out.success, out.fail, total, extra
        )
    };
    ProgressEvent::new("done", message)
        .at(total, total)
        .emit(app, event);
    ProcessResult {
        success_count: out.success,
        fail_count: out.fail,
        total,
        errors: out.errors,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::batch::capture_events;
    use crate::commands::llm_client::test_support::TempDir;
    use std::sync::atomic::AtomicUsize;

    const EVENT: &str = "llm-batch-test";

    #[tokio::test]
    async fn reports_success_warning_failure_and_one_terminal_event() {
        static CANCEL: AtomicBool = AtomicBool::new(false);
        let app = tauri::test::mock_app();
        let events = capture_events(app.handle(), EVENT);
        let files = ["a.txt", "警告 [完成].txt", "c.txt"].map(PathBuf::from);
        let out = run_file_batch(app.handle(), EVENT, &files, 1, &CANCEL, |path| async move {
            let filename = super::super::file_name_lossy(&path);
            if filename == "c.txt" {
                ItemOutcome::Failed {
                    filename,
                    message: "bad reply".into(),
                }
            } else {
                ItemOutcome::Done {
                    message: filename.clone(),
                    warning: filename != "a.txt",
                }
            }
        })
        .await;
        assert_eq!((out.success, out.fail), (2, 1));
        assert_eq!(out.warning_files, [files[1].clone()]);
        assert_eq!(out.error_files, [files[2].clone()]);
        let result = finish_batch(app.handle(), EVENT, "标签排序完成", 3, out, "");
        assert_eq!(result.errors, ["c.txt: bad reply"]);
        let events = events.lock().unwrap();
        assert_eq!(events.len(), 4);
        for (index, status) in ["success", "warning", "error", "done"].iter().enumerate() {
            assert_eq!(events[index]["status"], *status);
        }
        assert_eq!(events[1]["filename"], "警告 [完成].txt");
        assert_eq!(events[2]["filename"], "c.txt");
        assert_eq!(events[2]["message"], "[错误] c.txt: bad reply");
        assert_eq!(events[3]["message"], "标签排序完成: 成功 2, 失败 1, 共 3");
    }

    #[tokio::test]
    async fn enforces_concurrency_limit() {
        static CANCEL: AtomicBool = AtomicBool::new(false);
        let app = tauri::test::mock_app();
        let active = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        let (active_work, peak_work) = (active.clone(), peak.clone());
        let files: Vec<_> = (0..8).map(|n| PathBuf::from(n.to_string())).collect();
        let out = run_file_batch(app.handle(), EVENT, &files, 2, &CANCEL, move |_| {
            let (active, peak) = (active_work.clone(), peak_work.clone());
            async move {
                let current = active.fetch_add(1, Ordering::SeqCst) + 1;
                peak.fetch_max(current, Ordering::SeqCst);
                tokio::time::sleep(std::time::Duration::from_millis(5)).await;
                active.fetch_sub(1, Ordering::SeqCst);
                ItemOutcome::Done {
                    message: "ok".into(),
                    warning: false,
                }
            }
        })
        .await;
        assert_eq!(out.success, 8);
        assert_eq!(peak.load(Ordering::SeqCst), 2);
        assert_eq!(active.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn cancellation_discards_late_errors_and_does_not_start_queued_work() {
        static CANCEL: AtomicBool = AtomicBool::new(false);
        let app = tauri::test::mock_app();
        let events = capture_events(app.handle(), EVENT);
        let started = Arc::new(AtomicUsize::new(0));
        let started_work = started.clone();
        let files = [PathBuf::from("a"), PathBuf::from("b")];
        let out = run_file_batch(app.handle(), EVENT, &files, 1, &CANCEL, move |_| {
            let started = started_work.clone();
            async move {
                started.fetch_add(1, Ordering::SeqCst);
                CANCEL.store(true, Ordering::SeqCst);
                ItemOutcome::Failed {
                    filename: "a".into(),
                    message: "已取消".into(),
                }
            }
        })
        .await;
        assert!(out.cancelled);
        assert_eq!((out.success, out.fail), (0, 0));
        assert!(out.error_files.is_empty());
        assert_eq!(started.load(Ordering::SeqCst), 1);
        finish_batch(app.handle(), EVENT, "完成", 2, out, "");
        let events = events.lock().unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0]["message"], "已取消: 成功 0, 失败 0, 共处理 0/2");
    }

    #[tokio::test]
    async fn cancels_in_flight_work() {
        static CANCEL: AtomicBool = AtomicBool::new(false);
        let app = tauri::test::mock_app();
        let files = [PathBuf::from("a")];
        let batch = run_file_batch(app.handle(), EVENT, &files, 1, &CANCEL, |_| async {
            std::future::pending().await
        });
        let cancel = async {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            CANCEL.store(true, Ordering::SeqCst);
        };
        let (out, ()) = tokio::time::timeout(std::time::Duration::from_secs(2), async {
            tokio::join!(batch, cancel)
        })
        .await
        .unwrap();
        assert!(out.cancelled);
        assert_eq!((out.success, out.fail), (0, 0));
    }

    #[tokio::test]
    async fn cancellation_keeps_previously_finished_items() {
        static CANCEL: AtomicBool = AtomicBool::new(false);
        let app = tauri::test::mock_app();
        let files = [
            PathBuf::from("done"),
            PathBuf::from("cancel"),
            PathBuf::from("queued"),
        ];
        let out = run_file_batch(app.handle(), EVENT, &files, 1, &CANCEL, |path| async move {
            if path == Path::new("cancel") {
                CANCEL.store(true, Ordering::SeqCst);
            }
            assert_ne!(path, Path::new("queued"));
            ItemOutcome::Done {
                message: "ok".into(),
                warning: false,
            }
        })
        .await;
        assert!(out.cancelled);
        assert_eq!((out.success, out.fail), (1, 0));
    }

    #[tokio::test]
    async fn pre_cancelled_batch_never_starts_work() {
        static CANCEL: AtomicBool = AtomicBool::new(true);
        let app = tauri::test::mock_app();
        let files = [PathBuf::from("a")];
        let out = run_file_batch(app.handle(), EVENT, &files, 1, &CANCEL, |_| async {
            panic!("cancelled work must not start")
        })
        .await;
        assert!(out.cancelled);
        assert_eq!((out.success, out.fail), (0, 0));
    }

    #[tokio::test]
    async fn worker_panic_is_reported_as_failure_with_filename() {
        static CANCEL: AtomicBool = AtomicBool::new(false);
        let app = tauri::test::mock_app();
        let events = capture_events(app.handle(), EVENT);
        let files = [PathBuf::from("a.txt")];
        let out = run_file_batch(app.handle(), EVENT, &files, 1, &CANCEL, |_| async {
            panic!("mock worker panic")
        })
        .await;
        assert_eq!((out.success, out.fail), (0, 1));
        assert_eq!(out.error_files, files);
        assert!(out.errors[0].contains("a.txt: 任务执行失败"));
        let events = events.lock().unwrap();
        assert_eq!(events[0]["status"], "error");
        assert_eq!(events[0]["filename"], "a.txt");
    }

    #[test]
    fn archives_failures_recursively_and_skips_in_place_warnings() {
        let root = TempDir::new("llm_archive");
        let sub = root.join("nested");
        std::fs::create_dir_all(&sub).unwrap();
        let failed = sub.join("failed.txt");
        let warning = sub.join("warning.txt");
        std::fs::write(&failed, "bad").unwrap();
        std::fs::write(&warning, "warn").unwrap();
        let out = BatchOutcome {
            error_files: vec![failed],
            warning_files: vec![warning],
            ..Default::default()
        };
        let message = archive_problem_files(&root, &root, true, &out, true);
        assert!(root.join("Fail/nested/failed.txt").is_file());
        assert!(!root.join("Warn").exists());
        assert_eq!(
            message,
            "，1 个错误文件已复制到 Fail/，输出与输入目录相同，已跳过 Warn/ 复制"
        );
        let output = root.join("out");
        archive_problem_files(&root, &output, true, &out, false);
        assert!(output.join("Warn/nested/warning.txt").is_file());
    }
}
