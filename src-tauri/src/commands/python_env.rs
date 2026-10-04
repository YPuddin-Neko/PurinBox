//! Python 环境自动管理：打标、超分、人物裁切、美学评分、聚类共用同一个 venv。
//!
//! 首次使用时用受支持的系统 Python（3.11–3.13）或下载的独立版 Python 3.12 建 venv、装基础依赖；
//! x86_64 的 Windows / Linux 上有 NVIDIA GPU 时按需把 onnxruntime 换成 GPU 版，Windows 上按需换装 CUDA 版 torch。
//! 改动 venv 的操作（部署、补装、换装、重置）都持同一把安装锁串行执行，取消按发起方（owner）隔离。

use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Runtime};

use super::http_download::{self, DownloadError, DownloadProgress};
use super::python_proc::{self, PythonCommand};
use super::ProgressEvent;

/// 进度事件名（前端监听此事件）
const PROGRESS_EVENT: &str = "python-env-progress";
const DOWNLOAD_EVENT: &str = "python-env-download";

/// 取消时各步骤返回的错误。前端只把以「已取消」开头的错误当作用户取消
const CANCELLED: &str = "已取消";

/// 当前这次部署/安装的取消标志，只由登记的发起方置位（见 `cancel_setup_for`）
static SETUP_CANCELLED: AtomicBool = AtomicBool::new(false);

/// 全局 setup 互斥锁 — 串行化整个环境安装流程，
/// 防止多个功能（tagger/upscale/cluster 等）并发触发 setup 导致两个 pip install 互相踩踏
static SETUP_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// 当前环境部署/升级的发起方（在 SETUP_LOCK 内登记）。
/// 取消按归属隔离：某个功能的取消只中止它自己发起的部署/升级。
static SETUP_OWNER: std::sync::Mutex<Option<&'static str>> = std::sync::Mutex::new(None);

/// 发起方还没持锁时到达的取消（排队中、两次安装之间），由它下次拿锁时消费
static PENDING_CANCELS: std::sync::Mutex<Vec<&'static str>> = std::sync::Mutex::new(Vec::new());

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

fn has_pending_cancel(owner: &'static str) -> bool {
    PENDING_CANCELS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .contains(&owner)
}

async fn acquire_setup(owner: &'static str) -> Result<SetupOwnerGuard, String> {
    let lock = SETUP_LOCK.lock();
    tokio::pin!(lock);
    loop {
        if take_pending_cancel(owner) {
            return Err(CANCELLED.into());
        }
        tokio::select! {
            guard = &mut lock => {
                // 与取消命令共用归属锁，避免登记后复位吞掉刚到的取消。
                let mut current = SETUP_OWNER.lock().unwrap_or_else(|e| e.into_inner());
                if take_pending_cancel(owner) {
                    return Err(CANCELLED.into());
                }
                SETUP_CANCELLED.store(false, Ordering::SeqCst);
                *current = Some(owner);
                return Ok(SetupOwnerGuard { _lock: guard });
            }
            _ = tokio::time::sleep(Duration::from_millis(50)) => {}
        }
    }
}

/// 持安装锁执行 `work`（建 venv、pip、删除环境），返回它的结果。
///
/// `work` 在单独的任务里运行、锁也交给它：等结果的一方被丢弃（abort、select、timeout）时，
/// 正在跑的 pip 和删除停不下来（阻塞线程无法中止），锁要等它们真正结束才释放，
/// 否则别的部署会趁 pip 还在写 venv 时进来，把 venv 删掉重建。
/// 开始前已取消就不执行；做完时已取消，成功的结果也按取消返回。
async fn run_locked<T: Send + 'static>(
    guard: SetupOwnerGuard,
    work: impl Future<Output = Result<T, String>> + Send + 'static,
) -> Result<T, String> {
    tokio::spawn(async move {
        let _guard = guard;
        if is_cancelled() {
            return Err(CANCELLED.to_string());
        }
        match work.await {
            Ok(_) if is_cancelled() => Err(CANCELLED.to_string()),
            result => result,
        }
    })
    .await
    .map_err(|e| format!("安装任务异常: {}", e))?
}

/// `run_locked` 的同步版本：`work` 在阻塞线程池里运行
async fn run_locked_blocking<T: Send + 'static>(
    guard: SetupOwnerGuard,
    work: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    run_locked(guard, async move { blocking(work).await? }).await
}

/// 在阻塞线程池里运行子进程和文件操作，不占用异步运行时的线程
async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> T + Send + 'static,
) -> Result<T, String> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|e| format!("后台任务异常: {}", e))
}

/// 仅当 owner 正是当前部署/升级的发起方时才置取消标志；
/// owner 尚未持锁时记入 PENDING_CANCELS，由它下次拿锁时消费。
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

