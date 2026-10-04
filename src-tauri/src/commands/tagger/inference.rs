use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use serde_json::{json, Value};
use tauri::{AppHandle, Runtime};

use super::models::ModelDefinition;
use super::{OnnxModelInfo, ProcessResult, ProgressEvent, TaggerOptions, NON_UTF8_NAME};
use crate::commands::batch::{finish_run, BatchCounts};
use crate::commands::python_proc::{
    self, stderr_warnings, PythonCommand, PythonSession, SessionError, StderrNoise,
    PYTHON_SILENCE_LIMIT,
};
use crate::commands::{collect_image_files_with_recursive, file_name_lossy, ProblemArchive};

/// 打标与准备已有标签共用的进度事件
pub(super) const EVENT: &str = "tagger-progress";

/// 全局打标取消标志（打标与 txt → JSON 转换共用）
pub(super) static TAGGING_CANCELLED: AtomicBool = AtomicBool::new(false);

/// 打标会话或转换进程的 PID，取消时按进程树终止
pub(super) static PYTHON_PROCESS: Mutex<Option<u32>> = Mutex::new(None);

/// 打标会话的退出命令
const QUIT: &str = r#"{"cmd":"quit"}"#;

/// 等 Python 的时限：`ready` 是加载模型（总时限），`silence` 是逐张推理时两条消息的最长间隔
#[derive(Debug, Clone, Copy)]
struct Limits {
    ready: Duration,
    silence: Duration,
}

const LIMITS: Limits = Limits {
    ready: Duration::from_secs(120),
    silence: PYTHON_SILENCE_LIMIT,
};

/// 取消打标
pub fn cancel_tagging() {
    TAGGING_CANCELLED.store(true, Ordering::SeqCst);
    python_proc::kill_registered_pid(&PYTHON_PROCESS);
}

/// 重置取消标志（开始新任务前调用）
pub fn reset_tagging_cancel() {
    TAGGING_CANCELLED.store(false, Ordering::SeqCst);
}

/// 检查是否已取消
pub fn is_tagging_cancelled() -> bool {
    TAGGING_CANCELLED.load(Ordering::SeqCst)
}

/// 自动检测 ONNX 模型的输入信息（使用 Python 调用）
pub fn detect_model_info(model_path: &str) -> Result<OnnxModelInfo, String> {
    let python = crate::commands::python_env::get_python_exe().ok_or("未找到可用的 Python 环境")?;
    let script = python_proc::find_script("tagger_inference.py")?;
    let output = PythonCommand::new(&python)
        .arg(&script)
        .args(["--detect", model_path])
        .output()
        .map_err(|e| format!("启动 Python 失败: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("模型检测失败: {}", stderr.trim()));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines() {
        if let Ok(val) = serde_json::from_str::<Value>(line) {
            if val.get("type").and_then(|v| v.as_str()) == Some("model_info") {
                let input_size = val
                    .get("input_size")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(448) as u32;
                let shape: Vec<i64> = val
                    .get("input_shape")
                    .and_then(|v| v.as_array())
                    .map(|a| a.iter().filter_map(|v| v.as_i64()).collect())
                    .unwrap_or_default();

                return Ok(OnnxModelInfo {
                    input_size,
                    input_shape: shape,
                });
            }
        }
    }

    Err("无法解析模型信息".into())
}

/// 打标的完成文案，跳过的图片单独注明
fn tagging_summary(counts: &BatchCounts, skipped: u32) -> String {
    if skipped > 0 {
        format!(
            "打标完成: 成功 {}（含跳过 {}）, 失败 {}, 共 {}",
            counts.success, skipped, counts.failed, counts.total
        )
    } else {
        counts.summary("打标完成")
    }
}

/// 没走到逐张推理就结束的一轮（全部跳过、准备环境时取消）的终态 done
pub(super) fn finish_tagging<R: Runtime>(
    app: &AppHandle<R>,
    result: &ProcessResult,
    skipped: u32,
    cancelled: bool,
) {
    finish_run(
        app,
        EVENT,
        &BatchCounts::from(result),
        cancelled,
        |counts| tagging_summary(counts, skipped),
    );
}

/// 标签文件格式：除 "json" 外都按 txt 写（与 Python 端的判断一致）
fn label_format(options: &TaggerOptions) -> &'static str {
    if options.output_format == "json" {
        "json"
    } else {
        "txt"
    }
}

fn should_skip(path: &Path, options: &TaggerOptions) -> bool {
    options.existing_tags_action == "skip"
        && if options.hybrid_mode {
            super::has_labels(path)
        } else {
            super::has_label(path, label_format(options))
        }
}

