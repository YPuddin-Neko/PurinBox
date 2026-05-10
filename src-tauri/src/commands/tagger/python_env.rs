//! Python 环境自动管理
//! 首次使用时自动下载 standalone Python + 安装 onnxruntime 依赖

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::Emitter;

use super::ProgressEvent;

/// 全局取消标志
static SETUP_CANCELLED: AtomicBool = AtomicBool::new(false);

pub fn cancel_setup() {
    SETUP_CANCELLED.store(true, Ordering::SeqCst);
}

fn is_cancelled() -> bool {
    SETUP_CANCELLED.load(Ordering::SeqCst)
}

/// Python standalone 下载信息
struct PythonDownloadInfo {
    url: &'static str,
}

fn get_download_info() -> PythonDownloadInfo {
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    {
        PythonDownloadInfo {
            url: "https://github.com/astral-sh/python-build-standalone/releases/download/20260414/cpython-3.12.13+20260414-aarch64-apple-darwin-install_only_stripped.tar.gz",

        }
    }
    #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
    {
        PythonDownloadInfo {
            url: "https://github.com/astral-sh/python-build-standalone/releases/download/20260414/cpython-3.12.13+20260414-x86_64-apple-darwin-install_only_stripped.tar.gz",

        }
    }
    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    {
        PythonDownloadInfo {
            url: "https://github.com/astral-sh/python-build-standalone/releases/download/20260414/cpython-3.12.13+20260414-x86_64-pc-windows-msvc-install_only_stripped.tar.gz",

        }
    }
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    {
        PythonDownloadInfo {
            url: "https://github.com/astral-sh/python-build-standalone/releases/download/20260414/cpython-3.12.13+20260414-x86_64-unknown-linux-gnu-install_only_stripped.tar.gz",

        }
    }
}

/// 获取 env 根目录（存放 Python 环境等）
fn get_env_dir() -> PathBuf {
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|p| p.to_path_buf()))
        .unwrap_or_else(|| PathBuf::from("."));

    if cfg!(debug_assertions) {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or(exe_dir)
            .join("env")
    } else {
        exe_dir.join("env")
    }
}

/// 获取 Python 安装目录 (standalone 解释器)
fn get_python_dir() -> PathBuf {
    get_env_dir().join("python").join("base")
}

/// 获取 venv 目录
fn get_venv_dir() -> PathBuf {
    get_env_dir().join("python").join("venv")
}

/// 获取 venv 中的 python 可执行文件路径
fn get_venv_python() -> PathBuf {
    #[cfg(target_os = "windows")]
    { get_venv_dir().join("Scripts").join("python.exe") }
    #[cfg(not(target_os = "windows"))]
    { get_venv_dir().join("bin").join("python3") }
}

/// 获取 standalone Python 可执行文件路径
fn get_standalone_python() -> PathBuf {
    #[cfg(target_os = "windows")]
    { get_python_dir().join("python.exe") }
    #[cfg(not(target_os = "windows"))]
    { get_python_dir().join("bin").join("python3") }
}

/// 检查 Python 环境是否就绪（venv 存在且有 onnxruntime）
pub fn is_ready() -> bool {
    let python = get_venv_python();
    if !python.exists() {
        return false;
    }
    // 快速检查 onnxruntime 是否可用
    let mut cmd = std::process::Command::new(&python);
    cmd.args(["-c", "import onnxruntime"]);
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000);
    }
    cmd.output().map(|o| o.status.success()).unwrap_or(false)
}

/// 获取就绪的 Python 路径（如果已设置好）
pub fn get_python_exe() -> Option<String> {
    if is_ready() {
        Some(get_venv_python().to_string_lossy().to_string())
    } else {
        None
    }
}

/// 发送进度事件
fn emit_progress(app: &tauri::AppHandle, message: &str, status: &str) {
    let _ = app.emit("tagger-progress", ProgressEvent {
        current: 0, total: 0,
        filename: String::new(),
        status: status.to_string(),
        message: message.to_string(),
    });
}

