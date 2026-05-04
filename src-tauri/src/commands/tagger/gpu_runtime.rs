use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::Emitter;

use super::ProgressEvent;

/// 下载取消标志
static ORT_DL_CANCELLED: AtomicBool = AtomicBool::new(false);

/// ONNX Runtime 版本（与 ort 2.0.0-rc.12 兼容）
const ORT_VERSION: &str = "1.22.0";

/// ONNX Runtime 状态
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrtRuntimeStatus {
    pub available: bool,
    pub path: String,
    pub version: String,
    pub files: Vec<String>,
}

/// 获取 ONNX Runtime 存储目录（软件根目录/env/ort/）
pub fn get_ort_dir() -> PathBuf {
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|p| p.to_path_buf()))
        .unwrap_or_else(|| PathBuf::from("."));
    let new_dir = exe_dir.join("env").join("ort");
    // 兼容旧路径 env/ort-gpu/ → env/ort/
    if !new_dir.exists() {
        let old_dir = exe_dir.join("env").join("ort-gpu");
        if old_dir.exists() {
            let _ = std::fs::rename(&old_dir, &new_dir);
        }
    }
    new_dir
}

/// 检查 ONNX Runtime 是否已下载
pub fn check_ort_runtime() -> OrtRuntimeStatus {
    let dir = get_ort_dir();

    #[cfg(target_os = "windows")]
    let required_files = vec!["onnxruntime.dll"];

    #[cfg(target_os = "macos")]
    let required_files = vec!["libonnxruntime.dylib"];

    #[cfg(target_os = "linux")]
    let required_files = vec!["libonnxruntime.so"];

    let mut found_files = Vec::new();
    let mut all_found = true;

    for file in &required_files {
        let path = dir.join(file);
        if path.exists() {
            found_files.push(file.to_string());
        } else {
            all_found = false;
        }
    }

    #[cfg(target_os = "windows")]
    {
        for extra in &["onnxruntime_providers_cuda.dll", "onnxruntime_providers_shared.dll"] {
            if dir.join(extra).exists() {
                found_files.push(extra.to_string());
            }
        }
    }

    OrtRuntimeStatus {
        available: all_found,
        path: dir.to_string_lossy().to_string(),
        version: ORT_VERSION.to_string(),
        files: found_files,
    }
}

/// 在 ort 加载前设置环境变量
pub fn setup_ort_env() -> bool {
    let dir = get_ort_dir();

    #[cfg(target_os = "windows")]
    let lib_name = "onnxruntime.dll";

    #[cfg(target_os = "macos")]
    let lib_name = "libonnxruntime.dylib";

    #[cfg(target_os = "linux")]
    let lib_name = "libonnxruntime.so";

    let lib_path = dir.join(lib_name);

    if lib_path.exists() {
        std::env::set_var("ORT_DYLIB_PATH", &lib_path);
        true
    } else {
        false
    }
}

/// 取消下载
pub fn cancel_ort_download() {
    ORT_DL_CANCELLED.store(true, Ordering::SeqCst);
}

/// 下载 ONNX Runtime
pub async fn download_ort_runtime(app: &tauri::AppHandle) -> Result<(), String> {
    ORT_DL_CANCELLED.store(false, Ordering::SeqCst);

    let dir = get_ort_dir();
    if !dir.exists() {
        std::fs::create_dir_all(&dir)
            .map_err(|e| format!("创建目录失败: {}", e))?;
    }

    // 根据平台选择下载 URL（官方 Microsoft ONNX Runtime releases）
    #[cfg(target_os = "windows")]
    let (url, archive_name) = (
        format!("https://github.com/microsoft/onnxruntime/releases/download/v{}/onnxruntime-win-x64-gpu-{}.zip", ORT_VERSION, ORT_VERSION),
        format!("onnxruntime-win-x64-gpu-{}.zip", ORT_VERSION),
    );

    #[cfg(target_os = "linux")]
    let (url, archive_name) = (
        format!("https://github.com/microsoft/onnxruntime/releases/download/v{}/onnxruntime-linux-x64-gpu-{}.tgz", ORT_VERSION, ORT_VERSION),
        format!("onnxruntime-linux-x64-gpu-{}.tgz", ORT_VERSION),
    );

    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    let (url, archive_name) = (
        format!("https://github.com/microsoft/onnxruntime/releases/download/v{}/onnxruntime-osx-arm64-{}.tgz", ORT_VERSION, ORT_VERSION),
        format!("onnxruntime-osx-arm64-{}.tgz", ORT_VERSION),
    );

    #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
    let (url, archive_name) = (
        format!("https://github.com/microsoft/onnxruntime/releases/download/v{}/onnxruntime-osx-x86_64-{}.tgz", ORT_VERSION, ORT_VERSION),
        format!("onnxruntime-osx-x86_64-{}.tgz", ORT_VERSION),
    );

    let archive_path = dir.join(&archive_name);

    let _ = app.emit("tagger-progress", ProgressEvent {
        current: 0, total: 0, filename: String::new(),
        status: "info".to_string(),
        message: format!("开始下载 ONNX Runtime v{}...", ORT_VERSION),
    });

    download_with_progress(app, &url, &archive_path, &archive_name).await?;

    if ORT_DL_CANCELLED.load(Ordering::SeqCst) {
        let _ = std::fs::remove_file(&archive_path);
        return Err("下载已取消".into());
    }

    let _ = app.emit("tagger-progress", ProgressEvent {
        current: 0, total: 0, filename: String::new(),
        status: "info".to_string(),
        message: "正在解压 ONNX Runtime...".to_string(),
    });

    extract_runtime(&archive_path, &dir)?;
    let _ = std::fs::remove_file(&archive_path);

    let status = check_ort_runtime();
    if status.available {
        setup_ort_env();
        let _ = app.emit("tagger-progress", ProgressEvent {
            current: 0, total: 0, filename: String::new(),
            status: "success".to_string(),
            message: format!("ONNX Runtime v{} 安装成功！重启应用后生效", ORT_VERSION),
        });
        Ok(())
    } else {
        Err("解压后未找到所需文件".into())
    }
}