/// 标签写到哪里（发给 Python 的 tag_output_path）：辅助打标写草稿，否则写图片旁的同名标签文件
fn tag_output_path(image: &Path, options: &TaggerOptions) -> PathBuf {
    if options.hybrid_mode {
        super::hybrid::draft_path(image, label_format(options))
    } else {
        image.with_extension(label_format(options))
    }
}

/// 图片和标签路径都能原样放进 JSON（都是有效 UTF-8）
fn sendable(image: &Path, options: &TaggerOptions) -> bool {
    image.to_str().is_some() && tag_output_path(image, options).to_str().is_some()
}

/// 一张图的打标命令：打标选项 + 图片路径 + 标签写入路径（`sendable` 已保证路径无损）
fn image_command(base: &Value, image: &Path, options: &TaggerOptions) -> Value {
    let mut command = base.clone();
    command["image_path"] = Value::from(image.to_string_lossy());
    command["tag_output_path"] = Value::from(tag_output_path(image, options).to_string_lossy());
    command
}

pub(super) fn all_skipped(options: &TaggerOptions) -> Result<Option<ProcessResult>, String> {
    let files =
        collect_image_files_with_recursive(Path::new(&options.input_path), options.recursive)?;
    if files.is_empty() {
        return Err("输入目录中没有找到图片文件".into());
    }
    Ok(files
        .iter()
        .all(|path| should_skip(path, options))
        .then(|| ProcessResult {
            total: files.len() as u32,
            success_count: files.len() as u32,
            ..Default::default()
        }))
}

/// 辅助打标：清掉本轮要重打的图片的旧草稿和图片已不存在的残留草稿，
/// 避免推理失败后调优阶段读到上一轮的结果。跳过的图片（已有标签）的草稿不动：
/// 跑过准备阶段时是刚从已有标签复制来的；没跑时调优阶段同样按已有标签跳过它们并删掉草稿
fn clear_pending_drafts(
    input: &Path,
    files: &[&PathBuf],
    options: &TaggerOptions,
) -> Result<(), String> {
    super::hybrid::clear_residual_drafts(input, options.recursive, true)?;
    for path in files {
        super::hybrid::clear_drafts(path)?;
    }
    Ok(())
}

/// 一轮任务的计数与逐文件事件（打标、准备已有标签共用），current 是已处理数
pub(super) struct Tally<'a, R: Runtime> {
    app: &'a AppHandle<R>,
    pub(super) counts: BatchCounts,
    errors: Vec<String>,
    failed: Vec<PathBuf>,
}

impl<'a, R: Runtime> Tally<'a, R> {
    pub(super) fn new(app: &'a AppHandle<R>, total: u32) -> Self {
        Tally {
            app,
            counts: BatchCounts {
                total,
                ..Default::default()
            },
            errors: Vec::new(),
            failed: Vec::new(),
        }
    }

    fn emit(&self, status: &str, message: String, path: &Path) {
        ProgressEvent::new(status, message)
            .at(self.counts.processed(), self.counts.total)
            .file(file_name_lossy(path))
            .emit(self.app, EVENT);
    }

    /// Python 的 log 消息，current 是正在处理的那张
    pub(super) fn log(&self, msg: &Value) {
        ProgressEvent::python_log(msg, self.counts.processed() + 1, self.counts.total)
            .emit(self.app, EVENT);
    }

    /// 计一张成功，发 `status` 事件
    pub(super) fn succeed(&mut self, path: &Path, status: &str, message: String) {
        self.counts.success += 1;
        self.emit(status, message, path);
    }

    /// 计一张失败：记下原因和文件，发 error「[错误] 文件名: 原因」
    pub(super) fn fail(&mut self, path: &Path, reason: &str) {
        self.counts.failed += 1;
        let name = file_name_lossy(path);
        self.errors.push(format!("{}: {}", name, reason));
        self.failed.push(path.to_path_buf());
        self.emit("error", format!("[错误] {}: {}", name, reason), path);
    }

    fn archive(&self, input: &Path, recursive: bool) {
        ProblemArchive::new(input, input, recursive).report(self.app, EVENT, &self.failed, &[]);
    }

    /// 正常结束或被取消：失败的文件复制进 Fail/，再发唯一的终态 done
    pub(super) fn finish(
        self,
        input: &Path,
        recursive: bool,
        cancelled: bool,
        summary: impl FnOnce(&BatchCounts) -> String,
    ) -> ProcessResult {
        self.archive(input, recursive);
        finish_run(self.app, EVENT, &self.counts, cancelled, summary);
        ProcessResult {
            success_count: self.counts.success,
            fail_count: self.counts.failed,
            total: self.counts.total,
            errors: self.errors,
        }
    }

    fn finish_tagging(
        self,
        input: &Path,
        recursive: bool,
        skipped: u32,
        cancelled: bool,
    ) -> ProcessResult {
        self.finish(input, recursive, cancelled, |counts| {
            tagging_summary(counts, skipped)
        })
    }

