use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Duration;
use tauri::Emitter;

use super::http_download::{download_client, download_to_file, huggingface_url, DownloadProgress};
use super::python_proc::{self, ProtocolReader, Recv, PYTHON_SILENCE_LIMIT};
use super::{ProcessResult, ProgressEvent};

/// 美学评分选项
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AestheticOptions {
    pub input_path: String,
    #[serde(default)]
    pub output_path: String,
    #[serde(default)]
    pub use_gpu: bool,
    /// 复制而非移动（工作流用：输出是临时目录时移动会让原图随清理被删）
    #[serde(default)]
    pub copy_files: bool,
    #[serde(default = "default_batch_size")]
    pub batch_size: u32,
    #[serde(default)]
    pub recursive: bool,
}

fn default_batch_size() -> u32 {
    1
}

/// 全局取消标志
static AESTHETIC_CANCELLED: AtomicBool = AtomicBool::new(false);
static AESTHETIC_PROCESS: Mutex<Option<Child>> = Mutex::new(None);
static DOWNLOAD_CANCELLED: AtomicBool = AtomicBool::new(false);

fn get_aesthetic_model_dir() -> PathBuf {
    super::config_paths::models_dir("aesthetic_models").join("swinv2pv3_v0_448_ls0.2_x")
}

fn is_model_downloaded() -> bool {
    let dir = get_aesthetic_model_dir();
    dir.join("model.onnx").is_file() && dir.join("meta.json").is_file()
}

fn kill_process() {
    super::kill_child_tree(&AESTHETIC_PROCESS);
}

struct ProcessCleanup;
impl Drop for ProcessCleanup {
    fn drop(&mut self) {
        kill_process();
    }
}

async fn download_model(app: &tauri::AppHandle) -> Result<(), String> {
    let client = download_client()?;
    let model_dir = get_aesthetic_model_dir();
    ProgressEvent::new("info", "开始下载美学评分模型...").emit(app, "aesthetic-progress");
    for filename in ["model.onnx", "meta.json"] {
        if DOWNLOAD_CANCELLED.load(Ordering::SeqCst) {
            let _ = app.emit(
                "aesthetic-download",
                DownloadProgress::cancelled("下载已取消"),
            );
            return Err("下载已取消".into());
        }
        let dest = model_dir.join(filename);
        // 已完整下载的权重保留，重试时只补缺失文件。
        if dest.is_file() {
            continue;
        }
        let url = huggingface_url(
            "deepghs/anime_aesthetic",
            &format!("swinv2pv3_v0_448_ls0.2_x/{}", filename),
        );
        let _ = app.emit("aesthetic-download", DownloadProgress::starting(filename));
        let downloaded =
            download_to_file(client.get(url), &dest, filename, &DOWNLOAD_CANCELLED, |p| {
                let _ = app.emit("aesthetic-download", p);
            })
            .await;
        if let Err(error) = downloaded {
            let progress = if DOWNLOAD_CANCELLED.load(Ordering::SeqCst) {
                DownloadProgress::cancelled("下载已取消")
            } else {
                DownloadProgress::error(error.to_string())
            };
            let _ = app.emit("aesthetic-download", progress);
            return Err(error.into());
        }
    }
    if DOWNLOAD_CANCELLED.load(Ordering::SeqCst) {
        let _ = app.emit(
            "aesthetic-download",
            DownloadProgress::cancelled("下载已取消"),
        );
        return Err("下载已取消".into());
    }
    let _ = app.emit(
        "aesthetic-download",
        DownloadProgress::done("美学评分模型下载完成"),
    );
    ProgressEvent::new("success", "美学评分模型下载完成").emit(app, "aesthetic-progress");
    Ok(())
}

