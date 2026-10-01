//! 纯 Rust 逐文件批处理的公共骨架。
//!
//! - [`BatchJob`]：命令级互斥闸 + 取消标志，并把同步任务放进阻塞线程池执行；
//! - [`FileBatch`]：逐文件发 processing / success / error 事件并计数，
//!   结束时恰好发一次终态 done：取消标志已置位发"已取消"，否则发汇总。

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use tauri::{AppHandle, Runtime};

use super::{file_name_lossy, BusyGuard, ProcessResult, ProgressEvent};

/// 一个批处理命令的全局状态，声明成 `static JOB: BatchJob = BatchJob::new("翻转");`
pub(crate) struct BatchJob {
    what: &'static str,
    running: AtomicBool,
    cancel: AtomicBool,
}

impl BatchJob {
    /// `what` 用在互斥报错里："已有{what}任务正在进行，请先等待完成或取消"
    pub(crate) const fn new(what: &'static str) -> Self {
        Self {
            what,
            running: AtomicBool::new(false),
            cancel: AtomicBool::new(false),
        }
    }

    /// 给 `cancel_*` 命令调用
    pub(crate) fn cancel(&self) {
        self.cancel.store(true, Ordering::SeqCst);
    }

    pub(crate) fn cancel_flag(&self) -> &AtomicBool {
        &self.cancel
    }

    /// 占住互斥闸、复位取消标志，再把 `task` 放进阻塞线程池执行。
    /// 已有同类任务在跑时直接返回 BusyGuard 的报错；`task` panic 时返回"任务执行失败: …"。
    pub(crate) async fn run<T, F>(&'static self, task: F) -> Result<T, String>
    where
        T: Send + 'static,
        F: FnOnce() -> Result<T, String> + Send + 'static,
    {
        let _busy = BusyGuard::acquire(&self.running, self.what)?;
        self.cancel.store(false, Ordering::SeqCst);
        tokio::task::spawn_blocking(task)
            .await
            .map_err(|e| format!("任务执行失败: {}", e))?
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OutcomeKind {
    Done,
    Unchanged,
    Skipped,
}

/// 单个文件处理成功后的结果：决定怎么计数，以及发什么 status 和 message
#[derive(Debug)]
pub(crate) struct FileOutcome {
    kind: OutcomeKind,
    status: &'static str,
    message: String,
}

impl FileOutcome {
    /// 已处理，计入成功数
    pub(crate) fn done(message: impl Into<String>) -> Self {
        Self::new(OutcomeKind::Done, message)
    }

    /// 无需处理、已原样输出（如复制到输出目录），计入成功数，同时计入 `BatchCounts::unchanged`
    pub(crate) fn unchanged(message: impl Into<String>) -> Self {
        Self::new(OutcomeKind::Unchanged, message)
    }

    /// 跳过，不计入成功数，计入 `BatchCounts::skipped`
    pub(crate) fn skipped(message: impl Into<String>) -> Self {
        Self::new(OutcomeKind::Skipped, message)
    }

    /// 改发别的 status；三种结果默认都发 "success"
    pub(crate) fn with_status(mut self, status: &'static str) -> Self {
        self.status = status;
        self
    }

    fn new(kind: OutcomeKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            status: "success",
            message: message.into(),
        }
    }
}

/// 交给汇总文案的计数
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct BatchCounts {
    /// done + unchanged，即 `ProcessResult::success_count`
    pub success: u32,
    /// `success` 里原样输出的数量
    pub unchanged: u32,
    pub skipped: u32,
    pub failed: u32,
    pub total: u32,
}

impl BatchCounts {
    /// "{label}: 成功 {success}, 失败 {failed}, 共 {total}"
    pub(crate) fn summary(&self, label: &str) -> String {
        format!(
            "{}: 成功 {}, 失败 {}, 共 {}",
            label, self.success, self.failed, self.total
        )
    }
}

/// 传给逐文件闭包的当前文件
pub(crate) struct BatchItem<'a, R: Runtime> {
    pub path: &'a Path,
    /// 文件名（`file_name_lossy`），事件的 filename 也用它
    pub name: &'a str,
    current: u32,
    total: u32,
    app: &'a AppHandle<R>,
    event: &'a str,
}

impl<R: Runtime> BatchItem<'_, R> {
    /// 以当前文件的 current/total/filename 额外发一条事件
    pub(crate) fn emit(&self, status: &str, message: impl Into<String>) {
        ProgressEvent::new(status, message)
            .at(self.current, self.total)
            .file(self.name)
            .emit(self.app, self.event);
    }
}