    /// 还没开始逐张处理就失败（模型加载失败等）：已失败的文件照样归集，返回错误原文
    pub(super) fn give_up(self, input: &Path, recursive: bool, error: String) -> String {
        self.archive(input, recursive);
        error
    }

    /// 中途中止：`unprocessed` 全部计为失败，连同之前失败的复制进 Fail/，不发 done，
    /// 返回作为命令 Err 的文案（原因、未处理张数和计数）
    pub(super) fn abort(
        mut self,
        input: &Path,
        recursive: bool,
        reason: &str,
        unprocessed: &[PathBuf],
    ) -> String {
        self.counts.failed += unprocessed.len() as u32;
        self.failed.extend_from_slice(unprocessed);
        self.archive(input, recursive);
        format!(
            "{}，{} 张未处理；成功 {}, 失败 {}, 共 {}",
            reason,
            unprocessed.len(),
            self.counts.success,
            self.counts.failed,
            self.counts.total
        )
    }
}

pub fn run_tagging(
    app: &tauri::AppHandle,
    options: &TaggerOptions,
    python: &str,
    model: &ModelDefinition,
    model_dir: &Path,
) -> Result<ProcessResult, String> {
    let script = python_proc::find_script("tagger_inference.py")?;
    run_tagging_process(app, options, python, &script, model, model_dir, LIMITS)
}