fn run_aesthetic_scoring<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    python: &str,
    options: &AestheticOptions,
    model_path: &Path,
    files: &[PathBuf],
) -> Result<ProcessResult, String> {
    kill_process();
    let _cleanup = ProcessCleanup;
    let mut result = ProcessResult {
        total: files.len() as u32,
        ..Default::default()
    };
    if AESTHETIC_CANCELLED.load(Ordering::SeqCst) {
        return Ok(result);
    }
    let script = python_proc::find_script("aesthetic_inference.py")?;
    let mut cmd = python_proc::hidden_command(python);
    cmd.arg(&script)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env("NO_COLOR", "1")
        .env("PYTHONUNBUFFERED", "1")
        .env("PYTHONIOENCODING", "utf-8");
    python_proc::configure_python_command(&mut cmd, options.use_gpu);
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("启动 Python 进程失败: {}", e))?;
    let (Some(mut stdin), Some(stdout), Some(stderr)) =
        (child.stdin.take(), child.stdout.take(), child.stderr.take())
    else {
        super::kill_process_tree(child.id());
        let _ = child.kill();
        let _ = child.wait();
        return Err("无法获取 Python 进程管道".into());
    };
    *AESTHETIC_PROCESS.lock().unwrap_or_else(|e| e.into_inner()) = Some(child);
    let app_err = app.clone();
    std::thread::spawn(move || {
        python_proc::for_each_stderr_line(stderr, |line| {
            if !python_proc::is_runtime_noise(&line) {
                ProgressEvent::new("warning", format!("[Python] {}", line))
                    .emit(&app_err, "aesthetic-progress");
            }
        });
    });
    let reader = ProtocolReader::spawn(stdout);
    let init = serde_json::json!({"cmd": "init", "model_path": model_path.to_string_lossy(), "use_gpu": options.use_gpu});
    writeln!(stdin, "{}", init).map_err(|e| format!("发送 init 命令失败: {}", e))?;
    loop {
        if AESTHETIC_CANCELLED.load(Ordering::SeqCst) {
            return Ok(result);
        }
        match reader.recv(Duration::from_secs(180)) {
            Recv::Msg(msg) => match msg["type"].as_str().unwrap_or("") {
                "log" => ProgressEvent::python_log(&msg, 0, 0).emit(app, "aesthetic-progress"),
                "ready" => break,
                "error" => {
                    return Err(format!(
                        "初始化失败: {}",
                        msg["message"].as_str().unwrap_or("")
                    ))
                }
                _ => {}
            },
            _ if AESTHETIC_CANCELLED.load(Ordering::SeqCst) => return Ok(result),
            _ => return Err("Python 进程未能成功初始化（退出或加载超时 180 秒）".into()),
        }
    }
    ProgressEvent::new("info", format!("读取到 {} 张图片", result.total))
        .at(0, result.total)
        .emit(app, "aesthetic-progress");
    let input_dir = Path::new(&options.input_path);
    let batch_size = options.batch_size.max(1) as usize;
    'batches: for (batch_index, batch) in files.chunks(batch_size).enumerate() {
        if AESTHETIC_CANCELLED.load(Ordering::SeqCst) {
            break;
        }
        let offset = batch_index * batch_size;
        let first_name = super::file_name_lossy(&batch[0]);
        let message = if batch.len() > 1 {
            format!(
                "正在评分: {} 等 {} 张 ({}/{})",
                first_name,
                batch.len(),
                offset + 1,
                result.total
            )
        } else {
            format!("正在评分: {} ({}/{})", first_name, offset + 1, result.total)
        };
        ProgressEvent::new("processing", message)
            .at(offset as u32 + 1, result.total)
            .file(&first_name)
            .emit(app, "aesthetic-progress");
        let images: Vec<_> = batch
            .iter()
            .map(|path| {
                let relative_dir = if options.output_path.is_empty() {
                    String::new()
                } else {
                    super::relative_dir_for_input(input_dir, path, options.recursive)
                        .map(|p| p.to_string_lossy().replace('\\', "/"))
                        .unwrap_or_default()
                };
                serde_json::json!({
                    "image_path": path.to_string_lossy(), "copy_files": options.copy_files,
                    "output_path": options.output_path, "relative_dir": relative_dir,
                })
            })
            .collect();
        let mut pending: HashSet<String> = batch
            .iter()
            .map(|p| p.to_string_lossy().into_owned())
            .collect();
        let command = serde_json::json!({"cmd": "score_batch", "images": images});
        if let Err(e) = writeln!(stdin, "{}", command) {
            if !AESTHETIC_CANCELLED.load(Ordering::SeqCst) {
                result.fail_count += pending.len() as u32;
                result.errors.push(format!("批量发送失败: {}", e));
            }
            break;
        }
        while !pending.is_empty() {
            if AESTHETIC_CANCELLED.load(Ordering::SeqCst) {
                break 'batches;
            }
            let msg = match reader.recv(PYTHON_SILENCE_LIMIT) {
                Recv::Msg(msg) => msg,
                received => {
                    if !AESTHETIC_CANCELLED.load(Ordering::SeqCst) {
                        result.fail_count += pending.len() as u32;
                        result.errors.push(
                            match received {
                                Recv::TimedOut => "Python 进程无响应（300 秒）",
                                _ => "Python 进程退出",
                            }
                            .into(),
                        );
                    }
                    break 'batches;
                }
            };
            let kind = msg["type"].as_str().unwrap_or("");
            if kind == "log" {
                ProgressEvent::python_log(
                    &msg,
                    result.success_count + result.fail_count + 1,
                    result.total,
                )
                .emit(app, "aesthetic-progress");
                continue;
            }
            if !matches!(kind, "result" | "error") {
                continue;
            }
            if kind == "error" && msg.get("image_path").is_none() {
                result.fail_count += pending.len() as u32;
                result
                    .errors
                    .push(msg["message"].as_str().unwrap_or("Python 处理失败").into());
                break 'batches;
            }
            let path = msg["image_path"].as_str().unwrap_or("");
            if !pending.remove(path) {
                continue;
            }
            let filename = super::file_name_lossy(Path::new(path));
            let (status, message) = if kind == "result" {
                result.success_count += 1;
                (
                    "success",
                    format!(
                        "[完成] {} → {} (分数: {:.2}, 置信度: {:.1}%)",
                        filename,
                        msg["label"].as_str().unwrap_or("?"),
                        msg["score"].as_f64().unwrap_or(0.0),
                        msg["confidence"].as_f64().unwrap_or(0.0) * 100.0
                    ),
                )
            } else {
                result.fail_count += 1;
                let text = msg["message"].as_str().unwrap_or("unknown");
                result.errors.push(format!("{}: {}", filename, text));
                ("error", format!("[错误] {}: {}", filename, text))
            };
            ProgressEvent::new(status, message)
                .at(result.success_count + result.fail_count, result.total)
                .file(filename)
                .emit(app, "aesthetic-progress");
        }
    }
    let _ = writeln!(stdin, r#"{{"cmd":"quit"}}"#);
    Ok(result)
}