/// 逐文件批处理驱动，见 [`FileBatch::run`]
pub(crate) struct FileBatch<'a, R: Runtime> {
    app: &'a AppHandle<R>,
    event: &'a str,
    cancel: &'a AtomicBool,
    processing: Option<&'a str>,
    error_prefix: &'a str,
}

impl<'a, R: Runtime> FileBatch<'a, R> {
    /// 默认：每个文件先发 processing "正在处理: {文件名}"，失败事件不加前缀
    pub(crate) fn new(app: &'a AppHandle<R>, event: &'a str, cancel: &'a AtomicBool) -> Self {
        Self {
            app,
            event,
            cancel,
            processing: Some("正在处理"),
            error_prefix: "",
        }
    }

    /// processing 事件发 "{verb}: {文件名}"
    pub(crate) fn processing(mut self, verb: &'a str) -> Self {
        self.processing = Some(verb);
        self
    }

    /// 不自动发 processing；需要时在闭包里用 `item.emit("processing", …)` 自己发
    pub(crate) fn no_processing(mut self) -> Self {
        self.processing = None;
        self
    }

    /// 失败事件的 message 前缀，如 "[失败] "；`ProcessResult::errors` 里不带前缀
    pub(crate) fn error_prefix(mut self, prefix: &'a str) -> Self {
        self.error_prefix = prefix;
        self
    }

    /// 逐个处理 `files`，事件依次为（current 从 1 起，filename 为文件名）：
    /// - processing（可配置或关闭）、闭包里 `item.emit` 发的事件；
    /// - `Ok(outcome)` → outcome 的 status 和 message；
    /// - `Err(e)` → error "{error_prefix}{文件名}: {e}"，errors 记 "{文件名}: {e}"。
    ///
    /// 每个文件开始前检查取消标志，置位就停。结束时恰好发一次 done（filename 为空）：
    /// 取消标志已置位（包括处理最后一张时才点取消）发"已取消: 已处理 {n}, 共 {total}"，
    /// current 为 n；否则发 `summary(&counts)`，current 为 total。
    pub(crate) fn run<F, S>(self, files: &[PathBuf], mut process: F, summary: S) -> ProcessResult
    where
        F: FnMut(&BatchItem<'_, R>) -> Result<FileOutcome, String>,
        S: FnOnce(&BatchCounts) -> String,
    {
        let total = files.len() as u32;
        let mut counts = BatchCounts {
            total,
            ..Default::default()
        };
        let mut errors = Vec::new();
        let mut processed = 0u32;

        for path in files {
            if self.cancel.load(Ordering::SeqCst) {
                break;
            }
            let name = file_name_lossy(path);
            let item = BatchItem {
                path,
                name: &name,
                current: processed + 1,
                total,
                app: self.app,
                event: self.event,
            };
            if let Some(verb) = self.processing {
                item.emit("processing", format!("{}: {}", verb, name));
            }
            match process(&item) {
                Ok(outcome) => {
                    match outcome.kind {
                        OutcomeKind::Done => counts.success += 1,
                        OutcomeKind::Unchanged => {
                            counts.success += 1;
                            counts.unchanged += 1;
                        }
                        OutcomeKind::Skipped => counts.skipped += 1,
                    }
                    item.emit(outcome.status, outcome.message);
                }
                Err(e) => {
                    counts.failed += 1;
                    let err_msg = format!("{}: {}", name, e);
                    item.emit("error", format!("{}{}", self.error_prefix, err_msg));
                    errors.push(err_msg);
                }
            }
            processed += 1;
        }

        let done = if self.cancel.load(Ordering::SeqCst) {
            ProgressEvent::new(
                "done",
                format!("已取消: 已处理 {}, 共 {}", processed, total),
            )
            .at(processed, total)
        } else {
            ProgressEvent::new("done", summary(&counts)).at(total, total)
        };
        done.emit(self.app, self.event);

        ProcessResult {
            success_count: counts.success,
            fail_count: counts.failed,
            total,
            errors,
        }
    }
}

/// 测试用：收集 `app` 上 `event` 的事件 payload
#[cfg(test)]
pub(crate) fn capture_events<R: Runtime>(
    app: &AppHandle<R>,
    event: &str,
) -> std::sync::Arc<std::sync::Mutex<Vec<serde_json::Value>>> {
    use tauri::Listener;
    let events = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let sink = events.clone();
    app.listen_any(event, move |e| {
        sink.lock()
            .unwrap()
            .push(serde_json::from_str(e.payload()).unwrap());
    });
    events
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};
    use std::cell::Cell;

    const EVENT: &str = "batch-test-progress";

    fn files(names: &[&str]) -> Vec<PathBuf> {
        names.iter().map(|n| Path::new("/data").join(n)).collect()
    }