/// 按批把图片交给常驻的 Python 打标会话。
///
/// - 已有标签要跳过的、文件名不是有效 UTF-8 的先处理掉，其余的才发给 Python；
/// - Python 进程退出、无响应、回了不带图片的批次级错误或对不上的结果时中止：
///   没有结果的图片全部计为失败、复制进 Fail/，返回 Err（不发 done），辅助打标据此停止后续阶段；
/// - 取消时发带 cancelled 的终态 done 并返回 Ok。
fn run_tagging_process<R: Runtime>(
    app: &AppHandle<R>,
    options: &TaggerOptions,
    python: &str,
    script: &Path,
    model: &ModelDefinition,
    model_dir: &Path,
    limits: Limits,
) -> Result<ProcessResult, String> {
    let input_dir = Path::new(&options.input_path);
    let files = collect_image_files_with_recursive(input_dir, options.recursive)?;
    let total = files.len() as u32;
    let mut tally = Tally::new(app, total);
    if is_tagging_cancelled() {
        return Ok(tally.finish_tagging(input_dir, options.recursive, 0, true));
    }

    let skip: Vec<bool> = files
        .iter()
        .map(|path| should_skip(path, options))
        .collect();
    if options.hybrid_mode {
        let to_tag: Vec<&PathBuf> = files
            .iter()
            .zip(&skip)
            .filter(|(_, skip)| !**skip)
            .map(|(path, _)| path)
            .collect();
        clear_pending_drafts(input_dir, &to_tag, options)?;
    }
    ProgressEvent::new("info", format!("读取到 {} 张图片", total))
        .at(0, total)
        .emit(app, EVENT);
    let mut skipped = 0;
    let mut work = Vec::new();
    for (path, skip) in files.iter().zip(skip) {
        if skip {
            skipped += 1;
            let message = format!("[跳过] {}（已有标签或原文件受保护）", file_name_lossy(path));
            tally.succeed(path, "success", message);
        } else if sendable(path, options) {
            work.push(path);
        } else {
            tally.fail(path, NON_UTF8_NAME);
        }
    }
    if work.is_empty() {
        let cancelled = is_tagging_cancelled();
        return Ok(tally.finish_tagging(input_dir, options.recursive, skipped, cancelled));
    }

    let mut session = match PythonSession::start(
        PythonCommand::new(python)
            .arg(script)
            .use_gpu(options.use_gpu)
            .background_priority(options.use_gpu && model.heavy_gpu),
        &PYTHON_PROCESS,
        &TAGGING_CANCELLED,
        stderr_warnings(app, EVENT, StderrNoise::Runtime, Some(&TAGGING_CANCELLED)),
    ) {
        Ok(session) => session,
        Err(SessionError::Cancelled) => {
            return Ok(tally.finish_tagging(input_dir, options.recursive, skipped, true))
        }
        Err(error) => return Err(tally.give_up(input_dir, options.recursive, error.to_string())),
    };
    let init = json!({
        "cmd": "init",
        "model_path": model_dir.join("model.onnx").to_string_lossy(),
        "tags_path": model_dir.join(model.tags_basename()).to_string_lossy(),
        "use_gpu": options.use_gpu,
        "input_size": model.input_size,
        "preprocess_mode": model.preprocess_mode,
        "output_kind": model.output_kind,
        "category_thresholds": model.category_thresholds,
        "conservative_cuda": model.heavy_gpu,
    });
    let ready = session.send(&init).and_then(|()| {
        session.wait_ready(limits.ready, |msg| {
            ProgressEvent::python_log(msg, 0, 0).emit(app, EVENT)
        })
    });
    match ready {
        Ok(()) => {}
        Err(SessionError::Cancelled) => {
            session.shutdown(None);
            return Ok(tally.finish_tagging(input_dir, options.recursive, skipped, true));
        }
        Err(error) => {
            if matches!(error, SessionError::TimedOut(_)) {
                session.kill();
            }
            let message = init_error(&error, &session.stderr_tail());
            session.shutdown(Some(QUIT));
            return Err(tally.give_up(input_dir, options.recursive, message));
        }
    }

    let base = serde_json::to_value(options).map_err(|e| format!("打标选项无法序列化: {}", e))?;
    let batch_size = options.batch_size.max(1) as usize;
    let mut next = 0;
    let mut pending: Vec<&PathBuf> = Vec::new();
    let mut failure = None;
    'batches: while next < work.len() && !is_tagging_cancelled() {
        let batch = &work[next..(next + batch_size).min(work.len())];
        next += batch.len();
        let name = file_name_lossy(batch[0]);
        let current = tally.counts.processed() + 1;
        let message = if batch.len() > 1 {
            format!(
                "正在处理: {} 等 {} 张 ({}/{})",
                name,
                batch.len(),
                current,
                total
            )
        } else {
            format!("正在处理: {} ({}/{})", name, current, total)
        };
        ProgressEvent::new("processing", message)
            .at(current, total)
            .file(name)
            .emit(app, EVENT);
        let images: Vec<Value> = batch
            .iter()
            .map(|path| image_command(&base, path, options))
            .collect();
        pending = batch.to_vec();
        if let Err(error) = session.send(&json!({"cmd": "tag_batch", "images": images})) {
            failure = interruption(error);
            break;
        }

        while !pending.is_empty() {
            let msg = match session.recv(limits.silence) {
                Ok(msg) => msg,
                Err(error) => {
                    if matches!(error, SessionError::TimedOut(_)) {
                        session.kill();
                    }
                    failure = interruption(error);
                    break 'batches;
                }
            };
            match msg["type"].as_str().unwrap_or("") {
                "log" => tally.log(&msg),
                kind @ ("result" | "error") => {
                    let index = match locate(&pending, &msg) {
                        Ok(index) => index,
                        Err(reason) => {
                            failure = Some(reason);
                            break 'batches;
                        }
                    };
                    let path = pending.remove(index);
                    let name = file_name_lossy(path);
                    if kind == "error" {
                        tally.fail(path, msg["message"].as_str().unwrap_or("未知错误"));
                    } else if msg["skipped"].as_bool().unwrap_or(false) {
                        skipped += 1;
                        let message = format!("[跳过] {}（已有标签或原文件受保护）", name);
                        tally.succeed(path, "success", message);
                    } else {
                        let count = msg["tag_count"].as_u64().unwrap_or(0);
                        tally.succeed(
                            path,
                            "success",
                            format!("[完成] {} → {} 个标签", name, count),
                        );
                    }
                }
                _ => {}
            }
        }
    }

    session.shutdown(Some(QUIT));
    if let Some(reason) = failure {
        let unprocessed: Vec<PathBuf> = pending
            .iter()
            .chain(&work[next..])
            .map(|path| path.to_path_buf())
            .collect();
        return Err(tally.abort(input_dir, options.recursive, &reason, &unprocessed));
    }
    let cancelled = is_tagging_cancelled();
    Ok(tally.finish_tagging(input_dir, options.recursive, skipped, cancelled))
}

/// 模型加载失败的错误文案，带上能说明原因的部分：脚本回的错误，或进程的 stderr 尾部
fn init_error(error: &SessionError, stderr_tail: &str) -> String {
    match error {
        SessionError::Script(message) => format!("模型初始化失败: {}", message),
        SessionError::TimedOut(limit) if stderr_tail.is_empty() => {
            format!("模型加载超时（{} 秒）", limit.as_secs())
        }
        SessionError::TimedOut(limit) => {
            format!("模型加载超时（{} 秒）: {}", limit.as_secs(), stderr_tail)
        }
        other => format!("模型初始化失败: {}", other),
    }
}

/// 推理中会话出错时中止本轮的原因；取消返回 None
fn interruption(error: SessionError) -> Option<String> {
    match error {
        SessionError::Cancelled => None,
        SessionError::Exited { stderr } if stderr.is_empty() => {
            Some("Python 进程异常退出".to_string())
        }
        SessionError::Exited { stderr } => Some(format!("Python 进程异常退出: {}", stderr)),
        SessionError::TimedOut(limit) => Some(format!(
            "Python 超过 {} 秒无响应，已终止进程",
            limit.as_secs()
        )),
        other => Some(other.to_string()),
    }
}

