use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use super::models::ModelDefinition;
use super::{OnnxModelInfo, ProcessResult, ProgressEvent, TaggerOptions};
use crate::commands::python_proc::{self, ProtocolReader, Recv, PYTHON_SILENCE_LIMIT};
use crate::commands::{collect_image_files_with_recursive, file_name_lossy, report_failed_copies};

/// 全局打标取消标志
static TAGGING_CANCELLED: AtomicBool = AtomicBool::new(false);

/// 当前打标/转换任务的 Python 子进程（每次任务新建），取消时经此终止
static PYTHON_PROCESS: Mutex<Option<Child>> = Mutex::new(None);

/// 取消打标
pub fn cancel_tagging() {
    TAGGING_CANCELLED.store(true, Ordering::SeqCst);
    kill_python_process();
}

/// 重置取消标志（开始新任务前调用）
pub fn reset_tagging_cancel() {
    TAGGING_CANCELLED.store(false, Ordering::SeqCst);
}

/// 检查是否已取消
pub fn is_tagging_cancelled() -> bool {
    TAGGING_CANCELLED.load(Ordering::SeqCst)
}

/// 把子进程句柄登记到全局，让 cancel_tagging / kill_python_process 能终止它。
/// 转换模式也必须登记，否则取消时杀不到那个进程。
pub(crate) fn register_python_process(child: Child) {
    *PYTHON_PROCESS.lock().unwrap_or_else(|e| e.into_inner()) = Some(child);
    if is_tagging_cancelled() {
        kill_python_process();
    }
}

/// 取出全局句柄（已被取消杀掉时为 None），供调用方 wait 回收
pub(crate) fn take_python_process() -> Option<Child> {
    PYTHON_PROCESS.lock().ok().and_then(|mut g| g.take())
}

/// 杀死正在运行的 Python 推理进程
pub fn kill_python_process() {
    // 按进程树终止：Python 会派生工作进程，单杀直接子进程会留下孤儿进程
    crate::commands::kill_child_tree(&PYTHON_PROCESS);
}

/// 自动检测 ONNX 模型的输入信息（使用 Python 调用）
pub fn detect_model_info(model_path: &str) -> Result<OnnxModelInfo, String> {
    let python = crate::commands::python_env::get_python_exe().ok_or("未找到可用的 Python 环境")?;
    let script = python_proc::find_script("tagger_inference.py")?;

    let mut cmd = Command::new(&python);
    cmd.args([script.to_string_lossy().as_ref(), "--detect", model_path])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env("PYTHONIOENCODING", "utf-8");
    // Windows 下隐藏控制台窗口
    python_proc::configure_python_command(&mut cmd, false);
    let output = cmd
        .output()
        .map_err(|e| format!("启动 Python 失败: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("模型检测失败: {}", stderr));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines() {
        if let Ok(val) = serde_json::from_str::<serde_json::Value>(line) {
            if val.get("type").and_then(|v| v.as_str()) == Some("model_info") {
                let input_size = val
                    .get("input_size")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(448) as u32;
                let input_format = val
                    .get("input_format")
                    .and_then(|v| v.as_str())
                    .unwrap_or("NHWC")
                    .to_string();
                let shape: Vec<i64> = val
                    .get("input_shape")
                    .and_then(|v| v.as_array())
                    .map(|a| a.iter().filter_map(|v| v.as_i64()).collect())
                    .unwrap_or_default();

                return Ok(OnnxModelInfo {
                    input_size,
                    input_format,
                    input_shape: shape,
                });
            }
        }
    }

    Err("无法解析模型信息".into())
}

pub(super) fn emit_summary<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    result: &ProcessResult,
    skipped: u32,
    cancelled: bool,
) {
    let completed = result.success_count + result.fail_count;
    let message = if cancelled {
        format!(
            "已取消: 已完成 {}/{}, 成功 {}, 失败 {}",
            completed, result.total, result.success_count, result.fail_count
        )
    } else if skipped > 0 {
        format!(
            "打标完成: 成功 {}（含跳过 {}）, 失败 {}, 共 {}",
            result.success_count, skipped, result.fail_count, result.total
        )
    } else {
        format!(
            "打标完成: 成功 {}, 失败 {}, 共 {}",
            result.success_count, result.fail_count, result.total
        )
    };
    ProgressEvent::new("done", message)
        .at(
            if cancelled { completed } else { result.total },
            result.total,
        )
        .emit(app, "tagger-progress");
}