    fn ev(current: u32, total: u32, filename: &str, status: &str, message: &str) -> Value {
        json!({
            "current": current,
            "total": total,
            "filename": filename,
            "status": status,
            "message": message,
        })
    }

    fn done_count(events: &[Value]) -> usize {
        events.iter().filter(|e| e["status"] == "done").count()
    }

    #[test]
    fn all_success() {
        let app = tauri::test::mock_app();
        let log = capture_events(app.handle(), EVENT);
        let cancel = AtomicBool::new(false);

        let r = FileBatch::new(app.handle(), EVENT, &cancel).run(
            &files(&["a.png", "b.png"]),
            |item| Ok(FileOutcome::done(format!("[翻转] {} ✓", item.name))),
            |c| c.summary("处理完成"),
        );

        assert_eq!(
            *log.lock().unwrap(),
            vec![
                ev(1, 2, "a.png", "processing", "正在处理: a.png"),
                ev(1, 2, "a.png", "success", "[翻转] a.png ✓"),
                ev(2, 2, "b.png", "processing", "正在处理: b.png"),
                ev(2, 2, "b.png", "success", "[翻转] b.png ✓"),
                ev(2, 2, "", "done", "处理完成: 成功 2, 失败 0, 共 2"),
            ]
        );
        assert_eq!((r.success_count, r.fail_count, r.total), (2, 0, 2));
        assert!(r.errors.is_empty());
    }

    #[test]
    fn failures_carry_prefix_only_in_events() {
        let app = tauri::test::mock_app();
        let log = capture_events(app.handle(), EVENT);
        let cancel = AtomicBool::new(false);

        let r = FileBatch::new(app.handle(), EVENT, &cancel)
            .error_prefix("[失败] ")
            .run(
                &files(&["a.png", "b.png"]),
                |item| {
                    if item.name == "b.png" {
                        Err("无法解码图片".to_string())
                    } else {
                        Ok(FileOutcome::done("ok"))
                    }
                },
                |c| c.summary("处理完成"),
            );

        assert_eq!(
            *log.lock().unwrap(),
            vec![
                ev(1, 2, "a.png", "processing", "正在处理: a.png"),
                ev(1, 2, "a.png", "success", "ok"),
                ev(2, 2, "b.png", "processing", "正在处理: b.png"),
                ev(2, 2, "b.png", "error", "[失败] b.png: 无法解码图片"),
                ev(2, 2, "", "done", "处理完成: 成功 1, 失败 1, 共 2"),
            ]
        );
        assert_eq!((r.success_count, r.fail_count, r.total), (1, 1, 2));
        assert_eq!(r.errors, vec!["b.png: 无法解码图片".to_string()]);
    }

    #[test]
    fn skipped_and_unchanged_are_counted_apart() {
        let app = tauri::test::mock_app();
        let log = capture_events(app.handle(), EVENT);
        let cancel = AtomicBool::new(false);

        let r = FileBatch::new(app.handle(), EVENT, &cancel)
            .processing("正在检测")
            .run(
                &files(&["a.png", "b.png", "c.png", "d.png"]),
                |item| {
                    Ok(match item.name {
                        "a.png" => FileOutcome::done("转换"),
                        "b.png" => FileOutcome::unchanged("原样复制"),
                        "c.png" => FileOutcome::skipped("不匹配"),
                        _ => FileOutcome::unchanged("同格式复制").with_status("skipped"),
                    })
                },
                |c| {
                    format!(
                        "成功 {} 原样 {} 跳过 {} 失败 {} 共 {}",
                        c.success, c.unchanged, c.skipped, c.failed, c.total
                    )
                },
            );

        let events = log.lock().unwrap();
        let outcomes: Vec<_> = events
            .iter()
            .filter(|e| e["status"] != "processing")
            .map(|e| {
                (
                    e["status"].as_str().unwrap(),
                    e["message"].as_str().unwrap(),
                )
            })
            .collect();
        assert_eq!(
            outcomes,
            vec![
                ("success", "转换"),
                ("success", "原样复制"),
                ("success", "不匹配"),
                ("skipped", "同格式复制"),
                ("done", "成功 3 原样 2 跳过 1 失败 0 共 4"),
            ]
        );
        assert_eq!(
            events[0],
            ev(1, 4, "a.png", "processing", "正在检测: a.png")
        );
        assert_eq!((r.success_count, r.fail_count, r.total), (3, 0, 4));
    }

