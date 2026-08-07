//! Python 环境自动管理
//! 全局共享模块 — 供 tagger、upscale、person_crop 等功能共用
//! 首次使用时自动下载 standalone Python + 安装基础依赖

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::Emitter;

use super::ProgressEvent;

/// Python 下载进度事件
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PythonDownloadProgress {
    pub filename: String,
    pub downloaded: u64,
    pub total: u64,
    pub percent: f32,
    pub speed_mbps: f64,
    pub status: String,
    pub message: String,
}

/// 进度事件名（前端监听此事件）
const PROGRESS_EVENT: &str = "python-env-progress";
const DOWNLOAD_EVENT: &str = "python-env-download";

/// 全局取消标志
static SETUP_CANCELLED: AtomicBool = AtomicBool::new(false);

/// 全局 setup 互斥锁 — 串行化整个环境安装流程，
/// 防止多个功能（tagger/upscale/cluster 等）并发触发 setup 导致两个 pip install 互相踩踏
static SETUP_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

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
    #[cfg(all(target_os = "linux", target_arch = "aarch64"))]
    {
        PythonDownloadInfo {
            url: "https://github.com/astral-sh/python-build-standalone/releases/download/20260414/cpython-3.12.13+20260414-aarch64-unknown-linux-gnu-install_only_stripped.tar.gz",

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
    {
        get_venv_dir().join("Scripts").join("python.exe")
    }
    #[cfg(not(target_os = "windows"))]
    {
        let p3 = get_venv_dir().join("bin").join("python3");
        if p3.exists() {
            return p3;
        }
        let p = get_venv_dir().join("bin").join("python");
        if p.exists() {
            return p;
        }
        p3 // 默认返回 python3 路径
    }
}

/// 获取 standalone Python 可执行文件路径
fn get_standalone_python() -> PathBuf {
    #[cfg(target_os = "windows")]
    {
        get_python_dir().join("python.exe")
    }
    #[cfg(not(target_os = "windows"))]
    {
        get_python_dir().join("bin").join("python3")
    }
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

/// 重置 Python 环境（删除 venv 和 standalone）
#[tauri::command]
pub fn reset_python_env() -> Result<String, String> {
    let python_root = get_env_dir().join("python");
    if python_root.exists() {
        std::fs::remove_dir_all(&python_root)
            .map_err(|e| format!("删除 Python 环境失败: {}", e))?;
    }
    Ok("Python 环境已重置".to_string())
}

/// 手动部署 Python 环境（设置页按钮）
#[tauri::command]
pub async fn deploy_python_env(app: tauri::AppHandle) -> Result<String, String> {
    setup_python_env(&app).await
}

/// 获取 Python 环境信息（供设置页显示）
#[tauri::command]
pub fn get_python_env_info() -> Result<PythonEnvInfo, String> {
    let python = match get_python_exe() {
        Some(p) => p,
        None => {
            return Ok(PythonEnvInfo {
                available: false,
                version: String::new(),
                path: String::new(),
            })
        }
    };

    // 获取版本
    let mut cmd = std::process::Command::new(&python);
    cmd.args(["--version"]);
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000);
    }

    let version = match cmd.output() {
        Ok(output) if output.status.success() => {
            String::from_utf8_lossy(&output.stdout).trim().to_string()
        }
        _ => String::new(),
    };

    Ok(PythonEnvInfo {
        available: !version.is_empty(),
        version,
        path: python,
    })
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PythonEnvInfo {
    pub available: bool,
    pub version: String,
    pub path: String,
}

/// 发送进度事件
fn emit_progress(app: &tauri::AppHandle, message: &str, status: &str) {
    let _ = app.emit(
        PROGRESS_EVENT,
        ProgressEvent {
            current: 0,
            total: 0,
            filename: String::new(),
            status: status.to_string(),
            message: message.to_string(),
            ..Default::default()
        },
    );
}

/// 最低要求的 Python 次版本号（3.10+）
const MIN_PYTHON_MINOR: u32 = 10;

/// 从 "Python 3.x.y" 字符串中提取次版本号
fn parse_python_minor(ver_str: &str) -> Option<u32> {
    // "Python 3.12.13" → 12
    let s = ver_str.trim();
    let after = s.strip_prefix("Python 3.")?;
    let minor_str: String = after.chars().take_while(|c| c.is_ascii_digit()).collect();
    minor_str.parse().ok()
}

/// 检测系统安装的 Python 3（非 standalone）
/// 要求 >= 3.10，低于此版本的跳过（依赖包不再支持旧版本）
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
                    + String::from_utf8_lossy(&output.stderr).as_ref();
                if ver_output.contains("Python 3") {
                    let version = ver_output.trim().to_string();
                    // 检查版本是否 >= 3.10
                    if let Some(minor) = parse_python_minor(&version) {
                        if minor < MIN_PYTHON_MINOR {
                            continue; // 版本太旧，跳过
                        }
                    }
                    // 验证可以运行 -m venv
                    let mut test = std::process::Command::new(name);
                    test.args(["-c", "import venv"]);
                    #[cfg(target_os = "windows")]
                    {
                        use std::os::windows::process::CommandExt;
                        test.creation_flags(0x08000000);
                    }
                    if test.output().map(|o| o.status.success()).unwrap_or(false) {
                        let real_path = resolve_python_path(name);
                        return Some((real_path, version));
                    }
                }
            }
        }
    }

    // Windows: 尝试常见安装路径（仅 3.10+）
    #[cfg(target_os = "windows")]
    {
        let paths = [
            r"C:\Python312\python.exe",
            r"C:\Python311\python.exe",
            r"C:\Python310\python.exe",
        ];
        for p in &paths {
            if std::path::Path::new(p).exists() {
                let mut cmd = std::process::Command::new(p);
                cmd.args(["--version"]);
                use std::os::windows::process::CommandExt;
                cmd.creation_flags(0x08000000);
                let version = cmd
                    .output()
                    .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
                    .unwrap_or_else(|_| "Python 3".to_string());
                if let Some(minor) = parse_python_minor(&version) {
                    if minor < MIN_PYTHON_MINOR {
                        continue;
                    }
                }
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

/// 完整的 Python 环境设置流程（入口，全局串行化）
pub async fn setup_python_env(app: &tauri::AppHandle) -> Result<String, String> {
    // 串行化整个 setup 流程；tokio::sync::Mutex 的 guard 可安全地跨 await 持有
    let _setup_guard = SETUP_LOCK.lock().await;
    setup_python_env_inner(app).await
}

/// setup 实际逻辑（调用方必须已持有 SETUP_LOCK）
async fn setup_python_env_inner(app: &tauri::AppHandle) -> Result<String, String> {
    SETUP_CANCELLED.store(false, Ordering::SeqCst);

    let venv_python = get_venv_python();

    // 1. 如果 venv 已就绪，直接返回（is_ready 内部会运行子进程，放入 blocking 线程）
    if tokio::task::spawn_blocking(is_ready).await.unwrap_or(false) {
        return Ok(venv_python.to_string_lossy().to_string());
    }

    // 2. 优先检测系统 Python（内部多次调用 cmd.output()，放入 blocking 线程）
    let detected = tokio::task::spawn_blocking(detect_system_python)
        .await
        .map_err(|e| format!("检测线程异常: {}", e))?;
    if let Some((sys_python, _sys_version)) = detected {
        // 步骤 1 已确认环境未就绪，这里总是（重）建 venv
        {
            let venv_dir = get_venv_dir();
            // 清理旧的 venv（可能有残留的 broken symlink）
            if venv_dir.exists() {
                let _ = std::fs::remove_dir_all(&venv_dir);
            }
            // 确保父目录存在
            let python_parent = get_env_dir().join("python");
            std::fs::create_dir_all(&python_parent).map_err(|e| format!("创建目录失败: {}", e))?;

            // venv 创建可能耗时较久，cmd.output() 放入 blocking 线程
            let sys_python_clone = sys_python.clone();
            let venv_dir_str = venv_dir.to_string_lossy().to_string();
            let output = tokio::task::spawn_blocking(move || {
                let mut cmd = std::process::Command::new(&sys_python_clone);
                cmd.args(["-m", "venv", &venv_dir_str])
                    .env("PYTHONIOENCODING", "utf-8");
                #[cfg(target_os = "windows")]
                {
                    use std::os::windows::process::CommandExt;
                    cmd.creation_flags(0x08000000);
                }
                cmd.output()
            })
            .await
            .map_err(|e| format!("创建 venv 线程异常: {}", e))?
            .map_err(|e| format!("创建 venv 失败: {}", e))?;
            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                emit_progress(
                    app,
                    &format!(
                        "@pythonEnv.venvFailed|{}",
                        stderr.to_string().chars().take(100).collect::<String>()
                    ),
                    "info",
                );
                // 失败则回退到下载 standalone
                return setup_with_standalone(app).await;
            }
        }

        if is_cancelled() {
            return Err("已取消".into());
        }

        // 安装依赖（pip install 可阻塞数分钟，放入 blocking 线程）
        let app2 = app.clone();
        tokio::task::spawn_blocking(move || install_deps(&app2))
            .await
            .map_err(|e| format!("安装线程异常: {}", e))??;

        if is_cancelled() {
            return Err("已取消".into());
        }

        if !tokio::task::spawn_blocking(is_ready).await.unwrap_or(false) {
            return Err("Python 环境安装后验证失败".into());
        }

        emit_progress(app, "@pythonEnv.ready", "success");
        return Ok(venv_python.to_string_lossy().to_string());
    }

    // 3. 系统没有 Python，下载 standalone 版本
    setup_with_standalone(app).await
}

/// 使用 standalone Python 完成环境设置（下载+venv+依赖）
async fn setup_with_standalone(app: &tauri::AppHandle) -> Result<String, String> {
    let python_exe = get_standalone_python();
    let venv_python = get_venv_python();

    // 下载 standalone Python
    if !python_exe.exists() {
        download_python(app).await?;
    }

    if is_cancelled() {
        return Err("已取消".into());
    }

    // 创建 venv（内部 cmd.output() 可能耗时，放入 blocking 线程）
    if !venv_python.exists() {
        let app2 = app.clone();
        tokio::task::spawn_blocking(move || create_venv(&app2))
            .await
            .map_err(|e| format!("创建 venv 线程异常: {}", e))??;
    }

    if is_cancelled() {
        return Err("已取消".into());
    }

    // 安装依赖（pip install 可阻塞数分钟，放入 blocking 线程）
    let app2 = app.clone();
    tokio::task::spawn_blocking(move || install_deps(&app2))
        .await
        .map_err(|e| format!("安装线程异常: {}", e))??;

    if is_cancelled() {
        return Err("已取消".into());
    }

    // 验证
    if !tokio::task::spawn_blocking(is_ready).await.unwrap_or(false) {
        return Err("Python 环境安装后验证失败".into());
    }

    emit_progress(app, "@pythonEnv.ready", "success");
    Ok(venv_python.to_string_lossy().to_string())
}

/// 下载 standalone Python
async fn download_python(app: &tauri::AppHandle) -> Result<(), String> {
    let info = get_download_info();
    let python_dir = get_python_dir();
    let env_dir = get_env_dir();

    if !env_dir.exists() {
        std::fs::create_dir_all(&env_dir).map_err(|e| format!("创建 env 目录失败: {}", e))?;
    }

    emit_progress(
        app,
        &format!(
            "@pythonEnv.downloading|{}",
            info.url.split('/').next_back().unwrap_or("python")
        ),
        "info",
    );

    // 下载
    let client = crate::commands::proxy_config::build_http_client()
        .build()
        .map_err(|e| format!("创建 HTTP 客户端失败: {}", e))?;
    let resp = client
        .get(info.url)
        .send()
        .await
        .map_err(|e| format!("下载失败: {}", e))?;

    if !resp.status().is_success() {
        return Err(format!("下载失败: HTTP {}", resp.status()));
    }

    let total_size = resp.content_length().unwrap_or(0);
    let archive_path = env_dir.join("python_download.tar.gz");

    // 先写入 .part 临时文件，校验完成后再原子替换，避免中断残件
    let part_path = super::prepare_part_file(&archive_path);

    // 流式写入
    use futures_util::StreamExt;
    let mut stream = resp.bytes_stream();
    let mut file = tokio::fs::File::create(&part_path)
        .await
        .map_err(|e| format!("创建文件失败: {}", e))?;

    let mut downloaded: u64 = 0;
    let mut last_pct: u64 = 0;
    let start_time = std::time::Instant::now();

    // 下载循环结果 — 任何错误（含取消）统一在循环外清理 .part 残件
    let mut result: Result<(), String> = Ok(());

    while let Some(chunk) = stream.next().await {
        if is_cancelled() {
            result = Err("已取消".into());
            break;
        }

        let bytes = match chunk {
            Ok(b) => b,
            Err(e) => {
                result = Err(format!("下载错误: {}", e));
                break;
            }
        };
        use tokio::io::AsyncWriteExt;
        if let Err(e) = file.write_all(&bytes).await {
            result = Err(format!("写入失败: {}", e));
            break;
        }

        downloaded += bytes.len() as u64;
        let pct = if total_size > 0 {
            downloaded * 100 / total_size
        } else {
            0
        };
        if pct != last_pct {
            last_pct = pct;
            let mb = downloaded as f64 / 1024.0 / 1024.0;
            let total_mb = total_size as f64 / 1024.0 / 1024.0;
            let elapsed = start_time.elapsed().as_secs_f64();
            let avg_speed = if elapsed > 0.0 {
                downloaded as f64 / elapsed / 1_048_576.0
            } else {
                0.0
            };
            let _ = app.emit(
                DOWNLOAD_EVENT,
                PythonDownloadProgress {
                    filename: "python".into(),
                    downloaded,
                    total: total_size,
                    percent: pct as f32,
                    speed_mbps: avg_speed,
                    status: "downloading".to_string(),
                    message: format!(
                        "Python — {:.1}/{:.1} MB ({:.1} MB/s)",
                        mb, total_mb, avg_speed
                    ),
                },
            );
        }
    }

    // 确保缓冲数据全部落盘
    if result.is_ok() {
        use tokio::io::AsyncWriteExt;
        if let Err(e) = file.flush().await {
            result = Err(format!("写入失败: {}", e));
        }
    }
    drop(file);

    // 任何错误路径（包括取消）都删除 .part 残件
    if let Err(e) = result {
        let _ = tokio::fs::remove_file(&part_path).await;
        return Err(e);
    }

    // 校验字节数并原子替换到最终文件
    super::finalize_part_file(&part_path, &archive_path, downloaded, total_size)?;

    // 通知前端下载完成，清除日志中的进度条
    let _ = app.emit(
        DOWNLOAD_EVENT,
        PythonDownloadProgress {
            filename: "python".into(),
            downloaded: total_size,
            total: total_size,
            percent: 100.0,
            speed_mbps: 0.0,
            status: "done".to_string(),
            message: "Python 下载完成".to_string(),
        },
    );
    emit_progress(app, "@pythonEnv.extracting", "info");

    // 解压到临时目录
    let extract_tmp = env_dir.join("_python_extract_tmp");
    if extract_tmp.exists() {
        let _ = std::fs::remove_dir_all(&extract_tmp);
    }
    std::fs::create_dir_all(&extract_tmp).map_err(|e| format!("创建临时目录失败: {}", e))?;

    let archive_path_clone = archive_path.clone();
    let extract_tmp_clone = extract_tmp.clone();
    tokio::task::spawn_blocking(move || extract_tar_gz(&archive_path_clone, &extract_tmp_clone))
        .await
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
    std::fs::create_dir_all(&python_parent).map_err(|e| format!("创建 python 目录失败: {}", e))?;

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

    emit_progress(app, "@pythonEnv.downloadDone", "success");
    Ok(())
}

/// 解压 tar.gz
fn extract_tar_gz(archive: &std::path::Path, dest: &std::path::Path) -> Result<(), String> {
    use std::process::Command;

    let mut cmd = Command::new("tar");
    cmd.args([
        "xzf",
        &archive.to_string_lossy(),
        "-C",
        &dest.to_string_lossy(),
    ]);

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
    cmd.args(["-m", "venv", &venv_dir.to_string_lossy()])
        .env("PYTHONIOENCODING", "utf-8");

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

    emit_progress(app, "@pythonEnv.venvCreated", "success");
    Ok(())
}

/// 安装基础依赖。
///
/// 统一装 CPU 版 onnxruntime，确保任何环境（AMD/Intel/无独显/缺 cuDNN）
/// 都能正常运行。onnxruntime-gpu 在缺少 CUDA 库时 import 会直接报错，
/// 不适合作为默认依赖。GPU 升级由 ensure_onnx_gpu_runtime 在检测到
/// NVIDIA 环境后单独处理。
fn install_deps(app: &tauri::AppHandle) -> Result<(), String> {
    let python = get_venv_python();
    let python_str = python.to_string_lossy().to_string();
    // 固定版本，避免供应链风险
    pip_install_with_python(app, &python_str, &["onnxruntime==1.25.1", "numpy==2.2.6", "pillow==11.3.0"])
}

/// 修复历史环境：把旧版遗留的 CPU-only onnxruntime 换成 onnxruntime-gpu。
///
/// 老版本（<= v0.3.22）的基础依赖装的是 `onnxruntime`（CPU-only），
/// 现在基础依赖已改为 `onnxruntime-gpu`，但既有用户的 venv 里仍是旧包。
/// 两个包会争抢同一个 `onnxruntime` 模块名，必须先卸载再装。
#[cfg(not(target_os = "macos"))]
pub fn upgrade_onnxruntime_to_gpu(app: &tauri::AppHandle, python: &str) -> Result<(), String> {
    SETUP_CANCELLED.store(false, Ordering::SeqCst);

    emit_progress(app, "@pythonEnv.installGpu", "info");
    emit_progress(app, "@pythonEnv.uninstallCpu", "info");

    // 两个包共用 onnxruntime 模块名，同时存在会冲突，先一并卸载
    {
        let mut cmd = std::process::Command::new(python);
        cmd.args([
            "-m",
            "pip",
            "uninstall",
            "-y",
            "onnxruntime",
            "onnxruntime-gpu",
        ])
        .env("PYTHONIOENCODING", "utf-8");
        #[cfg(target_os = "windows")]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x08000000);
        }
        let _ = cmd.output();
    }

    pip_install_with_python(app, python, &["onnxruntime-gpu==1.25.1"])
}

/// 本会话是否已尝试过把 CPU-only onnxruntime 升级为 GPU 包（避免反复重装）
#[cfg(not(target_os = "macos"))]
static ORT_UPGRADE_TRIED: AtomicBool = AtomicBool::new(false);

/// 在指定 Python 下执行探测脚本并返回 stdout（隐藏 Windows 控制台窗口）
async fn probe_python(python: &str, script: &'static str) -> Option<String> {
    let p = python.to_string();
    tokio::task::spawn_blocking(move || {
        let mut cmd = std::process::Command::new(&p);
        cmd.args(["-c", script]).env("PYTHONIOENCODING", "utf-8");
        #[cfg(target_os = "windows")]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x08000000);
        }
        cmd.output().ok().and_then(|o| {
            if o.status.success() {
                Some(String::from_utf8_lossy(&o.stdout).trim().to_string())
            } else {
                None
            }
        })
    })
    .await
    .ok()
    .flatten()
}

