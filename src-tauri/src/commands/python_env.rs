//! Python 环境自动管理
//! 全局共享模块 — 供 tagger、upscale、person_crop 等功能共用
//! 首次使用时自动下载 standalone Python + 安装基础依赖

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::Emitter;

use super::ProgressEvent;

use super::http_download::{self, DownloadError, DownloadProgress};
use super::python_proc::{configure_python_command, hidden_command};

/// 进度事件名（前端监听此事件）
const PROGRESS_EVENT: &str = "python-env-progress";
const DOWNLOAD_EVENT: &str = "python-env-download";

/// 全局取消标志
static SETUP_CANCELLED: AtomicBool = AtomicBool::new(false);

/// 全局 setup 互斥锁 — 串行化整个环境安装流程，
/// 防止多个功能（tagger/upscale/cluster 等）并发触发 setup 导致两个 pip install 互相踩踏
static SETUP_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// 当前环境部署/升级的发起方（在 SETUP_LOCK 内登记）。
/// 取消按归属隔离：某个功能的取消只中止它自己发起的部署/升级。
static SETUP_OWNER: std::sync::Mutex<Option<&'static str>> = std::sync::Mutex::new(None);

struct SetupOwnerGuard {
    _lock: tokio::sync::MutexGuard<'static, ()>,
}

impl Drop for SetupOwnerGuard {
    fn drop(&mut self) {
        *SETUP_OWNER.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }
}

fn take_pending_cancel(owner: &'static str) -> bool {
    let mut pending = PENDING_CANCELS.lock().unwrap_or_else(|e| e.into_inner());
    let cancelled = pending.contains(&owner);
    pending.retain(|o| *o != owner);
    cancelled
}

async fn acquire_setup(owner: &'static str) -> Result<SetupOwnerGuard, String> {
    let lock = SETUP_LOCK.lock();
    tokio::pin!(lock);
    loop {
        if take_pending_cancel(owner) {
            return Err("已取消".into());
        }
        tokio::select! {
            guard = &mut lock => {
                // 与取消命令共用归属锁，避免登记后复位吞掉刚到的取消。
                let mut current = SETUP_OWNER.lock().unwrap_or_else(|e| e.into_inner());
                if take_pending_cancel(owner) {
                    return Err("已取消".into());
                }
                SETUP_CANCELLED.store(false, Ordering::SeqCst);
                *current = Some(owner);
                return Ok(SetupOwnerGuard { _lock: guard });
            }
            _ = tokio::time::sleep(std::time::Duration::from_millis(50)) => {}
        }
    }
}

async fn run_owned_install(
    guard: SetupOwnerGuard,
    install: impl FnOnce() -> Result<(), String> + Send + 'static,
) -> Result<(), String> {
    // blocking 任务不能被 abort；锁随实际安装任务持有，不能随等待它的 future 提前释放。
    tokio::task::spawn_blocking(move || {
        let _guard = guard;
        if is_cancelled() {
            return Err("已取消".into());
        }
        let result = install();
        if is_cancelled() {
            Err("已取消".into())
        } else {
            result
        }
    })
    .await
    .map_err(|e| format!("安装线程异常: {}", e))?
}

/// 保存排队中或安装阶段之间的取消，避免后续安装重新复位取消状态。
static PENDING_CANCELS: std::sync::Mutex<Vec<&'static str>> = std::sync::Mutex::new(Vec::new());

/// 仅当 owner 正是当前部署/升级的发起方时才置取消标志；
/// owner 尚未持锁时记入 PENDING_CANCELS，由等待方消费。
pub fn cancel_setup_for(owner: &'static str) {
    let owned = SETUP_OWNER.lock().unwrap_or_else(|e| e.into_inner());
    if *owned == Some(owner) {
        SETUP_CANCELLED.store(true, Ordering::SeqCst);
    } else {
        let mut pending = PENDING_CANCELS.lock().unwrap_or_else(|e| e.into_inner());
        if !pending.contains(&owner) {
            pending.push(owner);
        }
    }
}

fn is_cancelled() -> bool {
    SETUP_CANCELLED.load(Ordering::SeqCst)
}

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
const PY_TRIPLE: &str = "aarch64-apple-darwin";
#[cfg(all(target_os = "macos", target_arch = "x86_64"))]
const PY_TRIPLE: &str = "x86_64-apple-darwin";
#[cfg(all(target_os = "windows", target_arch = "x86_64"))]
const PY_TRIPLE: &str = "x86_64-pc-windows-msvc";
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
const PY_TRIPLE: &str = "x86_64-unknown-linux-gnu";
#[cfg(all(target_os = "linux", target_arch = "aarch64"))]
const PY_TRIPLE: &str = "aarch64-unknown-linux-gnu";

fn python_download_url() -> String {
    const RELEASE: &str = "20260414";
    const VERSION: &str = "3.12.13";
    format!("https://github.com/astral-sh/python-build-standalone/releases/download/{RELEASE}/cpython-{VERSION}+{RELEASE}-{PY_TRIPLE}-install_only_stripped.tar.gz")
}

