use serde::{Deserialize, Serialize};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use futures_util::StreamExt;
use tauri::Emitter;

use super::{collect_image_files, ProcessResult, ProgressEvent};

// ===== DeepGHS Anime Detection Models =====

/// 5 个裁切类型对应的模型定义
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CropModelDef {
    pub id: &'static str,
    pub name: &'static str,
    pub crop_type: &'static str,  // "person" | "halfbody" | "head" | "face" | "eyes"
    pub repo: &'static str,
    pub subfolder: &'static str,
    pub size_mb: f64,
}

const CROP_MODELS: &[CropModelDef] = &[
    CropModelDef {
        id: "person_detect_v1.1_m",
        name: "全身检测 (person_detect_v1.1_m)",
        crop_type: "person",
        repo: "deepghs/anime_person_detection",
        subfolder: "person_detect_v1.1_m",
        size_mb: 103.0,
    },
    CropModelDef {
        id: "halfbody_detect_v1.0_s",
        name: "半身检测 (halfbody_detect_v1.0_s)",
        crop_type: "halfbody",
        repo: "deepghs/anime_halfbody_detection",
        subfolder: "halfbody_detect_v1.0_s",
        size_mb: 22.0,
    },
    CropModelDef {
        id: "head_detect_v2.0_x",
        name: "头部检测 (head_detect_v2.0_x)",
        crop_type: "head",
        repo: "deepghs/anime_head_detection",
        subfolder: "head_detect_v2.0_x",
        size_mb: 247.0,
    },
    CropModelDef {
        id: "eye_detect_v1.0_s",
        name: "眼部检测 (eye_detect_v1.0_s)",
        crop_type: "eyes",
        repo: "deepghs/anime_eye_detection",
        subfolder: "eye_detect_v1.0_s",
        size_mb: 22.0,
    },
];

/// 模型存储目录
fn get_models_dir() -> PathBuf {
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|p| p.to_path_buf()))
        .unwrap_or_else(|| PathBuf::from("."));

    let base = if cfg!(debug_assertions) {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or(exe_dir)
    } else {
        exe_dir
    };
    base.join("models").join("crop_models")
}

fn model_onnx_path(model: &CropModelDef) -> PathBuf {
    get_models_dir().join(model.id).join("model.onnx")
}

/// 模型状态信息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CropModelInfo {
    pub id: String,
    pub name: String,
    pub crop_type: String,
    pub size_mb: f64,
    pub downloaded: bool,
    pub path: String,
}

#[tauri::command]
pub fn get_person_crop_models() -> Result<Vec<CropModelInfo>, String> {
    Ok(CROP_MODELS.iter().map(|m| {
        let path = model_onnx_path(m);
        CropModelInfo {
            id: m.id.to_string(),
            name: m.name.to_string(),
            crop_type: m.crop_type.to_string(),
            size_mb: m.size_mb,
            downloaded: path.exists(),
            path: path.to_string_lossy().to_string(),
        }
    }).collect())
}

/// 下载进度
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CropDownloadProgress {
    pub downloaded: u64,
    pub total: u64,
    pub percent: f32,
    pub speed_mbps: f64,
    pub status: String,
    pub message: String,
}

static DOWNLOAD_CANCEL: AtomicBool = AtomicBool::new(false);