/// 检测系统安装的 Python 3（非 standalone）
/// 返回 (命令/路径, 版本号)
fn detect_system_python() -> Option<(String, String)> {
    let candidates = if cfg!(target_os = "windows") {
        vec!["python3", "python", "py"]
    } else {
        vec!["python3", "python"]
    };

    for name in &candidates {
        let mut cmd = std::process::Command::new(name);
        cmd.args(["--version"]);
        #[cfg(target_os = "windows")]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x08000000);
        }
        if let Ok(output) = cmd.output() {
            if output.status.success() {
                let ver_output = String::from_utf8_lossy(&output.stdout).to_string()
                    + &String::from_utf8_lossy(&output.stderr).to_string();
                if ver_output.contains("Python 3") {
                    // 提取版本号
                    let version = ver_output.trim().to_string();
                    // 验证可以运行 -m venv
                    let mut test = std::process::Command::new(name);
                    test.args(["-c", "import venv"]);
                    #[cfg(target_os = "windows")]
                    {
                        use std::os::windows::process::CommandExt;
                        test.creation_flags(0x08000000);
                    }
                    if test.output().map(|o| o.status.success()).unwrap_or(false) {
                        // 获取实际路径
                        let real_path = resolve_python_path(name);
                        return Some((real_path, version));
                    }
                }
            }
        }
    }

    // Windows: 尝试常见安装路径
    #[cfg(target_os = "windows")]
    {
        let paths = [
            r"C:\Python312\python.exe",
            r"C:\Python311\python.exe",
            r"C:\Python310\python.exe",
        ];
        for p in &paths {
            if std::path::Path::new(p).exists() {
                // 获取版本号
                let mut cmd = std::process::Command::new(p);
                cmd.args(["--version"]);
                use std::os::windows::process::CommandExt;
                cmd.creation_flags(0x08000000);
                let version = cmd.output()
                    .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
                    .unwrap_or_else(|_| "Python 3".to_string());
                return Some((p.to_string(), version));
            }
        }
    }

    None
}

/// 解析 Python 命令的实际可执行文件路径
fn resolve_python_path(name: &str) -> String {
    #[cfg(target_os = "windows")]
    {
        let mut cmd = std::process::Command::new("where");
        cmd.arg(name);
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000);
        if let Ok(output) = cmd.output() {
            if output.status.success() {
                let path = String::from_utf8_lossy(&output.stdout);
                if let Some(first_line) = path.lines().next() {
                    let p = first_line.trim();
                    if !p.is_empty() {
                        return p.to_string();
                    }
                }
            }
        }
    }
    #[cfg(not(target_os = "windows"))]
    {
        let mut cmd = std::process::Command::new("which");
        cmd.arg(name);
        if let Ok(output) = cmd.output() {
            if output.status.success() {
                let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
                if !path.is_empty() {
                    return path;
                }
            }
        }
    }
    name.to_string()
}

/// 完整的 Python 环境设置流程
pub async fn setup_python_env(app: &tauri::AppHandle) -> Result<String, String> {
    SETUP_CANCELLED.store(false, Ordering::SeqCst);

    let venv_python = get_venv_python();

    // 1. 如果 venv 已就绪，直接返回
    if is_ready() {
        return Ok(venv_python.to_string_lossy().to_string());
    }

    // 2. 优先检测系统 Python
    if let Some((sys_python, sys_version)) = detect_system_python() {
        emit_progress(app, &format!("检测到系统 Python: {} ({})", sys_version, sys_python), "success");

        // 用系统 Python 创建 venv（如果 venv 不存在）
        if !venv_python.exists() {
            emit_progress(app, "正在使用系统 Python 创建虚拟环境...", "info");
            let venv_dir = get_venv_dir();
            // 确保父目录存在
            let python_parent = get_env_dir().join("python");
            std::fs::create_dir_all(&python_parent)
                .map_err(|e| format!("创建目录失败: {}", e))?;

            let mut cmd = std::process::Command::new(&sys_python);
            cmd.args(["-m", "venv", &venv_dir.to_string_lossy()]);
            #[cfg(target_os = "windows")]
            {
                use std::os::windows::process::CommandExt;
                cmd.creation_flags(0x08000000);
            }
            let output = cmd.output().map_err(|e| format!("创建 venv 失败: {}", e))?;
            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                emit_progress(app, &format!("系统 Python 创建 venv 失败: {}，将下载独立版本", stderr), "info");
                // 失败则回退到下载 standalone
                return setup_with_standalone(app).await;
            }
            emit_progress(app, "✓ 虚拟环境已创建（使用系统 Python）", "success");
        }

        if is_cancelled() { return Err("已取消".into()); }

        // 安装依赖
        emit_progress(app, "正在安装推理依赖 (onnxruntime, numpy, pillow)...", "info");
        install_deps(app)?;

        if is_cancelled() { return Err("已取消".into()); }

        if !is_ready() {
            return Err("Python 环境安装后验证失败".into());
        }

        emit_progress(app, "✓ Python 推理环境准备完成（使用系统 Python）", "success");
        return Ok(venv_python.to_string_lossy().to_string());
    }

    // 3. 系统没有 Python，下载 standalone 版本
    emit_progress(app, "未检测到系统 Python，将下载独立 Python 环境...", "info");
    setup_with_standalone(app).await
}