/// onnxruntime 探测：输出「providers|是否装了 onnxruntime-gpu 包」
const ONNX_PROBE: &str = "\
import importlib.metadata as md
try:
    import onnxruntime as ort
    providers = ','.join(ort.get_available_providers())
except Exception:
    providers = ''
try:
    md.version('onnxruntime-gpu')
    gpu_pkg = '1'
except Exception:
    gpu_pkg = '0'
print(providers + '|' + gpu_pkg)";

/// torch 探测：输出「是否安装|是否 CUDA 构建|cuda 可用|mps 可用」
const TORCH_PROBE: &str = "\
try:
    import torch
except Exception:
    print('0|0|0|0')
else:
    cuda_build = '1' if torch.version.cuda else '0'
    try:
        cuda_ok = '1' if torch.cuda.is_available() else '0'
    except Exception:
        cuda_ok = '0'
    try:
        mps_ok = '1' if (hasattr(torch.backends, 'mps') and torch.backends.mps.is_available()) else '0'
    except Exception:
        mps_ok = '0'
    print('1|' + cuda_build + '|' + cuda_ok + '|' + mps_ok)";

/// 检测机器上是否存在 NVIDIA GPU
#[cfg(target_os = "windows")]
async fn has_nvidia_gpu() -> bool {
    tokio::task::spawn_blocking(|| {
        let mut lines = Vec::new();
        super::tagger::inference::detect_nvidia_env_pub(&mut lines)
    })
    .await
    .unwrap_or(false)
}