/// 下载所有缺失的模型（一键下载模型包）
#[tauri::command]
pub async fn download_person_crop_model(app: tauri::AppHandle, model_id: String) -> Result<String, String> {
    DOWNLOAD_CANCEL.store(false, Ordering::SeqCst);

    // 支持 model_id = "all" 下载全部，或单个模型 id
    let models_to_download: Vec<&CropModelDef> = if model_id == "all" {
        CROP_MODELS.iter().filter(|m| !model_onnx_path(m).exists()).collect()
    } else {
        CROP_MODELS.iter().filter(|m| m.id == model_id).collect()
    };

    if models_to_download.is_empty() {
        let _ = app.emit("person-crop-download", CropDownloadProgress {
            downloaded: 0, total: 0, percent: 100.0, speed_mbps: 0.0,
            status: "done".into(), message: "所有模型已就绪".into(),
        });
        return Ok("all_ready".into());
    }

    let client = crate::commands::proxy_config::build_http_client()
        .user_agent("PurinBox/0.1.7")
        .timeout(std::time::Duration::from_secs(600))
        .build()
        .map_err(|e| format!("HTTP 客户端失败: {}", e))?;

    for (idx, model) in models_to_download.iter().enumerate() {
        if DOWNLOAD_CANCEL.load(Ordering::SeqCst) {
            return Err("下载已取消".into());
        }

        let dest = model_onnx_path(model);
        if dest.exists() { continue; }

        let dir = dest.parent().unwrap();
        if !dir.exists() {
            std::fs::create_dir_all(dir).map_err(|e| format!("创建目录失败: {}", e))?;
        }

        let url = format!(
            "https://huggingface.co/{}/resolve/main/{}/model.onnx",
            model.repo, model.subfolder
        );

        let prefix = format!("[{}/{}] {}", idx + 1, models_to_download.len(), model.name);

        let _ = app.emit("person-crop-download", CropDownloadProgress {
            downloaded: 0, total: 0, percent: 0.0, speed_mbps: 0.0,
            status: "downloading".into(),
            message: format!("{} — 开始下载...", prefix),
        });

        let resp = client.get(&url).send().await.map_err(|e| format!("请求失败: {}", e))?;
        if !resp.status().is_success() {
            return Err(format!("HTTP {}: {}", resp.status(), url));
        }

        let total_size = resp.content_length().unwrap_or(0);
        let mut stream = resp.bytes_stream();
        let mut file = tokio::fs::File::create(&dest).await.map_err(|e| format!("创建文件失败: {}", e))?;
        let mut downloaded: u64 = 0;
        let mut last_t = std::time::Instant::now();
        let mut last_b: u64 = 0;
        let start = std::time::Instant::now();

        while let Some(chunk) = stream.next().await {
            if DOWNLOAD_CANCEL.load(Ordering::SeqCst) {
                drop(file);
                let _ = tokio::fs::remove_file(&dest).await;
                return Err("下载已取消".into());
            }
            let chunk = chunk.map_err(|e| format!("下载失败: {}", e))?;
            tokio::io::AsyncWriteExt::write_all(&mut file, &chunk).await.map_err(|e| format!("写入失败: {}", e))?;
            downloaded += chunk.len() as u64;

            let now = std::time::Instant::now();
            let elapsed_ms = now.duration_since(last_t).as_millis();
            if elapsed_ms >= 500 || (total_size > 0 && downloaded >= total_size) {
                let speed = if elapsed_ms > 0 { (downloaded - last_b) as f64 / elapsed_ms as f64 * 1000.0 / 1_048_576.0 } else { 0.0 };
                last_t = now; last_b = downloaded;
                let pct = if total_size > 0 { (downloaded as f64 / total_size as f64 * 100.0) as f32 } else { 0.0 };
                let avg = { let t = start.elapsed().as_secs_f64(); if t > 0.0 { downloaded as f64 / t / 1_048_576.0 } else { 0.0 } };
                let mb_done = downloaded as f64 / 1_048_576.0;
                let msg = if total_size > 0 {
                    format!("{} — {:.1}/{:.1} MB ({:.1} MB/s)", prefix, mb_done, total_size as f64 / 1_048_576.0, avg)
                } else {
                    format!("{} — {:.1} MB ({:.1} MB/s)", prefix, mb_done, avg)
                };
                let _ = app.emit("person-crop-download", CropDownloadProgress {
                    downloaded, total: total_size, percent: pct, speed_mbps: speed,
                    status: "downloading".into(), message: msg,
                });
            }
        }

        let _ = app.emit("person-crop-download", CropDownloadProgress {
            downloaded, total: total_size, percent: 100.0, speed_mbps: 0.0,
            status: "done".into(),
            message: format!("{} — 下载完成 ✓", prefix),
        });
    }

    Ok("done".into())
}

