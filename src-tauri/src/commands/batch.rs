//! 批处理的公共骨架。
//!
//! - [`BatchJob`]：命令级互斥闸 + 取消标志，并把同步任务放进阻塞线程池执行；
//! - [`FileBatch`]：纯 Rust 逐文件处理，发 processing / success / error 事件并计数，
//!   结束时按需把失败文件归集进 Fail/，再恰好发一次终态 done；
//! - [`terminal_event`] / [`finish_run`]：自己驱动循环的命令（打标、美学、超分、人物裁切、
//!   聚类等）用的终态 done，与 `FileBatch`、`llm_batch` 的取消文案一致。

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use tauri::{AppHandle, Runtime};

use super::{file_name_lossy, BusyGuard, ProblemArchive, ProcessResult, ProgressEvent};

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
        super::spawn_blocking_with_progress(task)
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

/// 单个文件处理成功后的结果：决定怎么计数，以及 success 事件的 message
#[derive(Debug)]
pub(crate) struct FileOutcome {
    kind: OutcomeKind,
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

    fn new(kind: OutcomeKind, message: impl Into<String>) -> Self {
        Self {
            kind,
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

    /// 已经处理完的数量（成功、跳过、失败都算）
    pub(crate) fn processed(&self) -> u32 {
        self.success + self.skipped + self.failed
    }

    /// 取消时的终态文案（全应用统一）："已取消: 已处理 {processed}/{total}, 成功 {success}, 失败 {failed}"
    pub(crate) fn cancelled_summary(&self) -> String {
        format!(
            "已取消: 已处理 {}/{}, 成功 {}, 失败 {}",
            self.processed(),
            self.total,
            self.success,
            self.failed
        )
    }
}

/// 按 `ProcessResult` 计数（success_count 里已含的跳过不再单独计）
impl From<&ProcessResult> for BatchCounts {
    fn from(result: &ProcessResult) -> Self {
        Self {
            success: result.success_count,
            failed: result.fail_count,
            total: result.total,
            ..Default::default()
        }
    }
}

/// 一轮任务的终态 done（filename 为空，尚未发出）：
/// - `cancelled`：`counts.cancelled_summary()`，current 为已处理数，带 `cancelled: true`；
/// - 否则：`summary(counts)`，current 为 total。
pub(crate) fn terminal_event(
    counts: &BatchCounts,
    cancelled: bool,
    summary: impl FnOnce(&BatchCounts) -> String,
) -> ProgressEvent {
    if cancelled {
        cancelled_terminal_event(counts)
    } else {
        ProgressEvent::new("done", summary(counts)).at(counts.total, counts.total)
    }
}

fn cancelled_terminal_event(counts: &BatchCounts) -> ProgressEvent {
    ProgressEvent::new("done", counts.cancelled_summary())
        .at(counts.processed(), counts.total)
        .cancelled()
}

/// 发出一轮任务唯一的终态 done（见 [`terminal_event`]），给自己驱动循环的命令用；
/// `FileBatch::run` 与 `llm_batch::BatchOutcome::finish` 已包含这一步。
///
/// 用法：`finish_run(&app, EVENT, &BatchCounts::from(&result), cancelled, |c| c.summary("美学评分完成"))`
pub(crate) fn finish_run<R: Runtime>(
    app: &AppHandle<R>,
    event: &str,
    counts: &BatchCounts,
    cancelled: bool,
    summary: impl FnOnce(&BatchCounts) -> String,
) {
    terminal_event(counts, cancelled, summary).emit(app, event);
}

/// 自己驱动循环的任务命令的一轮：开始时 `begin_run`，之后发的事件都显式带上这一轮的运行 ID。
/// 同一通道上另一轮紧接着开始时（没有互斥锁的命令、并行的测试），这一轮余下的事件仍带自己的 ID，
/// 不会被前端当成新一轮的事件
pub(crate) struct RunEvents<'a, R: Runtime> {
    app: &'a AppHandle<R>,
    event: &'a str,
    run_id: u64,
}

impl<'a, R: Runtime> RunEvents<'a, R> {
    pub(crate) fn begin(app: &'a AppHandle<R>, event: &'a str) -> Self {
        Self {
            app,
            event,
            run_id: super::begin_run(event),
        }
    }