/// result / error 消息属于 `pending` 里的哪一张：按 image_path 原样比对；只剩一张时不带路径的也归它。
/// 对不上（路径不在本批、多张待处理时不带路径）时返回中止原因——等下去只会等到静默超时
fn locate(pending: &[&PathBuf], msg: &Value) -> Result<usize, String> {
    match msg["image_path"].as_str() {
        Some(path) => pending
            .iter()
            .position(|p| p.to_str() == Some(path))
            .ok_or_else(|| format!("Python 返回了无法对应的结果: {}", path)),
        None if pending.len() == 1 => Ok(0),
        None if msg["type"] == "error" => Err(format!(
            "Python 推理错误: {}",
            msg["message"].as_str().unwrap_or("未知错误")
        )),
        None => Err("Python 返回的结果缺少图片路径".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::batch::capture_events;
    use crate::commands::test_support::{test_python, TempDir};
    use tauri::Listener;

    fn options(root: &Path) -> TaggerOptions {
        serde_json::from_value(json!({
            "input_path": root, "model_id": "mock", "general_threshold": 0.35,
            "character_threshold": 0.85, "enabled_categories": ["general"],
            "use_gpu": false, "batch_size": 3,
        }))
        .unwrap()
    }

    /// 用假脚本跑一轮；脚本在 init 时按 preprocess_mode 选择行为
    fn run_fake(
        temp: &Path,
        opts: &TaggerOptions,
        mode: &str,
        limits: Limits,
    ) -> (Result<ProcessResult, String>, Vec<Value>) {
        let script = temp.join("fake.py");
        std::fs::write(&script, FAKE_SESSION).unwrap();
        let mut model = super::super::models::get_builtin_models().remove(0);
        model.preprocess_mode = mode.into();
        reset_tagging_cancel();
        let app = tauri::test::mock_app();
        let log = capture_events(app.handle(), EVENT);
        app.listen_any(EVENT, |event| {
            let value: Value = serde_json::from_str(event.payload()).unwrap();
            if value["message"] == "cancel_now" {
                cancel_tagging();
            }
        });
        let result = run_tagging_process(
            app.handle(),
            opts,
            &test_python(),
            &script,
            &model,
            temp,
            limits,
        );
        let events = log.lock().unwrap().clone();
        (result, events)
    }

    /// 测试里等 Python 的时限：留出负载高时启动解释器的余量，又不必等满 120 / 300 秒
    const QUICK: Limits = Limits {
        ready: Duration::from_secs(3),
        silence: Duration::from_secs(3),
    };

    const FAKE_SESSION: &str = r#"
import json, sys, time
from pathlib import Path
def emit(**v): print(json.dumps(v), flush=True)
mode = ''
for line in sys.stdin:
    cmd = json.loads(line)
    if cmd['cmd'] == 'init':
        mode = cmd['preprocess_mode']
        if mode == 'cancel_load':
            emit(type='log', message='cancel_now')
            time.sleep(30)
        if mode == 'init_crash':
            sys.stderr.write('ImportError: DLL load failed\n')
            sys.exit(3)
        if mode == 'init_error':
            emit(type='error', message='ValueError: 词表损坏')
            continue
        if mode == 'init_hang':
            sys.stderr.write('loading huge model\n')
            sys.stderr.flush()
            time.sleep(30)
        emit(type='ready')
    elif cmd['cmd'] == 'tag_batch':
        images = cmd['images']
        for item in images:
            assert item['tag_output_path'], item
        if mode == 'cancel_infer':
            emit(type='log', message='cancel_now')
            time.sleep(30)
        if mode == 'hybrid':
            assert len(images) == 1, images
            item = images[0]
            target = Path(item['tag_output_path'])
            assert str(target) == item['image_path'] + '.purin-local-' + item['output_format']
            assert not target.exists(), 'stale intermediate was not cleared'
            target.write_text('fresh tags')
            emit(type='result', image_path=item['image_path'], tag_count=2)
        elif mode == 'skip':
            assert len(images) == 1, images
            assert images[0]['tag_output_path'] == str(Path(images[0]['image_path']).with_suffix('.txt'))
            emit(type='result', image_path=images[0]['image_path'], tag_count=2)
        elif mode == 'reordered':
            emit(type='error', image_path=images[2]['image_path'], message='preprocess failed')
            emit(type='result', image_path=images[0]['image_path'], tag_count=2)
            emit(type='result', image_path=images[1]['image_path'], tag_count=0, skipped=True)
        elif mode == 'crash':
            if Path(images[0]['image_path']).name == 'b.png':
                sys.stderr.write('MemoryError\n')
                sys.exit(1)
            emit(type='result', image_path=images[0]['image_path'], tag_count=1)
        elif mode == 'hang':
            if Path(images[0]['image_path']).name == 'b.png':
                time.sleep(30)
            emit(type='result', image_path=images[0]['image_path'], tag_count=1)
        elif mode == 'batch_error':
            emit(type='error', message='RuntimeError: session broken')
        elif mode == 'mismatch':
            emit(type='result', image_path=images[0]['image_path'] + '.other', tag_count=1)
            time.sleep(30)
    elif cmd['cmd'] == 'quit': break
"#;

    #[test]
    fn existing_label_skip_matches_output_format() {
        let temp = TempDir::new("tagger_skip");
        let path = temp.join("image.png");
        let mut opts = options(&temp);
        opts.existing_tags_action = "skip".into();
        std::fs::write(path.with_extension("json"), "{}").unwrap();
        assert!(!should_skip(&path, &opts));
        opts.output_format = "json".into();
        assert!(should_skip(&path, &opts));
        std::fs::write(path.with_extension("json"), "").unwrap();
        assert!(!should_skip(&path, &opts), "空文件不算已有标签");
        std::fs::remove_file(path.with_extension("json")).unwrap();
        std::fs::write(path.with_extension("txt"), "solo").unwrap();
        assert!(!should_skip(&path, &opts));
        opts.output_format = "txt".into();
        assert!(should_skip(&path, &opts));
        opts.existing_tags_action = "overwrite".into();
        assert!(!should_skip(&path, &opts));
    }

    #[test]
    fn hybrid_skip_checks_both_formats_and_ignores_intermediates() {
        let temp = TempDir::new("hybrid_skip");
        let path = temp.join("image.png");
        std::fs::write(&path, "fixture").unwrap();
        for format in ["txt", "json"] {
            let mut opts = options(&temp);
            opts.hybrid_mode = true;
            opts.output_format = format.into();
            let draft = super::super::hybrid::draft_path(&path, format);
            let orphan = super::super::hybrid::draft_path(&temp.join("gone.png"), format);
            std::fs::write(&draft, "stale").unwrap();
            std::fs::write(&orphan, "stale").unwrap();
            assert!(!should_skip(&path, &opts));
            assert!(all_skipped(&opts).unwrap().is_none());
            clear_pending_drafts(&temp, &[], &opts).unwrap();
            assert!(draft.exists(), "跳过的图片的草稿要保留");
            assert!(!orphan.exists());
            clear_pending_drafts(&temp, &[&path], &opts).unwrap();
            assert!(!draft.exists());
            for existing in ["txt", "json"] {
                std::fs::write(path.with_extension(existing), "existing").unwrap();
                for action in ["skip", "overwrite"] {
                    opts.existing_tags_action = action.into();
                    assert_eq!(should_skip(&path, &opts), action == "skip");
                    assert_eq!(all_skipped(&opts).unwrap().is_some(), action == "skip");
                }
                assert!(!should_skip(&path, &opts));
                std::fs::remove_file(path.with_extension(existing)).unwrap();
            }
        }
    }

    #[test]
    fn hybrid_scan_respects_single_image_recursion_and_duplicate_names() {
        let temp = TempDir::new("hybrid_recursive");
        std::fs::create_dir(temp.join("nested")).unwrap();
        let first = temp.join("a.png");
        let second = temp.join("nested/a.png");
        for path in [&first, &second] {
            std::fs::write(path, "image").unwrap();
        }
        std::fs::write(first.with_extension("txt"), "existing").unwrap();
        let mut opts = options(&temp);
        opts.hybrid_mode = true;
        opts.existing_tags_action = "skip".into();
        assert_eq!(all_skipped(&opts).unwrap().unwrap().total, 1);
        opts.recursive = true;
        assert!(all_skipped(&opts).unwrap().is_none());
        assert_ne!(
            super::super::hybrid::draft_path(&first, "txt"),
            super::super::hybrid::draft_path(&second, "txt")
        );
        opts.input_path = first.to_string_lossy().into_owned();
        assert_eq!(all_skipped(&opts).unwrap().unwrap().total, 1);
    }

    #[test]
    fn output_paths_come_from_rust_and_non_utf8_names_are_not_sent() {
        let mut opts = options(Path::new("/data"));
        let image = Path::new("/data/a.b.png");
        assert_eq!(tag_output_path(image, &opts), Path::new("/data/a.b.txt"));
        opts.output_format = "json".into();
        assert_eq!(tag_output_path(image, &opts), Path::new("/data/a.b.json"));
        opts.hybrid_mode = true;
        assert_eq!(
            tag_output_path(image, &opts),
            super::super::hybrid::draft_path(image, "json")
        );
        let command = image_command(&json!({"output_format": "json"}), image, &opts);
        assert_eq!(command["image_path"], "/data/a.b.png");
        assert_eq!(command["tag_output_path"], "/data/a.b.png.purin-local-json");
        assert!(sendable(image, &opts));
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStrExt;
            let odd = Path::new(std::ffi::OsStr::from_bytes(b"/data/bad\xff.png"));
            assert!(!sendable(odd, &opts));
            // 即使误传进来也只会有损替换，不会 panic
            let _ = image_command(&json!({}), odd, &opts);
        }
    }

    #[test]
    fn mismatched_or_unlabelled_results_stop_the_batch() {
        let (a, b) = (PathBuf::from("/d/a.png"), PathBuf::from("/d/b.png"));
        let both = [&a, &b];
        assert_eq!(
            locate(&both, &json!({"type": "result", "image_path": "/d/b.png"})),
            Ok(1)
        );
        assert!(
            locate(&both, &json!({"type": "result", "image_path": "/d/c.png"}))
                .unwrap_err()
                .contains("无法对应")
        );
        assert_eq!(
            locate(&both, &json!({"type": "error", "message": "boom"})),
            Err("Python 推理错误: boom".to_string())
        );
        assert!(locate(&both, &json!({"type": "result"})).is_err());
        assert_eq!(
            locate(&[&a], &json!({"type": "error", "message": "boom"})),
            Ok(0)
        );
    }

    #[test]
    fn protocol_results_skip_hybrid_and_cancellation_are_accounted_by_path() {
        let _lock = super::super::TAGGER_TEST_LOCK.lock().unwrap();
        let temp = TempDir::new("tagger_protocol");
        for mode in ["reordered", "skip", "hybrid", "cancel_load", "cancel_infer"] {
            let input = temp.join(mode);
            std::fs::create_dir_all(&input).unwrap();
            for name in ["a.png", "b.png", "c.png"] {
                std::fs::write(input.join(name), "fixture").unwrap();
            }
            let mut opts = options(&input);
            if mode == "skip" {
                opts.existing_tags_action = "skip".into();
                std::fs::write(input.join("a.txt"), "saved").unwrap();
                std::fs::write(input.join("b.txt"), "saved").unwrap();
            }
            if mode == "hybrid" {
                opts.hybrid_mode = true;
                opts.existing_tags_action = "skip".into();
                std::fs::write(input.join("a.txt"), "saved").unwrap();
                std::fs::write(input.join("b.json"), "saved").unwrap();
                std::fs::write(
                    super::super::hybrid::draft_path(&input.join("c.png"), "txt"),
                    "stale",
                )
                .unwrap();
            }
            let (result, events) = run_fake(&temp, &opts, mode, LIMITS);
            let result = result.unwrap();
            let terminal: Vec<_> = events.iter().filter(|e| e["status"] == "done").collect();
            assert_eq!(terminal.len(), 1, "{mode}: {events:?}");
            match mode {
                "reordered" => {
                    assert_eq!((result.success_count, result.fail_count), (2, 1));
                    assert_eq!(result.errors, ["c.png: preprocess failed"]);
                    assert!(!input.join("Fail/a.png").exists());
                    assert!(!input.join("Fail/b.png").exists());
                    assert!(input.join("Fail/c.png").exists());
                    assert_eq!(
                        terminal[0]["message"],
                        "打标完成: 成功 2（含跳过 1）, 失败 1, 共 3"
                    );
                }
                "skip" => {
                    assert_eq!((result.success_count, result.fail_count), (3, 0));
                    assert_eq!(
                        terminal[0]["message"],
                        "打标完成: 成功 3（含跳过 2）, 失败 0, 共 3"
                    );
                }
                "hybrid" => {
                    assert_eq!((result.success_count, result.fail_count), (3, 0));
                    assert_eq!(
                        std::fs::read_to_string(input.join("a.txt")).unwrap(),
                        "saved"
                    );
                    assert_eq!(
                        std::fs::read_to_string(input.join("b.json")).unwrap(),
                        "saved"
                    );
                    assert!(!input.join("c.txt").exists());
                    assert_eq!(
                        std::fs::read_to_string(super::super::hybrid::draft_path(
                            &input.join("c.png"),
                            "txt"
                        ))
                        .unwrap(),
                        "fresh tags"
                    );
                }
                _ => {
                    assert_eq!((result.success_count, result.fail_count), (0, 0));
                    assert!(!input.join("Fail").exists());
                    assert!(terminal[0]["message"]
                        .as_str()
                        .unwrap()
                        .starts_with("已取消"));
                    assert_eq!(terminal[0]["cancelled"], true);
                    assert_eq!(terminal[0]["current"], 0);
                    assert!(!events.iter().any(|e| e["status"] == "error"));
                }
            }
        }
        reset_tagging_cancel();
        assert!(PYTHON_PROCESS.lock().unwrap().is_none());
    }

    /// Python 中途退出、无响应、批次级错误、结果对不上：没有结果的图片（含还没发出去的）都计为失败、
    /// 进 Fail/，返回带原因的 Err，不发 done，也不会等到静默超时
    #[test]
    fn interrupted_session_fails_every_unprocessed_image() {
        let _lock = super::super::TAGGER_TEST_LOCK.lock().unwrap();
        for (mode, batch_size, expected) in [
            (
                "crash",
                1,
                "Python 进程异常退出: MemoryError，2 张未处理；成功 1, 失败 2, 共 3",
            ),
            (
                "hang",
                1,
                "Python 超过 3 秒无响应，已终止进程，2 张未处理；成功 1, 失败 2, 共 3",
            ),
            (
                "batch_error",
                2,
                "Python 推理错误: RuntimeError: session broken，3 张未处理；成功 0, 失败 3, 共 3",
            ),
            ("mismatch", 2, "无法对应的结果"),
        ] {
            let temp = TempDir::new("tagger_interrupted");
            let input = temp.join("dataset");
            std::fs::create_dir_all(&input).unwrap();
            for name in ["a.png", "b.png", "c.png"] {
                std::fs::write(input.join(name), "fixture").unwrap();
            }
            let mut opts = options(&input);
            opts.batch_size = batch_size;
            let started = std::time::Instant::now();
            let (result, events) = run_fake(&temp, &opts, mode, QUICK);
            assert!(started.elapsed() < Duration::from_secs(15), "{mode}");
            let error = result.unwrap_err();
            assert!(error.contains(expected), "{mode}: {error}");
            assert!(
                !events.iter().any(|e| e["status"] == "done"),
                "{mode}: {events:?}"
            );
            let last = events.last().unwrap();
            assert_eq!(last["status"], "info", "{mode}: {events:?}");
            assert!(
                last["message"].as_str().unwrap().contains("Fail/"),
                "{mode}"
            );
            assert_eq!(
                input.join("Fail/a.png").exists(),
                mode != "crash" && mode != "hang"
            );
            assert!(input.join("Fail/b.png").exists(), "{mode}");
            assert!(input.join("Fail/c.png").exists(), "{mode}");
            assert!(PYTHON_PROCESS.lock().unwrap().is_none(), "{mode}");
        }
        reset_tagging_cancel();
    }

    /// 模型加载失败：错误里带上原因（脚本的错误、退出前的 stderr），不归集、不发 done
    #[test]
    fn init_failures_explain_why() {
        let _lock = super::super::TAGGER_TEST_LOCK.lock().unwrap();
        for (mode, expected) in [
            (
                "init_crash",
                "模型初始化失败: Python 进程已退出: ImportError: DLL load failed",
            ),
            ("init_error", "模型初始化失败: ValueError: 词表损坏"),
            ("init_hang", "模型加载超时（3 秒）: loading huge model"),
        ] {
            let temp = TempDir::new("tagger_init_failure");
            let input = temp.join("dataset");
            std::fs::create_dir_all(&input).unwrap();
            std::fs::write(input.join("a.png"), "fixture").unwrap();
            let (result, events) = run_fake(&temp, &options(&input), mode, QUICK);
            assert_eq!(result.unwrap_err(), expected, "{mode}");
            assert!(!events.iter().any(|e| e["status"] == "done"), "{mode}");
            assert!(!input.join("Fail").exists(), "{mode}");
            assert!(PYTHON_PROCESS.lock().unwrap().is_none(), "{mode}");
        }
        reset_tagging_cancel();
    }

    /// 全部跳过时不启动 Python：解释器路径无效也能正常结束
    #[test]
    fn nothing_to_tag_does_not_start_python() {
        let _lock = super::super::TAGGER_TEST_LOCK.lock().unwrap();
        reset_tagging_cancel();
        let temp = TempDir::new("tagger_nothing_to_do");
        std::fs::write(temp.join("a.png"), "fixture").unwrap();
        std::fs::write(temp.join("a.txt"), "saved").unwrap();
        let mut opts = options(&temp);
        opts.existing_tags_action = "skip".into();
        let model = super::super::models::get_builtin_models().remove(0);
        let app = tauri::test::mock_app();
        let events = capture_events(app.handle(), EVENT);
        let result = run_tagging_process(
            app.handle(),
            &opts,
            "/nonexistent/python",
            &temp.join("missing.py"),
            &model,
            &temp,
            LIMITS,
        )
        .unwrap();
        assert_eq!(
            (result.success_count, result.fail_count, result.total),
            (1, 0, 1)
        );
        let events = events.lock().unwrap();
        assert_eq!(
            events.last().unwrap()["message"],
            "打标完成: 成功 1（含跳过 1）, 失败 0, 共 1"
        );
    }
}
