use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use tauri::Emitter;

use super::http_download::{download_client, download_to_file, DownloadProgress};
use super::image_io::{probe_image, save_like_source, SourceInfo};
use super::python_proc::{self, PidRegistration};
use super::{
    collect_image_files_with_recursive_excluding, output_path_for_input, ProcessResult,
    ProgressEvent,
};

/// 超分子进程句柄（Python 或 NCNN），用于强制取消
static ACTIVE_CHILD: Mutex<Option<u32>> = Mutex::new(None);

// ===== Upscale Engine Definitions =====

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpscaleEngineInfo {
    pub id: String,
    pub name: String,
    pub downloaded: bool,
    pub scales: Vec<u32>,
    pub models: Vec<UpscaleModelChoice>,
    pub supports_denoise: bool,
    pub denoise_range: (i32, i32),
    pub supports_cpu: bool,
    pub use_python: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpscaleModelChoice {
    pub id: String,
    pub name: String,
}

struct EngineDef {
    id: &'static str,
    name: &'static str,
    bin_name: &'static str,
    scales: &'static [u32],
    supports_denoise: bool,
    denoise_range: (i32, i32),
    supports_cpu: bool,
    use_python: bool,
    /// (id, name)；NCNN 引擎的 id 同时是引擎目录下的模型目录名
    models: &'static [(&'static str, &'static str)],
    download_url: &'static str,
}

const ENGINES: &[EngineDef] = &[
    EngineDef {
        id: "realcugan",
        name: "Real-CUGAN",
        bin_name: "realcugan-ncnn-vulkan",
        use_python: false,
        scales: &[2, 3, 4],
        supports_denoise: true,
        denoise_range: (-1, 3),
        supports_cpu: true,
        models: &[
            ("models-se", "标准版 (SE)"),
            ("models-pro", "Pro版"),
            ("models-nose", "无降噪版"),
        ],
        #[cfg(target_os = "macos")]
        download_url: "https://github.com/nihui/realcugan-ncnn-vulkan/releases/download/20220728/realcugan-ncnn-vulkan-20220728-macos.zip",
        #[cfg(target_os = "windows")]
        download_url: "https://github.com/nihui/realcugan-ncnn-vulkan/releases/download/20220728/realcugan-ncnn-vulkan-20220728-windows.zip",
        #[cfg(target_os = "linux")]
        download_url: "https://github.com/nihui/realcugan-ncnn-vulkan/releases/download/20220728/realcugan-ncnn-vulkan-20220728-ubuntu.zip",
    },
    EngineDef {
        id: "realesrgan",
        name: "Real-ESRGAN",
        bin_name: "",
        scales: &[2, 4],
        supports_denoise: false,
        denoise_range: (0, 0),
        supports_cpu: true,
        use_python: true,
        models: &[
            ("realesrgan-x4plus", "通用 (x4plus)"),
            ("realesrgan-x4plus-anime", "动漫 (x4plus-anime)"),
        ],
        #[cfg(target_os = "macos")]
        download_url: "",
        #[cfg(target_os = "windows")]
        download_url: "",
        #[cfg(target_os = "linux")]
        download_url: "",
    },
    EngineDef {
        id: "waifu2x",
        name: "Waifu2x",
        bin_name: "waifu2x-ncnn-vulkan",
        use_python: false,
        scales: &[1, 2, 4, 8],
        supports_denoise: true,
        denoise_range: (-1, 3),
        supports_cpu: false,
        models: &[
            ("models-cunet", "动漫 (CUNet)"),
            ("models-upconv_7_anime_style_art_rgb", "动漫 (轻量)"),
            ("models-upconv_7_photo", "真实照片"),
        ],
        #[cfg(target_os = "macos")]
        download_url: "https://github.com/nihui/waifu2x-ncnn-vulkan/releases/download/20250915/waifu2x-ncnn-vulkan-20250915-macos.zip",
        #[cfg(target_os = "windows")]
        download_url: "https://github.com/nihui/waifu2x-ncnn-vulkan/releases/download/20250915/waifu2x-ncnn-vulkan-20250915-windows.zip",
        #[cfg(target_os = "linux")]
        download_url: "https://github.com/nihui/waifu2x-ncnn-vulkan/releases/download/20250915/waifu2x-ncnn-vulkan-20250915-linux.zip",
    },
];

// ===== Paths =====

fn get_upscale_dir() -> PathBuf {
    super::config_paths::models_dir("upscale_engines")
}

fn engine_dir(engine_id: &str) -> PathBuf {
    get_upscale_dir().join(engine_id)
}

fn engine_binary(engine: &EngineDef) -> PathBuf {
    let dir = engine_dir(engine.id);
    #[cfg(target_os = "windows")]
    {
        dir.join(format!("{}.exe", engine.bin_name))
    }
    #[cfg(not(target_os = "windows"))]
    {
        dir.join(engine.bin_name)
    }
}

const ESRGAN_MODELS: &[(&str, &str, &str)] = &[
    ("realesrgan-x4plus", "RealESRGAN_x4plus.onnx", "https://github.com/YPuddin-Neko/PurinBox/releases/download/models/RealESRGAN_x4plus.onnx"),
    ("realesrgan-x4plus-anime", "RealESRGAN_x4plus_anime_6B.onnx", "https://github.com/YPuddin-Neko/PurinBox/releases/download/models/RealESRGAN_x4plus_anime_6B.onnx"),
];

fn realesrgan_weights_dir() -> PathBuf {
    engine_dir("realesrgan")
}

fn all_esrgan_weights_ready(dir: &Path) -> bool {
    ESRGAN_MODELS
        .iter()
        .all(|(_, file, _)| dir.join(file).is_file())
}

async fn is_engine_ready(engine: &EngineDef) -> bool {
    if engine.use_python {
        if !all_esrgan_weights_ready(&realesrgan_weights_dir()) {
            return false;
        }
        let python = super::python_env::get_venv_python();
        return python.is_file()
            && super::python_env::probe_python(
                &python.to_string_lossy(),
                "import onnxruntime, cv2",
            )
            .await
            .is_some();
    }
    // 解压中断可能留下二进制但没有模型目录。
    let dir = engine_dir(engine.id);
    engine_binary(engine).is_file() && engine.models.iter().any(|m| dir.join(m.0).is_dir())
}

#[tauri::command]
pub async fn get_upscale_engines() -> Result<Vec<UpscaleEngineInfo>, String> {
    let mut engines = Vec::new();
    for e in ENGINES {
        engines.push(UpscaleEngineInfo {
            id: e.id.into(),
            name: e.name.into(),
            downloaded: is_engine_ready(e).await,
            scales: e.scales.to_vec(),
            models: e
                .models
                .iter()
                .map(|(id, name)| UpscaleModelChoice {
                    id: (*id).into(),
                    name: (*name).into(),
                })
                .collect(),
            supports_denoise: e.supports_denoise,
            denoise_range: e.denoise_range,
            supports_cpu: e.supports_cpu,
            use_python: e.use_python,
        });
    }
    Ok(engines)
}

// ===== Download =====

static DOWNLOAD_CANCEL: AtomicBool = AtomicBool::new(false);

fn check_download_cancelled() -> Result<(), String> {
    if DOWNLOAD_CANCEL.load(Ordering::SeqCst) {
        Err("下载已取消".into())
    } else {
        Ok(())
    }
}

#[tauri::command]
pub async fn download_upscale_engine(
    app: tauri::AppHandle,
    engine_id: String,
) -> Result<String, String> {
    let _busy = super::BusyGuard::acquire(&UPSCALE_RUNNING, "超分")?;
    DOWNLOAD_CANCEL.store(false, Ordering::SeqCst);
    CANCEL_FLAG.store(false, Ordering::SeqCst);
    let engine = ENGINES
        .iter()
        .find(|e| e.id == engine_id)
        .ok_or_else(|| format!("未知引擎: {}", engine_id))?;
    let outcome = async {
        if is_engine_ready(engine).await {
            check_download_cancelled()?;
            return Ok("already_ready".to_string());
        }
        check_download_cancelled()?;
        if engine.use_python {
            download_python_engine(&app).await?;
        } else {
            let dest_dir = engine_dir(engine.id);
            std::fs::create_dir_all(&dest_dir).map_err(|e| format!("创建目录失败: {}", e))?;
            let zip_path = dest_dir.join("_download.zip");
            let client = download_client()?;
            let _ = app.emit(
                "upscale-download",
                DownloadProgress::new("downloading", 0.0, format!("正在下载 {} ...", engine.name)),
            );
            download_to_file(
                client.get(engine.download_url),
                &zip_path,
                engine.name,
                &DOWNLOAD_CANCEL,
                |p| {
                    let _ = app.emit("upscale-download", p);
                },
            )
            .await
            .map_err(String::from)?;
            check_download_cancelled()?;
            let _ = app.emit(
                "upscale-download",
                DownloadProgress::new("extracting", 99.0, format!("{} — 正在解压...", engine.name)),
            );
            let archive = zip_path.clone();
            tokio::task::spawn_blocking(move || extract_zip(&archive, &dest_dir))
                .await
                .map_err(|e| format!("解压任务失败: {}", e))?
                .map_err(|e| format!("解压失败: {}", e))?;
            let _ = tokio::fs::remove_file(&zip_path).await;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(
                    engine_binary(engine),
                    std::fs::Permissions::from_mode(0o755),
                )
                .map_err(|e| format!("设置引擎权限失败: {}", e))?;
            }
        }
        check_download_cancelled()?;
        Ok("done".into())
    }
    .await;
    let progress = match &outcome {
        Ok(_) => DownloadProgress::done(format!("{} 已就绪", engine.name)),
        Err(_) if DOWNLOAD_CANCEL.load(Ordering::SeqCst) => {
            DownloadProgress::cancelled("下载已取消")
        }
        Err(error) => DownloadProgress::error(error),
    };
    let _ = app.emit("upscale-download", progress);
    outcome
}