/// 使用 standalone Python 完成环境设置（下载+venv+依赖）
async fn setup_with_standalone(app: &tauri::AppHandle) -> Result<String, String> {
    let python_exe = get_standalone_python();
    let venv_python = get_venv_python();

    // 下载 standalone Python
    if !python_exe.exists() {
        emit_progress(app, "正在下载 Python 运行环境...", "info");
        download_python(app).await?;
    }

    if is_cancelled() { return Err("已取消".into()); }

    // 创建 venv
    if !venv_python.exists() {
        emit_progress(app, "正在创建 Python 虚拟环境...", "info");
        create_venv(app)?;
    }

    if is_cancelled() { return Err("已取消".into()); }

    // 安装依赖
    emit_progress(app, "正在安装推理依赖 (onnxruntime, numpy, pillow)...", "info");
    install_deps(app)?;

    if is_cancelled() { return Err("已取消".into()); }

    // 验证
    if !is_ready() {
        return Err("Python 环境安装后验证失败".into());
    }

    emit_progress(app, "✓ Python 推理环境准备完成", "success");
    Ok(venv_python.to_string_lossy().to_string())
}

/// 下载 standalone Python
async fn download_python(app: &tauri::AppHandle) -> Result<(), String> {
    let info = get_download_info();
    let python_dir = get_python_dir();
    let env_dir = get_env_dir();

    if !env_dir.exists() {
        std::fs::create_dir_all(&env_dir)
            .map_err(|e| format!("创建 env 目录失败: {}", e))?;
    }

    emit_progress(app, &format!("下载: {}", info.url.split('/').next_back().unwrap_or("python")), "info");

    // 下载
    let client = crate::commands::proxy_config::build_http_client()
        .build()
        .map_err(|e| format!("创建 HTTP 客户端失败: {}", e))?;
    let resp = client.get(info.url)
        .send().await
        .map_err(|e| format!("下载失败: {}", e))?;

    if !resp.status().is_success() {
        return Err(format!("下载失败: HTTP {}", resp.status()));
    }

    let total_size = resp.content_length().unwrap_or(0);
    let archive_path = env_dir.join("python_download.tar.gz");

    // 流式写入
    use futures_util::StreamExt;
    let mut stream = resp.bytes_stream();
    let mut file = tokio::fs::File::create(&archive_path).await
        .map_err(|e| format!("创建文件失败: {}", e))?;

    let mut downloaded: u64 = 0;
    let mut last_pct: u64 = 0;

    while let Some(chunk) = stream.next().await {
        if is_cancelled() {
            let _ = tokio::fs::remove_file(&archive_path).await;
            return Err("已取消".into());
        }

        let bytes = chunk.map_err(|e| format!("下载错误: {}", e))?;
        use tokio::io::AsyncWriteExt;
        file.write_all(&bytes).await
            .map_err(|e| format!("写入失败: {}", e))?;

        downloaded += bytes.len() as u64;
        let pct = if total_size > 0 { downloaded * 100 / total_size } else { 0 };
        if pct != last_pct {
            last_pct = pct;
            let mb = downloaded as f64 / 1024.0 / 1024.0;
            let total_mb = total_size as f64 / 1024.0 / 1024.0;
            emit_progress(app, &format!("⬇ 下载 Python: {:.1}/{:.1} MB ({}%)", mb, total_mb, pct), "download-progress");
        }
    }

    drop(file);
    emit_progress(app, "正在解压 Python...", "info");

    // 解压到临时目录
    let extract_tmp = env_dir.join("_python_extract_tmp");
    if extract_tmp.exists() {
        let _ = std::fs::remove_dir_all(&extract_tmp);
    }
    std::fs::create_dir_all(&extract_tmp)
        .map_err(|e| format!("创建临时目录失败: {}", e))?;

    let archive_path_clone = archive_path.clone();
    let extract_tmp_clone = extract_tmp.clone();
    tokio::task::spawn_blocking(move || {
        extract_tar_gz(&archive_path_clone, &extract_tmp_clone)
    }).await
    .map_err(|e| format!("解压任务失败: {}", e))??;

    // 清理下载文件
    let _ = tokio::fs::remove_file(&archive_path).await;

    // 移动: _python_extract_tmp/python/ → env/python/base/
    let extracted_python = extract_tmp.join("python");
    if !extracted_python.exists() {
        // 尝试查找解压出来的目录
        let _ = std::fs::remove_dir_all(&extract_tmp);
        return Err("解压后未找到 python 目录".into());
    }

    // 确保目标父目录存在
    let python_parent = get_env_dir().join("python");
    std::fs::create_dir_all(&python_parent)
        .map_err(|e| format!("创建 python 目录失败: {}", e))?;

    // 如果 base/ 已存在，先删除
    if python_dir.exists() {
        let _ = std::fs::remove_dir_all(&python_dir);
    }

    std::fs::rename(&extracted_python, &python_dir)
        .map_err(|e| format!("移动 Python 目录失败: {}", e))?;

    // 清理临时目录
    let _ = std::fs::remove_dir_all(&extract_tmp);

    // 验证
    let python_exe = get_standalone_python();
    if !python_exe.exists() {
        return Err(format!("Python 解压后未找到: {}", python_exe.display()));
    }

    emit_progress(app, "✓ Python 下载完成", "success");
    Ok(())
}