#[tauri::command]
pub fn cancel_person_crop_download() {
    DOWNLOAD_CANCEL.store(true, Ordering::SeqCst);
}


// ===== Person Crop Processing =====

static CANCEL_FLAG: AtomicBool = AtomicBool::new(false);
static CHILD_PROCESS: Mutex<Option<u32>> = Mutex::new(None);

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
}

#[tauri::command]
pub async fn start_person_crop(
    app: tauri::AppHandle,
    options: PersonCropOptions,
) -> Result<ProcessResult, String> {
    CANCEL_FLAG.store(false, Ordering::SeqCst);

    // Ensure Python environment is ready (onnxruntime + numpy + pillow)
    let _ = app.emit("person-crop-progress", ProgressEvent {
        current: 0, total: 0, filename: String::new(),
        status: "info".to_string(), message: "正在检查 Python 环境...".to_string(),
    });
    super::python_env::setup_python_env(&app).await?;

    tokio::task::spawn_blocking(move || {
        run_person_crop(&app, &options)
    })
    .await
    .map_err(|e| format!("任务执行失败: {}", e))?
}

#[tauri::command]
pub fn cancel_person_crop() {
    CANCEL_FLAG.store(true, Ordering::SeqCst);
    if let Ok(mut guard) = CHILD_PROCESS.lock() {
        if let Some(pid) = guard.take() {
            #[cfg(target_os = "windows")]
            { let _ = Command::new("taskkill").args(["/F", "/T", "/PID", &pid.to_string()]).output(); }
            #[cfg(not(target_os = "windows"))]
            { let _ = Command::new("kill").args(["-9", &pid.to_string()]).output(); }
        }
    }
}

fn find_python() -> Result<String, String> {
    // 1. 优先使用 python_env 模块管理的环境
    if let Some(python) = super::python_env::get_python_exe() {
        return Ok(python);
    }
    // 2. 系统 Python
    for name in &["python3", "python"] {
        if let Ok(output) = Command::new(name).args(["--version"]).output() {
            if output.status.success() {
                let ver = String::from_utf8_lossy(&output.stdout).to_string()
                    + &String::from_utf8_lossy(&output.stderr).to_string();
                if ver.contains("Python 3") {
                    return Ok(name.to_string());
                }
            }
        }
    }
    Err("未找到可用的 Python 环境。请先在「图片打标」页面初始化 Python 环境。".into())
}

fn get_crop_script_path() -> Result<std::path::PathBuf, String> {
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|p| p.to_path_buf()))
        .unwrap_or_else(|| std::path::PathBuf::from("."));

    let candidates = vec![
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("scripts/person_crop.py"),
        exe_dir.join("scripts/person_crop.py"),
        exe_dir.join("person_crop.py"),
        exe_dir.join("../Resources/scripts/person_crop.py"),
    ];

    for path in &candidates {
        if path.exists() {
            return Ok(path.canonicalize().unwrap_or_else(|_| path.clone()));
        }
    }

    let paths_str = candidates.iter()
        .enumerate()
        .map(|(i, p)| format!("  {}. {}", i + 1, p.display()))
        .collect::<Vec<_>>()
        .join("\n");
    Err(format!("裁切脚本未找到。\n搜索路径:\n{}", paths_str))
}