fn extract_zip(zip_path: &Path, dest_dir: &Path) -> Result<(), String> {
    let file = std::fs::File::open(zip_path).map_err(|e| format!("打开zip失败: {}", e))?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| format!("读取zip失败: {}", e))?;

    // Find the prefix — zip files often have a top-level directory
    let mut prefix = String::new();
    if let Some(first) = archive.file_names().next() {
        if let Some(slash_pos) = first.find('/') {
            prefix = first[..=slash_pos].to_string();
        }
    }

    for i in 0..archive.len() {
        let mut entry = archive
            .by_index(i)
            .map_err(|e| format!("zip entry error: {}", e))?;
        let raw_name = entry.name().to_string();

        // Zip Slip 防护: enclosed_name() 对包含 ../、绝对路径等不安全条目返回 None，直接跳过
        if entry.enclosed_name().is_none() {
            eprintln!("[upscale] 跳过不安全的 zip 条目: {}", raw_name);
            continue;
        }

        let relative = if !prefix.is_empty() && raw_name.starts_with(&prefix) {
            &raw_name[prefix.len()..]
        } else {
            &raw_name
        };

        if relative.is_empty() {
            continue;
        }

        let out_path = dest_dir.join(relative);

        if entry.is_dir() {
            let _ = std::fs::create_dir_all(&out_path);
        } else {
            if let Some(parent) = out_path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let mut outfile = std::fs::File::create(&out_path)
                .map_err(|e| format!("创建文件失败 {}: {}", out_path.display(), e))?;
            std::io::copy(&mut entry, &mut outfile)
                .map_err(|e| format!("写入失败 {}: {}", out_path.display(), e))?;
        }
    }

    Ok(())
}