fn should_skip(path: &Path, options: &TaggerOptions) -> bool {
    options.existing_tags_action == "skip"
        && if options.hybrid_mode {
            super::hybrid::has_labels(path)
        } else if options.output_format == "json" {
            path.with_extension("json").exists()
        } else {
            path.with_extension("txt").exists()
                || (options.also_skip_json && path.with_extension("json").exists())
        }
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

fn clear_pending_drafts(files: &[PathBuf], options: &TaggerOptions) -> Result<(), String> {
    if options.hybrid_mode {
        // 先清掉本轮待重打的旧中间文件，避免推理失败后 VLM 读到上次结果。
        for path in files.iter().filter(|path| !should_skip(path, options)) {
            super::hybrid::clear_drafts(path)?;
        }
    }
    Ok(())
}

// 所有提前返回路径都必须回收已登记的子进程。
pub(super) struct ProcessGuard;
impl Drop for ProcessGuard {
    fn drop(&mut self) {
        kill_python_process();
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
    run_tagging_process(app, options, python, model, model_dir, &script)
}

fn run_tagging_process<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    options: &TaggerOptions,
    python: &str,
    model: &ModelDefinition,
    model_dir: &Path,
    script: &Path,
) -> Result<ProcessResult, String> {
    kill_python_process();
    let input_dir = Path::new(&options.input_path);
    let files = collect_image_files_with_recursive(input_dir, options.recursive)?;
    let total = files.len() as u32;
    let mut result = ProcessResult {
        total,
        ..Default::default()
    };
    if is_tagging_cancelled() {
        emit_summary(app, &result, 0, true);
        return Ok(result);
    }

    clear_pending_drafts(&files, options)?;
    let mut cmd = Command::new(python);
    cmd.arg(script)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env("NO_COLOR", "1")
        .env("PYTHONUNBUFFERED", "1")
        .env("PYTHONIOENCODING", "utf-8");
    python_proc::configure_python_command_with_priority(
        &mut cmd,
        options.use_gpu,
        options.use_gpu && model.heavy_gpu,
    );
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("启动 Python 进程失败: {}", e))?;
    let (mut stdin, stdout, stderr) =
        match (child.stdin.take(), child.stdout.take(), child.stderr.take()) {
            (Some(i), Some(o), Some(e)) => (i, o, e),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("无法获取 Python 进程管道".into());
            }
        };
    register_python_process(child);
    let _guard = ProcessGuard;
    let app_err = app.clone();
    let stderr_reader = std::thread::spawn(move || {
        python_proc::for_each_stderr_line(stderr, |line| {
            if !python_proc::is_runtime_noise(&line) && !is_tagging_cancelled() {
                ProgressEvent::new("warning", format!("[Python] {}", line))
                    .emit(&app_err, "tagger-progress");
            }
        });
    });
    let reader = ProtocolReader::spawn(stdout);
    let init_cmd = serde_json::json!({
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
    if let Err(e) = writeln!(stdin, "{}", init_cmd) {
        if is_tagging_cancelled() {
            emit_summary(app, &result, 0, true);
            return Ok(result);
        }
        return Err(format!("发送 init 命令失败: {}", e));
    }

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(120);
    loop {
        let received = reader.recv_until(deadline);
        if is_tagging_cancelled() {
            emit_summary(app, &result, 0, true);
            return Ok(result);
        }
        match received {
            Recv::Msg(msg) => match msg["type"].as_str().unwrap_or("") {
                "ready" => break,
                "log" => ProgressEvent::python_log(&msg, 0, 0).emit(app, "tagger-progress"),
                "error" => {
                    return Err(format!(
                        "Python 推理错误: {}",
                        msg["message"].as_str().unwrap_or("")
                    ));
                }
                _ => {}
            },
            Recv::Closed => return Err("Python 进程未能成功初始化".into()),
            Recv::TimedOut => return Err("模型加载超时(120秒)".into()),
        }
    }

    ProgressEvent::new("info", format!("读取到 {} 张图片", total))
        .at(0, total)
        .emit(app, "tagger-progress");
    let base_cmd = serde_json::json!(options);
    let mut skipped = 0;
    let mut failed_files = Vec::new();
    'batches: for chunk in files.chunks(options.batch_size.max(1) as usize) {
        if is_tagging_cancelled() {
            break;
        }
        let mut pending = Vec::new();
        for path in chunk {
            if should_skip(path, options) {
                skipped += 1;
                result.success_count += 1;
                let name = file_name_lossy(path);
                ProgressEvent::new(
                    "success",
                    format!("[跳过] {}（已有标签或原文件受保护）", name),
                )
                .at(result.success_count + result.fail_count, total)
                .file(name)
                .emit(app, "tagger-progress");
            } else {
                pending.push(path.clone());
            }
        }
        if pending.is_empty() {
            continue;
        }
        let name = file_name_lossy(&pending[0]);
        let current = result.success_count + result.fail_count + 1;
        let message = if pending.len() > 1 {
            format!(
                "正在处理: {} 等 {} 张 ({}/{})",
                name,
                pending.len(),
                current,
                total
            )
        } else {
            format!("正在处理: {} ({}/{})", name, current, total)
        };
        ProgressEvent::new("processing", message)
            .at(current, total)
            .file(name)
            .emit(app, "tagger-progress");
        let images: Vec<_> = pending
            .iter()
            .map(|path| {
                let mut value = base_cmd.clone();
                value["image_path"] = serde_json::json!(path.to_string_lossy());
                if options.hybrid_mode {
                    value["tag_output_path"] =
                        serde_json::json!(super::hybrid::draft_path(path, &options.output_format));
                }
                value
            })
            .collect();
        if let Err(e) = writeln!(
            stdin,
            "{}",
            serde_json::json!({"cmd": "tag_batch", "images": images})
        ) {
            if !is_tagging_cancelled() {
                fail_pending(
                    &mut result,
                    &mut failed_files,
                    &pending,
                    format!("批量发送失败: {}", e),
                );
            }
            break;
        }

        while !pending.is_empty() {
            let received = reader.recv(PYTHON_SILENCE_LIMIT);
            // 取消会关闭管道；在解释 EOF/error 前检查，不能把中断中的图片归入 Fail/。
            if is_tagging_cancelled() {
                break 'batches;
            }
            match received {
                Recv::Msg(msg) => match msg["type"].as_str().unwrap_or("") {
                    "log" => ProgressEvent::python_log(
                        &msg,
                        result.success_count + result.fail_count + 1,
                        total,
                    )
                    .emit(app, "tagger-progress"),
                    "result" | "error" => {
                        let path = msg["image_path"].as_str().map(PathBuf::from);
                        let index = path
                            .as_ref()
                            .and_then(|p| pending.iter().position(|f| f == p));
                        let index = match index {
                            Some(i) => i,
                            None if path.is_none() && pending.len() == 1 => 0,
                            None if path.is_none() && msg["type"] == "error" => {
                                fail_pending(
                                    &mut result,
                                    &mut failed_files,
                                    &pending,
                                    msg["message"]
                                        .as_str()
                                        .unwrap_or("Python 推理错误")
                                        .to_string(),
                                );
                                break 'batches;
                            }
                            _ => continue,
                        };
                        let file = pending.remove(index);
                        let name = file_name_lossy(&file);
                        let (status, message) = if msg["type"] == "result" {
                            result.success_count += 1;
                            if msg["skipped"].as_bool().unwrap_or(false) {
                                skipped += 1;
                                (
                                    "success",
                                    format!("[跳过] {}（已有标签或原文件受保护）", name),
                                )
                            } else {
                                (
                                    "success",
                                    format!(
                                        "[完成] {} → {} 个标签",
                                        name,
                                        msg["tag_count"].as_u64().unwrap_or(0)
                                    ),
                                )
                            }
                        } else {
                            result.fail_count += 1;
                            let text = msg["message"].as_str().unwrap_or("unknown");
                            result.errors.push(format!("{}: {}", name, text));
                            failed_files.push(file);
                            ("error", format!("[错误] {}: {}", name, text))
                        };
                        ProgressEvent::new(status, message)
                            .at(result.success_count + result.fail_count, total)
                            .file(name)
                            .emit(app, "tagger-progress");
                    }
                    _ => {}
                },
                Recv::Closed | Recv::TimedOut => {
                    let message = if matches!(received, Recv::TimedOut) {
                        format!(
                            "Python 超过 {} 秒无响应，已终止进程",
                            PYTHON_SILENCE_LIMIT.as_secs()
                        )
                    } else {
                        "Python 进程退出".to_string()
                    };
                    fail_pending(&mut result, &mut failed_files, &pending, message);
                    kill_python_process();
                    break 'batches;
                }
            }
        }
    }

    let _ = writeln!(stdin, "{{\"cmd\":\"quit\"}}");
    drop(stdin);
    if let Some(mut child) = take_python_process() {
        let _ = child.wait();
    }
    let _ = stderr_reader.join();
    report_failed_copies(
        app,
        "tagger-progress",
        input_dir,
        &failed_files,
        options.recursive,
        total,
    );
    emit_summary(app, &result, skipped, is_tagging_cancelled());
    Ok(result)
}