/// 统一入口：探测 onnxruntime 的 GPU ExecutionProvider 可用性。
///
/// 不安装 CUDA、不下载 CUDA 运行时，只使用本机既有的 CUDA 环境。
///
/// 基础依赖装的是 CPU 版 onnxruntime（保证任何环境都能跑），因此这里承担
/// 「有 NVIDIA 就换成 onnxruntime-gpu」这一步。无 NVIDIA 的机器（AMD/Intel/
/// 核显）不会触发升级，继续用 CPU 版，避免装上无法加载 CUDA 库的 GPU 包。
/// 每个会话最多尝试升级一次，失败不重复下载。
/// 返回 Ok(true) 表示有 GPU 加速，Ok(false) 表示按 CPU 运行。
pub async fn ensure_onnx_gpu_runtime(
    app: &tauri::AppHandle,
    python: &str,
) -> Result<bool, String> {
    let _ = app;
    let probe = probe_python(python, ONNX_PROBE).await.unwrap_or_default();
    let (providers, gpu_pkg) = probe.split_once('|').unwrap_or(("", "0"));

    // 已有 GPU EP → 直接用
    if providers.contains("CUDAExecutionProvider") || providers.contains("CoreMLExecutionProvider") {
        return Ok(true);
    }

    // 装的是 CPU-only 包（旧版本遗留），且本机确有 NVIDIA GPU → 换成 GPU 包
    #[cfg(not(target_os = "macos"))]
    {
        let is_cpu_only_pkg = gpu_pkg.trim() != "1";
        if is_cpu_only_pkg
            && !ORT_UPGRADE_TRIED.load(Ordering::SeqCst)
            && has_nvidia_gpu().await
        {
            ORT_UPGRADE_TRIED.store(true, Ordering::SeqCst);

            let p = python.to_string();
            let app2 = app.clone();
            tokio::task::spawn_blocking(move || upgrade_onnxruntime_to_gpu(&app2, &p))
                .await
                .map_err(|e| format!("安装线程异常: {}", e))??;

            let probe = probe_python(python, ONNX_PROBE).await.unwrap_or_default();
            let providers = probe.split('|').next().unwrap_or("");
            return Ok(providers.contains("CUDAExecutionProvider"));
        }
    }

    #[cfg(target_os = "macos")]
    let _ = gpu_pkg;

    Ok(false)
}

