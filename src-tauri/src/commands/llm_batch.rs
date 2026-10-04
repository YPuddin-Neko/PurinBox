//! LLM 批任务（标签细化、标签排序等）的并发驱动：信号量限并发、逐条发事件、取消即停，
//! 收尾时归集问题文件并发唯一的终态 done（见 `BatchOutcome::finish`）。

use futures_util::{stream::FuturesUnordered, StreamExt};
use std::future::Future;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use super::batch::{finish_run, BatchCounts};
use super::{ProblemArchive, ProcessResult, ProgressEvent};

pub(crate) enum ItemOutcome {
    Done { message: String, warning: bool },
    Failed { filename: String, message: String },
}

impl ItemOutcome {
    /// 处理成功的一项：`summary` 后接警告（" ⚠ a; b"，此时按警告计）；
    /// 没有警告时再接 `unchanged_note`——结果与原内容相同时由调用方给出，如 " (未变化)"
    pub(crate) fn completed(
        summary: String,
        warnings: &[String],
        unchanged_note: Option<&str>,
    ) -> Self {
        let warning = !warnings.is_empty();
        let mut message = summary;
        if warning {
            message.push_str(" ⚠ ");
            message.push_str(&warnings.join("; "));
        } else if let Some(note) = unchanged_note {
            message.push_str(note);
        }
        ItemOutcome::Done { message, warning }
    }
}

#[derive(Default)]
pub(crate) struct BatchOutcome {
    pub success: u32,
    pub fail: u32,
    /// 本批文件总数（含取消后没开始的）
    pub total: u32,
    pub errors: Vec<String>,
    pub error_files: Vec<PathBuf>,
    pub warning_files: Vec<PathBuf>,
    pub cancelled: bool,
}

impl BatchOutcome {
    /// 交给收尾文案的计数；`ItemOutcome::Done` 里的跳过已计入 success
    pub(crate) fn counts(&self) -> BatchCounts {
        BatchCounts {
            success: self.success,
            failed: self.fail,
            total: self.total,
            ..Default::default()
        }
    }

    /// 收尾：失败文件复制进 Fail/、警告文件复制进 Warn/（各发一条事件，见 `ProblemArchive::archive`），
    /// 再发唯一的终态 done——取消时是统一的取消文案并带 `cancelled: true`，
    /// 否则是 "{label}: 成功 {s}, 失败 {f}, 共 {total}"。
    ///
    /// 用法：`outcome.finish(&app, EVENT, "标签排序完成", &ProblemArchive::new(input, output, recursive))`
    pub(crate) fn finish<R: tauri::Runtime>(
        self,
        app: &tauri::AppHandle<R>,
        event: &str,
        label: &str,
        archive: &ProblemArchive<'_>,
    ) -> ProcessResult {
        archive.report(app, event, &self.error_files, &self.warning_files);
        finish_run(app, event, &self.counts(), self.cancelled, |c| {
            c.summary(label)
        });
        self.into_result()
    }

    fn into_result(self) -> ProcessResult {
        ProcessResult {
            success_count: self.success,
            fail_count: self.fail,
            total: self.total,
            errors: self.errors,
        }
    }
}

async fn until_cancelled(cancel: &AtomicBool) {
    while !cancel.load(Ordering::SeqCst) {
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    }
}

