use serde::{Deserialize, Serialize};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use futures_util::StreamExt;
use tauri::Emitter;

use super::{ProcessResult, ProgressEvent};
use super::{finalize_part_file, prepare_part_file};

/// 美学评分选项
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AestheticOptions {
    pub input_path: String,
    #[serde(default)]
    pub output_path: String,
    #[serde(default)]
    pub use_gpu: bool,
    #[serde(default = "default_true")]
    pub move_files: bool,
    #[serde(default = "default_batch_size")]
    pub batch_size: u32,
}

fn default_true() -> bool { true }
fn default_batch_size() -> u32 { 1 }

/// 全局取消标志
static AESTHETIC_CANCELLED: AtomicBool = AtomicBool::new(false);
static AESTHETIC_PROCESS: Mutex<Option<Child>> = Mutex::new(None);
static DOWNLOAD_CANCELLED: AtomicBool = AtomicBool::new(false);

/// 模型存储目录
fn get_aesthetic_model_dir() -> PathBuf {
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

    base.join("models").join("aesthetic_models").join("swinv2pv3_v0_448_ls0.2_x")
}

/// 模型是否已下载
fn is_model_downloaded() -> bool {
    let dir = get_aesthetic_model_dir();
    dir.join("model.onnx").exists() && dir.join("meta.json").exists()
}

/// 获取推理脚本路径
fn get_script_path() -> Result<PathBuf, String> {
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|p| p.to_path_buf()))
        .unwrap_or_else(|| PathBuf::from("."));

    let candidates = vec![
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("scripts/aesthetic_inference.py"),
        exe_dir.join("scripts/aesthetic_inference.py"),
        exe_dir.join("aesthetic_inference.py"),
        exe_dir.join("../Resources/scripts/aesthetic_inference.py"),
    ];

    for path in &candidates {
        if path.exists() {
            return Ok(path.canonicalize().unwrap_or_else(|_| path.clone()));
        }
    }

    Err("美学评分推理脚本未找到".into())
}

/// 查找 Python
fn find_python() -> Result<String, String> {
    // 使用 tagger 的 Python 环境
    if let Some(python) = super::python_env::get_python_exe() {
        return Ok(python);
    }

    for name in &["python3", "python"] {
        if let Ok(output) = Command::new(name).args(["--version"]).output() {
            if output.status.success() {
                let ver = String::from_utf8_lossy(&output.stdout);
                if ver.contains("Python 3") {
                    return Ok(name.to_string());
                }
            }
        }
    }

    Err("未找到可用的 Python 环境".into())
}

/// 去除 ANSI 转义序列
fn strip_ansi_codes(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            if chars.peek() == Some(&'[') {
                chars.next();
                while let Some(&next) = chars.peek() {
                    chars.next();
                    if next.is_ascii_alphabetic() { break; }
                }
            }
        } else {
            result.push(c);
        }
    }
    result
}

/// UTF-8 安全行读取 — 非 UTF-8 字节用 lossy 替换
fn read_utf8_line(reader: &mut impl BufRead) -> Option<String> {
    let mut buf = Vec::new();
    match reader.read_until(b'\n', &mut buf) {
        Ok(0) => None,
        Ok(_) => {
            if buf.last() == Some(&b'\n') { buf.pop(); }
            if buf.last() == Some(&b'\r') { buf.pop(); }
            Some(String::from_utf8(buf).unwrap_or_else(|e| String::from_utf8_lossy(e.as_bytes()).to_string()))
        }
        Err(_) => None,
    }
}

fn kill_process() {
    if let Ok(mut guard) = AESTHETIC_PROCESS.lock() {
        if let Some(ref mut child) = *guard {
            let _ = child.kill();
            let _ = child.wait();
        }
        *guard = None;
    }
}

/// 下载进度事件
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DownloadProgress {
    pub filename: String,
    pub downloaded: u64,
    pub total: u64,
    pub percent: f32,
    pub speed_mbps: f64,
    pub status: String,
    pub message: String,
}