/// 带进度的下载
async fn download_with_progress(
    app: &tauri::AppHandle,
    url: &str,
    dest: &std::path::Path,
    label: &str,
) -> Result<(), String> {
    let client = reqwest::Client::builder()
        .user_agent("PurinBox/0.1.3")
        .timeout(std::time::Duration::from_secs(1800))
        .build()
        .map_err(|e| format!("创建 HTTP 客户端失败: {}", e))?;

    let response = client.get(url).send().await
        .map_err(|e| format!("下载请求失败: {}", e))?;

    if !response.status().is_success() {
        return Err(format!("下载失败 (HTTP {}): {}", response.status(), url));
    }

    let total_size = response.content_length().unwrap_or(0);
    let mut stream = response.bytes_stream();
    let mut file = tokio::fs::File::create(dest).await
        .map_err(|e| format!("创建文件失败: {}", e))?;

    let mut downloaded: u64 = 0;
    let start = std::time::Instant::now();
    let mut last_report = std::time::Instant::now();

    while let Some(chunk) = stream.next().await {
        if ORT_DL_CANCELLED.load(Ordering::SeqCst) {
            drop(file);
            let _ = tokio::fs::remove_file(dest).await;
            return Err("下载已取消".into());
        }

        let chunk = chunk.map_err(|e| format!("下载数据失败: {}", e))?;
        tokio::io::AsyncWriteExt::write_all(&mut file, &chunk).await
            .map_err(|e| format!("写入失败: {}", e))?;

        downloaded += chunk.len() as u64;

        if last_report.elapsed().as_millis() >= 500 || (total_size > 0 && downloaded >= total_size) {
            last_report = std::time::Instant::now();
            let elapsed = start.elapsed().as_secs_f64();
            let speed = if elapsed > 0.0 { downloaded as f64 / elapsed / 1_048_576.0 } else { 0.0 };
            let pct = if total_size > 0 { (downloaded as f64 / total_size as f64 * 100.0) as u32 } else { 0 };
            let mb_done = downloaded as f64 / 1_048_576.0;
            let msg = if total_size > 0 {
                let mb_total = total_size as f64 / 1_048_576.0;
                format!("[GPU Runtime] {} — {:.1}/{:.1} MB ({:.1} MB/s) {}%", label, mb_done, mb_total, speed, pct)
            } else {
                format!("[GPU Runtime] {} — {:.1} MB ({:.1} MB/s)", label, mb_done, speed)
            };

            let _ = app.emit("tagger-download", super::download::DownloadProgress {
                filename: label.to_string(),
                downloaded, total: total_size,
                percent: pct as f32,
                speed_mbps: speed,
                status: "downloading".to_string(),
                message: msg,
            });
        }
    }

    let _ = app.emit("tagger-download", super::download::DownloadProgress {
        filename: label.to_string(),
        downloaded, total: total_size,
        percent: 100.0,
        speed_mbps: 0.0,
        status: "done".to_string(),
        message: "下载完成".to_string(),
    });

    Ok(())
}

/// 解压运行时文件到目标目录
#[cfg(target_os = "windows")]
fn extract_runtime(archive: &std::path::Path, dest: &std::path::Path) -> Result<(), String> {
    let file = std::fs::File::open(archive)
        .map_err(|e| format!("打开压缩包失败: {}", e))?;
    let mut zip = zip::ZipArchive::new(file)
        .map_err(|e| format!("读取 ZIP 失败: {}", e))?;

    // 需要提取的 DLL 文件
    let target_files = [
        "onnxruntime.dll",
        "onnxruntime_providers_cuda.dll",
        "onnxruntime_providers_shared.dll",
    ];

    for i in 0..zip.len() {
        let mut entry = zip.by_index(i)
            .map_err(|e| format!("读取 ZIP 条目失败: {}", e))?;

        let name = entry.name().to_string();
        let filename = std::path::Path::new(&name)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("");

        if target_files.contains(&filename) {
            let out_path = dest.join(filename);
            let mut out_file = std::fs::File::create(&out_path)
                .map_err(|e| format!("创建文件失败: {}", e))?;
            std::io::copy(&mut entry, &mut out_file)
                .map_err(|e| format!("解压失败: {}", e))?;
        }
    }

    Ok(())
}

#[cfg(not(target_os = "windows"))]
fn extract_runtime(archive: &std::path::Path, dest: &std::path::Path) -> Result<(), String> {
    use std::process::Command;
    // 先解压到 dest（--strip-components=1 去掉顶层目录名）
    let status = Command::new("tar")
        .args(["xzf", &archive.to_string_lossy(), "-C", &dest.to_string_lossy(), "--strip-components=1"])
        .status()
        .map_err(|e| format!("解压失败: {}", e))?;

    if !status.success() {
        return Err("tar 解压失败".into());
    }

    // 将 lib/ 子目录中的文件移到 dest 根目录
    let lib_dir = dest.join("lib");
    if lib_dir.exists() {
        if let Ok(entries) = std::fs::read_dir(&lib_dir) {
            for entry in entries.flatten() {
                let fname = entry.file_name();
                let target = dest.join(&fname);
                let _ = std::fs::rename(entry.path(), &target);
            }
        }
        let _ = std::fs::remove_dir_all(&lib_dir);
    }
    Ok(())
}