fn terminal_event(result: &ProcessResult, cancelled: bool) -> ProgressEvent {
    let message = if cancelled {
        format!(
            "已取消: 成功 {}, 失败 {}",
            result.success_count, result.fail_count
        )
    } else {
        format!(
            "美学评分完成: 成功 {}, 失败 {}, 共 {}",
            result.success_count, result.fail_count, result.total
        )
    };
    ProgressEvent::new("done", message).at(
        if cancelled {
            result.success_count + result.fail_count
        } else {
            result.total
        },
        result.total,
    )
}

#[tauri::command]
pub async fn start_aesthetic_scoring(
    app: tauri::AppHandle,
    options: AestheticOptions,
) -> Result<ProcessResult, String> {
    static RUNNING: AtomicBool = AtomicBool::new(false);
    let _busy = super::BusyGuard::acquire(&RUNNING, "美学评分")?;
    AESTHETIC_CANCELLED.store(false, Ordering::SeqCst);
    DOWNLOAD_CANCELLED.store(false, Ordering::SeqCst);
    let scan = options.clone();
    let files = tokio::task::spawn_blocking(move || {
        let output = (!scan.output_path.is_empty()).then(|| Path::new(&scan.output_path));
        super::collect_image_files_with_recursive_excluding(
            Path::new(&scan.input_path),
            scan.recursive,
            output,
        )
    })
    .await
    .map_err(|e| format!("读取图片失败: {}", e))??;
    let total = files.len() as u32;
    let outcome = async {
        if total == 0 || AESTHETIC_CANCELLED.load(Ordering::SeqCst) {
            return Ok(ProcessResult {
                total,
                ..Default::default()
            });
        }
        let python = super::python_env::setup_python_env(&app, "aesthetic").await?;
        if AESTHETIC_CANCELLED.load(Ordering::SeqCst) {
            return Err("已取消".into());
        }
        super::python_env::ensure_onnx_gpu_runtime(&app, &python, "aesthetic").await?;
        if AESTHETIC_CANCELLED.load(Ordering::SeqCst) {
            return Err("已取消".into());
        }
        if !is_model_downloaded() {
            download_model(&app).await?;
        }
        let model_path = get_aesthetic_model_dir().join("model.onnx");
        let app_run = app.clone();
        tokio::task::spawn_blocking(move || {
            run_aesthetic_scoring(&app_run, &python, &options, &model_path, &files)
        })
        .await
        .map_err(|e| format!("任务执行失败: {}", e))?
    }
    .await;
    let cancelled = AESTHETIC_CANCELLED.load(Ordering::SeqCst);
    let result = if cancelled {
        outcome.unwrap_or(ProcessResult {
            total,
            ..Default::default()
        })
    } else {
        outcome?
    };
    terminal_event(&result, cancelled).emit(&app, "aesthetic-progress");
    Ok(result)
}