/// 解压 tar.gz
fn extract_tar_gz(archive: &std::path::Path, dest: &std::path::Path) -> Result<(), String> {
    use std::process::Command;

    let mut cmd = Command::new("tar");
    cmd.args(["xzf", &archive.to_string_lossy(), "-C", &dest.to_string_lossy()]);

    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000);
    }

    let status = cmd.status().map_err(|e| format!("解压失败: {}", e))?;
    if !status.success() {
        return Err("解压 Python 失败".into());
    }
    Ok(())
}

/// 创建 venv
fn create_venv(app: &tauri::AppHandle) -> Result<(), String> {
    let python = get_standalone_python();
    let venv_dir = get_venv_dir();

    let mut cmd = std::process::Command::new(&python);
    cmd.args(["-m", "venv", &venv_dir.to_string_lossy()]);

    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000);
    }

    let output = cmd.output().map_err(|e| format!("创建 venv 失败: {}", e))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("创建 venv 失败: {}", stderr));
    }

    emit_progress(app, "✓ 虚拟环境已创建", "success");
    Ok(())
}

/// 安装依赖
fn install_deps(app: &tauri::AppHandle) -> Result<(), String> {
    let python = get_venv_python();
    let python_str = python.to_string_lossy().to_string();
    pip_install_with_python(app, &python_str, &["onnxruntime", "numpy", "pillow"])
}

/// 安装 GPU 版 onnxruntime（替换 CPU 版）
#[cfg(target_os = "windows")]
pub fn install_gpu_deps(app: &tauri::AppHandle) -> Result<(), String> {
    // 重置取消标志
    SETUP_CANCELLED.store(false, Ordering::SeqCst);

    let python = get_active_python()?;

    emit_progress(app, "安装 onnxruntime-gpu==1.25.1 (需要 cuDNN 9.x)...", "info");

    // 先卸载 CPU 版
    emit_progress(app, "卸载 CPU 版 onnxruntime...", "info");
    {
        let mut cmd = std::process::Command::new(&python);
        cmd.args(["-m", "pip", "uninstall", "-y", "onnxruntime"]);
        #[cfg(target_os = "windows")]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x08000000);
        }
        let _ = cmd.output();
    }

    pip_install_with_python(app, &python, &["onnxruntime-gpu==1.25.1"])
}

/// 获取当前可用的 Python 路径（venv 优先，系统其次）
#[cfg(target_os = "windows")]
fn get_active_python() -> Result<String, String> {
    // 1. 管理的 venv
    if let Some(p) = get_python_exe() {
        return Ok(p);
    }
    // 2. 系统 Python
    for name in &["python3", "python"] {
        let mut cmd = std::process::Command::new(name);
        cmd.args(["--version"]);
        #[cfg(target_os = "windows")]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x08000000);
        }
        if let Ok(output) = cmd.output() {
            if output.status.success() {
                let ver = String::from_utf8_lossy(&output.stdout);
                if ver.contains("Python 3") {
                    return Ok(name.to_string());
                }
            }
        }
    }
    Err("未找到可用的 Python".into())
}

/// 使用指定 Python 执行 pip install
fn pip_install_with_python(app: &tauri::AppHandle, python: &str, deps: &[&str]) -> Result<(), String> {
    for dep in deps {
        if is_cancelled() {
            return Err("已取消".into());
        }

        emit_progress(app, &format!("安装 {}...", dep), "info");

        let mut cmd = std::process::Command::new(python);
        cmd.args(["-m", "pip", "install", "--disable-pip-version-check", "--no-cache-dir", dep]);

        #[cfg(target_os = "windows")]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x08000000);
        }

        let output = cmd.output().map_err(|e| format!("安装 {} 失败: {}", dep, e))?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(format!("安装 {} 失败: {}", dep, stderr));
        }

        emit_progress(app, &format!("✓ {} 已安装", dep), "success");
    }

    Ok(())
}