/// 统一入口：探测 PyTorch 是否可用 GPU（CUDA / MPS）
/// 探测 PyTorch 是否可用 GPU（CUDA / MPS）。
///
/// 如果 torch 已装但是 CPU-only 构建，且本机有 NVIDIA GPU，
/// 则自动换成 CUDA 版本（从 PyTorch 官方 wheel index 安装，体积约 2GB，
/// 仅首次触发一次）。这不是"下载 CUDA"——torch wheel 自带所需的
/// CUDA 运行时库，用户无需另装 CUDA Toolkit。
/// 返回 Ok(true) 表示有 GPU 加速，Ok(false) 表示按 CPU 运行。
pub async fn ensure_torch_gpu_runtime(
    app: &tauri::AppHandle,
    python: &str,
) -> Result<bool, String> {
    let probe = probe_python(python, TORCH_PROBE).await.unwrap_or_default();
    let f: Vec<&str> = probe.split('|').map(|s| s.trim()).collect();
    #[allow(unused_variables)]
    let installed   = f.first().is_some_and(|v| *v == "1");
    #[allow(unused_variables)]
    let cuda_build  = f.get(1).is_some_and(|v| *v == "1");
    let cuda_ok     = f.get(2).is_some_and(|v| *v == "1");
    let mps_ok      = f.get(3).is_some_and(|v| *v == "1");
    let _ = app;

    // GPU 已可用 → 直接用
    if cuda_ok || mps_ok {
        return Ok(true);
    }

    // 已装但是 CPU-only 构建，且本机有 NVIDIA → 换成 CUDA 版
    #[cfg(target_os = "windows")]
    if installed && !cuda_build && !TORCH_UPGRADE_TRIED.load(Ordering::SeqCst) && has_nvidia_gpu().await {
        TORCH_UPGRADE_TRIED.store(true, Ordering::SeqCst);

        // 先卸载 CPU 版（两者文件冲突，必须先移除）
        {
            let p = python.to_string();
            let _ = tokio::task::spawn_blocking(move || {
                let mut cmd = std::process::Command::new(&p);
                cmd.args(["-m", "pip", "uninstall", "-y", "torch", "torchvision"])
                    .env("PYTHONIOENCODING", "utf-8");
                use std::os::windows::process::CommandExt;
                cmd.creation_flags(0x08000000);
                cmd.output()
            }).await;
        }

        // 安装 CUDA 版（torch wheel 自带 CUDA 运行时，不依赖系统 CUDA Toolkit）
        let p = python.to_string();
        let app2 = app.clone();
        tokio::task::spawn_blocking(move || {
            let mut cmd = std::process::Command::new(&p);
            cmd.args([
                "-m", "pip", "install",
                "--disable-pip-version-check", "--no-cache-dir",
                "torch", "torchvision",
                "--index-url", "https://download.pytorch.org/whl/cu121",
            ])
            .env("PYTHONIOENCODING", "utf-8");
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x08000000);
            let output = cmd.output().map_err(|e| format!("安装失败: {}", e))?;
            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                return Err(format!("安装 CUDA 版 torch 失败: {}", stderr));
            }
            let _ = app2; // 进度由 pip 自身输出，这里不额外 emit
            Ok(())
        })
        .await
        .map_err(|e| format!("安装线程异常: {}", e))??;

        // 复检
        let probe = probe_python(python, TORCH_PROBE).await.unwrap_or_default();
        let f: Vec<&str> = probe.split('|').map(|s| s.trim()).collect();
        return Ok(f.get(2).is_some_and(|v| *v == "1"));
    }

    Ok(false)
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