/// 下载模型
async fn download_model(app: &tauri::AppHandle) -> Result<(), String> {
    DOWNLOAD_CANCELLED.store(false, Ordering::SeqCst);

    let model_dir = get_aesthetic_model_dir();
    if !model_dir.exists() {
        std::fs::create_dir_all(&model_dir)
            .map_err(|e| format!("创建模型目录失败: {}", e))?;
    }

    let _ = app.emit("aesthetic-progress", ProgressEvent {
        current: 0, total: 0, filename: String::new(),
        status: "info".to_string(),
        message: "开始下载美学评分模型...".to_string(),
    ..Default::default()
    });

    let base_url = "https://huggingface.co/deepghs/anime_aesthetic/resolve/main/swinv2pv3_v0_448_ls0.2_x";

    // 下载 model.onnx
    let model_url = format!("{}/model.onnx", base_url);
    let model_dest = model_dir.join("model.onnx");
    download_file(app, &model_url, &model_dest, "model.onnx").await?;

    if DOWNLOAD_CANCELLED.load(Ordering::SeqCst) {
        let _ = std::fs::remove_file(&model_dest);
        return Err("下载已取消".into());
    }

    // 下载 meta.json
    let meta_url = format!("{}/meta.json", base_url);
    let meta_dest = model_dir.join("meta.json");
    download_file(app, &meta_url, &meta_dest, "meta.json").await?;

    let _ = app.emit("aesthetic-progress", ProgressEvent {
        current: 0, total: 0, filename: String::new(),
        status: "success".to_string(),
        message: "美学评分模型下载完成".to_string(),
    ..Default::default()
    });

    Ok(())
}

async fn download_file(
    app: &tauri::AppHandle,
    url: &str,
    dest: &Path,
    label: &str,
) -> Result<(), String> {
    let client = crate::commands::proxy_config::build_http_client()
        .user_agent("PurinBox/0.3.13")
        .timeout(std::time::Duration::from_secs(600))
        .build()
        .map_err(|e| format!("创建 HTTP 客户端失败: {}", e))?;

    let response = client
        .get(url)
        .send()
        .await
        .map_err(|e| format!("下载请求失败 ({}): {}", url, e))?;

    if !response.status().is_success() {
        return Err(format!("下载失败 (HTTP {}): {}", response.status(), url));
    }

    let total_size = response.content_length().unwrap_or(0);
    let mut stream = response.bytes_stream();

    // 先写入 .part 临时文件，校验完成后再原子替换到最终路径，避免中断残件被当作完整文件
    let part_path = prepare_part_file(dest);
    let mut file = tokio::fs::File::create(&part_path)
        .await
        .map_err(|e| format!("创建文件失败: {}", e))?;

    let mut downloaded: u64 = 0;
    let mut last_report_time = std::time::Instant::now();
    let mut last_report_bytes: u64 = 0;
    let start_time = std::time::Instant::now();

    let _ = app.emit("aesthetic-download", DownloadProgress {
        filename: label.into(),
        downloaded: 0, total: total_size, percent: 0.0, speed_mbps: 0.0,
        status: "downloading".to_string(),
        message: format!("正在下载 {}", label),
    });

    // 下载循环结果 — 任何错误（含取消）统一在循环外清理 .part 残件
    let mut result: Result<(), String> = Ok(());

    while let Some(chunk) = stream.next().await {
        if DOWNLOAD_CANCELLED.load(Ordering::SeqCst) {
            result = Err("下载已取消".into());
            break;
        }

        let chunk = match chunk {
            Ok(c) => c,
            Err(e) => {
                result = Err(format!("下载数据失败: {}", e));
                break;
            }
        };

        if let Err(e) = tokio::io::AsyncWriteExt::write_all(&mut file, &chunk).await {
            result = Err(format!("写入文件失败: {}", e));
            break;
        }

        downloaded += chunk.len() as u64;

        let now = std::time::Instant::now();
        let elapsed_since_report = now.duration_since(last_report_time).as_millis();
        if elapsed_since_report >= 500 || (total_size > 0 && downloaded >= total_size) {
            last_report_time = now;
            let elapsed_total = start_time.elapsed().as_secs_f64();
            let avg_speed = if elapsed_total > 0.0 { downloaded as f64 / elapsed_total / 1_048_576.0 } else { 0.0 };
            last_report_bytes = downloaded;

            let percent = if total_size > 0 {
                (downloaded as f64 / total_size as f64 * 100.0) as f32
            } else { 0.0 };

            let mb_done = downloaded as f64 / 1_048_576.0;
            let message = if total_size > 0 {
                let mb_total = total_size as f64 / 1_048_576.0;
                format!("{} — {:.1}/{:.1} MB ({:.1} MB/s)", label, mb_done, mb_total, avg_speed)
            } else {
                format!("{} — {:.1} MB ({:.1} MB/s)", label, mb_done, avg_speed)
            };

            let _ = app.emit("aesthetic-download", DownloadProgress {
                filename: label.into(),
                downloaded, total: total_size, percent,
                speed_mbps: avg_speed,
                status: "downloading".to_string(),
                message,
            });
        }
    }

    let _ = last_report_bytes; // suppress warning

    // 确保缓冲数据全部落盘
    if result.is_ok() {
        if let Err(e) = tokio::io::AsyncWriteExt::flush(&mut file).await {
            result = Err(format!("写入文件失败: {}", e));
        }
    }
    drop(file);

    // 任何错误路径（包括取消）都删除 .part 残件
    if let Err(e) = result {
        let _ = tokio::fs::remove_file(&part_path).await;
        return Err(e);
    }

    // 校验字节数并原子替换到最终文件
    finalize_part_file(&part_path, dest, downloaded, total_size)?;

    Ok(())
}