/// 以至多 `concurrency` 个并发处理 `files`，每条结果按完成顺序发一条事件
/// （success / warning / error "[错误] {文件名}: {原因}"，current 为已完成数）。
///
/// 取消后不再启动新条目，进行中的条目立即放弃。取消之后才返回的失败多半由取消本身引起
/// （如节流等待被打断），不计入失败也不进 Fail/；已经完成的条目（文件可能已写盘）照常计数。
///
/// 不开始新一轮运行：命令在发第一条事件之前自己调用 `begin_run`。
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
            // biased：结果和取消同时就绪时先收结果，已写盘的条目不被丢掉
            let result = tokio::select! {
                biased;
                result = work(file) => result,
                _ = until_cancelled(cancel) => return None,
            };
            match result {
                ItemOutcome::Failed { .. } if cancel.load(Ordering::SeqCst) => None,
                result => Some(result),
            }
        });
        tasks.push(async move { (path, handle.await) });
    }

    let mut out = BatchOutcome {
        total: files.len() as u32,
        ..Default::default()
    };
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::batch::{capture_events, capture_raw_events};
    use crate::commands::test_support::TempDir;
    use serde_json::json;
    use std::path::Path;
    use std::sync::atomic::AtomicUsize;

    const EVENT: &str = "llm-batch-test";

    #[tokio::test]
    async fn reports_success_warning_failure_and_one_terminal_event() {
        static CANCEL: AtomicBool = AtomicBool::new(false);
        let app = tauri::test::mock_app();
        let events = capture_events(app.handle(), EVENT);
        let root = TempDir::new("llm_batch_report");
        let files = ["a.txt", "警告 [完成].txt", "c.txt"].map(|name| root.join(name));
        for file in &files {
            std::fs::write(file, "tags").unwrap();
        }
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
        assert_eq!((out.success, out.fail, out.total), (2, 1, 3));
        assert_eq!(out.warning_files, [files[1].clone()]);
        assert_eq!(out.error_files, [files[2].clone()]);
        let archive = ProblemArchive::new(&root, &root, false).skip_warnings(true);
        let result = out.finish(app.handle(), EVENT, "标签排序完成", &archive);
        assert_eq!(result.errors, ["c.txt: bad reply"]);
        assert!(root.join("Fail/c.txt").is_file());
        let events = events.lock().unwrap();
        let statuses: Vec<_> = events.iter().map(|e| e["status"].clone()).collect();
        assert_eq!(
            statuses,
            ["success", "warning", "error", "info", "info", "done"]
        );
        assert_eq!(events[1]["filename"], "警告 [完成].txt");
        assert_eq!(events[2]["filename"], "c.txt");
        assert_eq!(events[2]["message"], "[错误] c.txt: bad reply");
        assert_eq!(events[5]["message"], "标签排序完成: 成功 2, 失败 1, 共 3");
    }

    #[test]
    fn completed_appends_warnings_or_unchanged_note() {
        let done = |outcome: ItemOutcome| match outcome {
            ItemOutcome::Done { message, warning } => (message, warning),
            ItemOutcome::Failed { .. } => panic!("应为完成"),
        };
        let warnings = ["缺失: a".to_string(), "新增: b".to_string()];
        assert_eq!(
            done(ItemOutcome::completed(
                "[完成] x".into(),
                &warnings,
                Some(" (未变化)")
            )),
            ("[完成] x ⚠ 缺失: a; 新增: b".to_string(), true)
        );
        assert_eq!(
            done(ItemOutcome::completed(
                "[完成] x".into(),
                &[],
                Some(" (未变化)")
            )),
            ("[完成] x (未变化)".to_string(), false)
        );
        assert_eq!(
            done(ItemOutcome::completed("[完成] x".into(), &[], None)),
            ("[完成] x".to_string(), false)
        );
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
        let root = TempDir::new("llm_batch_cancel");
        out.finish(
            app.handle(),
            EVENT,
            "完成",
            &ProblemArchive::new(&root, &root, false),
        );
        let events = events.lock().unwrap();
        assert_eq!(
            *events,
            [json!({
                "current": 0, "total": 2, "filename": "", "status": "done",
                "message": "已取消: 已处理 0/2, 成功 0, 失败 0", "cancelled": true,
            })]
        );
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

    /// 取消那一刻刚完成的条目（文件已写盘）照常计数、发事件；排队中的不再开始
    #[tokio::test]
    async fn cancellation_keeps_finished_items() {
        static CANCEL: AtomicBool = AtomicBool::new(false);
        let app = tauri::test::mock_app();
        let events = capture_events(app.handle(), EVENT);
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
        assert_eq!((out.success, out.fail), (2, 0));
        let filenames: Vec<_> = events
            .lock()
            .unwrap()
            .iter()
            .map(|e| e["filename"].as_str().unwrap().to_string())
            .collect();
        assert_eq!(filenames, ["done", "cancel"]);
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
        assert_eq!((out.success, out.fail, out.total), (0, 0, 1));
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

    fn problem_files(root: &Path) -> BatchOutcome {
        let sub = root.join("nested");
        std::fs::create_dir_all(&sub).unwrap();
        let failed = sub.join("failed.txt");
        let warning = sub.join("warning.txt");
        std::fs::write(&failed, "bad").unwrap();
        std::fs::write(&warning, "warn").unwrap();
        BatchOutcome {
            success: 1,
            fail: 1,
            total: 3,
            errors: vec!["failed.txt: bad".into()],
            error_files: vec![failed],
            warning_files: vec![warning],
            ..Default::default()
        }
    }

    /// 收尾：问题文件各发一条说明，再发唯一的 done；就地精修不复制 Warn/
    #[test]
    fn finish_archives_problem_files_then_reports_done() {
        let root = TempDir::new("llm_finish");
        let app = tauri::test::mock_app();
        let events = capture_events(app.handle(), EVENT);
        let archive = ProblemArchive::new(&root, &root, true).skip_warnings(true);
        let result = problem_files(&root).finish(app.handle(), EVENT, "标签细化完成", &archive);

        assert!(root.join("Fail/nested/failed.txt").is_file());
        assert!(!root.join("Warn").exists());
        assert_eq!(
            (result.success_count, result.fail_count, result.total),
            (1, 1, 3)
        );
        assert_eq!(result.errors, ["failed.txt: bad"]);
        let info = |message: &str| json!({"current": 0, "total": 0, "filename": "", "status": "info", "message": message});
        assert_eq!(
            *events.lock().unwrap(),
            [
                info("已将 1 个失败文件复制到 Fail/ 文件夹"),
                info("输出与输入目录相同，已跳过 Warn/ 复制"),
                json!({"current": 3, "total": 3, "filename": "", "status": "done",
                       "message": "标签细化完成: 成功 1, 失败 1, 共 3"}),
            ]
        );

        let output = root.join("out");
        let mut cancelled = problem_files(&root);
        cancelled.cancelled = true;
        let app = tauri::test::mock_app();
        let events = capture_events(app.handle(), EVENT);
        cancelled.finish(
            app.handle(),
            EVENT,
            "标签排序完成",
            &ProblemArchive::new(&root, &output, true),
        );
        assert!(output.join("Warn/nested/warning.txt").is_file());
        assert!(output.join("Fail/nested/failed.txt").is_file());
        let events = events.lock().unwrap();
        assert_eq!(events.len(), 3);
        assert_eq!(events[1]["message"], "已将 1 个警告文件复制到 Warn/ 文件夹");
        assert_eq!(
            events[2],
            json!({"current": 2, "total": 3, "filename": "", "status": "done",
                   "message": "已取消: 已处理 2/3, 成功 1, 失败 1", "cancelled": true})
        );
    }

    /// 本批的事件都属于命令开始的那一轮
    #[tokio::test]
    async fn events_carry_the_commands_run_id() {
        const RUN_EVENT: &str = "llm-batch-test-run-id";
        static CANCEL: AtomicBool = AtomicBool::new(false);
        let app = tauri::test::mock_app();
        let events = capture_raw_events(app.handle(), RUN_EVENT);
        let run_id = crate::commands::begin_run(RUN_EVENT);
        let files = [PathBuf::from("a"), PathBuf::from("b")];
        let out = run_file_batch(app.handle(), RUN_EVENT, &files, 2, &CANCEL, |_| async {
            ItemOutcome::Done {
                message: "ok".into(),
                warning: false,
            }
        })
        .await;
        let root = TempDir::new("llm_run_id");
        out.finish(
            app.handle(),
            RUN_EVENT,
            "完成",
            &ProblemArchive::new(&root, &root, false),
        );
        let events = events.lock().unwrap();
        assert_eq!(events.len(), 3);
        assert!(events.iter().all(|e| e["run_id"] == run_id), "{events:?}");
    }
}
