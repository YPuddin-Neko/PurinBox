use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use tauri::Emitter;

use super::http_download::{download_client, download_to_file, huggingface_url, DownloadProgress};
use super::python_proc::{self, PidRegistration, ProtocolReader, Recv, PYTHON_SILENCE_LIMIT};
use super::{
    collect_image_files_with_recursive_excluding, output_dir_for_input, ProcessResult,
    ProgressEvent,
};

// ===== DeepGHS Anime Detection Models =====

/// 4 个裁切类型对应的模型定义
struct CropModelDef {
    id: &'static str,
    name: &'static str,
    crop_type: &'static str, // "person" | "halfbody" | "head" | "eyes"
    repo: &'static str,
    subfolder: &'static str,
}

const CROP_MODELS: &[CropModelDef] = &[
    CropModelDef {
        id: "person_detect_v1.1_m",
        name: "全身检测 (person_detect_v1.1_m)",
        crop_type: "person",
        repo: "deepghs/anime_person_detection",
        subfolder: "person_detect_v1.1_m",
    },
    CropModelDef {
        id: "halfbody_detect_v1.0_s",
        name: "半身检测 (halfbody_detect_v1.0_s)",
        crop_type: "halfbody",
        repo: "deepghs/anime_halfbody_detection",
        subfolder: "halfbody_detect_v1.0_s",
    },
    CropModelDef {
        id: "head_detect_v2.0_x",
        name: "头部检测 (head_detect_v2.0_x)",
        crop_type: "head",
        repo: "deepghs/anime_head_detection",
        subfolder: "head_detect_v2.0_x",
    },
    CropModelDef {
        id: "eye_detect_v1.0_s",
        name: "眼部检测 (eye_detect_v1.0_s)",
        crop_type: "eyes",
        repo: "deepghs/anime_eye_detection",
        subfolder: "eye_detect_v1.0_s",
    },
];

fn get_models_dir() -> PathBuf {
    super::config_paths::models_dir("crop_models")
}

fn model_onnx_path(model: &CropModelDef) -> PathBuf {
    get_models_dir().join(model.id).join("model.onnx")
}

/// 模型状态信息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CropModelInfo {
    pub crop_type: String,
    pub downloaded: bool,
}

#[tauri::command]
pub fn get_person_crop_models() -> Result<Vec<CropModelInfo>, String> {
    Ok(CROP_MODELS
        .iter()
        .map(|m| CropModelInfo {
            crop_type: m.crop_type.to_string(),
            downloaded: model_onnx_path(m).exists(),
        })
        .collect())
}

#[tauri::command]
pub async fn download_person_crop_model(app: tauri::AppHandle) -> Result<String, String> {
    let _busy = super::BusyGuard::acquire(&PERSON_CROP_RUNNING, "人物裁切")?;
    CANCEL_FLAG.store(false, Ordering::SeqCst);
    let models: Vec<_> = CROP_MODELS
        .iter()
        .filter(|m| !model_onnx_path(m).is_file())
        .collect();
    if models.is_empty() {
        let _ = app.emit(
            "person-crop-download",
            DownloadProgress::done("所有模型已就绪"),
        );
        return Ok("all_ready".into());
    }
    let client = download_client()?;
    for (index, model) in models.iter().enumerate() {
        let prefix = format!("[{}/{}] {}", index + 1, models.len(), model.name);
        let url = huggingface_url(model.repo, &format!("{}/model.onnx", model.subfolder));
        let _ = app.emit(
            "person-crop-download",
            DownloadProgress::new("downloading", 0.0, format!("{} — 开始下载...", prefix)),
        );
        let outcome = download_to_file(
            client.get(url),
            &model_onnx_path(model),
            &prefix,
            &CANCEL_FLAG,
            |p| {
                let _ = app.emit("person-crop-download", p);
            },
        )
        .await;
        if let Err(error) = outcome {
            let p = if CANCEL_FLAG.load(Ordering::SeqCst) {
                DownloadProgress::cancelled("下载已取消")
            } else {
                DownloadProgress::error(error.to_string())
            };
            let _ = app.emit("person-crop-download", p);
            return Err(error.into());
        }
        if CANCEL_FLAG.load(Ordering::SeqCst) {
            let _ = app.emit(
                "person-crop-download",
                DownloadProgress::cancelled("下载已取消"),
            );
            return Err("下载已取消".into());
        }
        let _ = app.emit(
            "person-crop-download",
            DownloadProgress::done(format!("{} — 下载完成 ✓", prefix)),
        );
    }
    Ok("done".into())
}