/// 执行批量美学评分
fn run_aesthetic_scoring(
    app: &tauri::AppHandle,
    options: &AestheticOptions,
    model_path: &Path,
) -> Result<ProcessResult, String> {
    kill_process();

    let python = find_python()?;
    let script = get_script_path()?;

    let mut cmd = Command::new(&python);
    cmd.arg(script.to_string_lossy().as_ref())
       .stdin(Stdio::piped())
       .stdout(Stdio::piped())
       .stderr(Stdio::piped())
       .env("NO_COLOR", "1")
       .env("PYTHONUNBUFFERED", "1")
       .env("PYTHONIOENCODING", "utf-8");

    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000);

        if options.use_gpu {
            let mut path = std::env::var("PATH").unwrap_or_default();
            let mut add_dir = |dir: &str| {
                if std::path::Path::new(dir).exists() && !path.contains(dir) {
                    path = format!("{};{}", dir, path);
                }
            };

            // CUDA 路径
            for (_key, val) in super::tagger::inference::get_cuda_env_vars() {
                add_dir(&format!(r"{}\bin", val));
                add_dir(&format!(r"{}\bin\x64", val));
                add_dir(&format!(r"{}\lib\x64", val));
            }

            // cuDNN 路径
            if let Ok(cudnn_path) = std::env::var("CUDNN_PATH") {
                add_dir(&format!(r"{}\bin", cudnn_path));
            }

            cmd.env("PATH", &path);
        }
    }

    let mut child = cmd.spawn()
        .map_err(|e| format!("启动 Python 进程失败: {}", e))?;

    // 取出管道句柄（take 出管道后 Child 仍可 kill/wait）
    let (mut stdin, stdout, stderr) = match (child.stdin.take(), child.stdout.take(), child.stderr.take()) {
        (Some(i), Some(o), Some(e)) => (i, o, e),
        _ => {
            let _ = child.kill();
            let _ = child.wait();
            return Err("无法获取 Python 进程管道".into());
        }
    };

    // 把 Child 句柄存入全局，这样 cancel_aesthetic_scoring() -> kill_process() 才能真正杀掉进程
    *AESTHETIC_PROCESS.lock().unwrap_or_else(|e| e.into_inner()) = Some(child);

    // stderr 读取线程
    let app_err = app.clone();
    std::thread::spawn(move || {
        let mut reader = BufReader::new(stderr);
        let mut buf = Vec::new();
        use std::io::Read;
        let mut byte = [0u8; 1];
        loop {
            match reader.read(&mut byte) {
                Ok(0) => break,
                Ok(_) => {
                    if byte[0] == b'\n' {
                        let line = String::from_utf8(buf.clone()).unwrap_or_else(|_| {
                            let (s, _, _) = encoding_rs::GBK.decode(&buf); s.to_string()
                        });
                        buf.clear();
                        let clean = strip_ansi_codes(&line);
                        let clean = clean.trim();
                        if clean.is_empty() { continue; }
                        let lower = clean.to_lowercase();
                        if lower.contains("context leak")
                            || lower.contains("msgtracer")
                            || lower.contains("onnxruntime")
                            || lower.contains("could not load")
                            || lower.contains("loaded library")
                            || lower.contains("ep error")
                            || lower.contains("provider") {
                            continue;
                        }
                        let _ = app_err.emit("aesthetic-progress", ProgressEvent {
                            current: 0, total: 0, filename: String::new(),
                            status: "warning".to_string(),
                            message: format!("[Python] {}", clean),
                        ..Default::default()
                        });
                    } else if byte[0] != b'\r' {
                        buf.push(byte[0]);
                    }
                }
                Err(_) => break,
            }
        }
    });

    // 发送 init 命令
    let init_cmd = serde_json::json!({
        "cmd": "init",
        "model_path": model_path.to_string_lossy(),
        "use_gpu": options.use_gpu,
    });

    if let Err(e) = writeln!(stdin, "{}", init_cmd) {
        kill_process();
        return Err(format!("发送 init 命令失败: {}", e));
    }

    // 等待 ready — UTF-8 安全行读取
    let mut reader = BufReader::new(stdout);

    let mut ready = false;
    let timeout = std::time::Instant::now();
    while let Some(line) = read_utf8_line(&mut reader) {
        if AESTHETIC_CANCELLED.load(Ordering::SeqCst) {
            kill_process();
            return Ok(ProcessResult { success_count: 0, fail_count: 0, total: 0, errors: vec![] });
        }

        if let Ok(msg) = serde_json::from_str::<serde_json::Value>(&line) {
            let msg_type = msg.get("type").and_then(|v| v.as_str()).unwrap_or("");
            match msg_type {
                "log" => {
                    let text = msg.get("message").and_then(|v| v.as_str()).unwrap_or("").to_string();
                    let i18n_key = msg.get("i18n_key").and_then(|v| v.as_str()).map(|s| s.to_string());
                    let i18n_params = msg.get("i18n_params").cloned();
                    let _ = app.emit("aesthetic-progress", ProgressEvent {
                        current: 0, total: 0, filename: String::new(),
                        status: "info".to_string(),
                        message: text,
                        i18n_key,
                        i18n_params,
                    });
                }
                "error" => {
                    let text = msg.get("message").and_then(|v| v.as_str()).unwrap_or("");
                    kill_process();
                    return Err(format!("初始化失败: {}", text));
                }
                "ready" => {
                    ready = true;
                    break;
                }
                _ => {}
            }
        }

        if timeout.elapsed() > std::time::Duration::from_secs(120) {
            kill_process();
            return Err("模型加载超时(120秒)".into());
        }
    }

    if !ready {
        kill_process();
        if AESTHETIC_CANCELLED.load(Ordering::SeqCst) {
            return Ok(ProcessResult { success_count: 0, fail_count: 0, total: 0, errors: vec![] });
        }
        return Err("Python 进程未能成功初始化".into());
    }

    // 收集图片（失败时杀掉已启动的 Python 进程，避免泄漏）
    let input_dir = Path::new(&options.input_path);
    let files = match super::collect_image_files(input_dir) {
        Ok(f) => f,
        Err(e) => {
            kill_process();
            return Err(e);
        }
    };
    let total = files.len() as u32;
    let mut success_count = 0u32;
    let mut fail_count = 0u32;
    let mut errors = Vec::new();

    let _ = app.emit("aesthetic-progress", ProgressEvent {
        current: 0, total,
        filename: String::new(),
        status: "info".to_string(),
        message: format!("读取到 {} 张图片", total),
    ..Default::default()
    });

    let batch_size = options.batch_size.max(1) as usize;

    let mut i = 0usize;
    while i < files.len() {
        if AESTHETIC_CANCELLED.load(Ordering::SeqCst) {
            let _ = app.emit("aesthetic-progress", ProgressEvent {
                current: i as u32, total,
                filename: String::new(),
                status: "done".to_string(),
                message: format!("已取消: 成功 {}, 失败 {}", success_count, fail_count),
            ..Default::default()
            });
            break;
        }

        let end = (i + batch_size).min(files.len());
        let batch_files = &files[i..end];
        let batch_len = batch_files.len();

        let first_name = batch_files[0].file_name().unwrap_or_default().to_string_lossy().to_string();
        let _ = app.emit("aesthetic-progress", ProgressEvent {
            current: i as u32 + 1, total,
            filename: first_name.clone(),
            status: "processing".to_string(),
            message: if batch_len > 1 {
                format!("正在评分: {} 等 {} 张 ({}/{})", first_name, batch_len, i + 1, total)
            } else {
                format!("正在评分: {} ({}/{})", first_name, i + 1, total)
            },
        ..Default::default()
        });

        if batch_len == 1 {
            // 单张模式
            let file_path = &batch_files[0];
            let filename = file_path.file_name().unwrap_or_default().to_string_lossy().to_string();
            let score_cmd = serde_json::json!({
                "cmd": "score",
                "image_path": file_path.to_string_lossy(),
                "move_files": options.move_files,
                "output_path": if options.output_path.is_empty() { "" } else { &options.output_path },
            });
            if let Err(e) = writeln!(stdin, "{}", score_cmd) {
                fail_count += 1;
                errors.push(format!("{}: 发送命令失败: {}", filename, e));
                break;
            }
            loop {
                match read_utf8_line(&mut reader) {
                    Some(line) => {
                        if let Ok(msg) = serde_json::from_str::<serde_json::Value>(&line) {
                            let msg_type = msg.get("type").and_then(|v| v.as_str()).unwrap_or("");
                            match msg_type {
                                "result" => {
                                    let label = msg.get("label").and_then(|v| v.as_str()).unwrap_or("?");
                                    let score = msg.get("score").and_then(|v| v.as_f64()).unwrap_or(0.0);
                                    let confidence = msg.get("confidence").and_then(|v| v.as_f64()).unwrap_or(0.0);
                                    success_count += 1;
                                    let _ = app.emit("aesthetic-progress", ProgressEvent {
                                        current: i as u32 + 1, total,
                                        filename: filename.clone(),
                                        status: "success".to_string(),
                                        message: format!("[完成] {} → {} (分数: {:.2}, 置信度: {:.1}%)", filename, label, score, confidence * 100.0),
                                    ..Default::default()
                                    });
                                    break;
                                }
                                "error" => {
                                    let text = msg.get("message").and_then(|v| v.as_str()).unwrap_or("unknown");
                                    fail_count += 1;
                                    errors.push(format!("{}: {}", filename, text));
                                    let _ = app.emit("aesthetic-progress", ProgressEvent {
                                        current: i as u32 + 1, total,
                                        filename: filename.clone(),
                                        status: "error".to_string(),
                                        message: format!("[错误] {}: {}", filename, text),
                                    ..Default::default()
                                    });
                                    break;
                                }
                                "log" => {
                                    let text = msg.get("message").and_then(|v| v.as_str()).unwrap_or("").to_string();
                                    let i18n_key = msg.get("i18n_key").and_then(|v| v.as_str()).map(|s| s.to_string());
                                    let i18n_params = msg.get("i18n_params").cloned();
                                    let _ = app.emit("aesthetic-progress", ProgressEvent {
                                        current: i as u32 + 1, total,
                                        filename: filename.clone(),
                                        status: "info".to_string(),
                                        message: text, i18n_key, i18n_params,
                                    });
                                }
                                _ => { break; }
                            }
                        }
                    }
                    None => { fail_count += 1; errors.push(format!("{}: Python 进程退出", filename)); break; }
                }
            }
        } else {
            // 批量模式
            let images: Vec<serde_json::Value> = batch_files.iter().map(|fp| {
                serde_json::json!({
                    "image_path": fp.to_string_lossy(),
                    "move_files": options.move_files,
                    "output_path": if options.output_path.is_empty() { "" } else { &options.output_path },
                })
            }).collect();
            let batch_cmd = serde_json::json!({ "cmd": "score_batch", "images": images });
            if let Err(e) = writeln!(stdin, "{}", batch_cmd) {
                fail_count += batch_len as u32;
                errors.push(format!("批量发送失败: {}", e));
                break;
            }
            let mut results_read = 0usize;
            while results_read < batch_len {
                match read_utf8_line(&mut reader) {
                    Some(line) => {
                        if let Ok(msg) = serde_json::from_str::<serde_json::Value>(&line) {
                            let msg_type = msg.get("type").and_then(|v| v.as_str()).unwrap_or("");
                            match msg_type {
                                "result" => {
                                    let img_path = msg.get("image_path").and_then(|v| v.as_str()).unwrap_or("");
                                    let fname = std::path::Path::new(img_path).file_name().unwrap_or_default().to_string_lossy().to_string();
                                    let label = msg.get("label").and_then(|v| v.as_str()).unwrap_or("?");
                                    let score = msg.get("score").and_then(|v| v.as_f64()).unwrap_or(0.0);
                                    let confidence = msg.get("confidence").and_then(|v| v.as_f64()).unwrap_or(0.0);
                                    success_count += 1;
                                    results_read += 1;
                                    let _ = app.emit("aesthetic-progress", ProgressEvent {
                                        current: (i + results_read) as u32, total,
                                        filename: fname.clone(),
                                        status: "success".to_string(),
                                        message: format!("[完成] {} → {} (分数: {:.2}, 置信度: {:.1}%)", fname, label, score, confidence * 100.0),
                                    ..Default::default()
                                    });
                                }
                                "error" => {
                                    let img_path = msg.get("image_path").and_then(|v| v.as_str()).unwrap_or("");
                                    let fname = std::path::Path::new(img_path).file_name().unwrap_or_default().to_string_lossy().to_string();
                                    let text = msg.get("message").and_then(|v| v.as_str()).unwrap_or("unknown");
                                    fail_count += 1;
                                    results_read += 1;
                                    errors.push(format!("{}: {}", fname, text));
                                    let _ = app.emit("aesthetic-progress", ProgressEvent {
                                        current: (i + results_read) as u32, total,
                                        filename: fname.clone(),
                                        status: "error".to_string(),
                                        message: format!("[错误] {}: {}", fname, text),
                                    ..Default::default()
                                    });
                                }
                                "log" => {
                                    let text = msg.get("message").and_then(|v| v.as_str()).unwrap_or("").to_string();
                                    let i18n_key = msg.get("i18n_key").and_then(|v| v.as_str()).map(|s| s.to_string());
                                    let i18n_params = msg.get("i18n_params").cloned();
                                    let _ = app.emit("aesthetic-progress", ProgressEvent {
                                        current: (i + results_read) as u32 + 1, total,
                                        filename: String::new(),
                                        status: "info".to_string(),
                                        message: text, i18n_key, i18n_params,
                                    });
                                }
                                _ => { results_read += 1; }
                            }
                        }
                    }
                    None => { fail_count += (batch_len - results_read) as u32; errors.push("Python 进程退出".to_string()); break; }
                }
            }
        }
        i = end;
    }

    let _ = writeln!(stdin, r#"{{"cmd":"quit"}}"#);
    // 取出全局句柄并等待进程退出（若已被取消杀掉则为 None）
    if let Some(mut child) = AESTHETIC_PROCESS.lock().ok().and_then(|mut g| g.take()) {
        let _ = child.wait();
    }

    let _ = app.emit("aesthetic-progress", ProgressEvent {
        current: total, total, filename: String::new(),
        status: "done".to_string(),
        message: format!("美学评分完成: 成功 {}, 失败 {}, 共 {}", success_count, fail_count, total),
    ..Default::default()
    });

    Ok(ProcessResult { success_count, fail_count, total, errors })
}