    /// 交给 `FileBatch::in_run`
    pub(crate) fn run_id(&self) -> u64 {
        self.run_id
    }

    pub(crate) fn emit(&self, progress: ProgressEvent) {
        progress.for_run(self.run_id).emit(self.app, self.event);
    }

    /// 发这一轮唯一的终态 done（见 [`terminal_event`]）
    pub(crate) fn finish(
        &self,
        counts: &BatchCounts,
        cancelled: bool,
        summary: impl FnOnce(&BatchCounts) -> String,
    ) {
        self.emit(terminal_event(counts, cancelled, summary));
    }

    /// 用户取消时收尾：发这一轮唯一的终态 done（统一取消文案，带 `cancelled: true`），
    /// 返回命令要交给前端的 `Err("已取消")`（取消的 Err 文本以「已取消」开头，前端据此识别）
    pub(crate) fn finish_cancelled<T>(&self, counts: &BatchCounts) -> Result<T, String> {
        self.emit(cancelled_terminal_event(counts));
        Err("已取消".to_string())
    }
}

/// 传给逐文件闭包的当前文件
pub(crate) struct BatchItem<'a, R: Runtime> {
    pub path: &'a Path,
    /// 文件名（`file_name_lossy`），事件的 filename 也用它
    pub name: &'a str,
    current: u32,
    total: u32,
    run_id: u64,
    app: &'a AppHandle<R>,
    event: &'a str,
}

impl<R: Runtime> BatchItem<'_, R> {
    /// 以当前文件的 current/total/filename 发一条事件
    fn emit(&self, status: &str, message: impl Into<String>) {
        ProgressEvent::new(status, message)
            .at(self.current, self.total)
            .file(self.name)
            .for_run(self.run_id)
            .emit(self.app, self.event);
    }
}

/// 逐文件批处理驱动，见 [`FileBatch::run`]
pub(crate) struct FileBatch<'a, R: Runtime> {
    app: &'a AppHandle<R>,
    event: &'a str,
    cancel: &'a AtomicBool,
    error_prefix: &'a str,
    run_id: Option<u64>,
    archive: Option<ProblemArchive<'a>>,
}

impl<'a, R: Runtime> FileBatch<'a, R> {
    /// 默认：失败事件不加前缀，失败文件不归集
    pub(crate) fn new(app: &'a AppHandle<R>, event: &'a str, cancel: &'a AtomicBool) -> Self {
        Self {
            app,
            event,
            cancel,
            error_prefix: "",
            run_id: None,
            archive: None,
        }
    }

    /// 命令在 `run` 之前已经 `begin_run` 并发过事件（如"开始筛选…"）时传入那一轮的 ID，
    /// `run` 沿用它而不另起一轮
    pub(crate) fn in_run(mut self, run_id: u64) -> Self {
        self.run_id = Some(run_id);
        self
    }

    /// 结束时把失败的文件复制进 `<output_root>/Fail/`（递归时保留相对 `input_root` 的子目录），
    /// 并在 done 之前发一条事件（见 `ProblemArchive::archive`）。取消时已经失败的文件照样归集
    pub(crate) fn archive_failures(
        mut self,
        input_root: &'a Path,
        output_root: &'a Path,
        recursive: bool,
    ) -> Self {
        self.archive = Some(ProblemArchive::new(input_root, output_root, recursive));
        self
    }

    /// 失败事件的 message 前缀，如 "[失败] "；`ProcessResult::errors` 里不带前缀
    pub(crate) fn error_prefix(mut self, prefix: &'a str) -> Self {
        self.error_prefix = prefix;
        self
    }