async fn download_python_engine(app: &tauri::AppHandle) -> Result<(), String> {
    let emit = |pct, message| {
        let _ = app.emit(
            "upscale-download",
            DownloadProgress::new("downloading", pct, message),
        );
    };
    emit(5.0, "正在检查 Python 环境...".to_string());
    let python = super::python_env::setup_python_env(app, "upscale").await?;
    check_download_cancelled()?;
    super::python_env::ensure_onnx_gpu_runtime(app, &python, "upscale").await?;
    check_download_cancelled()?;
    let has_cv2 = super::python_env::probe_python(&python, "import cv2")
        .await
        .is_some();
    check_download_cancelled()?;
    if !has_cv2 {
        emit(15.0, "正在安装 OpenCV...".into());
        super::python_env::pip_install_for(
            app,
            &python,
            &["opencv-python-headless==4.11.0.86"],
            "upscale",
        )
        .await?;
    }
    check_download_cancelled()?;
    let weights_dir = realesrgan_weights_dir();
    let client = download_client()?;
    for (index, (_, filename, url)) in ESRGAN_MODELS.iter().enumerate() {
        check_download_cancelled()?;
        let dest = weights_dir.join(filename);
        let span = 68.0 / ESRGAN_MODELS.len() as f32;
        let base = 30.0 + index as f32 * span;
        if dest.is_file() {
            emit(base + span, format!("{} 已存在，跳过", filename));
            continue;
        }
        emit(base, format!("正在下载 {}", filename));
        download_to_file(
            client.get(*url),
            &dest,
            filename,
            &DOWNLOAD_CANCEL,
            |mut p| {
                p.percent = base + p.percent * span / 100.0;
                let _ = app.emit("upscale-download", p);
            },
        )
        .await
        .map_err(String::from)?;
    }
    check_download_cancelled()
}

// ===== Upscale Processing =====

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpscaleOptions {
    pub input_path: String,
    pub output_path: String,
    pub engine_id: String,
    pub model_id: String,
    pub scale: u32,
    pub denoise_level: i32,
    pub tta: bool,
    pub gpu_id: i32,
    pub tile_size: i32,
    #[serde(default)]
    pub recursive: bool,
}

static CANCEL_FLAG: AtomicBool = AtomicBool::new(false);
/// 互斥标志：超分页面与工作流节点共用同一组全局状态（CANCEL_FLAG/ACTIVE_CHILD/进度事件），
/// 并发运行会互相清对方的取消标志与子进程 PID，必须串行。
static UPSCALE_RUNNING: AtomicBool = AtomicBool::new(false);

struct PlannedImage {
    input: PathBuf,
    output: Result<PathBuf, String>,
}