    #[test]
    fn manual_processing_events() {
        let app = tauri::test::mock_app();
        let log = capture_events(app.handle(), EVENT);
        let cancel = AtomicBool::new(false);

        FileBatch::new(app.handle(), EVENT, &cancel)
            .no_processing()
            .error_prefix("[错误] ")
            .run(
                &files(&["a.jpg", "b.png"]),
                |item| {
                    if item.name.ends_with(".png") {
                        return Ok(FileOutcome::unchanged("已是 png").with_status("skipped"));
                    }
                    item.emit("processing", format!("正在转换: {}", item.name));
                    Ok(FileOutcome::done("已转换"))
                },
                |c| c.summary("转换完成"),
            );

        assert_eq!(
            *log.lock().unwrap(),
            vec![
                ev(1, 2, "a.jpg", "processing", "正在转换: a.jpg"),
                ev(1, 2, "a.jpg", "success", "已转换"),
                ev(2, 2, "b.png", "skipped", "已是 png"),
                ev(2, 2, "", "done", "转换完成: 成功 2, 失败 0, 共 2"),
            ]
        );
    }

    #[test]
    fn cancel_mid_batch_stops_before_next_file() {
        let app = tauri::test::mock_app();
        let log = capture_events(app.handle(), EVENT);
        let cancel = AtomicBool::new(false);

        let r = FileBatch::new(app.handle(), EVENT, &cancel).run(
            &files(&["a.png", "b.png", "c.png", "d.png"]),
            |item| {
                if item.name == "b.png" {
                    cancel.store(true, Ordering::SeqCst);
                }
                Ok(FileOutcome::done("ok"))
            },
            |c| c.summary("处理完成"),
        );

        let events = log.lock().unwrap();
        assert_eq!(events.len(), 5);
        assert_eq!(events[3], ev(2, 4, "b.png", "success", "ok"));
        assert_eq!(events[4], ev(2, 4, "", "done", "已取消: 已处理 2, 共 4"));
        assert_eq!(done_count(&events), 1);
        assert_eq!((r.success_count, r.fail_count, r.total), (2, 0, 4));
    }

    /// 处理最后一张时点取消：原先这里一条 done 都不发，任务面板会一直停在运行中
    #[test]
    fn cancel_during_last_file_still_ends_with_one_done() {
        let app = tauri::test::mock_app();
        let log = capture_events(app.handle(), EVENT);
        let cancel = AtomicBool::new(false);
        let summary_called = Cell::new(false);

        let r = FileBatch::new(app.handle(), EVENT, &cancel).run(
            &files(&["a.png", "b.png", "c.png"]),
            |item| {
                if item.name == "c.png" {
                    cancel.store(true, Ordering::SeqCst);
                }
                Ok(FileOutcome::done("ok"))
            },
            |c| {
                summary_called.set(true);
                c.summary("处理完成")
            },
        );

        let events = log.lock().unwrap();
        assert_eq!(
            events.last().unwrap(),
            &ev(3, 3, "", "done", "已取消: 已处理 3, 共 3")
        );
        assert_eq!(done_count(&events), 1);
        assert!(!summary_called.get());
        assert_eq!((r.success_count, r.total), (3, 3));
    }

    #[test]
    fn cancelled_before_start() {
        let app = tauri::test::mock_app();
        let log = capture_events(app.handle(), EVENT);
        let cancel = AtomicBool::new(true);
        let called = Cell::new(false);

        let r = FileBatch::new(app.handle(), EVENT, &cancel).run(
            &files(&["a.png", "b.png"]),
            |_| {
                called.set(true);
                Ok(FileOutcome::done("ok"))
            },
            |c| c.summary("处理完成"),
        );

        assert_eq!(
            *log.lock().unwrap(),
            vec![ev(0, 2, "", "done", "已取消: 已处理 0, 共 2")]
        );
        assert!(!called.get());
        assert_eq!((r.success_count, r.fail_count, r.total), (0, 0, 2));
    }

    #[test]
    fn empty_batch_reports_summary() {
        let app = tauri::test::mock_app();
        let log = capture_events(app.handle(), EVENT);
        let cancel = AtomicBool::new(false);

        FileBatch::new(app.handle(), EVENT, &cancel).run(
            &[],
            |_| Ok(FileOutcome::done("ok")),
            |c| c.summary("处理完成"),
        );

        assert_eq!(
            *log.lock().unwrap(),
            vec![ev(0, 0, "", "done", "处理完成: 成功 0, 失败 0, 共 0")]
        );
    }

    #[tokio::test]
    async fn job_run_resets_cancel_and_returns_task_result() {
        static JOB: BatchJob = BatchJob::new("测试");
        JOB.cancel();
        assert!(JOB.cancel_flag().load(Ordering::SeqCst));

        let seen = JOB
            .run(|| Ok(JOB.cancel_flag().load(Ordering::SeqCst)))
            .await;
        assert_eq!(seen, Ok(false));

        let failed = JOB.run(|| Err::<(), _>("输入路径无效".to_string())).await;
        assert_eq!(failed, Err("输入路径无效".to_string()));
    }
}