// ===== Person Crop Processing =====

static CANCEL_FLAG: AtomicBool = AtomicBool::new(false);
static CHILD_PROCESS: Mutex<Option<u32>> = Mutex::new(None);
static PERSON_CROP_RUNNING: AtomicBool = AtomicBool::new(false);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersonCropOptions {
    pub input_path: String,
    pub output_path: String,
    pub use_gpu: bool,
    // person (full body)
    pub person_enabled: bool,
    pub person_conf: f64,
    // upper body
    pub upper_enabled: bool,
    pub upper_conf: f64,
    pub upper_tag: String,
    // head
    pub head_enabled: bool,
    pub head_conf: f64,
    pub head_tag: String,
    pub head_scale: f64,
    // eyes
    pub eyes_enabled: bool,
    pub eyes_conf: f64,
    pub eyes_tag: String,
    pub eyes_scale: f64,
    // other
    pub keep_original_tags: bool,
    #[serde(default)]
    pub recursive: bool,
}

#[tauri::command]
pub async fn start_person_crop(
    app: tauri::AppHandle,
    options: PersonCropOptions,
) -> Result<ProcessResult, String> {
    let _busy = super::BusyGuard::acquire(&PERSON_CROP_RUNNING, "人物裁切")?;
    CANCEL_FLAG.store(false, Ordering::SeqCst);
    let scan = options.clone();
    let files = tokio::task::spawn_blocking(move || {
        collect_image_files_with_recursive_excluding(
            Path::new(&scan.input_path),
            scan.recursive,
            Some(Path::new(&scan.output_path)),
        )
    })
    .await
    .map_err(|e| format!("读取图片失败: {}", e))??;
    let total = files.len() as u32;
    let outcome = async {
        if files.is_empty() || CANCEL_FLAG.load(Ordering::SeqCst) {
            return Ok(ProcessResult {
                total,
                ..Default::default()
            });
        }
        let model_paths = build_model_paths(&options)?;
        let python = super::python_env::setup_python_env(&app, "person-crop").await?;
        if CANCEL_FLAG.load(Ordering::SeqCst) {
            return Err("已取消".into());
        }
        super::python_env::ensure_onnx_gpu_runtime(&app, &python, "person-crop").await?;
        if CANCEL_FLAG.load(Ordering::SeqCst) {
            return Err("已取消".into());
        }
        ProgressEvent::new("info", "开始裁切...").emit(&app, "person-crop-progress");
        let app_run = app.clone();
        tokio::task::spawn_blocking(move || {
            run_person_crop(&app_run, &python, &options, &files, model_paths)
        })
        .await
        .map_err(|e| format!("任务执行失败: {}", e))?
    }
    .await;
    let cancelled = CANCEL_FLAG.load(Ordering::SeqCst);
    let result = if cancelled {
        outcome.unwrap_or(ProcessResult {
            total,
            ..Default::default()
        })
    } else {
        outcome?
    };
    terminal_event(&result, cancelled).emit(&app, "person-crop-progress");
    Ok(result)
}

fn terminal_event(result: &ProcessResult, cancelled: bool) -> ProgressEvent {
    let processed = result.success_count + result.fail_count;
    ProgressEvent::new(
        "done",
        if cancelled {
            format!("已取消: 已处理 {}, 共 {}", processed, result.total)
        } else {
            format!(
                "处理完成: 成功 {}, 失败 {}, 共 {}",
                result.success_count, result.fail_count, result.total
            )
        },
    )
    .at(
        if cancelled { processed } else { result.total },
        result.total,
    )
}

#[tauri::command]
pub fn cancel_person_crop() {
    CANCEL_FLAG.store(true, Ordering::SeqCst);
    super::python_env::cancel_setup_for("person-crop");
    python_proc::kill_registered_pid(&CHILD_PROCESS);
}