fn plan_upscale(
    options: &UpscaleOptions,
    python: bool,
) -> Result<(Vec<PlannedImage>, usize), String> {
    let input =
        std::path::absolute(&options.input_path).map_err(|e| format!("解析输入路径失败: {}", e))?;
    let output = std::path::absolute(&options.output_path)
        .map_err(|e| format!("解析输出路径失败: {}", e))?;
    let mut files =
        collect_image_files_with_recursive_excluding(&input, options.recursive, Some(&output))?;
    let before = files.len();
    files.retain(|path| {
        if python {
            !super::has_extension(path, &["gif"])
        } else {
            !super::has_extension(path, &["tif", "tiff"])
        }
    });
    let skipped = before - files.len();
    if files.is_empty() {
        return Err(if skipped > 0 {
            if python {
                "未找到可处理的图片（Real-ESRGAN 引擎不支持 GIF 格式）"
            } else {
                "未找到可处理的图片（NCNN 引擎不支持 TIFF 格式）"
            }
        } else {
            "未找到任何图片"
        }
        .into());
    }
    let inputs = files.iter().map(|p| super::path_key_ci(p)).collect();
    let mut outputs = std::collections::HashSet::new();
    let plan = files
        .into_iter()
        .map(|path| {
            let target = output_path_for_input(
                &input,
                &path,
                &output,
                &super::file_name_lossy(&path),
                options.recursive,
            )
            .and_then(|target| {
                claim_output(&path, &target, &inputs, &mut outputs)?;
                Ok(target)
            });
            PlannedImage {
                input: path,
                output: target,
            }
        })
        .collect();
    Ok((plan, skipped))
}

#[tauri::command]
pub async fn start_upscale(
    app: tauri::AppHandle,
    options: UpscaleOptions,
) -> Result<ProcessResult, String> {
    let _busy = super::BusyGuard::acquire(&UPSCALE_RUNNING, "超分")?;
    CANCEL_FLAG.store(false, Ordering::SeqCst);
    let engine = ENGINES
        .iter()
        .find(|e| e.id == options.engine_id)
        .ok_or_else(|| format!("未知引擎: {}", options.engine_id))?;
    tokio::task::spawn_blocking(move || {
        let (plan, skipped) = plan_upscale(&options, engine.use_python)?;
        let total = plan.len() as u32;
        let mut message = format!(
            "开始超分: 共 {} 张, 引擎: {}, 倍率: {}x",
            total, engine.name, options.scale
        );
        if skipped > 0 {
            message.push_str(&format!("（已跳过 {} 个不支持的格式）", skipped));
        }
        ProgressEvent::new("processing", message)
            .at(0, total)
            .emit(&app, "upscale-progress");
        let outcome = if engine.use_python {
            run_python_upscale(&app, &options, &plan)
        } else {
            run_ncnn_upscale(&app, engine, &options, &plan)
        };
        let cancelled = CANCEL_FLAG.load(Ordering::SeqCst);
        let result = if cancelled {
            outcome.unwrap_or(ProcessResult {
                total,
                ..Default::default()
            })
        } else {
            outcome?
        };
        terminal_event(&result, cancelled).emit(&app, "upscale-progress");
        Ok(result)
    })
    .await
    .map_err(|e| format!("任务执行失败: {}", e))?
}

struct NcnnBatch<'a> {
    bin: PathBuf,
    engine_dir: PathBuf,
    model: &'static str,
    options: &'a UpscaleOptions,
}

fn run_ncnn_upscale(
    app: &tauri::AppHandle,
    engine: &EngineDef,
    options: &UpscaleOptions,
    plan: &[PlannedImage],
) -> Result<ProcessResult, String> {
    let bin = std::path::absolute(engine_binary(engine))
        .map_err(|e| format!("解析引擎路径失败: {}", e))?;
    if !bin.is_file() {
        return Err(format!("{} 尚未下载，请先下载", engine.name));
    }
    let engine_dir = engine_dir(engine.id);
    let model = engine
        .models
        .iter()
        .find(|m| m.0 == options.model_id)
        .unwrap_or(&engine.models[0])
        .0;
    if !engine_dir.join(model).is_dir() {
        return Err(format!(
            "模型目录缺失: {}。请删除引擎目录后重新下载 {}",
            engine_dir.join(model).display(),
            engine.name
        ));
    }
    let batch = NcnnBatch {
        bin,
        engine_dir,
        model,
        options,
    };
    let mut result = ProcessResult {
        total: plan.len() as u32,
        ..Default::default()
    };
    for (i, item) in plan.iter().enumerate() {
        if CANCEL_FLAG.load(Ordering::SeqCst) {
            break;
        }
        let filename = super::file_name_lossy(&item.input);
        ProgressEvent::new(
            "processing",
            format!("[{}/{}] 正在处理: {}", i + 1, result.total, filename),
        )
        .at(i as u32, result.total)
        .file(&filename)
        .emit(app, "upscale-progress");
        let outcome = match &item.output {
            Ok(output) => probe_image(&item.input)
                .and_then(|source| upscale_one(&batch, &item.input, output, &source, i)),
            Err(error) => Err(error.clone()),
        };
        if CANCEL_FLAG.load(Ordering::SeqCst) && outcome.is_err() {
            break;
        }
        let (status, message) = match outcome {
            Ok(()) => {
                result.success_count += 1;
                (
                    "success",
                    format!("[{}/{}] ✓ {}", i + 1, result.total, filename),
                )
            }
            Err(error) => {
                result.fail_count += 1;
                let message = format!("{}: {}", filename, error);
                result.errors.push(message.clone());
                (
                    "error",
                    format!("[{}/{}] ✗ {}", i + 1, result.total, message),
                )
            }
        };
        ProgressEvent::new(status, message)
            .at(i as u32 + 1, result.total)
            .file(filename)
            .emit(app, "upscale-progress");
    }
    Ok(result)
}