fn get_env_dir() -> PathBuf {
    super::config_paths::app_data_root().join("env")
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
pub(crate) fn get_venv_python() -> PathBuf {
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
    let mut cmd = hidden_command(&python);
    cmd.args(["-c", "import onnxruntime"]);
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
    setup_python_env(&app, "manual").await
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

    let mut cmd = hidden_command(&python);
    cmd.args(["--version"]);

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
        ProgressEvent::new(status, message.to_string()),
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
/// 返回解释器路径
fn detect_system_python() -> Option<String> {
    let candidates = if cfg!(target_os = "windows") {
        vec!["python3", "python", "py"]
    } else {
        vec!["python3", "python"]
    };

    for name in &candidates {
        let mut cmd = hidden_command(name);
        cmd.args(["--version"]);
        if let Ok(output) = cmd.output() {
            if output.status.success() {
                let ver_output = String::from_utf8_lossy(&output.stdout).to_string()
                    + String::from_utf8_lossy(&output.stderr).as_ref();
                if ver_output.contains("Python 3") {
                    let version = ver_output.trim().to_string();
                    // 低于 3.10 的解释器不参与候选。
                    if let Some(minor) = parse_python_minor(&version) {
                        if minor < MIN_PYTHON_MINOR {
                            continue; // 版本太旧，跳过
                        }
                    }
                    // 确认解释器可以导入 venv 模块。
                    let mut test = hidden_command(name);
                    test.args(["-c", "import venv"]);
                    if test.output().map(|o| o.status.success()).unwrap_or(false) {
                        return Some(resolve_python_path(name));
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
                let mut cmd = hidden_command(p);
                cmd.args(["--version"]);
                let version = cmd
                    .output()
                    .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
                    .unwrap_or_else(|_| "Python 3".to_string());
                if let Some(minor) = parse_python_minor(&version) {
                    if minor < MIN_PYTHON_MINOR {
                        continue;
                    }
                }
                return Some(p.to_string());
            }
        }
    }

    None
}

/// 解析 Python 命令的实际可执行文件路径
fn resolve_python_path(name: &str) -> String {
    #[cfg(target_os = "windows")]
    {
        let mut cmd = hidden_command("where");
        cmd.arg(name);
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
        let mut cmd = hidden_command("which");
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
pub async fn setup_python_env(
    app: &tauri::AppHandle,
    owner: &'static str,
) -> Result<String, String> {
    // 清掉同一 owner 的旧取消请求，避免影响新任务。
    PENDING_CANCELS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .retain(|o| *o != owner);
    let _owner = acquire_setup(owner).await?;
    setup_python_env_inner(app).await
}

/// setup 实际逻辑（调用方必须已持有 SETUP_LOCK）
async fn setup_python_env_inner(app: &tauri::AppHandle) -> Result<String, String> {
    let venv_python = get_venv_python();

    // 1. 如果 venv 已就绪，直接返回（is_ready 内部会运行子进程，放入 blocking 线程）
    if tokio::task::spawn_blocking(is_ready).await.unwrap_or(false) {
        return Ok(venv_python.to_string_lossy().to_string());
    }

    let detected = tokio::task::spawn_blocking(detect_system_python)
        .await
        .map_err(|e| format!("检测线程异常: {}", e))?;
    if is_cancelled() {
        return Err("已取消".into());
    }
    if let Some(python) = detected {
        match create_venv(app, PathBuf::from(python)).await {
            Ok(()) => return finish_setup(app).await,
            Err(e) => {
                if is_cancelled() {
                    return Err("已取消".into());
                }
                emit_progress(
                    app,
                    &format!(
                        "@pythonEnv.venvFailed|{}",
                        e.chars().take(100).collect::<String>()
                    ),
                    "info",
                );
            }
        }
    }
    setup_with_standalone(app).await
}

async fn setup_with_standalone(app: &tauri::AppHandle) -> Result<String, String> {
    let python = get_standalone_python();
    if !python.exists() {
        download_python(app).await?;
    }
    if is_cancelled() {
        return Err("已取消".into());
    }
    create_venv(app, python).await?;
    finish_setup(app).await
}

async fn finish_setup(app: &tauri::AppHandle) -> Result<String, String> {
    if is_cancelled() {
        return Err("已取消".into());
    }
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
    Ok(get_venv_python().to_string_lossy().to_string())
}

/// 下载 standalone Python
async fn download_python(app: &tauri::AppHandle) -> Result<(), String> {
    let url = python_download_url();
    let python_dir = get_python_dir();
    let env_dir = get_env_dir();

    if !env_dir.exists() {
        std::fs::create_dir_all(&env_dir).map_err(|e| format!("创建 env 目录失败: {}", e))?;
    }

    emit_progress(
        app,
        &format!(
            "@pythonEnv.downloading|{}",
            url.split('/').next_back().unwrap_or("python")
        ),
        "info",
    );

    let client = http_download::download_client()?;
    let archive_path = env_dir.join("python_download.tar.gz");
    let downloaded = http_download::download_to_file(
        client.get(&url),
        &archive_path,
        "Python",
        &SETUP_CANCELLED,
        |p| {
            let _ = app.emit(DOWNLOAD_EVENT, p.with_filename("python"));
        },
    )
    .await;
    if let Err(err) = downloaded {
        let progress = match &err {
            DownloadError::Cancelled => DownloadProgress::cancelled("已取消"),
            _ => DownloadProgress::error(err.to_string()),
        };
        let _ = app.emit(DOWNLOAD_EVENT, progress.with_filename("python"));
        return Err(match err {
            DownloadError::Cancelled => "已取消".into(),
            other => other.into(),
        });
    }
    let _ = app.emit(
        DOWNLOAD_EVENT,
        DownloadProgress::done("Python 下载完成").with_filename("python"),
    );
    if is_cancelled() {
        return Err("已取消".into());
    }
    emit_progress(app, "@pythonEnv.extracting", "info");

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

    let _ = tokio::fs::remove_file(&archive_path).await;

    // 移动: _python_extract_tmp/python/ → env/python/base/
    let extracted_python = extract_tmp.join("python");
    if !extracted_python.exists() {
        let _ = std::fs::remove_dir_all(&extract_tmp);
        return Err("解压后未找到 python 目录".into());
    }

    let python_parent = get_env_dir().join("python");
    std::fs::create_dir_all(&python_parent).map_err(|e| format!("创建 python 目录失败: {}", e))?;

    if python_dir.exists() {
        let _ = std::fs::remove_dir_all(&python_dir);
    }

    std::fs::rename(&extracted_python, &python_dir)
        .map_err(|e| format!("移动 Python 目录失败: {}", e))?;

    let _ = std::fs::remove_dir_all(&extract_tmp);

    let python_exe = get_standalone_python();
    if !python_exe.exists() {
        return Err(format!("Python 解压后未找到: {}", python_exe.display()));
    }

    emit_progress(app, "@pythonEnv.downloadDone", "success");
    Ok(())
}

/// 解压 tar.gz
fn extract_tar_gz(archive: &std::path::Path, dest: &std::path::Path) -> Result<(), String> {
    let mut cmd = hidden_command("tar");
    cmd.args([
        "xzf",
        &archive.to_string_lossy(),
        "-C",
        &dest.to_string_lossy(),
    ]);

    let status = cmd.status().map_err(|e| format!("解压失败: {}", e))?;
    if !status.success() {
        return Err("解压 Python 失败".into());
    }
    Ok(())
}

async fn create_venv(app: &tauri::AppHandle, python: PathBuf) -> Result<(), String> {
    let venv_dir = get_venv_dir();
    // 未就绪的 venv 可能指向已卸载的解释器，重建前移除残留。
    if venv_dir.exists() {
        std::fs::remove_dir_all(&venv_dir).map_err(|e| format!("清理 venv 失败: {}", e))?;
    }
    std::fs::create_dir_all(get_env_dir().join("python"))
        .map_err(|e| format!("创建目录失败: {}", e))?;
    tokio::task::spawn_blocking(move || {
        let output = hidden_command(python)
            .args(["-m", "venv", &venv_dir.to_string_lossy()])
            .env("PYTHONIOENCODING", "utf-8")
            .output()
            .map_err(|e| format!("创建 venv 失败: {}", e))?;
        if !output.status.success() {
            return Err(format!(
                "创建 venv 失败: {}",
                String::from_utf8_lossy(&output.stderr)
            ));
        }
        Ok(())
    })
    .await
    .map_err(|e| format!("创建 venv 线程异常: {}", e))??;
    emit_progress(app, "@pythonEnv.venvCreated", "success");
    Ok(())
}

fn run_pip(python: &str, args: &[&str], with_proxy: bool) -> std::io::Result<std::process::Output> {
    let mut cmd = hidden_command(python);
    cmd.args(["-m", "pip"])
        .args(args)
        .env("PYTHONIOENCODING", "utf-8");
    if with_proxy {
        super::proxy_config::apply_proxy_env(&mut cmd);
    }
    cancellable_output(cmd, &SETUP_CANCELLED)
}

fn cancellable_output(
    mut cmd: std::process::Command,
    cancelled: &AtomicBool,
) -> std::io::Result<std::process::Output> {
    use std::io::Read;
    use std::process::Stdio;

    let cancelled_error = || std::io::Error::new(std::io::ErrorKind::Interrupted, "已取消");
    if cancelled.load(Ordering::SeqCst) {
        return Err(cancelled_error());
    }
    configure_python_command(&mut cmd, false);
    let mut child = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let stdout = child.stdout.take().expect("piped stdout");
    let stderr = child.stderr.take().expect("piped stderr");
    std::thread::scope(|scope| {
        fn read(mut stream: impl Read) -> std::io::Result<Vec<u8>> {
            let mut bytes = Vec::new();
            stream.read_to_end(&mut bytes)?;
            Ok(bytes)
        }
        let stdout = scope.spawn(move || read(stdout));
        let stderr = scope.spawn(move || read(stderr));
        let mut status = None;
        let result = loop {
            if cancelled.load(Ordering::SeqCst) {
                break Err(cancelled_error());
            }
            if status.is_none() {
                match child.try_wait() {
                    Ok(value) => status = value,
                    Err(e) => break Err(e),
                }
            }
            if let Some(status) = status {
                if stdout.is_finished() && stderr.is_finished() {
                    break Ok(status);
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        };
        if result.is_err() {
            super::kill_process_tree(child.id());
            let _ = child.kill();
            let _ = child.wait();
        }
        let stdout = stdout
            .join()
            .map_err(|_| std::io::Error::other("读取 stdout 线程异常"))?;
        let stderr = stderr
            .join()
            .map_err(|_| std::io::Error::other("读取 stderr 线程异常"))?;
        Ok(std::process::Output {
            status: result?,
            stdout: stdout?,
            stderr: stderr?,
        })
    })
}

/// onnxruntime / onnxruntime-gpu 共用的固定版本
const ORT_VERSION: &str = "1.25.1";

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
    pip_install_with_python(
        app,
        &python_str,
        &[
            &format!("onnxruntime=={}", ORT_VERSION),
            "numpy==2.2.6",
            "pillow==11.3.0",
        ],
    )
}

/// 把 CPU-only onnxruntime 换成 onnxruntime-gpu，由 `ensure_onnx_gpu_runtime`
/// 在检测到 NVIDIA GPU 后调用。
/// 两个包会争抢同一个 `onnxruntime` 模块名，必须先卸载再装。
#[cfg(target_os = "windows")]
pub fn upgrade_onnxruntime_to_gpu(app: &tauri::AppHandle, python: &str) -> Result<(), String> {
    // 取消标志的复位在 ensure_onnx_gpu_runtime 的归属登记段完成（这里复位会吞掉别人的取消）

    emit_progress(app, "@pythonEnv.uninstallCpu", "info");
    let _ = run_pip(
        python,
        &["uninstall", "-y", "onnxruntime", "onnxruntime-gpu"],
        false,
    );
    if is_cancelled() {
        return Err("已取消".into());
    }
    emit_progress(app, "@pythonEnv.installGpu", "info");

    pip_install_with_python(app, python, &[&format!("onnxruntime-gpu=={}", ORT_VERSION)])
}

/// 本会话是否已尝试过把 CPU-only onnxruntime 升级为 GPU 包（避免反复重装）
#[cfg(target_os = "windows")]
static ORT_UPGRADE_TRIED: AtomicBool = AtomicBool::new(false);

/// 本会话是否已尝试过把 CPU-only torch 升级为 CUDA 构建（避免反复下载 ~2GB）
static TORCH_UPGRADE_TRIED: AtomicBool = AtomicBool::new(false);

/// 在指定 Python 下执行探测脚本并返回 stdout（隐藏 Windows 控制台窗口）
pub(crate) async fn probe_python(python: &str, script: &'static str) -> Option<String> {
    let p = python.to_string();
    tokio::task::spawn_blocking(move || {
        let mut cmd = hidden_command(&p);
        cmd.args(["-c", script]).env("PYTHONIOENCODING", "utf-8");
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
#[cfg(target_os = "windows")]
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

#[derive(Debug, PartialEq)]
struct TorchPackages {
    torch: bool,
    torchvision: bool,
    cuda_build: bool,
}

impl TorchPackages {
    fn parse(probe: &str) -> Result<Self, String> {
        let fields: Vec<_> = probe
            .lines()
            .last()
            .unwrap_or_default()
            .split('|')
            .collect();
        if fields.len() != 3 || fields.iter().any(|f| !matches!(*f, "0" | "1")) {
            return Err("PyTorch 依赖检测返回了无效结果".into());
        }
        Ok(Self {
            torch: fields[0] == "1",
            torchvision: fields[1] == "1",
            cuda_build: fields[2] == "1",
        })
    }
}

#[derive(Debug, PartialEq)]
struct TorchInstallPlan {
    packages: Vec<&'static str>,
    cuda: bool,
    replace_cpu: bool,
}

impl TorchInstallPlan {
    fn args(&self) -> Vec<&'static str> {
        let mut args = vec!["install", "--disable-pip-version-check", "--no-cache-dir"];
        if self.replace_cpu {
            args.push("--force-reinstall");
        }
        args.extend_from_slice(&self.packages);
        if self.cuda {
            args.extend(["--index-url", "https://download.pytorch.org/whl/cu121"]);
        }
        args
    }
}

fn torch_install_plan(
    installed: &TorchPackages,
    windows_nvidia: bool,
    upgrade_tried: bool,
) -> Option<TorchInstallPlan> {
    let missing = !installed.torch || !installed.torchvision;
    let replace_cpu = windows_nvidia && installed.torch && !installed.cuda_build;
    if !missing && (!replace_cpu || upgrade_tried) {
        return None;
    }
    let packages = if windows_nvidia {
        vec!["torch", "torchvision"]
    } else {
        [
            (installed.torch, "torch"),
            (installed.torchvision, "torchvision"),
        ]
        .into_iter()
        .filter_map(|(present, package)| (!present).then_some(package))
        .collect()
    };
    Some(TorchInstallPlan {
        packages,
        cuda: windows_nvidia,
        replace_cpu,
    })
}

// 只检查包和构建类型；不初始化 CUDA/MPS，非 Windows 不触发 GPU 换装。
const TORCH_PROBE: &str = "\
try:
    import torch
    torch_ok = True
    cuda_build = bool(torch.version.cuda)
except Exception:
    torch_ok = cuda_build = False
try:
    import torchvision
    vision_ok = True
except Exception:
    vision_ok = False
print('|'.join('1' if flag else '0' for flag in (torch_ok, vision_ok, cuda_build)))";

/// 检测机器上是否存在 NVIDIA GPU（nvidia-smi 探测）。
#[cfg(target_os = "windows")]
async fn has_nvidia_gpu() -> bool {
    static NVIDIA: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    tokio::task::spawn_blocking(|| {
        *NVIDIA.get_or_init(|| {
            // Windows GUI 进程的 PATH 未必包含 nvidia-smi，逐个候选路径尝试
            let candidates: &[&str] = if cfg!(target_os = "windows") {
                &[
                    "nvidia-smi",
                    "C:\\Windows\\System32\\nvidia-smi.exe",
                    "C:\\Program Files\\NVIDIA Corporation\\NVSMI\\nvidia-smi.exe",
                ]
            } else {
                &["nvidia-smi"]
            };
            for exe in candidates {
                let mut cmd = hidden_command(exe);
                cmd.arg("-L");
                if let Ok(out) = cmd.output() {
                    if out.status.success() {
                        return true;
                    }
                }
            }
            false
        })
    })
    .await
    .unwrap_or(false)
}

/// 统一入口：探测 onnxruntime 的 GPU ExecutionProvider 可用性，
/// 本机有 NVIDIA GPU 而装的是 CPU 版时换成 onnxruntime-gpu。
///
/// 不安装 CUDA、不下载 CUDA 运行时，只使用本机既有的 CUDA 环境。
/// 无 NVIDIA 的机器（AMD/Intel/核显）不会触发升级，继续用 CPU 版
/// （为什么默认装 CPU 版见 install_deps）。
/// 每个会话最多尝试升级一次，失败不重复下载。
#[cfg(target_os = "windows")]
pub async fn ensure_onnx_gpu_runtime(
    app: &tauri::AppHandle,
    python: &str,
    owner: &'static str,
) -> Result<(), String> {
    if ORT_UPGRADE_TRIED.load(Ordering::SeqCst) {
        return Ok(());
    }
    let probe = probe_python(python, ONNX_PROBE).await.unwrap_or_default();
    let (providers, gpu_pkg) = probe.split_once('|').unwrap_or(("", "0"));

    // 已有 GPU EP → 直接用
    if providers.contains("CUDAExecutionProvider") || providers.contains("CoreMLExecutionProvider")
    {
        return Ok(());
    }

    // 装的是 CPU-only 包（install_deps 的默认），且本机确有 NVIDIA GPU → 换成 GPU 包
    #[cfg(target_os = "windows")]
    {
        let is_cpu_only_pkg = gpu_pkg.trim() != "1";
        if is_cpu_only_pkg && !ORT_UPGRADE_TRIED.load(Ordering::SeqCst) && has_nvidia_gpu().await {
            // 持 SETUP_LOCK：pip 换装期间 import 探测会失败，若放任 setup_python_env
            // 并发进来会把正在写入的 venv 整个删除重建，环境半 CPU 半 GPU 或彻底损坏。
            // 锁内 CAS：并发调用只有一个执行升级，其余等锁释放（升级结束）后直接返回。
            let guard = acquire_setup(owner).await?;
            if ORT_UPGRADE_TRIED
                .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
                .is_ok()
            {
                let p = python.to_string();
                let app2 = app.clone();
                run_owned_install(guard, move || upgrade_onnxruntime_to_gpu(&app2, &p)).await?;
            }
        }
    }

    Ok(())
}

/// 安装缺失的 PyTorch 依赖；仅 Windows NVIDIA 环境会选择 CUDA wheel。
pub async fn ensure_torch_gpu_runtime<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    python: &str,
    owner: &'static str,
) -> Result<(), String> {
    let guard = acquire_setup(owner).await?;
    let probe = probe_python(python, TORCH_PROBE)
        .await
        .ok_or("PyTorch 依赖检测失败")?;
    let installed = TorchPackages::parse(&probe)?;
    #[cfg(target_os = "windows")]
    let windows_nvidia = has_nvidia_gpu().await;
    #[cfg(not(target_os = "windows"))]
    let windows_nvidia = false;

    let Some(plan) = torch_install_plan(
        &installed,
        windows_nvidia,
        TORCH_UPGRADE_TRIED.load(Ordering::SeqCst),
    ) else {
        return if is_cancelled() {
            Err("已取消".into())
        } else {
            Ok(())
        };
    };
    let python = python.to_string();
    let app = app.clone();
    run_owned_install(guard, move || {
        if plan.replace_cpu {
            TORCH_UPGRADE_TRIED.store(true, Ordering::SeqCst);
        }
        let label = plan.packages.join(", ");
        emit_pip_progress(&app, &label, 0, 1);
        let result = pip_result(run_pip(&python, &plan.args(), true), &label);
        finish_pip_progress(&app, &result);
        result
    })
    .await
}

/// 额外依赖与基础环境部署共用同一把锁，取消归属于调用方。
pub(crate) async fn pip_install_for<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    python: &str,
    deps: &[&str],
    owner: &'static str,
) -> Result<(), String> {
    let guard = acquire_setup(owner).await?;
    let app = app.clone();
    let python = python.to_string();
    let deps: Vec<String> = deps.iter().map(|dep| dep.to_string()).collect();
    run_owned_install(guard, move || {
        let deps: Vec<&str> = deps.iter().map(String::as_str).collect();
        pip_install_with_python(&app, &python, &deps)
    })
    .await
}

fn emit_pip_progress<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    dep: &str,
    index: usize,
    total: usize,
) {
    let _ = app.emit(
        DOWNLOAD_EVENT,
        DownloadProgress::new(
            "downloading",
            (index as f32 / total as f32) * 100.0,
            format!("@pythonEnv.installingDep|{}|{}|{}", dep, index + 1, total),
        )
        .with_filename(dep),
    );
}

fn finish_pip_progress<R: tauri::Runtime>(app: &tauri::AppHandle<R>, result: &Result<(), String>) {
    let progress = if is_cancelled() {
        DownloadProgress::cancelled("已取消")
    } else {
        match result {
            Ok(()) => DownloadProgress::done("@pythonEnv.depsInstalled"),
            Err(error) => DownloadProgress::error(error),
        }
    };
    let _ = app.emit(DOWNLOAD_EVENT, progress);
}

fn pip_result(output: std::io::Result<std::process::Output>, label: &str) -> Result<(), String> {
    if is_cancelled() {
        return Err("已取消".into());
    }
    let output = output.map_err(|e| format!("安装 {} 失败: {}", label, e))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(format!(
            "安装 {} 失败: {}",
            label,
            String::from_utf8_lossy(&output.stderr)
        ))
    }
}

/// 调用方必须已持有安装锁并登记 owner；异步调用方使用 pip_install_for。
pub fn pip_install_with_python<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    python: &str,
    deps: &[&str],
) -> Result<(), String> {
    let result = (|| {
        for (i, dep) in deps.iter().enumerate() {
            if is_cancelled() {
                return Err("已取消".into());
            }
            emit_pip_progress(app, dep, i, deps.len());
            pip_result(
                run_pip(
                    python,
                    &[
                        "install",
                        "--disable-pip-version-check",
                        "--no-cache-dir",
                        dep,
                    ],
                    true,
                ),
                dep,
            )?;
        }
        Ok(())
    })();
    finish_pip_progress(app, &result);
    result
}

#[cfg(not(target_os = "windows"))]
pub async fn ensure_onnx_gpu_runtime(
    _app: &tauri::AppHandle,
    _python: &str,
    _owner: &'static str,
) -> Result<(), String> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    static TEST_SETUP_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

    #[cfg(unix)]
    struct TestDir(PathBuf);

    #[cfg(unix)]
    impl TestDir {
        fn new() -> Self {
            static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!(
                "purinbox-python-env-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::SeqCst)
            ));
            std::fs::create_dir(&path).unwrap();
            Self(path)
        }

        fn path(&self) -> &std::path::Path {
            &self.0
        }
    }

    #[cfg(unix)]
    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn packages(torch: bool, torchvision: bool, cuda_build: bool) -> TorchPackages {
        TorchPackages {
            torch,
            torchvision,
            cuda_build,
        }
    }

    #[test]
    fn torch_probe_requires_three_boolean_fields() {
        assert_eq!(
            TorchPackages::parse("log\n1|0|1\n").unwrap(),
            packages(true, false, true)
        );
        for invalid in ["", "1|1", "1|1|0|0", "True|1|0"] {
            assert!(TorchPackages::parse(invalid).is_err());
        }
    }

    #[test]
    fn torch_probe_uses_stub_modules_without_initializing_gpu() {
        let python = get_venv_python();
        let python = if python.exists() {
            python
        } else {
            PathBuf::from("python3")
        };
        for (torch, vision, cuda, expected) in [
            (false, false, false, "0|0|0"),
            (true, false, false, "1|0|0"),
            (true, true, false, "1|1|0"),
            (true, true, true, "1|1|1"),
        ] {
            let script = format!(
                "import sys, types\nsys.modules['torch'] = {}\nsys.modules['torchvision'] = {}\n{}",
                if torch {
                    format!(
                        "types.SimpleNamespace(version=types.SimpleNamespace(cuda={}))",
                        if cuda { "'12.1'" } else { "None" }
                    )
                } else {
                    "None".into()
                },
                if vision {
                    "types.SimpleNamespace()"
                } else {
                    "None"
                },
                TORCH_PROBE,
            );
            let mut cmd = hidden_command(&python);
            cmd.args(["-I", "-B", "-c", &script]);
            let output = cancellable_output(cmd, &AtomicBool::new(false)).unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), expected);
        }
    }

    #[test]
    fn non_windows_only_installs_missing_packages() {
        for cuda in [false, true] {
            for tried in [false, true] {
                assert!(torch_install_plan(&packages(true, true, cuda), false, tried).is_none());
                for (torch, vision, expected) in [
                    (false, false, vec!["torch", "torchvision"]),
                    (false, true, vec!["torch"]),
                    (true, false, vec!["torchvision"]),
                ] {
                    let plan =
                        torch_install_plan(&packages(torch, vision, cuda), false, tried).unwrap();
                    assert_eq!(plan.packages, expected);
                    assert!(!plan.cuda);
                    assert!(!plan.replace_cpu);
                    assert!(!plan.args().contains(&"--index-url"));
                    assert!(!plan.args().contains(&"--upgrade"));
                }
            }
        }
    }

    #[test]
    fn windows_nvidia_installs_cuda_pair_directly() {
        let plan = torch_install_plan(&packages(false, false, false), true, false).unwrap();
        assert_eq!(
            plan.args(),
            [
                "install",
                "--disable-pip-version-check",
                "--no-cache-dir",
                "torch",
                "torchvision",
                "--index-url",
                "https://download.pytorch.org/whl/cu121",
            ]
        );
        assert!(!plan.replace_cpu);
    }

    #[test]
    fn windows_cpu_upgrade_is_one_install_and_only_one_attempt() {
        let cpu = packages(true, true, false);
        let plan = torch_install_plan(&cpu, true, false).unwrap();
        assert!(plan.replace_cpu);
        assert!(plan.args().contains(&"--force-reinstall"));
        assert_eq!(
            plan.args().iter().filter(|arg| **arg == "install").count(),
            1
        );
        assert!(!plan.args().contains(&"uninstall"));
        assert!(torch_install_plan(&cpu, true, true).is_none());
        assert!(torch_install_plan(&packages(true, true, true), true, false).is_none());
    }

    #[test]
    fn missing_vision_and_failed_first_install_remain_repairable() {
        for state in [packages(false, false, false), packages(true, false, true)] {
            let plan = torch_install_plan(&state, true, true).unwrap();
            assert!(plan.cuda);
            assert_eq!(plan.packages, ["torch", "torchvision"]);
        }
    }

    #[tokio::test]
    async fn owner_cancellation_isolated_and_queued_cancel_returns_without_lock() {
        let _serial = TEST_SETUP_LOCK.lock().await;
        let guard = acquire_setup("test-active").await.unwrap();
        cancel_setup_for("test-queued");
        cancel_setup_for("test-queued");
        assert!(!is_cancelled());
        let queued = tokio::time::timeout(
            std::time::Duration::from_secs(2),
            acquire_setup("test-queued"),
        )
        .await
        .unwrap();
        assert!(matches!(queued, Err(e) if e == "已取消"));
        assert_eq!(*SETUP_OWNER.lock().unwrap(), Some("test-active"));
        cancel_setup_for("test-active");
        assert!(is_cancelled());
        drop(guard);
        let _next = acquire_setup("test-next").await.unwrap();
        assert!(!is_cancelled());
    }

    #[tokio::test]
    async fn cancellation_between_setup_and_extra_install_is_not_cleared() {
        let _serial = TEST_SETUP_LOCK.lock().await;
        drop(acquire_setup("test-between").await.unwrap());
        cancel_setup_for("test-between");
        assert!(matches!(acquire_setup("test-between").await, Err(e) if e == "已取消"));
        assert!(acquire_setup("test-between").await.is_ok());
    }

    #[tokio::test]
    async fn aborted_waiter_keeps_lock_until_blocking_install_finishes() {
        let _serial = TEST_SETUP_LOCK.lock().await;
        let guard = acquire_setup("test-abort").await.unwrap();
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let (finish_tx, finish_rx) = std::sync::mpsc::channel();
        let task = tokio::spawn(run_owned_install(guard, move || {
            let _ = started_tx.send(());
            finish_rx
                .recv_timeout(std::time::Duration::from_secs(5))
                .unwrap();
            Ok(())
        }));
        started_rx.await.unwrap();
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        assert!(SETUP_LOCK.try_lock().is_err());
        assert_eq!(*SETUP_OWNER.lock().unwrap(), Some("test-abort"));
        finish_tx.send(()).unwrap();
        let _next = tokio::time::timeout(
            std::time::Duration::from_secs(2),
            acquire_setup("test-after-abort"),
        )
        .await
        .unwrap()
        .unwrap();
    }

    #[cfg(unix)]
    fn mock_python(root: &std::path::Path, probe: &str, body: &str) -> String {
        use std::os::unix::fs::PermissionsExt;
        let executable = root.join("mock-python");
        std::fs::write(&executable, format!(
            "#!/bin/sh\nif [ \"$1\" = '-c' ]; then printf '%s\\n' '{probe}'; exit 0; fi\nprintf '%s\\n' \"$@\" >> '{}/args'\n{body}\n", root.display()
        )).unwrap();
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
        executable.to_string_lossy().into_owned()
    }

    #[cfg(unix)]
    async fn wait_for_file(path: &std::path::Path) {
        tokio::time::timeout(std::time::Duration::from_secs(3), async {
            while !path.exists() {
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn mock_process_drains_both_pipes_and_preserves_invalid_utf8() {
        let mut cmd = hidden_command("/bin/sh");
        cmd.args([
            "-c",
            "head -c 131072 /dev/zero; head -c 131072 /dev/zero >&2; printf '\\377' >&2; exit 7",
        ]);
        let output = cancellable_output(cmd, &AtomicBool::new(false)).unwrap();
        assert_eq!(output.status.code(), Some(7));
        assert_eq!(output.stdout.len(), 131072);
        assert_eq!(output.stderr.len(), 131073);
        assert_eq!(output.stderr.last(), Some(&255));
    }

    #[cfg(unix)]
    #[test]
    fn cancelled_mock_process_is_never_spawned() {
        let root = TestDir::new();
        let marker = root.path().join("started");
        let mut cmd = hidden_command("/bin/sh");
        cmd.args(["-c", &format!("touch '{}'", marker.display())]);
        let error = cancellable_output(cmd, &AtomicBool::new(true)).unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::Interrupted);
        assert!(!marker.exists());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn missing_torch_installs_both_once_without_gpu_upgrade() {
        let _serial = TEST_SETUP_LOCK.lock().await;
        let root = TestDir::new();
        let python = mock_python(root.path(), "0|0|0", "exit 0");
        let app = tauri::test::mock_app();
        let events = super::super::batch::capture_events(app.handle(), DOWNLOAD_EVENT);
        ensure_torch_gpu_runtime(app.handle(), &python, "test-torch")
            .await
            .unwrap();
        let args = std::fs::read_to_string(root.path().join("args")).unwrap();
        assert_eq!(
            args,
            "-m\npip\ninstall\n--disable-pip-version-check\n--no-cache-dir\ntorch\ntorchvision\n"
        );
        assert_eq!(events.lock().unwrap().last().unwrap()["status"], "done");
        assert_eq!(*SETUP_OWNER.lock().unwrap(), None);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn installed_cpu_torch_is_left_unchanged_and_probe_errors_do_not_install() {
        let _serial = TEST_SETUP_LOCK.lock().await;
        let app = tauri::test::mock_app();
        for (probe, success) in [("1|1|0", true), ("bad-probe", false)] {
            let root = TestDir::new();
            let python = mock_python(root.path(), probe, "exit 0");
            assert_eq!(
                ensure_torch_gpu_runtime(app.handle(), &python, "test-torch-noop")
                    .await
                    .is_ok(),
                success
            );
            assert!(!root.path().join("args").exists());
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn extra_pip_is_serialized_and_queued_cancel_never_spawns() {
        let _serial = TEST_SETUP_LOCK.lock().await;
        let root = TestDir::new();
        let python = mock_python(root.path(), "", "exit 0");
        let app = tauri::test::mock_app();
        let guard = acquire_setup("test-lock-holder").await.unwrap();
        let handle = app.handle().clone();
        let task = tokio::spawn(async move {
            pip_install_for(&handle, &python, &["mock-dependency"], "test-pip-queue").await
        });
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        assert!(!root.path().join("args").exists());
        assert!(!task.is_finished());
        cancel_setup_for("test-pip-queue");
        let result = tokio::time::timeout(std::time::Duration::from_secs(2), task)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(result, Err("已取消".into()));
        assert!(!root.path().join("args").exists());
        assert!(!is_cancelled());
        drop(guard);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn extra_pip_cancellation_kills_owned_process_and_stops_next_package() {
        let _serial = TEST_SETUP_LOCK.lock().await;
        let root = TestDir::new();
        let pid_file = root.path().join("pid");
        let python = mock_python(
            root.path(),
            "",
            &format!("echo $$ > '{}'; sleep 30", pid_file.display()),
        );
        let app = tauri::test::mock_app();
        let events = super::super::batch::capture_events(app.handle(), DOWNLOAD_EVENT);
        let handle = app.handle().clone();
        let task = tokio::spawn(async move {
            pip_install_for(&handle, &python, &["first", "second"], "test-pip-running").await
        });
        wait_for_file(&pid_file).await;
        let pid = std::fs::read_to_string(&pid_file)
            .unwrap()
            .trim()
            .to_string();
        assert_eq!(*SETUP_OWNER.lock().unwrap(), Some("test-pip-running"));
        cancel_setup_for("test-pip-running");
        let result = tokio::time::timeout(std::time::Duration::from_secs(3), task)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(result, Err("已取消".into()));
        assert!(!std::fs::read_to_string(root.path().join("args"))
            .unwrap()
            .contains("second"));
        assert!(!hidden_command("kill")
            .args(["-0", &pid])
            .output()
            .unwrap()
            .status
            .success());
        assert_eq!(*SETUP_OWNER.lock().unwrap(), None);
        let events = events.lock().unwrap();
        assert_eq!(events.last().unwrap()["status"], "cancelled");
        assert_eq!(
            events
                .iter()
                .filter(|event| event["status"] == "cancelled")
                .count(),
            1
        );
        assert!(!events.iter().any(|event| event["status"] == "done"));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn extra_pip_failure_preserves_stderr_and_releases_owner() {
        let _serial = TEST_SETUP_LOCK.lock().await;
        let root = TestDir::new();
        let python = mock_python(root.path(), "", "printf 'mock failure' >&2; exit 9");
        let app = tauri::test::mock_app();
        let events = super::super::batch::capture_events(app.handle(), DOWNLOAD_EVENT);
        let error = pip_install_for(
            app.handle(),
            &python,
            &["first", "second"],
            "test-pip-error",
        )
        .await
        .unwrap_err();
        assert!(error.contains("mock failure"));
        assert!(!std::fs::read_to_string(root.path().join("args"))
            .unwrap()
            .contains("second"));
        assert_eq!(*SETUP_OWNER.lock().unwrap(), None);
        assert_eq!(events.lock().unwrap().last().unwrap()["status"], "error");
    }

    #[test]
    fn standalone_url_uses_the_current_platform_triple() {
        let url = python_download_url();
        assert!(url.starts_with(
            "https://github.com/astral-sh/python-build-standalone/releases/download/20260414/"
        ));
        assert!(url.ends_with(&format!("-{PY_TRIPLE}-install_only_stripped.tar.gz")));
        assert!(url.contains("cpython-3.12.13+20260414-"));
    }

    #[test]
    fn python_minor_parsing_rejects_unrelated_output() {
        assert_eq!(parse_python_minor("Python 3.12.13\n"), Some(12));
        assert_eq!(parse_python_minor("Python 3.9.1"), Some(9));
        assert_eq!(parse_python_minor("Python 2.7.18"), None);
        assert_eq!(parse_python_minor("not Python"), None);
    }

    #[tokio::test]
    async fn probe_only_runs_the_requested_script() {
        let python =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../env/python/venv/bin/python3");
        let python = if python.exists() {
            python.to_string_lossy().into_owned()
        } else {
            "python3".into()
        };
        assert_eq!(
            probe_python(&python, "print('probe-ok')").await.as_deref(),
            Some("probe-ok")
        );
        assert!(probe_python(&python, "raise RuntimeError('probe-failed')")
            .await
            .is_none());
    }
}