/// 构建每种裁切类型对应的模型路径映射
fn build_model_paths(options: &PersonCropOptions) -> Result<serde_json::Value, String> {
    let mut paths = serde_json::Map::new();
    
    let check_model = |crop_type: &str| -> Result<String, String> {
        let model = CROP_MODELS.iter().find(|m| m.crop_type == crop_type)
            .ok_or_else(|| format!("未找到 {} 类型模型定义", crop_type))?;
        let path = model_onnx_path(model);
        if !path.exists() {
            return Err(format!("{} 模型未下载，请先下载模型包", model.name));
        }
        Ok(path.to_string_lossy().to_string())
    };

    if options.person_enabled {
        paths.insert("person".into(), serde_json::Value::String(check_model("person")?));
    }
    if options.upper_enabled {
        paths.insert("halfbody".into(), serde_json::Value::String(check_model("halfbody")?));
    }
    if options.head_enabled {
        paths.insert("head".into(), serde_json::Value::String(check_model("head")?));
    }
    if options.eyes_enabled {
        paths.insert("eyes".into(), serde_json::Value::String(check_model("eyes")?));
    }

    if paths.is_empty() {
        return Err("请至少启用一种裁切类型".into());
    }

    Ok(serde_json::Value::Object(paths))
}

fn run_person_crop(app: &tauri::AppHandle, options: &PersonCropOptions) -> Result<ProcessResult, String> {
    let python = find_python()?;
    let script = get_crop_script_path()?;
    let model_paths = build_model_paths(options)?;

    let input = Path::new(&options.input_path);
    let output_dir = Path::new(&options.output_path);

    if !output_dir.exists() {
        std::fs::create_dir_all(output_dir)
            .map_err(|e| format!("无法创建输出目录: {}", e))?;
    }

    let files = collect_image_files(input)?;
    let total = files.len() as u32;

    if total == 0 {
        return Ok(ProcessResult { success_count: 0, fail_count: 0, total: 0, errors: vec![] });
    }

    let _ = app.emit("person-crop-progress", ProgressEvent {
        current: 0, total, filename: String::new(),
        status: "processing".to_string(),
        message: format!("正在启动 Python 环境... (共 {} 张图片)", total),
    });

    // 启动 Python 子进程
    let mut cmd = Command::new(&python);
    cmd.arg(script.to_str().unwrap_or(""))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000);
    }

    let mut child = cmd.spawn().map_err(|e| format!("无法启动 Python: {}", e))?;
    let pid = child.id();
    *CHILD_PROCESS.lock().unwrap() = Some(pid);

    let mut stdin = child.stdin.take().ok_or("无法获取 stdin")?;
    let stdout = child.stdout.take().ok_or("无法获取 stdout")?;

    // 发送初始化配置（多模型路径）
    let init_config = serde_json::json!({
        "model_paths": model_paths,
        "use_gpu": options.use_gpu,
    });
    writeln!(stdin, "{}", serde_json::to_string(&init_config).unwrap())
        .map_err(|e| format!("写入 stdin 失败: {}", e))?;
    stdin.flush().map_err(|e| format!("flush 失败: {}", e))?;

    // 等待 ready
    let reader = BufReader::new(stdout);
    let mut lines = reader.lines();

    let ready_line = lines.next()
        .ok_or("Python 进程无响应")?
        .map_err(|e| format!("读取响应失败: {}", e))?;

    let ready_json: serde_json::Value = serde_json::from_str(&ready_line)
        .map_err(|e| format!("解析响应失败: {} (原始: {})", e, ready_line))?;

    if let Some(err) = ready_json.get("error") {
        return Err(format!("模型加载失败: {}", err));
    }

    let _ = app.emit("person-crop-progress", ProgressEvent {
        current: 0, total, filename: String::new(),
        status: "processing".to_string(),
        message: "模型已加载，开始处理...".to_string(),
    });

    let mut success_count = 0u32;
    let mut fail_count = 0u32;
    let mut errors = Vec::new();

    for (i, file_path) in files.iter().enumerate() {
        if CANCEL_FLAG.load(Ordering::SeqCst) {
            let _ = app.emit("person-crop-progress", ProgressEvent {
                current: i as u32, total, filename: String::new(),
                status: "done".to_string(),
                message: "已取消".to_string(),
            });
            break;
        }

        let filename = file_path.file_name().unwrap_or_default().to_string_lossy().to_string();

        let _ = app.emit("person-crop-progress", ProgressEvent {
            current: i as u32 + 1, total,
            filename: filename.clone(),
            status: "processing".to_string(),
            message: format!("正在处理: {}", filename),
        });

        // 发送处理命令
        let cmd_json = serde_json::json!({
            "action": "process",
            "image_path": file_path.to_string_lossy(),
            "output_dir": options.output_path,
            "options": {
                "person_enabled": options.person_enabled,
                "person_conf": options.person_conf,
                "upper_enabled": options.upper_enabled,
                "upper_conf": options.upper_conf,
                "upper_tag": options.upper_tag,
                "head_enabled": options.head_enabled,
                "head_conf": options.head_conf,
                "head_tag": options.head_tag,
                "head_scale": options.head_scale,
                "eyes_enabled": options.eyes_enabled,
                "eyes_conf": options.eyes_conf,
                "eyes_tag": options.eyes_tag,
                "eyes_scale": options.eyes_scale,
                "keep_original_tags": options.keep_original_tags,
            }
        });

        if let Err(e) = writeln!(stdin, "{}", serde_json::to_string(&cmd_json).unwrap()) {
            errors.push(format!("{}: 写入失败: {}", filename, e));
            fail_count += 1;
            continue;
        }
        let _ = stdin.flush();

        // 读取结果
        match lines.next() {
            Some(Ok(result_line)) => {
                match serde_json::from_str::<serde_json::Value>(&result_line) {
                    Ok(result) => {
                        let status = result.get("status").and_then(|s| s.as_str()).unwrap_or("error");
                        let message = result.get("message").and_then(|s| s.as_str()).unwrap_or("").to_string();

                        match status {
                            "success" => {
                                success_count += 1;
                                let _ = app.emit("person-crop-progress", ProgressEvent {
                                    current: i as u32 + 1, total,
                                    filename: filename.clone(),
                                    status: "success".to_string(),
                                    message: format!("[成功] {} — {}", filename, message),
                                });
                            }
                            "skip" => {
                                success_count += 1;
                                let _ = app.emit("person-crop-progress", ProgressEvent {
                                    current: i as u32 + 1, total,
                                    filename: filename.clone(),
                                    status: "success".to_string(),
                                    message: format!("[跳过] {} — {}", filename, message),
                                });
                            }
                            _ => {
                                fail_count += 1;
                                errors.push(format!("{}: {}", filename, message));
                                let _ = app.emit("person-crop-progress", ProgressEvent {
                                    current: i as u32 + 1, total,
                                    filename: filename.clone(),
                                    status: "error".to_string(),
                                    message: format!("[失败] {} — {}", filename, message),
                                });
                            }
                        }
                    }
                    Err(e) => {
                        fail_count += 1;
                        errors.push(format!("{}: JSON 解析失败: {}", filename, e));
                    }
                }
            }
            Some(Err(e)) => {
                fail_count += 1;
                errors.push(format!("{}: 读取失败: {}", filename, e));
            }
            None => {
                fail_count += 1;
                errors.push(format!("{}: Python 进程已退出", filename));
                break;
            }
        }
    }

    // 终止 Python
    let _ = writeln!(stdin, "EXIT");
    let _ = stdin.flush();
    let _ = child.wait();
    *CHILD_PROCESS.lock().unwrap() = None;

    let _ = app.emit("person-crop-progress", ProgressEvent {
        current: total, total, filename: String::new(),
        status: "done".to_string(),
        message: format!("处理完成: 成功 {}, 失败 {}, 共 {}", success_count, fail_count, total),
    });

    Ok(ProcessResult { success_count, fail_count, total, errors })
}