/// 检查输出路径并登记占用；Err 为跳过原因
fn claim_output(
    file_path: &Path,
    out_file: &Path,
    input_set: &std::collections::HashSet<String>,
    used_outputs: &mut std::collections::HashSet<String>,
) -> Result<(), String> {
    // 大小写不敏感比较：photo.PNG 的输出 photo.png 在 Windows/macOS 上就是它自己
    let out_key = crate::commands::path_key_ci(out_file);
    // 输出与输入是同一个文件（输出目录就是输入目录）：跳过，避免原图被就地覆盖
    if out_key == crate::commands::path_key_ci(file_path) {
        return Err("输出与输入为同一文件，已跳过（请更换输出目录）".into());
    }
    // 输出名撞上另一张源图：跳过，避免把别的原图覆盖掉
    if input_set.contains(&out_key) {
        return Err("输出会覆盖另一张源图，已跳过（请更换输出目录）".into());
    }
    // 两张源图映射到同一个输出（大小写不同的同名文件）：后到者报错而不是静默覆盖
    if !used_outputs.insert(out_key) {
        return Err("输出文件名与其他输入冲突，已跳过".into());
    }
    Ok(())
}

/// 用 NCNN 超分一张图并写到 out_file；Err 为失败原因（不含文件名）
fn upscale_one(
    batch: &NcnnBatch,
    file_path: &Path,
    out_file: &Path,
    source: &SourceInfo,
    index: usize,
) -> Result<(), String> {
    let options = batch.options;
    // PNG 也经过格式写回，恢复源图 ICC，且引擎失败时保留已有输出。
    let engine_out = std::env::temp_dir().join(format!(
        "purinbox-upscale-{}-{}.png",
        std::process::id(),
        index
    ));
    let _ = std::fs::remove_file(&engine_out);
    let mut cmd = python_proc::hidden_command(&batch.bin);

    // NCNN 的 Windows 构建会把 -m 按 exe 所在目录再拼一次，传绝对路径会变成
    // {engine_dir}\{engine_dir}\models-xx 而找不到模型。因此 -m 只传相对目录名，
    // 并把工作目录固定为引擎目录，按 exe 目录或 cwd 解析的实现都能正确定位。
    cmd.current_dir(&batch.engine_dir);

    cmd.arg("-i")
        .arg(file_path)
        .arg("-o")
        .arg(&engine_out)
        .arg("-s")
        .arg(options.scale.to_string())
        .arg("-t")
        .arg(if options.tile_size < 32 {
            // NCNN 拒绝 1-31 的 tile 值；<32 一律 0（自动）
            "0".to_string()
        } else {
            options.tile_size.to_string()
        });

    // GPU 选择：仅 macOS 构建不支持 -g -1（CPU 模式）；
    // Windows/Linux 必须显式传 -g -1 才能走 CPU，省略会自动选 GPU
    if options.gpu_id >= 0 || cfg!(not(target_os = "macos")) {
        cmd.arg("-g").arg(options.gpu_id.to_string());
    }

    // Real-CUGAN / Waifu2x 的 -n 是噪声等级、-m 是模型目录
    cmd.arg("-n")
        .arg(options.denoise_level.to_string())
        .arg("-m")
        .arg(batch.model);

    // TTA mode: -x flag (all engines support it)
    if options.tta {
        cmd.arg("-x");
    }

    // 捕获输出（必须 piped，否则 wait_with_output 拿到的 stderr 恒为空，错误信息丢失）
    cmd.stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());

    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    let child = cmd.spawn().map_err(|e| format!("启动失败 - {}", e))?;
    let output = {
        let _registration = PidRegistration::new(&ACTIVE_CHILD, child.id());
        child.wait_with_output()
    };

    // 部分 NCNN 构建在 Vulkan 初始化失败后仍返回 0 并写出无效像素。
    let vulkan_error = output.as_ref().ok().and_then(|output| {
        if options.gpu_id < 0 && cfg!(not(target_os = "macos")) {
            return None;
        }
        String::from_utf8_lossy(&output.stderr)
            .lines()
            .find(|line| {
                line.contains("vkCreateInstance failed")
                    || line.contains("VK_ERROR_INCOMPATIBLE_DRIVER")
            })
            .map(str::to_owned)
    });
    let written = matches!(&output, Ok(o) if o.status.success())
        && engine_out.exists()
        && vulkan_error.is_none();
    let finalized = if written {
        finalize_engine_output(&engine_out, out_file, source)
    } else {
        Ok(())
    };
    let _ = std::fs::remove_file(&engine_out);

    let output = output.map_err(|e| format!("执行失败 - {}", e))?;
    if let Some(error) = vulkan_error {
        return Err(format!("Vulkan 初始化失败: {}", error));
    }
    finalized?;
    if written {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(stderr.lines().last().unwrap_or("未知错误").to_string())
    }
}