/// 使用指定 Python 执行 pip install（公开供其他模块调用）
pub fn pip_install_with_python(
    app: &tauri::AppHandle,
    python: &str,
    deps: &[&str],
) -> Result<(), String> {
    let total = deps.len();
    for (i, dep) in deps.iter().enumerate() {
        if is_cancelled() {
            return Err("已取消".into());
        }

        let msg = format!("@pythonEnv.installingDep|{}|{}|{}", dep, i + 1, total);

        // Emit download-style progress bar (single inline entry, no log spam)
        let _ = app.emit(
            DOWNLOAD_EVENT,
            PythonDownloadProgress {
                filename: dep.to_string(),
                downloaded: i as u64,
                total: total as u64,
                percent: (i as f32 / total as f32) * 100.0,
                speed_mbps: 0.0,
                status: "downloading".into(),
                message: msg,
            },
        );

        let mut cmd = std::process::Command::new(python);
        cmd.args([
            "-m",
            "pip",
            "install",
            "--disable-pip-version-check",
            "--no-cache-dir",
            dep,
        ])
        .env("PYTHONIOENCODING", "utf-8");

        #[cfg(target_os = "windows")]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x08000000);
        }

        let output = cmd
            .output()
            .map_err(|e| format!("安装 {} 失败: {}", dep, e))?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(format!("安装 {} 失败: {}", dep, stderr));
        }
    }

    // Emit done to clear progress bar
    let _ = app.emit(
        DOWNLOAD_EVENT,
        PythonDownloadProgress {
            filename: String::new(),
            downloaded: total as u64,
            total: total as u64,
            percent: 100.0,
            speed_mbps: 0.0,
            status: "done".into(),
            message: "@pythonEnv.depsInstalled".into(),
        },
    );

    Ok(())
}