    /// 逐个处理 `files`。没有 `in_run` 时先 `begin_run(event)` 开始新一轮，本轮事件都带这个运行 ID。
    /// 每个文件的事件依次为（current 从 1 起，filename 为文件名）：
    /// - processing "正在处理: {文件名}"；
    /// - `Ok(outcome)` → success，message 为 outcome 的；
    /// - `Err(e)` → error "{error_prefix}{文件名}: {e}"，errors 记 "{文件名}: {e}"。
    ///
    /// 每个文件开始前检查取消标志，置位就停。结束时先按 `archive_failures` 归集失败文件，
    /// 再恰好发一次 done（见 [`terminal_event`]）：取消标志已置位（包括处理最后一张时才点取消）
    /// 发统一的取消文案并带 `cancelled: true`，否则发 `summary(&counts)`。
    pub(crate) fn run<F, S>(self, files: &[PathBuf], mut process: F, summary: S) -> ProcessResult
    where
        F: FnMut(&BatchItem<'_, R>) -> Result<FileOutcome, String>,
        S: FnOnce(&BatchCounts) -> String,
    {
        let run_id = self.run_id.unwrap_or_else(|| super::begin_run(self.event));
        let total = files.len() as u32;
        let mut counts = BatchCounts {
            total,
            ..Default::default()
        };
        let mut errors = Vec::new();
        let mut failed_files = Vec::new();

        for path in files {
            if self.cancel.load(Ordering::SeqCst) {
                break;
            }
            let name = file_name_lossy(path);
            let item = BatchItem {
                path,
                name: &name,
                current: counts.processed() + 1,
                total,
                run_id,
                app: self.app,
                event: self.event,
            };
            item.emit("processing", format!("正在处理: {}", name));
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
                    item.emit("success", outcome.message);
                }
                Err(e) => {
                    counts.failed += 1;
                    let err_msg = format!("{}: {}", name, e);
                    item.emit("error", format!("{}{}", self.error_prefix, err_msg));
                    errors.push(err_msg);
                    failed_files.push(path.clone());
                }
            }
        }

        if let Some(archive) = &self.archive {
            for progress in archive.archive(&failed_files, &[]) {
                progress.for_run(run_id).emit(self.app, self.event);
            }
        }
        terminal_event(&counts, self.cancel.load(Ordering::SeqCst), summary)
            .for_run(run_id)
            .emit(self.app, self.event);

        ProcessResult {
            success_count: counts.success,
            fail_count: counts.failed,
            total,
            errors,
        }
    }
}

/// 测试用：收集 `app` 上 `event` 的事件 payload，去掉 run_id——它全局递增、取值随测试执行顺序变化，
/// 去掉后才能按内容比对。要检查 run_id 时用 [`capture_raw_events`]
#[cfg(test)]
pub(crate) fn capture_events<R: Runtime>(
    app: &AppHandle<R>,
    event: &str,
) -> std::sync::Arc<std::sync::Mutex<Vec<serde_json::Value>>> {
    capture(app, event, true)
}

/// 测试用：原样收集 `app` 上 `event` 的事件 payload（含 run_id）
#[cfg(test)]
pub(crate) fn capture_raw_events<R: Runtime>(
    app: &AppHandle<R>,
    event: &str,
) -> std::sync::Arc<std::sync::Mutex<Vec<serde_json::Value>>> {
    capture(app, event, false)
}