fn terminal_event(result: &ProcessResult, cancelled: bool) -> ProgressEvent {
    let processed = result.success_count + result.fail_count;
    let message = if cancelled {
        format!("已取消: 已处理 {}, 共 {}", processed, result.total)
    } else {
        format!(
            "完成: 成功 {}, 失败 {}, 共 {}",
            result.success_count, result.fail_count, result.total
        )
    };
    ProgressEvent::new("done", message).at(
        if cancelled { processed } else { result.total },
        result.total,
    )
}

/// 引擎只输出 PNG：把结果按源图格式写回最终路径
fn finalize_engine_output(
    engine_png: &Path,
    out_file: &Path,
    source: &SourceInfo,
) -> Result<(), String> {
    let img = image::open(engine_png).map_err(|e| format!("读取超分结果失败: {}", e))?;
    save_like_source(img, out_file, source)
}

// ===== Python Upscale =====

struct ManifestFile(PathBuf);
impl Drop for ManifestFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn run_python_upscale(
    app: &tauri::AppHandle,
    options: &UpscaleOptions,
    plan: &[PlannedImage],
) -> Result<ProcessResult, String> {
    let mut result = ProcessResult {
        total: plan.len() as u32,
        ..Default::default()
    };
    let mut pairs = Vec::new();
    for item in plan {
        if CANCEL_FLAG.load(Ordering::SeqCst) {
            return Ok(result);
        }
        match &item.output {
            Ok(output) => pairs.push((&item.input, output)),
            Err(error) => {
                result.fail_count += 1;
                let filename = super::file_name_lossy(&item.input);
                let message = format!("{}: {}", filename, error);
                result.errors.push(message.clone());
                ProgressEvent::new("error", message)
                    .at(result.fail_count, result.total)
                    .file(filename)
                    .emit(app, "upscale-progress");
            }
        }
    }
    if pairs.is_empty() {
        return Ok(result);
    }
    let python = super::python_env::get_venv_python();
    if !python.is_file() {
        return Err("Python 环境未就绪，请先安装".into());
    }
    let script = python_proc::find_script("realesrgan_upscale.py")?;
    let (_, model_file, _) = ESRGAN_MODELS
        .iter()
        .find(|(id, _, _)| *id == options.model_id)
        .ok_or_else(|| format!("未知模型: {}", options.model_id))?;
    let model = realesrgan_weights_dir().join(model_file);
    if !model.is_file() {
        return Err(format!("模型文件不存在: {}", model.display()));
    }
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "purinbox-upscale-{}-{}.json",
        std::process::id(),
        stamp
    ));
    let mut manifest = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .map_err(|e| format!("创建超分清单失败: {}", e))?;
    let manifest_path = ManifestFile(path);
    serde_json::to_writer(&mut manifest, &pairs).map_err(|e| format!("写入超分清单失败: {}", e))?;
    drop(manifest);
    let mut cmd = python_proc::hidden_command(python);
    cmd.arg(script)
        .arg("--files")
        .arg(&manifest_path.0)
        .arg("--model-path")
        .arg(model)
        .arg("--scale")
        .arg(options.scale.to_string())
        .arg("--tile")
        .arg(options.tile_size.to_string())
        .arg("--device")
        .arg(if options.gpu_id < 0 { "cpu" } else { "auto" });
    if options.tta {
        cmd.arg("--tta");
    }
    let app_err = app.clone();
    let rejected = result.fail_count;
    let initial_errors = result.errors.clone();
    let mut got_done = false;
    let exit = python_proc::run_json_lines_script(
        cmd,
        options.gpu_id >= 0,
        &ACTIVE_CHILD,
        &CANCEL_FLAG,
        move |line| {
            if !python_proc::is_runtime_noise(&line) {
                ProgressEvent::new("warning", format!("[Python] {}", line))
                    .emit(&app_err, "upscale-progress");
            }
        },
        |msg| {
            match msg["type"].as_str().unwrap_or("") {
                "log" => ProgressEvent::python_log(&msg, 0, 0).emit(app, "upscale-progress"),
                "error" => {
                    return Err(format!(
                        "Real-ESRGAN 错误: {}",
                        msg["message"].as_str().unwrap_or("")
                    ))
                }
                "progress" => {
                    let current = msg["current"].as_u64().unwrap_or(0) as u32 + rejected;
                    let status = msg["status"].as_str().unwrap_or("processing");
                    let message = msg["message"].as_str().unwrap_or("");
                    if status == "success" {
                        result.success_count += 1;
                    }
                    if status == "error" {
                        result.fail_count += 1;
                        result.errors.push(message.into());
                    }
                    ProgressEvent::new(status, message)
                        .at(current, result.total)
                        .file(msg["filename"].as_str().unwrap_or(""))
                        .emit(app, "upscale-progress");
                }
                "done" => {
                    result.success_count = msg["success_count"].as_u64().unwrap_or(0) as u32;
                    result.fail_count = rejected + msg["fail_count"].as_u64().unwrap_or(0) as u32;
                    result.errors.clone_from(&initial_errors);
                    result.errors.extend(
                        msg["errors"]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .filter_map(|v| v.as_str().map(str::to_owned)),
                    );
                    got_done = true;
                }
                _ => {}
            }
            Ok(())
        },
    )?;
    if !got_done && !exit.cancelled {
        return Err(format!(
            "超分进程异常退出（退出码 {:?}），未返回结果；详见上方 [Python] 日志",
            exit.code
        ));
    }
    Ok(result)
}