/// 清掉 owner 上一轮没用上的待处理取消（取消时环境部署早已结束，取消一直留在 PENDING_CANCELS 里）。
/// 各功能拿到自己的任务锁、开始新一轮时调用一次。环境部署的入口不清：
/// 任务开始后、进入部署之前点的取消要留给部署消费
pub(crate) fn clear_pending_cancel(owner: &str) {
    PENDING_CANCELS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .retain(|o| *o != owner);
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

/// 环境目录：`<env>/python/base` 是独立版解释器，`<env>/python/venv` 是 venv。
/// 部署流程都从它取路径，测试换成临时目录
#[derive(Clone)]
struct EnvLayout {
    env_dir: PathBuf,
}

impl EnvLayout {
    fn current() -> Self {
        EnvLayout {
            env_dir: super::config_paths::app_data_root().join("env"),
        }
    }

    /// 解释器和 venv 所在的目录，重置环境时整个删除
    fn python_root(&self) -> PathBuf {
        self.env_dir.join("python")
    }

    fn base_dir(&self) -> PathBuf {
        self.python_root().join("base")
    }

    fn venv_dir(&self) -> PathBuf {
        self.python_root().join("venv")
    }

    fn venv_python(&self) -> PathBuf {
        let venv = self.venv_dir();
        if cfg!(target_os = "windows") {
            return venv.join("Scripts").join("python.exe");
        }
        let python3 = venv.join("bin").join("python3");
        let python = venv.join("bin").join("python");
        if !python3.exists() && python.exists() {
            python
        } else {
            python3
        }
    }

    fn standalone_python(&self) -> PathBuf {
        if cfg!(target_os = "windows") {
            self.base_dir().join("python.exe")
        } else {
            self.base_dir().join("bin").join("python3")
        }
    }
}

/// venv 中的 python 可执行文件路径（不检查是否就绪）
pub(crate) fn get_venv_python() -> PathBuf {
    EnvLayout::current().venv_python()
}

/// 就绪（能导入 onnxruntime）的 venv 解释器路径
pub fn get_python_exe() -> Option<String> {
    let python = get_venv_python();
    onnxruntime_importable(&python).then(|| python.to_string_lossy().into_owned())
}

/// 重置 Python 环境（删除 venv 和独立版解释器）。持安装锁，不会删到正在安装的环境；
/// 删不掉的文件（Windows 上被运行中的 Python 占用）列在错误里
#[tauri::command]
pub async fn reset_python_env() -> Result<String, String> {
    clear_pending_cancel("manual");
    remove_env(EnvLayout::current(), "manual").await?;
    Ok("Python 环境已重置".to_string())
}

/// 持安装锁删除解释器和 venv。删干净后本会话的换装记录随之作废：新建的环境里又是 CPU 版，
/// 留着「已经试过」的记录，下次 GPU 任务就不会再换装
async fn remove_env(layout: EnvLayout, owner: &'static str) -> Result<(), String> {
    let guard = acquire_setup(owner).await?;
    run_locked_blocking(guard, move || {
        remove_dir_reporting(&layout.python_root())?;
        forget_gpu_upgrades();
        Ok(())
    })
    .await
}

/// 复位本会话的换装记录（onnxruntime GPU 版、CUDA 版 torch）。持安装锁调用，这时没有换装在进行
fn forget_gpu_upgrades() {
    TORCH_UPGRADE_TRIED.store(false, Ordering::SeqCst);
    #[cfg(any(target_os = "windows", target_os = "linux", test))]
    ORT_UPGRADE.reset();
}

/// 手动部署 Python 环境（设置页按钮）
#[tauri::command]
pub async fn deploy_python_env(app: tauri::AppHandle) -> Result<String, String> {
    clear_pending_cancel("manual");
    setup_python_env(&app, "manual").await
}

/// 获取 Python 环境信息（供设置页显示）。要运行解释器，放在阻塞线程里，不卡界面
#[tauri::command]
pub async fn get_python_env_info() -> Result<PythonEnvInfo, String> {
    blocking(python_env_info).await
}

fn python_env_info() -> PythonEnvInfo {
    let Some(path) = get_python_exe() else {
        return PythonEnvInfo {
            available: false,
            version: String::new(),
            path: String::new(),
        };
    };
    let version = PythonCommand::new(&path)
        .arg("--version")
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default();
    PythonEnvInfo {
        available: !version.is_empty(),
        version,
        path,
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PythonEnvInfo {
    pub available: bool,
    pub version: String,
    pub path: String,
}

/// 删除整个目录树，不存在视为成功。std 的 remove_dir_all 遇到第一个删不掉的文件就停下，
/// 这时逐个删除能删的，再把删不掉的列出来
fn remove_dir_reporting(root: &Path) -> Result<(), String> {
    match std::fs::remove_dir_all(root) {
        Ok(()) => return Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(_) => {}
    }
    let mut failed = Vec::new();
    remove_entry(root, &mut failed);
    if failed.is_empty() {
        return Ok(());
    }
    const LISTED: usize = 5;
    let listed: Vec<String> = failed
        .iter()
        .take(LISTED)
        .map(|(path, e)| format!("{}（{}）", path.display(), e))
        .collect();
    let more = if failed.len() > LISTED { " 等" } else { "" };
    Err(format!(
        "{} 个文件无法删除: {}{}",
        failed.len(),
        listed.join("; "),
        more
    ))
}

/// 删除 `path`（目录先删里面的内容），删不掉的记进 `failed`；返回是否已删掉
fn remove_entry(path: &Path, failed: &mut Vec<(PathBuf, std::io::Error)>) -> bool {
    let meta = match std::fs::symlink_metadata(path) {
        Ok(meta) => meta,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return true,
        Err(e) => {
            failed.push((path.to_path_buf(), e));
            return false;
        }
    };
    if meta.is_dir() {
        let entries = match std::fs::read_dir(path) {
            Ok(entries) => entries,
            Err(e) => {
                failed.push((path.to_path_buf(), e));
                return false;
            }
        };
        let mut emptied = true;
        for entry in entries.flatten() {
            emptied &= remove_entry(&entry.path(), failed);
        }
        // 里面留着删不掉的文件时目录本身必然删不掉，文件已经列出，不再列目录
        if !emptied {
            return false;
        }
        return match std::fs::remove_dir(path) {
            Ok(()) => true,
            Err(e) => {
                failed.push((path.to_path_buf(), e));
                false
            }
        };
    }
    // Windows 上指向目录的符号链接要用 remove_dir 删
    match std::fs::remove_file(path).or_else(|e| std::fs::remove_dir(path).map_err(|_| e)) {
        Ok(()) => true,
        Err(e) => {
            failed.push((path.to_path_buf(), e));
            false
        }
    }
}

/// 支持的 Python 3 次版本，取基础依赖两个约束的交集：
/// - onnxruntime / onnxruntime-gpu 1.25.1 要求 Python >= 3.11，没有 3.10 的包；
/// - numpy 2.2.6 只有 3.10–3.13 的预编译包，其他版本要从源码编译，基本装不上
const SUPPORTED_PYTHON_MINORS: std::ops::RangeInclusive<u32> = 11..=13;

/// 从 "Python 3.x.y" 字符串中提取次版本号
fn parse_python_minor(ver_str: &str) -> Option<u32> {
    // "Python 3.12.13" → 12
    let s = ver_str.trim();
    let after = s.strip_prefix("Python 3.")?;
    let minor_str: String = after.chars().take_while(|c| c.is_ascii_digit()).collect();
    minor_str.parse().ok()
}

fn is_supported_python(version: &str) -> bool {
    parse_python_minor(version).is_some_and(|minor| SUPPORTED_PYTHON_MINORS.contains(&minor))
}

/// 找一个版本受支持、带 venv 模块的系统 Python，返回解释器路径
fn detect_system_python() -> Option<PathBuf> {
    let candidates: &[&str] = if cfg!(target_os = "windows") {
        &["python3", "python", "py"]
    } else {
        &["python3", "python"]
    };
    if let Some(name) = candidates.iter().find(|name| usable_system_python(name)) {
        return Some(resolve_python_path(name));
    }

    // Windows: 官方安装包的默认位置（GUI 进程的 PATH 里未必有）
    #[cfg(target_os = "windows")]
    {
        let paths = [
            r"C:\Python313\python.exe",
            r"C:\Python312\python.exe",
            r"C:\Python311\python.exe",
        ];
        for path in paths {
            if Path::new(path).exists() && usable_system_python(path) {
                return Some(PathBuf::from(path));
            }
        }
    }

    None
}

/// `program` 是版本受支持、能导入 venv 模块的 Python
fn usable_system_python(program: &str) -> bool {
    let run = |args: &[&str]| {
        PythonCommand::new(program)
            .args(args)
            .output()
            .ok()
            .filter(|o| o.status.success())
    };
    let Some(version) = run(&["--version"]) else {
        return false;
    };
    let text = String::from_utf8_lossy(&version.stdout).into_owned()
        + String::from_utf8_lossy(&version.stderr).as_ref();
    is_supported_python(&text) && run(&["-c", "import venv"]).is_some()
}

/// 解析 Python 命令的实际可执行文件路径（`where` / `which` 输出的第一行），解析不到时原样返回
fn resolve_python_path(name: &str) -> PathBuf {
    let locator = if cfg!(target_os = "windows") {
        "where"
    } else {
        "which"
    };
    PythonCommand::new(locator)
        .arg(name)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| {
            String::from_utf8_lossy(&o.stdout)
                .lines()
                .next()
                .map(|line| line.trim().to_string())
        })
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(name))
}

/// venv 就绪的判据：能导入 onnxruntime
fn onnxruntime_importable(python: &Path) -> bool {
    python.exists()
        && PythonCommand::new(python)
            .args(["-c", "import onnxruntime"])
            .output()
            .is_ok_and(|o| o.status.success())
}

/// 解释器能运行、带 pip、版本受支持：这样的 venv 补装依赖就能用，不必重建
fn venv_interpreter_usable(python: &Path) -> bool {
    python.exists()
        && PythonCommand::new(python)
            .args([
                "-c",
                "import pip, sys; print('Python %d.%d' % sys.version_info[:2])",
            ])
            .output()
            .is_ok_and(|o| {
                o.status.success()
                    && is_supported_python(last_line(&String::from_utf8_lossy(&o.stdout)))
            })
}

/// 探测脚本打印在最后的结果行：venv 里的 .pth、sitecustomize 可能先往 stdout 打印别的内容
fn last_line(output: &str) -> &str {
    output
        .lines()
        .rev()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or_default()
}

fn verify_env(python: &Path) -> Result<(), String> {
    if onnxruntime_importable(python) {
        Ok(())
    } else {
        Err("Python 环境安装后验证失败".into())
    }
}

/// onnxruntime / onnxruntime-gpu 共用的固定版本
const ORT_VERSION: &str = "1.25.1";

/// `onnxruntime==版本` 或 `onnxruntime-gpu==版本`
fn ort_package(gpu: bool) -> String {
    let name = if gpu {
        "onnxruntime-gpu"
    } else {
        "onnxruntime"
    };
    format!("{}=={}", name, ORT_VERSION)
}

/// 带错误参数的进度消息 `@key|错误`，前端按当前语言翻译。错误只取开头：pip 的完整输出放进日志太长
fn message_with_error(key: &str, error: &str) -> String {
    format!("@{}|{}", key, error.chars().take(100).collect::<String>())
}

/// 改动 venv 的一次操作：进度发给 `app`，pip 带上应用内代理
struct Installer<R: Runtime> {
    app: AppHandle<R>,
    /// pip 的代理环境变量，入口处按当前代理配置生成一次（见 `proxy_config::pip_proxy_env`）
    proxy_env: Arc<[(&'static str, String)]>,
}

impl<R: Runtime> Clone for Installer<R> {
    fn clone(&self) -> Self {
        Installer {
            app: self.app.clone(),
            proxy_env: self.proxy_env.clone(),
        }
    }
}

impl<R: Runtime> Installer<R> {
    fn new(app: &AppHandle<R>) -> Self {
        Self::with_proxy_env(app, super::proxy_config::pip_proxy_env())
    }

    fn with_proxy_env(app: &AppHandle<R>, proxy_env: Vec<(&'static str, String)>) -> Self {
        Installer {
            app: app.clone(),
            proxy_env: proxy_env.into(),
        }
    }

    fn progress(&self, message: &str, status: &str) {
        ProgressEvent::new(status, message).emit(&self.app, PROGRESS_EVENT);
    }

    fn download_event(&self, progress: DownloadProgress) {
        let _ = self.app.emit(DOWNLOAD_EVENT, progress);
    }

    /// `python -m pip …`，`cancel` 置位时按进程树终止
    fn pip(
        &self,
        python: &Path,
        args: &[&str],
        cancel: &AtomicBool,
    ) -> std::io::Result<std::process::Output> {
        let mut cmd = PythonCommand::new(python).args(["-m", "pip"]).args(args);
        for (key, value) in self.proxy_env.iter() {
            cmd = cmd.env(key, value);
        }
        python_proc::output_cancellable(cmd, cancel)
    }

    /// 逐个 `pip install`：每个包发一条进度，全部结束后发一条完成 / 失败 / 取消
    fn install(&self, python: &Path, deps: &[&str], cancel: &AtomicBool) -> Result<(), String> {
        let result = deps.iter().enumerate().try_for_each(|(index, &dep)| {
            if cancel.load(Ordering::SeqCst) {
                return Err(CANCELLED.to_string());
            }
            self.pip_progress(dep, index, deps.len());
            let args = [
                "install",
                "--disable-pip-version-check",
                "--no-cache-dir",
                dep,
            ];
            pip_result(self.pip(python, &args, cancel), dep, cancel)
        });
        self.finish_pip_progress(&result, cancel);
        result
    }

    /// 一条 pip 命令（torch + torchvision 一起装），进度按一项显示
    fn pip_step(&self, python: &Path, args: &[&str], label: &str) -> Result<(), String> {
        self.pip_progress(label, 0, 1);
        let result = pip_result(
            self.pip(python, args, &SETUP_CANCELLED),
            label,
            &SETUP_CANCELLED,
        );
        self.finish_pip_progress(&result, &SETUP_CANCELLED);
        result
    }

    fn pip_progress(&self, dep: &str, index: usize, total: usize) {
        self.download_event(
            DownloadProgress::new(
                "downloading",
                (index as f32 / total as f32) * 100.0,
                format!("@pythonEnv.installingDep|{}|{}|{}", dep, index + 1, total),
            )
            .with_filename(dep),
        );
    }

    fn finish_pip_progress(&self, result: &Result<(), String>, cancel: &AtomicBool) {
        self.download_event(if cancel.load(Ordering::SeqCst) {
            DownloadProgress::cancelled(CANCELLED)
        } else {
            match result {
                Ok(()) => DownloadProgress::done("@pythonEnv.depsInstalled"),
                Err(error) => DownloadProgress::error(error),
            }
        });
    }

    /// 卸掉两种 onnxruntime 包。不可取消：卸到一半中止会留下登记还在、文件不全的包；
    /// 卸载失败（一般是本来就没装）不影响随后的安装，忽略
    fn uninstall_onnxruntime(&self, python: &Path) {
        let never = AtomicBool::new(false);
        let args = ["uninstall", "-y", "onnxruntime", "onnxruntime-gpu"];
        let _ = self.pip(python, &args, &never);
    }

    /// 安装基础依赖（固定版本，避免供应链风险）。
    ///
    /// 统一装 CPU 版 onnxruntime，确保任何环境（AMD/Intel/无独显/缺 cuDNN）
    /// 都能正常运行。onnxruntime-gpu 在缺少 CUDA 库时 import 会直接报错，
    /// 不适合作为默认依赖。GPU 升级由 ensure_onnx_gpu_runtime 在检测到
    /// NVIDIA 环境后单独处理。
    fn install_base_deps(&self, python: &Path) -> Result<(), String> {
        let ort = ort_package(false);
        self.install(
            python,
            &[ort.as_str(), "numpy==2.2.6", "pillow==11.3.0"],
            &SETUP_CANCELLED,
        )
    }

    /// venv 解释器可用、只是导入不了 onnxruntime（GPU 换装被打断等）：先卸掉 onnxruntime 包
    /// （登记还在、文件不全时 pip 认为已经装好，不会重装），再补装基础依赖
    fn repair_venv(&self, python: &Path) -> Result<(), String> {
        self.uninstall_onnxruntime(python);
        self.install_base_deps(python)?;
        verify_env(python)
    }

    /// 用 `base_python` 新建 venv。走到这里说明旧 venv 不可用（解释器缺失、版本不受支持或没有 pip），
    /// 先移除残留
    fn create_venv(&self, layout: &EnvLayout, base_python: &Path) -> Result<(), String> {
        let venv_dir = layout.venv_dir();
        if venv_dir.exists() {
            std::fs::remove_dir_all(&venv_dir).map_err(|e| format!("清理 venv 失败: {}", e))?;
        }
        std::fs::create_dir_all(layout.python_root())
            .map_err(|e| format!("创建目录失败: {}", e))?;
        let output = python_proc::output_cancellable(
            PythonCommand::new(base_python)
                .args(["-m", "venv"])
                .arg(&venv_dir),
            &SETUP_CANCELLED,
        );
        if is_cancelled() {
            return Err(CANCELLED.into());
        }
        let output = output.map_err(|e| format!("创建 venv 失败: {}", e))?;
        if !output.status.success() {
            return Err(format!(
                "创建 venv 失败: {}",
                String::from_utf8_lossy(&output.stderr)
            ));
        }
        self.progress("@pythonEnv.venvCreated", "success");
        Ok(())
    }

    /// 部署流程（调用方已持安装锁），返回 venv 解释器的路径：
    /// 1. venv 能导入 onnxruntime：直接用；
    /// 2. venv 解释器能运行、带 pip、版本受支持：只补装基础依赖，失败时保留 venv 报错——
    ///    另装的 torch 等大包都在这个 venv 里，重建要全部重新下载；
    /// 3. 用受支持的系统 Python 新建 venv 并装基础依赖，任何一步失败都改用独立版；
    /// 4. 用独立版 Python（没有就下载）新建 venv 并装基础依赖。
    async fn setup(
        &self,
        layout: &EnvLayout,
        detect_system_python: impl FnOnce() -> Option<PathBuf> + Send + 'static,
    ) -> Result<PathBuf, String> {
        let venv_python = layout.venv_python();
        let probe = venv_python.clone();
        if blocking(move || onnxruntime_importable(&probe)).await? {
            return Ok(venv_python);
        }
        let probe = venv_python.clone();
        if blocking(move || venv_interpreter_usable(&probe)).await? {
            let this = self.clone();
            let python = venv_python.clone();
            blocking(move || this.repair_venv(&python)).await??;
            self.progress("@pythonEnv.ready", "success");
            return Ok(venv_python);
        }

        let system_python = blocking(detect_system_python).await?;
        if is_cancelled() {
            return Err(CANCELLED.into());
        }
        if let Some(system_python) = system_python {
            match self.build_venv(layout, system_python).await {
                Ok(python) => return Ok(python),
                Err(_) if is_cancelled() => return Err(CANCELLED.into()),
                Err(e) => self.progress(&message_with_error("pythonEnv.venvFailed", &e), "info"),
            }
        }
        self.setup_with_standalone(layout).await
    }

    /// 用 `base_python` 新建 venv、装基础依赖并验证
    async fn build_venv(
        &self,
        layout: &EnvLayout,
        base_python: PathBuf,
    ) -> Result<PathBuf, String> {
        let this = self.clone();
        let layout = layout.clone();
        let python = blocking(move || {
            this.create_venv(&layout, &base_python)?;
            let python = layout.venv_python();
            this.install_base_deps(&python)?;
            verify_env(&python)?;
            Ok::<_, String>(python)
        })
        .await??;
        self.progress("@pythonEnv.ready", "success");
        Ok(python)
    }

    async fn setup_with_standalone(&self, layout: &EnvLayout) -> Result<PathBuf, String> {
        let python = layout.standalone_python();
        if !python.exists() {
            self.download_python(layout).await?;
        }
        if is_cancelled() {
            return Err(CANCELLED.into());
        }
        self.build_venv(layout, python).await
    }

    /// 下载独立版 Python 并解压到 `layout.base_dir()`
    async fn download_python(&self, layout: &EnvLayout) -> Result<(), String> {
        let url = python_download_url();
        let env_dir = layout.env_dir.clone();
        std::fs::create_dir_all(&env_dir).map_err(|e| format!("创建 env 目录失败: {}", e))?;
        self.progress(
            &format!(
                "@pythonEnv.downloading|{}",
                url.split('/').next_back().unwrap_or("python")
            ),
            "info",
        );

        let client = http_download::download_client()?;
        let archive = env_dir.join("python_download.tar.gz");
        let downloaded = http_download::download_to_file(
            client.get(&url),
            &archive,
            "Python",
            &SETUP_CANCELLED,
            |p| self.download_event(p.with_filename("python")),
        )
        .await;
        if let Err(err) = downloaded {
            self.download_event(
                DownloadProgress::from_error(&err, &SETUP_CANCELLED).with_filename("python"),
            );
            return Err(
                if matches!(err, DownloadError::Cancelled) || is_cancelled() {
                    CANCELLED.into()
                } else {
                    err.into()
                },
            );
        }
        self.download_event(DownloadProgress::done("Python 下载完成").with_filename("python"));
        if is_cancelled() {
            return Err(CANCELLED.into());
        }

        self.progress("@pythonEnv.extracting", "info");
        let base_dir = layout.base_dir();
        blocking(move || unpack_python(&archive, &env_dir, &base_dir)).await??;
        let python = layout.standalone_python();
        if !python.exists() {
            return Err(format!("Python 解压后未找到: {}", python.display()));
        }
        self.progress("@pythonEnv.downloadDone", "success");
        Ok(())
    }
}

/// 解压独立版 Python：先解到临时目录，再把其中的 python/ 整个改名为 `base_dir`
fn unpack_python(archive: &Path, env_dir: &Path, base_dir: &Path) -> Result<(), String> {
    let extract_tmp = env_dir.join("_python_extract_tmp");
    let _ = std::fs::remove_dir_all(&extract_tmp);
    std::fs::create_dir_all(&extract_tmp).map_err(|e| format!("创建临时目录失败: {}", e))?;
    let extracted = extract_tar_gz(archive, &extract_tmp);
    let _ = std::fs::remove_file(archive);
    let moved = extracted.and_then(|()| {
        let extracted_python = extract_tmp.join("python");
        if !extracted_python.exists() {
            return Err("解压后未找到 python 目录".to_string());
        }
        if let Some(parent) = base_dir.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("创建 python 目录失败: {}", e))?;
        }
        if base_dir.exists() {
            let _ = std::fs::remove_dir_all(base_dir);
        }
        std::fs::rename(&extracted_python, base_dir)
            .map_err(|e| format!("移动 Python 目录失败: {}", e))
    });
    let _ = std::fs::remove_dir_all(&extract_tmp);
    moved
}

fn extract_tar_gz(archive: &Path, dest: &Path) -> Result<(), String> {
    let output = PythonCommand::new("tar")
        .arg("xzf")
        .arg(archive)
        .arg("-C")
        .arg(dest)
        .output()
        .map_err(|e| format!("解压失败: {}", e))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(format!(
            "解压 Python 失败: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ))
    }
}

fn pip_result(
    output: std::io::Result<std::process::Output>,
    label: &str,
    cancel: &AtomicBool,
) -> Result<(), String> {
    if cancel.load(Ordering::SeqCst) {
        return Err(CANCELLED.into());
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

/// 部署 Python 环境（入口，全局串行化），返回 venv 解释器路径。取消归属 `owner`
pub async fn setup_python_env<R: Runtime>(
    app: &AppHandle<R>,
    owner: &'static str,
) -> Result<String, String> {
    let guard = acquire_setup(owner).await?;
    let installer = Installer::new(app);
    run_locked(guard, async move {
        let python = installer
            .setup(&EnvLayout::current(), detect_system_python)
            .await?;
        Ok(python.to_string_lossy().into_owned())
    })
    .await
}

/// 本会话是否已尝试过把 CPU-only torch 升级为 CUDA 构建（避免反复下载 ~2GB），重置环境后复位
static TORCH_UPGRADE_TRIED: AtomicBool = AtomicBool::new(false);

#[derive(Debug, PartialEq)]
struct TorchPackages {
    torch: bool,
    torchvision: bool,
    cuda_build: bool,
}

impl TorchPackages {
    fn parse(probe: &str) -> Result<Self, String> {
        let fields: Vec<_> = last_line(probe).split('|').collect();
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

/// 安装缺失的 PyTorch 依赖；仅 Windows NVIDIA 环境会选择 CUDA wheel。
pub async fn ensure_torch_gpu_runtime<R: Runtime>(
    app: &AppHandle<R>,
    python: &str,
    owner: &'static str,
) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    let windows_nvidia = has_nvidia_gpu().await;
    #[cfg(not(target_os = "windows"))]
    let windows_nvidia = false;
    ensure_torch(
        Installer::new(app),
        PathBuf::from(python),
        owner,
        windows_nvidia,
    )
    .await
}

/// 探测在拿安装锁之前做：冷启动 `import torch` 可能要一两分钟，持锁探测会让所有功能的环境检查一起排队
async fn ensure_torch<R: Runtime>(
    installer: Installer<R>,
    python: PathBuf,
    owner: &'static str,
    windows_nvidia: bool,
) -> Result<(), String> {
    let probe = probe_before_lock(owner, python.clone(), TORCH_PROBE)
        .await?
        .ok_or("PyTorch 依赖检测失败")?;
    let installed = TorchPackages::parse(&probe)?;
    let tried = || TORCH_UPGRADE_TRIED.load(Ordering::SeqCst);
    if torch_install_plan(&installed, windows_nvidia, tried()).is_none() {
        return Ok(());
    }
    let guard = acquire_setup(owner).await?;
    run_locked_blocking(guard, move || {
        // 排队期间别的任务可能已经换装过 CUDA 版
        let Some(plan) = torch_install_plan(&installed, windows_nvidia, tried()) else {
            return Ok(());
        };
        if plan.replace_cpu {
            TORCH_UPGRADE_TRIED.store(true, Ordering::SeqCst);
        }
        installer.pip_step(&python, &plan.args(), &plan.packages.join(", "))
    })
    .await
}

/// 额外依赖与基础环境部署共用同一把锁，取消归属于调用方。
pub(crate) async fn pip_install_for<R: Runtime>(
    app: &AppHandle<R>,
    python: &str,
    deps: &[&str],
    owner: &'static str,
) -> Result<(), String> {
    install_for(Installer::new(app), PathBuf::from(python), deps, owner).await
}

async fn install_for<R: Runtime>(
    installer: Installer<R>,
    python: PathBuf,
    deps: &[&str],
    owner: &'static str,
) -> Result<(), String> {
    let guard = acquire_setup(owner).await?;
    let deps: Vec<String> = deps.iter().map(|dep| dep.to_string()).collect();
    run_locked_blocking(guard, move || {
        let deps: Vec<&str> = deps.iter().map(String::as_str).collect();
        installer.install(&python, &deps, &SETUP_CANCELLED)
    })
    .await
}

/// onnxruntime 的 GPU 换装（Windows / Linux）。macOS 的 onnxruntime 默认就带 CoreML，没有要换的 GPU 包；
/// 测试构型在所有平台编译，用伪 Python 覆盖换装与恢复流程
#[cfg(any(target_os = "windows", target_os = "linux", test))]
mod onnx_gpu {
    use super::*;
    use std::sync::atomic::AtomicU8;

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

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub(super) enum Phase {
        NotTried,
        /// 有一个功能持安装锁在换装，venv 里暂时可能没有 onnxruntime
        Running,
        /// 换装已结束（成功，或失败后已尝试装回 CPU 版）
        Settled,
    }

    /// 本会话的换装进度。每个会话最多尝试一次（重置环境后重新算），失败不反复下载
    pub(super) struct OrtUpgrade(AtomicU8);

    impl OrtUpgrade {
        pub(super) const fn new() -> Self {
            OrtUpgrade(AtomicU8::new(0))
        }

        pub(super) fn phase(&self) -> Phase {
            match self.0.load(Ordering::SeqCst) {
                0 => Phase::NotTried,
                1 => Phase::Running,
                _ => Phase::Settled,
            }
        }

        /// 抢下这次换装（持安装锁时调用）；已经有人试过时返回 false
        pub(super) fn begin(&self) -> bool {
            self.0
                .compare_exchange(0, 1, Ordering::SeqCst, Ordering::SeqCst)
                .is_ok()
        }

        pub(super) fn settle(&self) {
            self.0.store(2, Ordering::SeqCst);
        }

        /// 环境被重置后回到「没试过」（持安装锁调用）
        pub(super) fn reset(&self) {
            self.0.store(0, Ordering::SeqCst);
        }

        /// 持安装锁调用：抢到就换装；没抢到说明排队期间别的功能已经换装过——
        /// 那次可能失败了、连 CPU 版都没装回来，所以确认 onnxruntime 还能导入
        pub(super) fn switch_or_verify<R: Runtime>(
            &self,
            installer: &Installer<R>,
            python: &Path,
        ) -> Result<(), String> {
            if self.begin() {
                let result = installer.switch_to_gpu(python);
                self.settle();
                result
            } else {
                installer.ensure_onnxruntime(python)
            }
        }
    }

    /// 探测结果说明需要换装：没有 CUDA EP，而且装的不是 GPU 包
    /// （GPU 包缺 CUDA 库时同样没有 CUDA EP，换装解决不了）
    pub(super) fn needs_gpu_package(probe: &str) -> bool {
        let (providers, gpu_pkg) = last_line(probe).split_once('|').unwrap_or(("", "0"));
        !providers.contains("CUDAExecutionProvider") && gpu_pkg.trim() != "1"
    }

    /// 见 `ensure_onnx_gpu_runtime`。`has_nvidia` 只在需要时才等（第一次会运行 nvidia-smi）
    pub(super) async fn ensure<R: Runtime>(
        installer: Installer<R>,
        python: PathBuf,
        owner: &'static str,
        state: &'static OrtUpgrade,
        has_nvidia: impl Future<Output = bool>,
    ) -> Result<(), String> {
        match state.phase() {
            Phase::Settled => return Ok(()),
            Phase::Running => {
                // 别的功能正在换装：等它结束，再确认 onnxruntime 能导入
                let guard = acquire_setup(owner).await?;
                return run_locked_blocking(guard, move || installer.ensure_onnxruntime(&python))
                    .await;
            }
            Phase::NotTried => {}
        }
        if !has_nvidia.await {
            return Ok(());
        }
        // 探测本身失败（解释器跑不起来、超时）时换装也装不上，不先卸掉现有的包
        let Some(probe) = probe_before_lock(owner, python.clone(), ONNX_PROBE).await? else {
            return Ok(());
        };
        if !needs_gpu_package(&probe) {
            return Ok(());
        }
        let guard = acquire_setup(owner).await?;
        run_locked_blocking(guard, move || state.switch_or_verify(&installer, &python)).await
    }

    fn importable_after_install(python: &Path, package: &str) -> Result<(), String> {
        if onnxruntime_importable(python) {
            Ok(())
        } else {
            Err(format!("{} 安装后无法导入", package))
        }
    }

    impl<R: Runtime> Installer<R> {
        /// 把 CPU 版 onnxruntime 换成 onnxruntime-gpu。两个包争用同一个 `onnxruntime` 模块名，
        /// 只能先卸后装；GPU 包没装上（失败、取消）或装上后导入不了时，不可取消地装回 CPU 版，
        /// 不让 venv 停在没有 onnxruntime 的状态。
        ///
        /// 装回成功又没有取消时只发一条警告、返回 Ok，任务按 CPU 版继续：本会话不再尝试换装，
        /// 之后的任务都按 CPU 版运行，这一次也不该失败
        pub(super) fn switch_to_gpu(&self, python: &Path) -> Result<(), String> {
            if is_cancelled() {
                return Err(CANCELLED.into());
            }
            self.progress("@pythonEnv.uninstallCpu", "info");
            self.uninstall_onnxruntime(python);
            self.progress("@pythonEnv.installGpu", "info");
            let gpu = ort_package(true);
            let installed = self
                .install(python, &[gpu.as_str()], &SETUP_CANCELLED)
                .and_then(|()| importable_after_install(python, &gpu));
            let Err(error) = installed else {
                return Ok(());
            };
            let restored = self.restore_cpu_onnxruntime(python);
            // 装回不可取消、可能要一阵子，期间点的取消同样按取消返回
            let cancelled = is_cancelled();
            let error = if cancelled {
                CANCELLED.to_string()
            } else {
                error
            };
            match restored {
                Err(restore) => Err(format!("{}；{}", error, restore)),
                Ok(()) if cancelled => Err(error),
                Ok(()) => {
                    self.progress(
                        &message_with_error("pythonEnv.gpuRuntimeFallback", &error),
                        "warning",
                    );
                    Ok(())
                }
            }
        }

        /// 能导入 onnxruntime 就什么都不做，否则装回 CPU 版
        pub(super) fn ensure_onnxruntime(&self, python: &Path) -> Result<(), String> {
            if onnxruntime_importable(python) {
                return Ok(());
            }
            self.restore_cpu_onnxruntime(python)
        }

        /// 不可取消地装回 CPU 版 onnxruntime：这时用户多半已经点了取消，
        /// 但 venv 不能留在没有 onnxruntime 的状态
        fn restore_cpu_onnxruntime(&self, python: &Path) -> Result<(), String> {
            let never = AtomicBool::new(false);
            self.uninstall_onnxruntime(python);
            let cpu = ort_package(false);
            self.install(python, &[cpu.as_str()], &never)
                .and_then(|()| importable_after_install(python, &cpu))
                .map_err(|e| format!("恢复 CPU 版 onnxruntime 失败: {}", e))
        }
    }
}

/// 本会话的 onnxruntime 换装进度。测试构型也编译：重置环境的测试要检查它被复位
#[cfg(any(target_os = "windows", target_os = "linux", test))]
static ORT_UPGRADE: onnx_gpu::OrtUpgrade = onnx_gpu::OrtUpgrade::new();

/// 统一入口：本机有 NVIDIA GPU、venv 里却是 CPU 版 onnxruntime 时换成 onnxruntime-gpu（仅 x86_64）。
///
/// 不安装 CUDA、不下载 CUDA 运行时，只使用本机既有的 CUDA 环境。无 NVIDIA 的机器
/// （AMD/Intel/核显）不触发换装，继续用 CPU 版（为什么默认装 CPU 版见 install_base_deps）。
/// 每个会话最多尝试一次（重置环境后重新算），失败不重复下载。
#[cfg(any(target_os = "windows", target_os = "linux"))]
pub async fn ensure_onnx_gpu_runtime<R: Runtime>(
    app: &AppHandle<R>,
    python: &str,
    owner: &'static str,
) -> Result<(), String> {
    // onnxruntime-gpu 只发布了 x86_64 的包（win_amd64、manylinux_x86_64）：ARM64 上换装必然失败，
    // 每个会话的第一个 GPU 任务都会白白卸掉 CPU 版、装 GPU 版失败、再装回
    if !cfg!(target_arch = "x86_64") {
        return Ok(());
    }
    onnx_gpu::ensure(
        Installer::new(app),
        PathBuf::from(python),
        owner,
        &ORT_UPGRADE,
        has_nvidia_gpu(),
    )
    .await
}

/// macOS 的 onnxruntime 默认就带 CoreML，没有要换装的 GPU 包
#[cfg(not(any(target_os = "windows", target_os = "linux")))]
pub async fn ensure_onnx_gpu_runtime<R: Runtime>(
    _app: &AppHandle<R>,
    _python: &str,
    _owner: &'static str,
) -> Result<(), String> {
    Ok(())
}

/// 本机是否有 NVIDIA GPU（`nvidia-smi -L`）。结果在本会话缓存，只有第一次真正运行 nvidia-smi
#[cfg(any(target_os = "windows", target_os = "linux", test))]
async fn has_nvidia_gpu() -> bool {
    static NVIDIA: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    if let Some(&cached) = NVIDIA.get() {
        return cached;
    }
    tokio::task::spawn_blocking(|| *NVIDIA.get_or_init(detect_nvidia_gpu))
        .await
        .unwrap_or(false)
}

#[cfg(any(target_os = "windows", target_os = "linux", test))]
fn detect_nvidia_gpu() -> bool {
    // Windows GUI 进程的 PATH 未必包含 nvidia-smi，逐个候选路径尝试
    let candidates: &[&str] = if cfg!(target_os = "windows") {
        &[
            "nvidia-smi",
            r"C:\Windows\System32\nvidia-smi.exe",
            r"C:\Program Files\NVIDIA Corporation\NVSMI\nvidia-smi.exe",
        ]
    } else {
        &["nvidia-smi"]
    };
    candidates.iter().any(|exe| {
        PythonCommand::new(exe)
            .arg("-L")
            .output()
            .is_ok_and(|out| out.status.success())
    })
}

/// 探测脚本的时限：Windows 上冷启动 `import torch` 要加载上 GB 的 DLL（杀毒软件逐个扫描），可能要一两分钟
const PROBE_TIMEOUT: Duration = Duration::from_secs(180);

/// 探测期间检查取消、时限的间隔
const PROBE_POLL: Duration = Duration::from_millis(50);

/// 在指定 Python 下执行探测脚本，成功时返回 stdout；脚本失败或超过 PROBE_TIMEOUT 时返回 None
pub(crate) async fn probe_python(python: &str, script: &'static str) -> Option<String> {
    probe_python_within(
        PathBuf::from(python),
        script.to_string(),
        PROBE_TIMEOUT,
        || false,
    )
    .await
    .unwrap_or(None)
}

/// 拿安装锁之前的探测。owner 这时没持锁，它的取消记在 PENDING_CANCELS 里：到达时结束探测、返回「已取消」。
/// 只查看不消费——探测刚好结束时到达的取消，还要由随后的 `acquire_setup` 生效
async fn probe_before_lock(
    owner: &'static str,
    python: PathBuf,
    script: &str,
) -> Result<Option<String>, String> {
    probe_python_within(python, script.to_string(), PROBE_TIMEOUT, move || {
        has_pending_cancel(owner)
    })
    .await
}

/// 成功时返回 stdout，脚本失败或超过 `limit` 时返回 Ok(None)，`cancelled()` 变真时返回「已取消」；
/// 超时和取消都按进程树终止探测进程
async fn probe_python_within(
    python: PathBuf,
    script: String,
    limit: Duration,
    cancelled: impl Fn() -> bool,
) -> Result<Option<String>, String> {
    if cancelled() {
        return Err(CANCELLED.into());
    }
    let stop = Arc::new(AtomicBool::new(false));
    let flag = stop.clone();
    let mut task = tokio::task::spawn_blocking(move || {
        python_proc::output_cancellable(PythonCommand::new(python).arg("-c").arg(script), &flag)
    });
    let deadline = tokio::time::Instant::now() + limit;
    let (joined, was_cancelled) = loop {
        tokio::select! {
            joined = &mut task => break (joined, false),
            _ = tokio::time::sleep(PROBE_POLL) => {}
        }
        let cancel = cancelled();
        if cancel || tokio::time::Instant::now() >= deadline {
            // 让运行器按进程树终止探测进程，再等它收尾
            stop.store(true, Ordering::SeqCst);
            break (task.await, cancel);
        }
    };
    if was_cancelled {
        return Err(CANCELLED.into());
    }
    let Ok(Ok(output)) = joined else {
        return Ok(None);
    };
    Ok(output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    use crate::commands::batch::capture_events;
    use crate::commands::test_support::TempDir;

    /// 用到全局安装锁、取消标志的测试串行执行
    pub(super) static TEST_SETUP_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

    /// 不读代理配置的安装上下文
    #[cfg(unix)]
    pub(super) fn test_installer(
        app: &tauri::App<tauri::test::MockRuntime>,
    ) -> Installer<tauri::test::MockRuntime> {
        Installer::with_proxy_env(app.handle(), Vec::new())
    }

    /// 等子进程把自己的 PID 写进 `path`。
    /// 全量测试时 macOS 上大量新建的伪程序同时首次执行，从 spawn 到脚本跑起来实测要 3 秒多，时限放宽
    #[cfg(unix)]
    pub(super) async fn wait_for_pid(path: &Path) -> u32 {
        tokio::time::timeout(Duration::from_secs(30), async {
            loop {
                let pid = std::fs::read_to_string(path)
                    .ok()
                    .and_then(|s| s.trim().parse().ok());
                if let Some(pid) = pid {
                    return pid;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap()
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
        assert_eq!(
            TorchPackages::parse("hook|output\n1|1|0\n\n").unwrap(),
            packages(true, true, false)
        );
        for invalid in ["", "1|1", "1|1|0|0", "True|1|0"] {
            assert!(TorchPackages::parse(invalid).is_err());
        }
    }

    #[test]
    fn torch_probe_uses_stub_modules_without_initializing_gpu() {
        let python = PathBuf::from(crate::commands::test_support::test_python());
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
            let output = PythonCommand::new(&python)
                .args(["-I", "-B", "-c", &script])
                .output()
                .unwrap();
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
        let queued = tokio::time::timeout(Duration::from_secs(2), acquire_setup("test-queued"))
            .await
            .unwrap();
        assert!(matches!(queued, Err(e) if e == CANCELLED));
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
        assert!(matches!(acquire_setup("test-between").await, Err(e) if e == CANCELLED));
        assert!(acquire_setup("test-between").await.is_ok());
    }

    #[tokio::test]
    async fn pending_cancel_is_kept_until_the_next_task_clears_it() {
        let _serial = TEST_SETUP_LOCK.lock().await;
        // 任务已开始、还没进入部署时点的取消：部署拿锁时生效
        cancel_setup_for("test-pending");
        assert!(matches!(acquire_setup("test-pending").await, Err(e) if e == CANCELLED));
        // 上一轮留下、没被消费的取消：新一轮开始时清掉，不影响这一轮
        cancel_setup_for("test-pending");
        clear_pending_cancel("test-pending");
        assert!(acquire_setup("test-pending").await.is_ok());
    }

    #[tokio::test]
    async fn aborted_waiter_keeps_lock_until_blocking_install_finishes() {
        let _serial = TEST_SETUP_LOCK.lock().await;
        let guard = acquire_setup("test-abort").await.unwrap();
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let (finish_tx, finish_rx) = std::sync::mpsc::channel();
        let task = tokio::spawn(run_locked_blocking(guard, move || {
            let _ = started_tx.send(());
            finish_rx.recv_timeout(Duration::from_secs(5)).unwrap();
            Ok(())
        }));
        started_rx.await.unwrap();
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        assert!(SETUP_LOCK.try_lock().is_err());
        assert_eq!(*SETUP_OWNER.lock().unwrap(), Some("test-abort"));
        finish_tx.send(()).unwrap();
        let _next = tokio::time::timeout(Duration::from_secs(2), acquire_setup("test-after-abort"))
            .await
            .unwrap()
            .unwrap();
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

    #[test]
    fn only_python_311_to_313_is_supported() {
        for (version, supported) in [
            ("Python 3.9.18", false),
            ("Python 3.10.14", false),
            ("Python 3.11.0", true),
            ("Python 3.12.13\n", true),
            ("Python 3.13.7", true),
            ("Python 3.14.0", false),
            ("Python 3", false),
            ("Python 2.7.18", false),
            ("", false),
        ] {
            assert_eq!(is_supported_python(version), supported, "{version:?}");
        }
    }

    #[test]
    fn gpu_package_is_needed_only_without_cuda_ep_and_gpu_package() {
        assert!(onnx_gpu::needs_gpu_package("CPUExecutionProvider|0"));
        assert!(onnx_gpu::needs_gpu_package("|0"));
        assert!(!onnx_gpu::needs_gpu_package(
            "CUDAExecutionProvider,CPUExecutionProvider|1"
        ));
        // GPU 包已装但缺 CUDA 库：换装解决不了
        assert!(!onnx_gpu::needs_gpu_package("CPUExecutionProvider|1"));
        // .pth 等先打印到 stdout 的内容不影响判断
        assert!(!onnx_gpu::needs_gpu_package(
            "hook|output\nCUDAExecutionProvider,CPUExecutionProvider|1"
        ));
        assert!(onnx_gpu::needs_gpu_package(
            "hook|1\nCPUExecutionProvider|0"
        ));
    }

    /// 重置环境删干净后，本会话的换装记录复位，下次 GPU 任务重新判断要不要换装
    #[tokio::test]
    async fn env_reset_forgets_gpu_upgrades() {
        let _serial = TEST_SETUP_LOCK.lock().await;
        let root = TempDir::new("python_env_reset_upgrades");
        let layout = EnvLayout {
            env_dir: root.join("env"),
        };
        std::fs::create_dir_all(layout.venv_dir().join("lib")).unwrap();
        std::fs::create_dir_all(layout.base_dir()).unwrap();
        TORCH_UPGRADE_TRIED.store(true, Ordering::SeqCst);
        ORT_UPGRADE.begin();
        ORT_UPGRADE.settle();
        remove_env(layout.clone(), "test-reset").await.unwrap();
        assert!(!layout.python_root().exists());
        assert!(!TORCH_UPGRADE_TRIED.load(Ordering::SeqCst));
        assert_eq!(ORT_UPGRADE.phase(), onnx_gpu::Phase::NotTried);
        assert_eq!(*SETUP_OWNER.lock().unwrap(), None);
    }

    #[tokio::test]
    async fn nvidia_detection_is_cached_for_the_session() {
        let first = has_nvidia_gpu().await;
        assert_eq!(has_nvidia_gpu().await, first);
        if cfg!(target_os = "macos") {
            assert!(!first);
        }
    }

    #[tokio::test]
    async fn probe_only_runs_the_requested_script() {
        let python = crate::commands::test_support::test_python();
        assert_eq!(
            probe_python(&python, "print('probe-ok')").await.as_deref(),
            Some("probe-ok")
        );
        assert!(probe_python(&python, "raise RuntimeError('probe-failed')")
            .await
            .is_none());
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
    #[tokio::test]
    async fn missing_torch_installs_both_once_without_gpu_upgrade() {
        let _serial = TEST_SETUP_LOCK.lock().await;
        let root = TempDir::new("python_env_torch_missing");
        let python = mock_python(&root, "0|0|0", "exit 0");
        let app = tauri::test::mock_app();
        let events = capture_events(app.handle(), DOWNLOAD_EVENT);
        ensure_torch(
            test_installer(&app),
            PathBuf::from(&python),
            "test-torch",
            false,
        )
        .await
        .unwrap();
        let args = std::fs::read_to_string(root.join("args")).unwrap();
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
            let root = TempDir::new("python_env_torch_noop");
            let python = mock_python(&root, probe, "exit 0");
            let result = ensure_torch(
                test_installer(&app),
                PathBuf::from(&python),
                "test-torch-noop",
                false,
            )
            .await;
            assert_eq!(result.is_ok(), success);
            assert!(!root.join("args").exists());
        }
    }

    /// 探测在拿安装锁之前：别的功能正在安装时，探测和「什么都不用装」的判断不排队
    #[cfg(unix)]
    #[tokio::test]
    async fn torch_probe_does_not_wait_for_the_setup_lock() {
        let _serial = TEST_SETUP_LOCK.lock().await;
        let _holder = acquire_setup("test-torch-holder").await.unwrap();
        let root = TempDir::new("python_env_torch_unlocked");
        let python = mock_python(&root, "1|1|0", "exit 0");
        let app = tauri::test::mock_app();
        let result = tokio::time::timeout(
            Duration::from_secs(5),
            ensure_torch(
                test_installer(&app),
                PathBuf::from(&python),
                "test-torch-unlocked",
                false,
            ),
        )
        .await;
        assert_eq!(result.unwrap(), Ok(()));
        assert!(!root.join("args").exists());
    }

    /// `/bin/sh -c 脚本` 充当探测：不新建可执行文件（macOS 首次运行新脚本要先过系统检查，时间不定）
    #[cfg(unix)]
    #[tokio::test]
    async fn hung_probe_is_killed_at_the_time_limit() {
        let root = TempDir::new("python_env_probe_timeout");
        let pid_file = root.join("pid");
        let script = format!("echo $$ > '{}'; sleep 30", pid_file.display());
        let started = std::time::Instant::now();
        let probe = probe_python_within(
            PathBuf::from("/bin/sh"),
            script,
            Duration::from_millis(500),
            || false,
        )
        .await;
        assert_eq!(probe, Ok(None));
        assert!(started.elapsed() < Duration::from_secs(5));
        let pid = wait_for_pid(&pid_file).await;
        assert!(crate::commands::test_support::wait_process_gone(pid));
    }

    /// 拿锁之前的探测卡住时，`owner` 的取消很快结束探测进程、按取消返回；
    /// 探测不消费这个取消，之后的 acquire_setup 照样按取消返回。
    /// `run` 用给定的解释器开始一次任务
    #[cfg(unix)]
    async fn assert_hung_probe_is_cancelled<F>(owner: &'static str, run: impl Fn(PathBuf) -> F)
    where
        F: Future<Output = Result<(), String>> + Send + 'static,
    {
        let root = TempDir::new("python_env_probe_cancel");
        let pid_file = root.join("pid");
        let python = crate::commands::test_support::fake_program(
            &root,
            "python",
            &format!("echo $$ > '{}'\nsleep 30\n", pid_file.display()),
        );
        let task = tokio::spawn(run(python.clone()));
        let pid = wait_for_pid(&pid_file).await;
        cancel_setup_for(owner);
        let result = tokio::time::timeout(Duration::from_secs(2), task)
            .await
            .expect("取消后探测仍在运行")
            .unwrap();
        assert_eq!(result, Err(CANCELLED.to_string()));
        assert!(crate::commands::test_support::wait_process_gone(pid));
        assert!(matches!(acquire_setup(owner).await, Err(e) if e == CANCELLED));

        // 开始前就取消了：不启动探测进程
        std::fs::remove_file(&pid_file).unwrap();
        cancel_setup_for(owner);
        assert_eq!(run(python).await, Err(CANCELLED.to_string()));
        assert!(!pid_file.exists());
        assert!(matches!(acquire_setup(owner).await, Err(e) if e == CANCELLED));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn cancel_stops_the_torch_probe_before_the_lock() {
        let _serial = TEST_SETUP_LOCK.lock().await;
        let app = tauri::test::mock_app();
        let installer = test_installer(&app);
        let owner = "test-torch-probe-cancel";
        assert_hung_probe_is_cancelled(owner, move |python| {
            ensure_torch(installer.clone(), python, owner, false)
        })
        .await;
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn cancel_stops_the_onnx_probe_before_the_lock() {
        static STATE: onnx_gpu::OrtUpgrade = onnx_gpu::OrtUpgrade::new();
        let _serial = TEST_SETUP_LOCK.lock().await;
        let app = tauri::test::mock_app();
        let installer = test_installer(&app);
        let owner = "test-onnx-probe-cancel";
        assert_hung_probe_is_cancelled(owner, move |python| {
            onnx_gpu::ensure(installer.clone(), python, owner, &STATE, async { true })
        })
        .await;
        assert_eq!(STATE.phase(), onnx_gpu::Phase::NotTried);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn extra_pip_is_serialized_and_queued_cancel_never_spawns() {
        let _serial = TEST_SETUP_LOCK.lock().await;
        let root = TempDir::new("python_env_pip_queue");
        let python = mock_python(&root, "", "exit 0");
        let app = tauri::test::mock_app();
        let guard = acquire_setup("test-lock-holder").await.unwrap();
        let installer = test_installer(&app);
        let task = tokio::spawn(async move {
            install_for(
                installer,
                PathBuf::from(python),
                &["mock-dependency"],
                "test-pip-queue",
            )
            .await
        });
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(!root.join("args").exists());
        assert!(!task.is_finished());
        cancel_setup_for("test-pip-queue");
        let result = tokio::time::timeout(Duration::from_secs(2), task)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(result, Err(CANCELLED.into()));
        assert!(!root.join("args").exists());
        assert!(!is_cancelled());
        drop(guard);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn extra_pip_cancellation_kills_owned_process_and_stops_next_package() {
        let _serial = TEST_SETUP_LOCK.lock().await;
        let root = TempDir::new("python_env_pip_cancel");
        let pid_file = root.join("pid");
        let python = mock_python(
            &root,
            "",
            &format!("echo $$ > '{}'; sleep 30", pid_file.display()),
        );
        let app = tauri::test::mock_app();
        let events = capture_events(app.handle(), DOWNLOAD_EVENT);
        let installer = test_installer(&app);
        let task = tokio::spawn(async move {
            install_for(
                installer,
                PathBuf::from(python),
                &["first", "second"],
                "test-pip-running",
            )
            .await
        });
        let pid = wait_for_pid(&pid_file).await;
        assert_eq!(*SETUP_OWNER.lock().unwrap(), Some("test-pip-running"));
        cancel_setup_for("test-pip-running");
        let result = tokio::time::timeout(Duration::from_secs(3), task)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(result, Err(CANCELLED.into()));
        assert!(!std::fs::read_to_string(root.join("args"))
            .unwrap()
            .contains("second"));
        assert!(crate::commands::test_support::wait_process_gone(pid));
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
        let root = TempDir::new("python_env_pip_error");
        let python = mock_python(&root, "", "printf 'mock failure' >&2; exit 9");
        let app = tauri::test::mock_app();
        let events = capture_events(app.handle(), DOWNLOAD_EVENT);
        let error = install_for(
            test_installer(&app),
            PathBuf::from(python),
            &["first", "second"],
            "test-pip-error",
        )
        .await
        .unwrap_err();
        assert!(error.contains("mock failure"));
        assert!(!std::fs::read_to_string(root.join("args"))
            .unwrap()
            .contains("second"));
        assert_eq!(*SETUP_OWNER.lock().unwrap(), None);
        assert_eq!(events.lock().unwrap().last().unwrap()["status"], "error");
    }

    /// 删不掉的文件（这里用只读目录模拟 Windows 上被占用的文件）逐个列出，能删的照常删掉
    #[cfg(unix)]
    #[test]
    fn env_removal_deletes_what_it_can_and_lists_the_rest() {
        use std::os::unix::fs::PermissionsExt;
        let root = TempDir::new("python_env_reset");
        let env = root.join("python");
        std::fs::create_dir_all(env.join("venv").join("lib")).unwrap();
        std::fs::write(env.join("venv").join("lib").join("a.py"), "a").unwrap();
        let locked = env.join("base").join("locked");
        std::fs::create_dir_all(&locked).unwrap();
        std::fs::write(locked.join("held.dll"), "x").unwrap();
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o500)).unwrap();
        // root 不受目录权限限制，这时模拟不出删不掉的文件
        if std::fs::write(locked.join("probe"), "").is_ok() {
            std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o700)).unwrap();
            return;
        }
        let result = remove_dir_reporting(&env);
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o700)).unwrap();
        let error = result.unwrap_err();
        assert!(error.starts_with("1 个文件无法删除: "), "{error}");
        assert!(error.contains("held.dll"), "{error}");
        assert!(!env.join("venv").exists());
        assert!(locked.join("held.dll").exists());
        assert_eq!(remove_dir_reporting(&env), Ok(()));
        assert!(!env.exists());
        assert_eq!(remove_dir_reporting(&env), Ok(()));
    }

    #[cfg(unix)]
    #[test]
    fn unpacked_python_replaces_base_dir_and_cleans_up() {
        let root = TempDir::new("python_env_unpack");
        let source = root.join("source");
        std::fs::create_dir_all(source.join("python").join("bin")).unwrap();
        std::fs::write(source.join("python").join("bin").join("python3"), "exe").unwrap();
        let env_dir = root.join("env");
        std::fs::create_dir_all(&env_dir).unwrap();
        let archive = env_dir.join("python_download.tar.gz");
        let packed = PythonCommand::new("tar")
            .arg("czf")
            .arg(&archive)
            .arg("-C")
            .arg(&source)
            .arg("python")
            .output()
            .unwrap();
        assert!(packed.status.success());
        let base = env_dir.join("python").join("base");
        std::fs::create_dir_all(base.join("stale")).unwrap();
        unpack_python(&archive, &env_dir, &base).unwrap();
        assert_eq!(
            std::fs::read_to_string(base.join("bin").join("python3")).unwrap(),
            "exe"
        );
        assert!(!base.join("stale").exists());
        assert!(!archive.exists());
        assert!(!env_dir.join("_python_extract_tmp").exists());
    }
}

/// 用伪 Python 覆盖部署、补装、换装与恢复流程
#[cfg(all(test, unix))]
mod venv_tests {
    use super::tests::{test_installer, wait_for_pid, TEST_SETUP_LOCK};
    use super::*;
    use crate::commands::batch::capture_events;
    use crate::commands::test_support::{wait_process_gone, TempDir};

    /// 伪 Python（sh 脚本），按状态目录里的文件模拟 onnxruntime 的安装状态和 pip 的结果：
    /// - `ort`：已装的 onnxruntime（cpu / gpu），没有这个文件就是没装；
    /// - `version`：`--version` 和 venv 探测输出的版本，默认 Python 3.12.4；
    /// - `noise`：每次 `-c` 先打印到 stdout 的内容（venv 里会打印东西的 .pth）；
    /// - 开关文件 `gpu_fails` / `gpu_hangs` / `cpu_fails` / `numpy_fails`：对应的 pip install 失败或卡住；
    /// - `calls`：pip / venv 调用的完整参数，每次一行；`env`：pip 看到的代理环境变量。
    ///
    /// `-m venv <dir>` 把模板复制成 `<dir>/bin/python3`
    const FAKE_PYTHON: &str = r#"#!/bin/sh
S='{state}'
version() { if [ -f "$S/version" ]; then cat "$S/version"; else echo 'Python 3.12.4'; fi; }
if [ "$1" = --version ]; then version; exit 0; fi
if [ "$1" = -c ]; then
  if [ -f "$S/noise" ]; then cat "$S/noise"; fi
  case "$2" in
    *importlib.metadata*)
      if [ ! -f "$S/ort" ]; then echo '|0'
      elif [ "$(cat "$S/ort")" = gpu ]; then echo 'CUDAExecutionProvider,CPUExecutionProvider|1'
      else echo 'CPUExecutionProvider|0'; fi
      exit 0 ;;
    'import onnxruntime') [ -f "$S/ort" ]; exit $? ;;
    'import venv') exit 0 ;;
    *'import pip'*) version; exit 0 ;;
  esac
  exit 1
fi
printf '%s\n' "$*" >> "$S/calls"
env | grep -E '^(HTTPS?_PROXY|https?_proxy)=' >> "$S/env"
case "$*" in
  '-m venv '*)
    mkdir -p "$3/bin" && cp '{template}' "$3/bin/python3" && chmod 700 "$3/bin/python3"
    exit $? ;;
  *'pip uninstall'*) rm -f "$S/ort"; exit 0 ;;
  *onnxruntime-gpu==*)
    if [ -f "$S/gpu_hangs" ]; then echo $$ > "$S/pid"; sleep 30; fi
    if [ -f "$S/gpu_fails" ]; then echo 'no matching distribution for onnxruntime-gpu' >&2; exit 1; fi
    echo gpu > "$S/ort"; exit 0 ;;
  *onnxruntime==*)
    if [ -f "$S/cpu_fails" ]; then echo 'no matching distribution for onnxruntime' >&2; exit 1; fi
    echo cpu > "$S/ort"; exit 0 ;;
  *numpy==*)
    if [ -f "$S/numpy_fails" ]; then echo 'numpy build failed' >&2; exit 1; fi
    exit 0 ;;
esac
exit 0
"#;

    struct FakePython {
        path: PathBuf,
        state: PathBuf,
    }

    impl FakePython {
        /// 在 `path` 建伪 Python，状态放在 `state`；`-m venv` 时复制 `venv_template`
        fn create(path: &Path, state: &Path, venv_template: &Path) -> Self {
            use std::os::unix::fs::PermissionsExt;
            std::fs::create_dir_all(state).unwrap();
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            let script = FAKE_PYTHON
                .replace("{state}", &state.to_string_lossy())
                .replace("{template}", &venv_template.to_string_lossy());
            std::fs::write(path, script).unwrap();
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).unwrap();
            FakePython {
                path: path.to_path_buf(),
                state: state.to_path_buf(),
            }
        }

        /// `<root>/<name>/python3`，`-m venv` 时复制自己
        fn standalone(root: &Path, name: &str) -> Self {
            let path = root.join(name).join("python3");
            Self::create(&path, &root.join(name).join("state"), &path)
        }

        fn set(&self, name: &str, content: &str) {
            std::fs::write(self.state.join(name), content).unwrap();
        }

        fn ort(&self) -> Option<String> {
            std::fs::read_to_string(self.state.join("ort"))
                .ok()
                .map(|s| s.trim().to_string())
        }

        fn calls(&self) -> Vec<String> {
            std::fs::read_to_string(self.state.join("calls"))
                .unwrap_or_default()
                .lines()
                .map(str::to_string)
                .collect()
        }
    }

    const UNINSTALL_ORT: &str = "-m pip uninstall -y onnxruntime onnxruntime-gpu";

    fn pip_install(dep: &str) -> String {
        format!("-m pip install --disable-pip-version-check --no-cache-dir {dep}")
    }

    fn test_layout(root: &Path) -> EnvLayout {
        EnvLayout {
            env_dir: root.join("env"),
        }
    }

    /// GPU 包装不上、CPU 版已装回：发一条警告，任务按 CPU 版继续；
    /// 同一会话之后的任务不再换装，同样按 CPU 版继续
    #[tokio::test]
    async fn failed_gpu_switch_falls_back_to_the_cpu_package() {
        static STATE: onnx_gpu::OrtUpgrade = onnx_gpu::OrtUpgrade::new();
        let _serial = TEST_SETUP_LOCK.lock().await;
        let root = TempDir::new("python_env_switch_fail");
        let python = FakePython::standalone(&root, "venv");
        python.set("ort", "cpu");
        python.set("gpu_fails", "");
        let app = tauri::test::mock_app();
        let events = capture_events(app.handle(), PROGRESS_EVENT);
        let ensure = || {
            onnx_gpu::ensure(
                test_installer(&app),
                python.path.clone(),
                "test-switch-fail",
                &STATE,
                async { true },
            )
        };
        ensure().await.unwrap();
        assert_eq!(python.ort().as_deref(), Some("cpu"));
        assert_eq!(
            python.calls(),
            [
                UNINSTALL_ORT.to_string(),
                pip_install("onnxruntime-gpu==1.25.1"),
                UNINSTALL_ORT.to_string(),
                pip_install("onnxruntime==1.25.1"),
            ]
        );
        assert_eq!(STATE.phase(), onnx_gpu::Phase::Settled);
        let warnings: Vec<String> = events
            .lock()
            .unwrap()
            .iter()
            .filter(|e| e["status"] == "warning")
            .map(|e| e["message"].as_str().unwrap_or_default().to_string())
            .collect();
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        assert!(
            warnings[0].starts_with(
                "@pythonEnv.gpuRuntimeFallback|安装 onnxruntime-gpu==1.25.1 失败: no matching distribution"
            ),
            "{warnings:?}"
        );

        ensure().await.unwrap();
        assert_eq!(python.calls().len(), 4);
    }

    #[tokio::test]
    async fn cancelled_gpu_switch_still_reinstalls_cpu_package() {
        let _serial = TEST_SETUP_LOCK.lock().await;
        let _owner = acquire_setup("test-switch-cancel").await.unwrap();
        let root = TempDir::new("python_env_switch_cancel");
        let python = FakePython::standalone(&root, "venv");
        python.set("ort", "cpu");
        python.set("gpu_hangs", "");
        let app = tauri::test::mock_app();
        let events = capture_events(app.handle(), DOWNLOAD_EVENT);
        let progress = capture_events(app.handle(), PROGRESS_EVENT);
        let installer = test_installer(&app);
        let path = python.path.clone();
        let task = tokio::task::spawn_blocking(move || installer.switch_to_gpu(&path));
        let pid = wait_for_pid(&python.state.join("pid")).await;
        cancel_setup_for("test-switch-cancel");
        let result = tokio::time::timeout(Duration::from_secs(5), task)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(result, Err(CANCELLED.to_string()));
        assert!(wait_process_gone(pid));
        assert_eq!(python.ort().as_deref(), Some("cpu"));
        assert_eq!(
            python.calls().last(),
            Some(&pip_install("onnxruntime==1.25.1"))
        );
        // 取消不是换装失败，不发改用 CPU 版的警告
        assert!(!progress
            .lock()
            .unwrap()
            .iter()
            .any(|e| e["status"] == "warning"));
        // GPU 包那次显示为取消，装回 CPU 版那次照常走完
        let statuses: Vec<String> = events
            .lock()
            .unwrap()
            .iter()
            .map(|e| e["status"].as_str().unwrap_or_default().to_string())
            .collect();
        assert_eq!(statuses.iter().filter(|s| *s == "cancelled").count(), 1);
        assert_eq!(statuses.last().map(String::as_str), Some("done"));
    }

    #[tokio::test]
    async fn gpu_switch_keeps_an_importable_gpu_package() {
        let _serial = TEST_SETUP_LOCK.lock().await;
        let _owner = acquire_setup("test-switch-ok").await.unwrap();
        let root = TempDir::new("python_env_switch_ok");
        let python = FakePython::standalone(&root, "venv");
        python.set("ort", "cpu");
        let app = tauri::test::mock_app();
        let installer = test_installer(&app);
        let path = python.path.clone();
        blocking(move || installer.switch_to_gpu(&path))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(python.ort().as_deref(), Some("gpu"));
        assert_eq!(
            python.calls(),
            [
                UNINSTALL_ORT.to_string(),
                pip_install("onnxruntime-gpu==1.25.1")
            ]
        );
    }

    #[tokio::test]
    async fn failed_restore_reports_both_errors() {
        let _serial = TEST_SETUP_LOCK.lock().await;
        let _owner = acquire_setup("test-restore-fail").await.unwrap();
        let root = TempDir::new("python_env_restore_fail");
        let python = FakePython::standalone(&root, "venv");
        python.set("ort", "cpu");
        python.set("gpu_fails", "");
        python.set("cpu_fails", "");
        let app = tauri::test::mock_app();
        let installer = test_installer(&app);
        let path = python.path.clone();
        let error = blocking(move || installer.switch_to_gpu(&path))
            .await
            .unwrap()
            .unwrap_err();
        assert!(
            error.starts_with("安装 onnxruntime-gpu==1.25.1 失败"),
            "{error}"
        );
        assert!(
            error.contains("恢复 CPU 版 onnxruntime 失败: 安装 onnxruntime==1.25.1 失败"),
            "{error}"
        );
        assert_eq!(python.ort(), None);
    }

    /// 排队等锁的功能拿到锁时换装已经结束：那次连 CPU 版都没装回来的话，由它装回
    #[tokio::test]
    async fn queued_feature_restores_onnxruntime_after_a_failed_switch() {
        static STATE: onnx_gpu::OrtUpgrade = onnx_gpu::OrtUpgrade::new();
        let _serial = TEST_SETUP_LOCK.lock().await;
        let _owner = acquire_setup("test-switch-queued").await.unwrap();
        assert!(STATE.begin());
        STATE.settle();
        let root = TempDir::new("python_env_switch_queued");
        let python = FakePython::standalone(&root, "venv");
        let app = tauri::test::mock_app();
        let verify = |installer: Installer<_>, path: PathBuf| {
            blocking(move || STATE.switch_or_verify(&installer, &path))
        };
        verify(test_installer(&app), python.path.clone())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(python.ort().as_deref(), Some("cpu"));
        assert_eq!(
            python.calls(),
            [
                UNINSTALL_ORT.to_string(),
                pip_install("onnxruntime==1.25.1")
            ]
        );
        // 能导入时什么都不装
        verify(test_installer(&app), python.path.clone())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(python.calls().len(), 2);
    }

    #[tokio::test]
    async fn gpu_switch_runs_once_and_only_with_nvidia() {
        static STATE: onnx_gpu::OrtUpgrade = onnx_gpu::OrtUpgrade::new();
        let _serial = TEST_SETUP_LOCK.lock().await;
        let root = TempDir::new("python_env_switch_once");
        let python = FakePython::standalone(&root, "venv");
        python.set("ort", "cpu");
        let app = tauri::test::mock_app();
        let ensure = |has_nvidia: bool| {
            onnx_gpu::ensure(
                test_installer(&app),
                python.path.clone(),
                "test-switch-once",
                &STATE,
                async move { has_nvidia },
            )
        };
        ensure(false).await.unwrap();
        assert!(python.calls().is_empty());
        assert_eq!(STATE.phase(), onnx_gpu::Phase::NotTried);

        ensure(true).await.unwrap();
        assert_eq!(python.ort().as_deref(), Some("gpu"));
        assert_eq!(STATE.phase(), onnx_gpu::Phase::Settled);
        assert_eq!(python.calls().len(), 2);
        assert_eq!(*SETUP_OWNER.lock().unwrap(), None);

        // 本会话已经换装过：不再探测、不再安装
        python.set("ort", "cpu");
        ensure(true).await.unwrap();
        assert_eq!(python.calls().len(), 2);
    }

    #[tokio::test]
    async fn other_features_wait_for_a_running_switch() {
        static STATE: onnx_gpu::OrtUpgrade = onnx_gpu::OrtUpgrade::new();
        let _serial = TEST_SETUP_LOCK.lock().await;
        assert!(STATE.begin());
        let holder = acquire_setup("test-switch-running").await.unwrap();
        let root = TempDir::new("python_env_switch_running");
        // 换装进行中：venv 里暂时没有 onnxruntime
        let python = FakePython::standalone(&root, "venv");
        let app = tauri::test::mock_app();
        let task = tokio::spawn(onnx_gpu::ensure(
            test_installer(&app),
            python.path.clone(),
            "test-switch-waiter",
            &STATE,
            async { true },
        ));
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert!(!task.is_finished());
        python.set("ort", "gpu");
        STATE.settle();
        drop(holder);
        tokio::time::timeout(Duration::from_secs(5), task)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert!(python.calls().is_empty());
    }

    #[tokio::test]
    async fn usable_venv_is_repaired_in_place() {
        let _serial = TEST_SETUP_LOCK.lock().await;
        let _owner = acquire_setup("test-repair").await.unwrap();
        let root = TempDir::new("python_env_repair");
        let layout = test_layout(&root);
        let venv = FakePython::create(
            &layout.venv_python(),
            &root.join("venv_state"),
            &root.join("unused"),
        );
        let extra = layout.venv_dir().join("lib").join("torch");
        std::fs::create_dir_all(&extra).unwrap();
        let app = tauri::test::mock_app();
        let events = capture_events(app.handle(), PROGRESS_EVENT);
        let python = test_installer(&app).setup(&layout, || None).await.unwrap();
        assert_eq!(python, layout.venv_python());
        assert!(extra.exists());
        assert_eq!(venv.ort().as_deref(), Some("cpu"));
        assert_eq!(
            venv.calls(),
            [
                UNINSTALL_ORT.to_string(),
                pip_install("onnxruntime==1.25.1"),
                pip_install("numpy==2.2.6"),
                pip_install("pillow==11.3.0"),
            ]
        );
        assert_eq!(
            events.lock().unwrap().last().unwrap()["message"],
            "@pythonEnv.ready"
        );
    }

    /// venv 里的 .pth 先往 stdout 打印东西：版本探测只看最后一行，照常就地补装，不删掉重建
    #[tokio::test]
    async fn venv_with_site_hook_output_is_repaired_in_place() {
        let _serial = TEST_SETUP_LOCK.lock().await;
        let _owner = acquire_setup("test-repair-noise").await.unwrap();
        let root = TempDir::new("python_env_repair_noise");
        let layout = test_layout(&root);
        let venv = FakePython::create(
            &layout.venv_python(),
            &root.join("venv_state"),
            &root.join("unused"),
        );
        venv.set("noise", "site hook loaded\n");
        let extra = layout.venv_dir().join("lib").join("torch");
        std::fs::create_dir_all(&extra).unwrap();
        // 误判为不可用时会改用这个独立版重建，不会去下载
        let fresh = FakePython::standalone(&root, "fresh_venv");
        let base = FakePython::create(
            &layout.standalone_python(),
            &root.join("base_state"),
            &fresh.path,
        );
        let app = tauri::test::mock_app();
        let python = test_installer(&app).setup(&layout, || None).await.unwrap();
        assert_eq!(python, layout.venv_python());
        assert!(extra.exists());
        assert!(base.calls().is_empty());
        assert_eq!(venv.ort().as_deref(), Some("cpu"));
    }

    #[tokio::test]
    async fn failed_repair_keeps_the_venv() {
        let _serial = TEST_SETUP_LOCK.lock().await;
        let _owner = acquire_setup("test-repair-fail").await.unwrap();
        let root = TempDir::new("python_env_repair_fail");
        let layout = test_layout(&root);
        let venv = FakePython::create(
            &layout.venv_python(),
            &root.join("venv_state"),
            &root.join("unused"),
        );
        venv.set("numpy_fails", "");
        let extra = layout.venv_dir().join("lib").join("torch");
        std::fs::create_dir_all(&extra).unwrap();
        let app = tauri::test::mock_app();
        let error = test_installer(&app)
            .setup(&layout, || None)
            .await
            .unwrap_err();
        assert!(error.contains("numpy build failed"), "{error}");
        assert!(extra.exists());
        assert!(layout.venv_python().exists());
        assert!(!venv.calls().iter().any(|call| call.starts_with("-m venv")));
    }

    #[tokio::test]
    async fn venv_on_unsupported_python_is_rebuilt_with_standalone_python() {
        let _serial = TEST_SETUP_LOCK.lock().await;
        let _owner = acquire_setup("test-rebuild").await.unwrap();
        let root = TempDir::new("python_env_rebuild");
        let layout = test_layout(&root);
        let old = FakePython::create(
            &layout.venv_python(),
            &root.join("old_state"),
            &root.join("unused"),
        );
        old.set("version", "Python 3.14.0");
        let extra = layout.venv_dir().join("lib").join("torch");
        std::fs::create_dir_all(&extra).unwrap();
        let fresh = FakePython::standalone(&root, "fresh_venv");
        let base = FakePython::create(
            &layout.standalone_python(),
            &root.join("base_state"),
            &fresh.path,
        );
        let app = tauri::test::mock_app();
        let python = test_installer(&app).setup(&layout, || None).await.unwrap();
        assert_eq!(python, layout.venv_python());
        assert!(!extra.exists());
        assert!(old.calls().is_empty());
        assert_eq!(
            base.calls(),
            [format!("-m venv {}", layout.venv_dir().display())]
        );
        assert_eq!(fresh.ort().as_deref(), Some("cpu"));
    }

    #[tokio::test]
    async fn system_python_dependency_failure_falls_back_to_standalone() {
        let _serial = TEST_SETUP_LOCK.lock().await;
        let _owner = acquire_setup("test-fallback").await.unwrap();
        let root = TempDir::new("python_env_fallback");
        let layout = test_layout(&root);
        let system_venv = FakePython::standalone(&root, "system_venv");
        system_venv.set("numpy_fails", "");
        let system = FakePython::create(
            &root.join("system").join("python3"),
            &root.join("system_state"),
            &system_venv.path,
        );
        let fresh = FakePython::standalone(&root, "standalone_venv");
        let base = FakePython::create(
            &layout.standalone_python(),
            &root.join("base_state"),
            &fresh.path,
        );
        let app = tauri::test::mock_app();
        let events = capture_events(app.handle(), PROGRESS_EVENT);
        let system_path = system.path.clone();
        let python = test_installer(&app)
            .setup(&layout, move || Some(system_path))
            .await
            .unwrap();
        assert_eq!(python, layout.venv_python());
        assert!(system_venv.calls().contains(&pip_install("numpy==2.2.6")));
        assert_eq!(system.calls().len(), 1);
        assert_eq!(base.calls().len(), 1);
        assert_eq!(fresh.ort().as_deref(), Some("cpu"));
        let messages: Vec<String> = events
            .lock()
            .unwrap()
            .iter()
            .map(|e| e["message"].as_str().unwrap_or_default().to_string())
            .collect();
        assert!(
            messages.iter().any(|m| m
                .starts_with("@pythonEnv.venvFailed|安装 numpy==2.2.6 失败: numpy build failed")),
            "{messages:?}"
        );
        assert_eq!(
            messages.last().map(String::as_str),
            Some("@pythonEnv.ready")
        );
    }

    #[test]
    fn system_python_outside_311_313_is_skipped() {
        let root = TempDir::new("python_env_system_version");
        let python = FakePython::standalone(&root, "system");
        let program = python.path.to_string_lossy().into_owned();
        for (version, usable) in [
            ("Python 3.9.18", false),
            ("Python 3.10.14", false),
            ("Python 3.11.9", true),
            ("Python 3.13.1", true),
            ("Python 3.14.0", false),
        ] {
            python.set("version", version);
            assert_eq!(usable_system_python(&program), usable, "{version}");
        }
    }

    #[tokio::test]
    async fn pip_gets_the_installer_proxy_env() {
        let root = TempDir::new("python_env_proxy");
        let python = FakePython::standalone(&root, "venv");
        let app = tauri::test::mock_app();
        let proxy = "http://127.0.0.1:9".to_string();
        let installer = Installer::with_proxy_env(
            app.handle(),
            vec![("HTTP_PROXY", proxy.clone()), ("https_proxy", proxy)],
        );
        let path = python.path.clone();
        blocking(move || installer.install(&path, &["dep"], &AtomicBool::new(false)))
            .await
            .unwrap()
            .unwrap();
        let env = std::fs::read_to_string(python.state.join("env")).unwrap();
        assert!(
            env.lines().any(|l| l == "HTTP_PROXY=http://127.0.0.1:9"),
            "{env}"
        );
        assert!(
            env.lines().any(|l| l == "https_proxy=http://127.0.0.1:9"),
            "{env}"
        );
    }
}