#[tauri::command]
pub fn cancel_aesthetic_scoring() {
    AESTHETIC_CANCELLED.store(true, Ordering::SeqCst);
    DOWNLOAD_CANCELLED.store(true, Ordering::SeqCst);
    super::python_env::cancel_setup_for("aesthetic");
    kill_process();
}

#[tauri::command]
pub fn force_cancel_aesthetic_scoring() {
    cancel_aesthetic_scoring();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn last_batch_cancellation_has_one_cancelled_terminal() {
        let result = ProcessResult {
            success_count: 2,
            total: 2,
            ..Default::default()
        };
        let event = terminal_event(&result, true);
        assert_eq!(event.status, "done");
        assert_eq!(event.current, 2);
        assert!(event.message.starts_with("已取消"));
        assert!(terminal_event(&result, false)
            .message
            .starts_with("美学评分完成"));
    }

    #[cfg(unix)]
    #[test]
    fn single_batch_skips_noise_and_cancels_after_final_result() {
        use std::os::unix::fs::PermissionsExt;
        use tauri::Listener;
        let root = std::env::temp_dir().join(format!("purin_ai_aesthetic_{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let image = root.join("a.png");
        let response = serde_json::json!({"type": "result", "image_path": image, "label": "good", "score": 3.0, "confidence": 1.0});
        let script = root.join("fake-python");
        std::fs::write(&script, format!("#!/bin/sh\nread -r line\nprintf '%s\\n' '{{\"type\":\"ready\"}}'\nread -r line\ncase \"$line\" in *score_batch*) ;; *) exit 3;; esac\nprintf '\\377noise\\n'\nprintf '%s\\n' '{{\"type\":\"result\",\"image_path\":\"other.png\"}}' '{{\"type\":\"log\",\"message\":\"fallback\"}}' '{}'\nread -r line\n", response)).unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700)).unwrap();
        let app = tauri::test::mock_app();
        let events = super::super::batch::capture_events(app.handle(), "aesthetic-progress");
        app.listen_any("aesthetic-progress", |event| {
            let message: serde_json::Value = serde_json::from_str(event.payload()).unwrap();
            if message["status"] == "success" {
                AESTHETIC_CANCELLED.store(true, Ordering::SeqCst);
                kill_process();
            }
        });
        AESTHETIC_CANCELLED.store(false, Ordering::SeqCst);
        let options = AestheticOptions {
            input_path: root.to_string_lossy().into_owned(),
            output_path: String::new(),
            use_gpu: false,
            copy_files: true,
            batch_size: 1,
            recursive: false,
        };
        let result = run_aesthetic_scoring(
            app.handle(),
            script.to_str().unwrap(),
            &options,
            &root.join("fake.onnx"),
            &[image],
        )
        .unwrap();
        assert_eq!((result.success_count, result.fail_count), (1, 0));
        terminal_event(&result, AESTHETIC_CANCELLED.load(Ordering::SeqCst))
            .emit(app.handle(), "aesthetic-progress");
        let events = events.lock().unwrap();
        let terminal: Vec<_> = events.iter().filter(|e| e["status"] == "done").collect();
        assert_eq!(terminal.len(), 1);
        assert!(terminal[0]["message"]
            .as_str()
            .unwrap()
            .starts_with("已取消"));
        assert!(events.iter().any(|e| e["message"] == "fallback"));
        assert!(AESTHETIC_PROCESS.lock().unwrap().is_none());
        AESTHETIC_CANCELLED.store(false, Ordering::SeqCst);
        std::fs::remove_dir_all(root).unwrap();
    }
}