#[tauri::command]
pub fn cancel_upscale() {
    CANCEL_FLAG.store(true, Ordering::SeqCst);
    DOWNLOAD_CANCEL.store(true, Ordering::SeqCst);
    super::python_env::cancel_setup_for("upscale");
}

#[tauri::command]
pub fn force_cancel_upscale() {
    cancel_upscale();
    python_proc::kill_registered_pid(&ACTIVE_CHILD);
}

#[cfg(test)]
mod keep_format_tests {
    use super::*;

    fn temp_dir(label: &str) -> PathBuf {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "purinbox_ai_{}_{}_{}",
            label,
            std::process::id(),
            stamp
        ));
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    fn options(root: &Path) -> UpscaleOptions {
        UpscaleOptions {
            input_path: root.join("in").to_string_lossy().into_owned(),
            output_path: root.join("out").to_string_lossy().into_owned(),
            engine_id: "realcugan".into(),
            model_id: "models-se".into(),
            scale: 2,
            denoise_level: 0,
            tta: false,
            gpu_id: -1,
            tile_size: 0,
            recursive: true,
        }
    }

    #[test]
    fn readiness_requires_both_named_weights() {
        let root = temp_dir("weights");
        std::fs::write(root.join("unrelated.onnx"), []).unwrap();
        assert!(!all_esrgan_weights_ready(&root));
        std::fs::write(root.join(ESRGAN_MODELS[0].1), []).unwrap();
        assert!(!all_esrgan_weights_ready(&root));
        std::fs::write(root.join(ESRGAN_MODELS[1].1), []).unwrap();
        assert!(all_esrgan_weights_ready(&root));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn both_engines_share_collection_and_output_guards() {
        let root = temp_dir("plan");
        for directory in ["in", "in/sub", "in/Fail", "in/out"] {
            std::fs::create_dir_all(root.join(directory)).unwrap();
        }
        for file in [
            "in/a.png",
            "in/sub/b.jpg",
            "in/Fail/ignored.png",
            "in/out/ignored.png",
            "in/c.gif",
            "in/d.tiff",
        ] {
            std::fs::write(root.join(file), []).unwrap();
        }
        let mut opts = options(&root);
        opts.output_path = root.join("in/out").to_string_lossy().into_owned();
        for python in [false, true] {
            let (plan, skipped) = plan_upscale(&opts, python).unwrap();
            assert_eq!(skipped, 1);
            assert_eq!(plan.len(), 3);
            assert!(plan.iter().all(|p| p.output.is_ok()));
            let sub = plan.iter().find(|p| p.input.ends_with("b.jpg")).unwrap();
            assert!(sub.output.as_ref().unwrap().ends_with("sub/b.jpg"));
            opts.output_path = opts.input_path.clone();
            let (same, _) = plan_upscale(&opts, python).unwrap();
            assert!(same
                .iter()
                .all(|p| p.output.as_ref().unwrap_err().contains("同一文件")));
            opts.output_path = root.join("in/out").to_string_lossy().into_owned();
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn output_guards_reject_other_sources_and_duplicate_names() {
        let inputs = [
            super::super::path_key_ci(Path::new("/tmp/a.png")),
            super::super::path_key_ci(Path::new("/tmp/b.png")),
        ]
        .into_iter()
        .collect();
        let mut used = std::collections::HashSet::new();
        assert!(claim_output(
            Path::new("/tmp/a.png"),
            Path::new("/tmp/b.png"),
            &inputs,
            &mut used
        )
        .unwrap_err()
        .contains("另一张源图"));
        claim_output(
            Path::new("/tmp/a.png"),
            Path::new("/tmp/out/a.png"),
            &inputs,
            &mut used,
        )
        .unwrap();
        assert!(claim_output(
            Path::new("/tmp/b.png"),
            Path::new("/tmp/out/a.png"),
            &inputs,
            &mut used
        )
        .unwrap_err()
        .contains("冲突"));
    }

    #[test]
    fn final_image_cancel_selects_cancelled_terminal() {
        let result = ProcessResult {
            success_count: 1,
            total: 1,
            ..Default::default()
        };
        let event = terminal_event(&result, true);
        assert_eq!(event.status, "done");
        assert_eq!(event.current, 1);
        assert!(event.message.starts_with("已取消"));
    }

    #[cfg(unix)]
    #[test]
    fn ncnn_png_restores_icc_and_failed_engine_preserves_output() {
        use image::{ImageDecoder, ImageEncoder};
        use std::os::unix::fs::PermissionsExt;
        let root = temp_dir("ncnn");
        let mut profile = vec![0u8; 128];
        profile[16..20].copy_from_slice(b"RGB ");
        let source = root.join("source.png");
        let mut bytes = Vec::new();
        let mut encoder = image::codecs::png::PngEncoder::new(&mut bytes);
        encoder.set_icc_profile(profile.clone()).unwrap();
        image::RgbImage::from_pixel(8, 8, image::Rgb([20, 40, 60]))
            .write_with_encoder(encoder)
            .unwrap();
        std::fs::write(&source, bytes).unwrap();
        image::RgbImage::from_pixel(16, 16, image::Rgb([80, 90, 100]))
            .save(root.join("engine.png"))
            .unwrap();
        let bin = root.join("fake-ncnn");
        std::fs::write(&bin, "#!/bin/sh\nwhile [ $# -gt 0 ]; do if [ \"$1\" = '-o' ]; then out=$2; fi; shift; done\ncp engine.png \"$out\"\n").unwrap();
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o700)).unwrap();
        let mut opts = options(&root);
        opts.gpu_id = 0;
        let batch = NcnnBatch {
            bin: bin.clone(),
            engine_dir: root.clone(),
            model: "models-se",
            options: &opts,
        };
        let out = root.join("out.png");
        upscale_one(&batch, &source, &out, &probe_image(&source).unwrap(), 999).unwrap();
        let mut decoder = image::ImageReader::open(&out)
            .unwrap()
            .into_decoder()
            .unwrap();
        assert_eq!(decoder.icc_profile().unwrap(), Some(profile));
        assert_eq!(decoder.dimensions(), (16, 16));
        let good = std::fs::read(&out).unwrap();
        std::fs::write(&bin, "#!/bin/sh\necho failed >&2\nexit 1\n").unwrap();
        assert!(upscale_one(&batch, &source, &out, &probe_image(&source).unwrap(), 999).is_err());
        assert_eq!(std::fs::read(&out).unwrap(), good);
        std::fs::write(&bin, "#!/bin/sh\nwhile [ $# -gt 0 ]; do if [ \"$1\" = '-o' ]; then out=$2; fi; shift; done\ncp engine.png \"$out\"\necho 'vkCreateInstance failed -9' >&2\nexit 0\n").unwrap();
        assert!(
            upscale_one(&batch, &source, &out, &probe_image(&source).unwrap(), 999)
                .unwrap_err()
                .starts_with("Vulkan 初始化失败")
        );
        assert_eq!(std::fs::read(&out).unwrap(), good);
        assert!(!std::env::temp_dir()
            .join(format!("purinbox-upscale-{}-999.png", std::process::id()))
            .exists());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn engine_png_is_rewritten_in_source_format() {
        let root =
            std::env::temp_dir().join(format!("purinbox_upscale_keep_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let src = root.join("a.jpg");
        let mut buf = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgb8(image::RgbImage::from_fn(16, 16, |x, y| {
            image::Rgb([(x * 16) as u8, (y * 16) as u8, 90])
        }))
        .write_with_encoder(image::codecs::jpeg::JpegEncoder::new_with_quality(
            &mut buf, 98,
        ))
        .unwrap();
        std::fs::write(&src, buf.into_inner()).unwrap();
        let engine_png = root.join("engine.png");
        image::RgbImage::from_pixel(64, 64, image::Rgb([10, 200, 30]))
            .save(&engine_png)
            .unwrap();

        let out = root.join("out").join("a.jpg");
        std::fs::create_dir_all(out.parent().unwrap()).unwrap();
        finalize_engine_output(&engine_png, &out, &probe_image(&src).unwrap()).unwrap();
        let bytes = std::fs::read(&out).unwrap();
        assert_eq!(
            image::guess_format(&bytes).unwrap(),
            image::ImageFormat::Jpeg
        );
        assert_eq!(image::load_from_memory(&bytes).unwrap().width(), 64);
        let _ = std::fs::remove_dir_all(&root);
    }
}