#[tauri::command]
pub fn force_cancel_person_crop() {
    cancel_person_crop();
}

/// 构建每种裁切类型对应的模型路径映射
fn build_model_paths(options: &PersonCropOptions) -> Result<serde_json::Value, String> {
    let mut paths = serde_json::Map::new();

    let check_model = |crop_type: &str| -> Result<String, String> {
        let model = CROP_MODELS
            .iter()
            .find(|m| m.crop_type == crop_type)
            .ok_or_else(|| format!("未找到 {} 类型模型定义", crop_type))?;
        let path = model_onnx_path(model);
        if !path.exists() {
            return Err(format!("{} 模型未下载，请先下载模型包", model.name));
        }
        Ok(path.to_string_lossy().to_string())
    };

    if options.person_enabled {
        paths.insert(
            "person".into(),
            serde_json::Value::String(check_model("person")?),
        );
    }
    if options.upper_enabled {
        paths.insert(
            "halfbody".into(),
            serde_json::Value::String(check_model("halfbody")?),
        );
    }
    if options.head_enabled {
        paths.insert(
            "head".into(),
            serde_json::Value::String(check_model("head")?),
        );
    }
    if options.eyes_enabled {
        paths.insert(
            "eyes".into(),
            serde_json::Value::String(check_model("eyes")?),
        );
    }

    if paths.is_empty() {
        return Err("请至少启用一种裁切类型".into());
    }

    Ok(serde_json::Value::Object(paths))
}

fn wait_image_result(
    reader: &ProtocolReader,
    image_path: &str,
    mut on_log: impl FnMut(&serde_json::Value),
) -> Result<serde_json::Value, String> {
    loop {
        match reader.recv(PYTHON_SILENCE_LIMIT) {
            Recv::Msg(msg) => match msg["type"].as_str().unwrap_or("") {
                "log" => on_log(&msg),
                "result" | "error" if msg["image_path"].as_str() == Some(image_path) => {
                    return Ok(msg)
                }
                "error" if msg.get("image_path").is_none() => {
                    return Err(msg["message"].as_str().unwrap_or("Python 处理失败").into())
                }
                _ => {}
            },
            Recv::Closed => return Err("Python 进程已退出".into()),
            Recv::TimedOut => return Err("Python 进程无响应（300 秒）".into()),
        }
    }
}