#[cfg(test)]
fn capture<R: Runtime>(
    app: &AppHandle<R>,
    event: &str,
    strip_run_id: bool,
) -> std::sync::Arc<std::sync::Mutex<Vec<serde_json::Value>>> {
    use tauri::Listener;
    let events = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let sink = events.clone();
    app.listen_any(event, move |e| {
        let mut payload: serde_json::Value = serde_json::from_str(e.payload()).unwrap();
        if strip_run_id {
            if let Some(fields) = payload.as_object_mut() {
                fields.shift_remove("run_id");
            }
        }
        sink.lock().unwrap().push(payload);
    });
    events
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::test_support::TempDir;
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

    fn cancelled_done(current: u32, total: u32, message: &str) -> Value {
        let mut done = ev(current, total, "", "done", message);
        done["cancelled"] = json!(true);
        done
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

        let r = FileBatch::new(app.handle(), EVENT, &cancel).run(
            &files(&["a.png", "b.png", "c.png", "d.png"]),
            |item| {
                Ok(match item.name {
                    "a.png" => FileOutcome::done("转换"),
                    "b.png" => FileOutcome::unchanged("原样复制"),
                    "c.png" => FileOutcome::skipped("不匹配"),
                    _ => FileOutcome::unchanged("同格式复制"),
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
                ("success", "同格式复制"),
                ("done", "成功 3 原样 2 跳过 1 失败 0 共 4"),
            ]
        );
        assert_eq!(
            events[0],
            ev(1, 4, "a.png", "processing", "正在处理: a.png")
        );
        assert_eq!((r.success_count, r.fail_count, r.total), (3, 0, 4));
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
        assert_eq!(
            events[4],
            cancelled_done(2, 4, "已取消: 已处理 2/4, 成功 2, 失败 0")
        );
        assert_eq!(done_count(&events), 1);
        assert_eq!((r.success_count, r.fail_count, r.total), (2, 0, 4));
    }

    /// 处理最后一张时点取消也要有终态，否则任务面板会一直停在运行中
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
            &cancelled_done(3, 3, "已取消: 已处理 3/3, 成功 3, 失败 0")
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
            vec![cancelled_done(0, 2, "已取消: 已处理 0/2, 成功 0, 失败 0")]
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

    fn run_ids(events: &[Value]) -> Vec<Option<u64>> {
        events.iter().map(|e| e["run_id"].as_u64()).collect()
    }

    /// 每轮开始新的运行 ID，本轮全部事件（含逐文件、done）都带它；下一轮的 ID 更大
    #[test]
    fn each_run_stamps_all_its_events_with_a_new_id() {
        const RUN_EVENT: &str = "batch-test-run-ids";
        let app = tauri::test::mock_app();
        let log = capture_raw_events(app.handle(), RUN_EVENT);
        let cancel = AtomicBool::new(false);
        let batch = || {
            FileBatch::new(app.handle(), RUN_EVENT, &cancel).run(
                &files(&["a.png", "b.png"]),
                |item| {
                    if item.name == "b.png" {
                        Err("bad".to_string())
                    } else {
                        Ok(FileOutcome::done("ok"))
                    }
                },
                |c| c.summary("处理完成"),
            )
        };
        batch();
        let first = run_ids(&log.lock().unwrap());
        assert_eq!(first.len(), 5);
        let id = first[0].expect("事件应带 run_id");
        assert!(first.iter().all(|r| *r == Some(id)), "{first:?}");

        log.lock().unwrap().clear();
        batch();
        let second = run_ids(&log.lock().unwrap());
        let next = second[0].unwrap();
        assert!(next > id);
        assert!(second.iter().all(|r| *r == Some(next)));
    }

    /// 命令先 begin_run 再发"开始…"事件时，FileBatch 沿用那一轮，不另起
    #[test]
    fn in_run_reuses_the_commands_run() {
        const RUN_EVENT: &str = "batch-test-in-run";
        let app = tauri::test::mock_app();
        let log = capture_raw_events(app.handle(), RUN_EVENT);
        let cancel = AtomicBool::new(false);

        let run_id = super::super::begin_run(RUN_EVENT);
        ProgressEvent::new("processing", "开始处理").emit(app.handle(), RUN_EVENT);
        FileBatch::new(app.handle(), RUN_EVENT, &cancel)
            .in_run(run_id)
            .run(
                &files(&["a.png"]),
                |_| Ok(FileOutcome::done("ok")),
                |c| c.summary("完成"),
            );
        // 之后通道上的事件仍属于这一轮
        ProgressEvent::new("info", "收尾").emit(app.handle(), RUN_EVENT);

        let events = log.lock().unwrap();
        assert_eq!(events.len(), 5);
        assert!(run_ids(&events).iter().all(|r| *r == Some(run_id)));
    }

    #[test]
    fn failures_are_archived_into_output_fail_before_done() {
        let root = TempDir::new("batch_archive");
        let (input, output) = (root.join("in"), root.join("out"));
        std::fs::create_dir_all(input.join("sub")).unwrap();
        for rel in ["a.png", "sub/b.png", "c.png"] {
            std::fs::write(input.join(rel), rel).unwrap();
        }
        let app = tauri::test::mock_app();
        let log = capture_events(app.handle(), EVENT);
        let cancel = AtomicBool::new(false);

        let batch_files = vec![
            input.join("a.png"),
            input.join("sub/b.png"),
            input.join("c.png"),
        ];
        let r = FileBatch::new(app.handle(), EVENT, &cancel)
            .archive_failures(&input, &output, true)
            .run(
                &batch_files,
                |item| {
                    if item.name == "a.png" {
                        Ok(FileOutcome::done("ok"))
                    } else {
                        Err("坏图".to_string())
                    }
                },
                |c| c.summary("处理完成"),
            );

        assert_eq!(r.fail_count, 2);
        assert_eq!(
            std::fs::read_to_string(output.join("Fail/sub/b.png")).unwrap(),
            "sub/b.png"
        );
        assert!(output.join("Fail/c.png").is_file());
        assert!(!output.join("Fail/a.png").exists());
        assert!(!input.join("Fail").exists());
        let events = log.lock().unwrap();
        let tail: Vec<_> = events[events.len() - 2..].to_vec();
        assert_eq!(
            tail,
            vec![
                ev(0, 0, "", "info", "已将 2 个失败文件复制到 Fail/ 文件夹"),
                ev(3, 3, "", "done", "处理完成: 成功 1, 失败 2, 共 3"),
            ]
        );
    }

    /// 取消前已经失败的文件照样归集；没有失败时不发归集事件
    #[test]
    fn cancelled_run_still_archives_earlier_failures() {
        let root = TempDir::new("batch_archive_cancel");
        for name in ["a.png", "b.png", "c.png"] {
            std::fs::write(root.join(name), name).unwrap();
        }
        let app = tauri::test::mock_app();
        let log = capture_events(app.handle(), EVENT);
        let cancel = AtomicBool::new(false);

        let batch_files: Vec<_> = ["a.png", "b.png", "c.png"]
            .iter()
            .map(|n| root.join(n))
            .collect();
        FileBatch::new(app.handle(), EVENT, &cancel)
            .archive_failures(&root, &root, false)
            .run(
                &batch_files,
                |item| {
                    if item.name == "b.png" {
                        cancel.store(true, Ordering::SeqCst);
                        return Ok(FileOutcome::skipped("跳过"));
                    }
                    Err("坏图".to_string())
                },
                |c| c.summary("处理完成"),
            );

        assert!(root.join("Fail/a.png").is_file());
        assert!(!root.join("Fail/b.png").exists() && !root.join("Fail/c.png").exists());
        let events = log.lock().unwrap();
        assert_eq!(
            events[events.len() - 2..],
            [
                ev(0, 0, "", "info", "已将 1 个失败文件复制到 Fail/ 文件夹"),
                cancelled_done(2, 3, "已取消: 已处理 2/3, 成功 0, 失败 1"),
            ]
        );

        let app = tauri::test::mock_app();
        let log = capture_events(app.handle(), EVENT);
        let cancel = AtomicBool::new(false);
        FileBatch::new(app.handle(), EVENT, &cancel)
            .archive_failures(
                Path::new("/nonexistent/in"),
                Path::new("/nonexistent/out"),
                false,
            )
            .run(
                &files(&["a.png"]),
                |_| Ok(FileOutcome::done("ok")),
                |c| c.summary("完成"),
            );
        assert_eq!(
            *log.lock().unwrap(),
            vec![
                ev(1, 1, "a.png", "processing", "正在处理: a.png"),
                ev(1, 1, "a.png", "success", "ok"),
                ev(1, 1, "", "done", "完成: 成功 1, 失败 0, 共 1"),
            ]
        );
    }

    #[test]
    fn terminal_event_for_finished_and_cancelled_runs() {
        let result = ProcessResult {
            success_count: 3,
            fail_count: 1,
            total: 10,
            errors: vec![],
        };
        let counts = BatchCounts::from(&result);
        assert_eq!(
            (counts.processed(), counts.skipped, counts.unchanged),
            (4, 0, 0)
        );

        let done = terminal_event(&counts, false, |c| c.summary("美学评分完成"));
        assert_eq!(
            (
                done.status.as_str(),
                done.current,
                done.total,
                done.cancelled
            ),
            ("done", 10, 10, false)
        );
        assert_eq!(done.message, "美学评分完成: 成功 3, 失败 1, 共 10");

        let summary_called = Cell::new(false);
        let cancelled = terminal_event(&counts, true, |c| {
            summary_called.set(true);
            c.summary("美学评分完成")
        });
        assert!(!summary_called.get());
        assert_eq!(
            (cancelled.current, cancelled.total, cancelled.cancelled),
            (4, 10, true)
        );
        assert_eq!(cancelled.message, "已取消: 已处理 4/10, 成功 3, 失败 1");

        // 跳过数计入"已处理"
        let with_skips = BatchCounts {
            success: 2,
            skipped: 3,
            failed: 1,
            total: 9,
            ..Default::default()
        };
        assert_eq!(
            with_skips.cancelled_summary(),
            "已取消: 已处理 6/9, 成功 2, 失败 1"
        );
    }

    /// 同一通道上后一轮开始后，前一轮余下的事件和终态仍带自己的 ID
    #[test]
    fn run_events_keep_their_own_id_when_runs_overlap() {
        const RUN_EVENT: &str = "batch-test-run-events";
        let app = tauri::test::mock_app();
        let log = capture_raw_events(app.handle(), RUN_EVENT);
        let first = RunEvents::begin(app.handle(), RUN_EVENT);
        first.emit(ProgressEvent::new("processing", "first"));
        let second = RunEvents::begin(app.handle(), RUN_EVENT);
        first.finish(
            &BatchCounts {
                success: 1,
                total: 1,
                ..Default::default()
            },
            false,
            |c| c.summary("完成"),
        );
        second.emit(ProgressEvent::new("processing", "second"));

        assert!(second.run_id() > first.run_id());
        assert_eq!(
            run_ids(&log.lock().unwrap()),
            [
                Some(first.run_id()),
                Some(first.run_id()),
                Some(second.run_id())
            ]
        );
        assert_eq!(
            log.lock().unwrap()[1]["message"],
            "完成: 成功 1, 失败 0, 共 1"
        );
    }

    /// 取消收尾：恰好一条带本轮 ID 与 cancelled 的 done，Err 以「已取消」开头
    #[test]
    fn finish_cancelled_emits_one_cancelled_done_and_returns_err() {
        const RUN_EVENT: &str = "batch-test-finish-cancelled";
        let app = tauri::test::mock_app();
        let log = capture_raw_events(app.handle(), RUN_EVENT);
        let run = RunEvents::begin(app.handle(), RUN_EVENT);
        let counts = BatchCounts {
            success: 2,
            skipped: 1,
            failed: 1,
            total: 6,
            ..Default::default()
        };

        let result: Result<u32, String> = run.finish_cancelled(&counts);

        assert_eq!(result, Err("已取消".to_string()));
        assert_eq!(
            *log.lock().unwrap(),
            vec![json!({
                "current": 4, "total": 6, "filename": "", "status": "done",
                "message": "已取消: 已处理 4/6, 成功 2, 失败 1",
                "run_id": run.run_id(), "cancelled": true,
            })]
        );
    }

    #[test]
    fn finish_run_emits_one_done_with_the_current_run_id() {
        const RUN_EVENT: &str = "batch-test-finish-run";
        let app = tauri::test::mock_app();
        let log = capture_raw_events(app.handle(), RUN_EVENT);
        let counts = BatchCounts {
            success: 1,
            total: 2,
            ..Default::default()
        };

        // 通道从没开始过任何一轮：不带 run_id
        finish_run(app.handle(), RUN_EVENT, &counts, false, |c| {
            c.summary("完成")
        });
        let run_id = super::super::begin_run(RUN_EVENT);
        finish_run(app.handle(), RUN_EVENT, &counts, true, |c| {
            c.summary("完成")
        });

        assert_eq!(
            *log.lock().unwrap(),
            vec![
                ev(2, 2, "", "done", "完成: 成功 1, 失败 0, 共 2"),
                json!({
                    "current": 1, "total": 2, "filename": "", "status": "done",
                    "message": "已取消: 已处理 1/2, 成功 1, 失败 0",
                    "run_id": run_id, "cancelled": true,
                }),
            ]
        );
    }
}