// ===== Tauri Commands =====

/// 开始美学评分
#[tauri::command]
pub async fn start_aesthetic_scoring(
    app: tauri::AppHandle,
    options: AestheticOptions,
) -> Result<ProcessResult, String> {
    // 重置取消标志
    AESTHETIC_CANCELLED.store(false, Ordering::SeqCst);

    // 检查 Python 环境
    let python_check = tokio::task::spawn_blocking(|| {
        super::tagger::inference::check_python_env()
    }).await.map_err(|e| format!("检测线程异常: {}", e))?;

    if python_check.is_err() {
        let _ = app.emit("aesthetic-progress", ProgressEvent {
            current: 0, total: 0, filename: String::new(),
            status: "info".to_string(),
            message: "Python 环境未就绪，正在自动配置...".to_string(),
        ..Default::default()
        });
        super::python_env::setup_python_env(&app).await?;
    }

    // 下载模型
    if !is_model_downloaded() {
        download_model(&app).await?;

        if AESTHETIC_CANCELLED.load(Ordering::SeqCst) {
            return Err("已取消".into());
        }
    }

    let model_path = get_aesthetic_model_dir().join("model.onnx");
    let opts = options.clone();
    let app_clone = app.clone();

    tokio::task::spawn_blocking(move || {
        run_aesthetic_scoring(&app_clone, &opts, &model_path)
    })
    .await
    .map_err(|e| format!("任务执行失败: {}", e))?
}

/// 取消美学评分
#[tauri::command]
pub fn cancel_aesthetic_scoring() {
    AESTHETIC_CANCELLED.store(true, Ordering::SeqCst);
    DOWNLOAD_CANCELLED.store(true, Ordering::SeqCst);
    kill_process();
}