fn run_person_crop<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    python: &str,
    options: &PersonCropOptions,
    files: &[PathBuf],
    model_paths: serde_json::Value,
) -> Result<ProcessResult, String> {
    let script = python_proc::find_script("person_crop.py")?;
    let input = Path::new(&options.input_path);
    let output_dir = Path::new(&options.output_path);
    let total = files.len() as u32;
    let mut result = ProcessResult {
        total,
        ..Default::default()
    };
    if CANCEL_FLAG.load(Ordering::SeqCst) {
        return Ok(result);
    }
    std::fs::create_dir_all(output_dir).map_err(|e| format!("无法创建输出目录: {}", e))?;
    ProgressEvent::new(
        "processing",
        format!("正在启动 Python 环境... (共 {} 张图片)", total),
    )
    .at(0, total)
    .emit(app, "person-crop-progress");
    let mut cmd = python_proc::hidden_command(python);
    cmd.arg(&script)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env("NO_COLOR", "1")
        .env("PYTHONUNBUFFERED", "1")
        .env("PYTHONIOENCODING", "utf-8");
    python_proc::configure_python_command(&mut cmd, options.use_gpu);
    let mut child = cmd.spawn().map_err(|e| format!("无法启动 Python: {}", e))?;
    let _registration = PidRegistration::new(&CHILD_PROCESS, child.id());
    let outcome = (|| {
        let mut stdin = child.stdin.take().ok_or("无法获取 stdin")?;
        let stdout = child.stdout.take().ok_or("无法获取 stdout")?;
        let stderr = child.stderr.take().ok_or("无法获取 stderr")?;
        let app_err = app.clone();
        std::thread::spawn(move || {
            python_proc::for_each_stderr_line(stderr, |line| {
                if !python_proc::is_runtime_noise(&line) {
                    ProgressEvent::new("warning", format!("[Python] {}", line))
                        .emit(&app_err, "person-crop-progress");
                }
            });
        });
        let init_config = serde_json::json!({
            "model_paths": model_paths, "use_gpu": options.use_gpu,
            "options": {
                "person_conf": options.person_conf, "upper_conf": options.upper_conf,
                "upper_tag": options.upper_tag, "head_conf": options.head_conf,
                "head_tag": options.head_tag, "head_scale": options.head_scale,
                "eyes_conf": options.eyes_conf, "eyes_tag": options.eyes_tag,
                "eyes_scale": options.eyes_scale, "keep_original_tags": options.keep_original_tags,
            },
        });
        writeln!(stdin, "{}", init_config).map_err(|e| format!("写入 stdin 失败: {}", e))?;
        let reader = ProtocolReader::spawn(stdout);
        loop {
            if CANCEL_FLAG.load(Ordering::SeqCst) {
                return Ok(result);
            }
            match reader.recv(PYTHON_SILENCE_LIMIT) {
                Recv::Msg(msg) => match msg["type"].as_str().unwrap_or("") {
                    "log" => {
                        ProgressEvent::python_log(&msg, 0, total).emit(app, "person-crop-progress")
                    }
                    "ready" => break,
                    "error" => {
                        return Err(format!(
                            "模型加载失败: {}",
                            msg["message"].as_str().unwrap_or("")
                        ))
                    }
                    _ => {}
                },
                _ if CANCEL_FLAG.load(Ordering::SeqCst) => return Ok(result),
                Recv::Closed => return Err("Python 进程无响应".into()),
                Recv::TimedOut => return Err("模型加载超时（300 秒）".into()),
            }
        }
        ProgressEvent::new("processing", "模型已加载，开始处理...")
            .at(0, total)
            .emit(app, "person-crop-progress");
        for (i, file_path) in files.iter().enumerate() {
            if CANCEL_FLAG.load(Ordering::SeqCst) {
                break;
            }
            let filename = super::file_name_lossy(file_path);
            let target = output_dir_for_input(input, file_path, output_dir, options.recursive)?;
            ProgressEvent::new("processing", format!("正在处理: {}", filename))
                .at(i as u32 + 1, total)
                .file(&filename)
                .emit(app, "person-crop-progress");
            let command = serde_json::json!({
                "action": "process", "image_path": file_path.to_string_lossy(), "output_dir": target.to_string_lossy(),
            });
            let response = writeln!(stdin, "{}", command)
                .map_err(|e| format!("写入失败: {}", e))
                .and_then(|()| {
                    wait_image_result(&reader, &file_path.to_string_lossy(), |msg| {
                        ProgressEvent::python_log(msg, i as u32 + 1, total)
                            .file(&filename)
                            .emit(app, "person-crop-progress");
                    })
                });
            // 被终止的当前图片没有完整结果，不计作失败。
            if CANCEL_FLAG.load(Ordering::SeqCst) {
                break;
            }
            let (status, message, disconnected) = match response {
                Ok(msg) => {
                    let detail = msg["message"].as_str().unwrap_or("");
                    match (msg["type"].as_str(), msg["status"].as_str()) {
                        (Some("result"), Some("success" | "skip")) => {
                            result.success_count += 1;
                            let label = if msg["status"] == "skip" {
                                "跳过"
                            } else {
                                "成功"
                            };
                            (
                                "success",
                                format!("[{}] {} — {}", label, filename, detail),
                                false,
                            )
                        }
                        _ => {
                            result.fail_count += 1;
                            result.errors.push(format!("{}: {}", filename, detail));
                            ("error", format!("[失败] {} — {}", filename, detail), false)
                        }
                    }
                }
                Err(error) => {
                    result.fail_count += 1;
                    result.errors.push(format!("{}: {}", filename, error));
                    ("error", format!("[失败] {} — {}", filename, error), true)
                }
            };
            ProgressEvent::new(status, message)
                .at(i as u32 + 1, total)
                .file(&filename)
                .emit(app, "person-crop-progress");
            if disconnected {
                break;
            }
        }
        let _ = writeln!(stdin, "EXIT");
        Ok(result)
    })();
    python_proc::kill_registered_pid(&CHILD_PROCESS);
    let _ = child.kill();
    let _ = child.wait();
    outcome
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn image_result_skips_noise_logs_and_other_images() {
        let stream = b"\xff\xfe noise\n{\"type\":\"ready\"}\n{\"type\":\"log\",\"message\":\"fallback\"}\n{\"type\":\"result\",\"image_path\":\"other.png\"}\n{\"type\":\"result\",\"image_path\":\"a.png\",\"status\":\"success\"}\n";
        let reader = ProtocolReader::spawn(std::io::Cursor::new(stream.to_vec()));
        let mut logs = Vec::new();
        let result = wait_image_result(&reader, "a.png", |msg| logs.push(msg.clone())).unwrap();
        assert_eq!(result["status"], "success");
        assert_eq!(logs.len(), 1);
        assert_eq!(logs[0]["message"], "fallback");
    }

    #[test]
    fn image_error_must_match_path_and_global_error_stops_wait() {
        let reader = ProtocolReader::spawn(std::io::Cursor::new(
            b"{\"type\":\"error\",\"message\":\"fatal\"}\n".to_vec(),
        ));
        assert_eq!(
            wait_image_result(&reader, "a.png", |_| {}).unwrap_err(),
            "fatal"
        );
    }

    #[test]
    fn cancellation_terminal_does_not_count_incomplete_image() {
        let result = ProcessResult {
            success_count: 2,
            total: 3,
            ..Default::default()
        };
        let event = terminal_event(&result, true);
        assert_eq!(event.status, "done");
        assert_eq!(event.current, 2);
        assert!(event.message.starts_with("已取消"));
        assert_eq!(result.fail_count, 0);
    }

    #[cfg(unix)]
    #[test]
    fn cancellation_during_image_does_not_add_failure_and_clears_pid() {
        use std::os::unix::fs::PermissionsExt;
        use tauri::Listener;
        let root = std::env::temp_dir().join(format!("purin_ai_crop_{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let script = root.join("fake-python");
        std::fs::write(&script, "#!/bin/sh\nread -r line\nprintf '%s\\n' '{\"type\":\"ready\"}'\nread -r line\nprintf '\\377noise\\n'\nprintf '%s\\n' '{\"type\":\"log\",\"message\":\"cancel-now\"}'\nread -r line\n").unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700)).unwrap();
        let app = tauri::test::mock_app();
        let events = super::super::batch::capture_events(app.handle(), "person-crop-progress");
        app.listen_any("person-crop-progress", |event| {
            let message: serde_json::Value = serde_json::from_str(event.payload()).unwrap();
            if message["message"] == "cancel-now" {
                CANCEL_FLAG.store(true, Ordering::SeqCst);
                python_proc::kill_registered_pid(&CHILD_PROCESS);
            }
        });
        CANCEL_FLAG.store(false, Ordering::SeqCst);
        let options = PersonCropOptions {
            input_path: root.to_string_lossy().into_owned(),
            output_path: root.join("out").to_string_lossy().into_owned(),
            use_gpu: false,
            person_enabled: true,
            person_conf: 0.3,
            upper_enabled: false,
            upper_conf: 0.5,
            upper_tag: String::new(),
            head_enabled: false,
            head_conf: 0.5,
            head_tag: String::new(),
            head_scale: 1.5,
            eyes_enabled: false,
            eyes_conf: 0.5,
            eyes_tag: String::new(),
            eyes_scale: 2.4,
            keep_original_tags: false,
            recursive: false,
        };
        let result = run_person_crop(
            app.handle(),
            script.to_str().unwrap(),
            &options,
            &[root.join("a.png")],
            serde_json::json!({"person": "fake.onnx"}),
        )
        .unwrap();
        assert_eq!(
            (result.success_count, result.fail_count, result.total),
            (0, 0, 1)
        );
        assert!(result.errors.is_empty());
        assert!(CHILD_PROCESS.lock().unwrap().is_none());
        terminal_event(&result, true).emit(app.handle(), "person-crop-progress");
        assert_eq!(
            events
                .lock()
                .unwrap()
                .iter()
                .filter(|e| e["status"] == "done")
                .count(),
            1
        );
        CANCEL_FLAG.store(false, Ordering::SeqCst);
        std::fs::remove_dir_all(root).unwrap();
    }
}