fn fail_pending(
    result: &mut ProcessResult,
    failed: &mut Vec<PathBuf>,
    pending: &[PathBuf],
    message: String,
) {
    result.fail_count += pending.len() as u32;
    failed.extend_from_slice(pending);
    result.errors.push(message);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::batch::capture_events;
    use crate::commands::http_download::test_support::TempDir;
    use serde_json::json;
    use tauri::Listener;

    fn options(root: &Path) -> TaggerOptions {
        serde_json::from_value(json!({
            "input_path": root, "model_id": "mock", "general_threshold": 0.35,
            "character_threshold": 0.85, "enabled_categories": ["general"],
            "use_gpu": false, "batch_size": 3,
        }))
        .unwrap()
    }

    #[test]
    fn existing_label_skip_matches_output_format() {
        let temp = TempDir::new("tagger_skip");
        let path = temp.join("image.png");
        let mut opts = options(&temp);
        opts.existing_tags_action = "skip".into();
        std::fs::write(path.with_extension("json"), "{}").unwrap();
        assert!(!should_skip(&path, &opts));
        opts.also_skip_json = true;
        assert!(should_skip(&path, &opts));
        opts.also_skip_json = false;
        opts.output_format = "json".into();
        assert!(should_skip(&path, &opts));
        std::fs::remove_file(path.with_extension("json")).unwrap();
        std::fs::write(path.with_extension("txt"), "").unwrap();
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
            std::fs::write(&draft, "stale").unwrap();
            assert!(!should_skip(&path, &opts));
            assert!(all_skipped(&opts).unwrap().is_none());
            clear_pending_drafts(&[path.clone()], &opts).unwrap();
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
    fn protocol_results_skip_and_cancellation_are_accounted_by_path() {
        let _lock = super::super::TAGGER_TEST_LOCK.lock().unwrap();
        let temp = TempDir::new("tagger_protocol");
        let script = temp.join("fake.py");
        std::fs::write(
            &script,
            r#"
import json, sys, time
def emit(**v): print(json.dumps(v), flush=True)
mode = ''
for line in sys.stdin:
    cmd = json.loads(line)
    if cmd['cmd'] == 'init':
        mode = cmd['preprocess_mode']
        if mode == 'cancel_load':
            emit(type='log', message='cancel_now')
            time.sleep(30)
        emit(type='ready')
    elif cmd['cmd'] == 'tag_batch':
        images = cmd['images']
        if mode == 'cancel_infer':
            emit(type='log', message='cancel_now')
            time.sleep(30)
        if mode == 'hybrid':
            from pathlib import Path
            assert len(images) == 1, images
            item = images[0]
            target = Path(item['tag_output_path'])
            assert str(target) == item['image_path'] + '.purin-local-' + item['output_format']
            assert not target.exists(), 'stale intermediate was not cleared'
            target.write_text('fresh tags')
            emit(type='result', image_path=item['image_path'], tag_count=2)
        elif mode == 'skip':
            assert len(images) == 1, images
            emit(type='result', image_path=images[0]['image_path'], tag_count=2)
        else:
            emit(type='error', image_path=images[2]['image_path'], message='preprocess failed')
            emit(type='result', image_path=images[0]['image_path'], tag_count=2)
            sys.exit(1)
    elif cmd['cmd'] == 'quit': break
"#,
        )
        .unwrap();
        let python = Path::new(env!("CARGO_MANIFEST_DIR")).join("../env/python/venv/bin/python3");
        let python = if python.exists() {
            python.to_string_lossy().into_owned()
        } else {
            "python3".into()
        };
        let mut model = super::super::models::get_builtin_models().remove(0);
        for mode in ["reordered", "skip", "hybrid", "cancel_load", "cancel_infer"] {
            let input = temp.join(mode);
            std::fs::create_dir_all(&input).unwrap();
            for name in ["a.png", "b.png", "c.png"] {
                std::fs::write(input.join(name), "fixture").unwrap();
            }
            model.preprocess_mode = mode.into();
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
            reset_tagging_cancel();
            let app = tauri::test::mock_app();
            let log = capture_events(app.handle(), "tagger-progress");
            app.listen_any("tagger-progress", |event| {
                let value: serde_json::Value = serde_json::from_str(event.payload()).unwrap();
                if value["message"] == "cancel_now" {
                    cancel_tagging();
                }
            });
            let result =
                run_tagging_process(app.handle(), &opts, &python, &model, &temp, &script).unwrap();
            let events = log.lock().unwrap();
            let terminal: Vec<_> = events.iter().filter(|e| e["status"] == "done").collect();
            assert_eq!(terminal.len(), 1, "{mode}: {events:?}");
            match mode {
                "reordered" => {
                    assert_eq!((result.success_count, result.fail_count), (1, 2));
                    assert!(!input.join("Fail/a.png").exists());
                    assert!(input.join("Fail/b.png").exists());
                    assert!(input.join("Fail/c.png").exists());
                }
                "skip" => assert_eq!((result.success_count, result.fail_count), (3, 0)),
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
                    assert_eq!(terminal[0]["current"], 0);
                    assert!(!events.iter().any(|e| e["status"] == "error"));
                }
            }
        }
        reset_tagging_cancel();
        assert!(take_python_process().is_none());
    }
}
