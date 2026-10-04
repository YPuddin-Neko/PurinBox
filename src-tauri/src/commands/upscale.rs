use image::ImageFormat;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use tauri::Emitter;

use super::http_download::{
    download_client, download_files, download_to_file, DownloadError, DownloadFile,
    DownloadFilesOptions, DownloadProgress,
};
use super::image_io::{
    load_image, probe_header, probe_image, save_like_source, ImageHeader, SourceInfo,
};
use super::python_proc::{
    self, abnormal_exit, ManifestFile, PidRegistration, PythonCommand, PythonTempTag, StderrNoise,
};
use super::python_task;
use super::{
    collect_image_files_with_recursive_excluding, output_path_for_input, ProcessResult,
    ProgressEvent,
};

const EVENT: &str = "upscale-progress";
const DOWNLOAD_EVENT: &str = "upscale-download";

/// Real-ESRGAN 引擎依赖的 OpenCV；人物裁切读写 16 位彩色图也要用它
pub(super) const OPENCV_PACKAGE: &str = "opencv-python-headless==4.11.0.86";

/// 失败时输出位置已有的文件不会被动到：结果都是先写临时文件、成功后再替换。
/// NCNN 和 Real-ESRGAN 的失败消息都由 `note_kept_output` 补上，脚本那边不拼
const KEPT_OUTPUT_NOTE: &str = "（已保留原有输出文件）";

/// 超分子进程（Python 或 NCNN）的 PID，供强制取消
static ACTIVE_CHILD: Mutex<Option<u32>> = Mutex::new(None);
/// 下载、环境部署和超分共用的取消标志
static CANCEL_FLAG: AtomicBool = AtomicBool::new(false);
/// 互斥标志：超分页面与工作流节点共用同一组全局状态（CANCEL_FLAG/ACTIVE_CHILD/进度事件），
/// 并发运行会互相清对方的取消标志与子进程 PID，必须串行。
static UPSCALE_RUNNING: AtomicBool = AtomicBool::new(false);

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
    /// NCNN 引擎的压缩包；Python 引擎的依赖和权重另行下载，为空
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

fn find_engine(engine_id: &str) -> Result<&'static EngineDef, String> {
    ENGINES
        .iter()
        .find(|e| e.id == engine_id)
        .ok_or_else(|| format!("未知引擎: {}", engine_id))
}

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

fn ensure_not_cancelled() -> Result<(), DownloadError> {
    if CANCEL_FLAG.load(Ordering::SeqCst) {
        Err(DownloadError::Cancelled)
    } else {
        Ok(())
    }
}

fn emit_download(app: &tauri::AppHandle, progress: DownloadProgress) {
    let _ = app.emit(DOWNLOAD_EVENT, progress);
}

#[tauri::command]
pub async fn download_upscale_engine(
    app: tauri::AppHandle,
    engine_id: String,
) -> Result<String, String> {
    let _busy = super::BusyGuard::acquire(&UPSCALE_RUNNING, "超分")?;
    CANCEL_FLAG.store(false, Ordering::SeqCst);
    super::python_env::clear_pending_cancel("upscale");
    let engine = find_engine(&engine_id)?;
    let outcome = prepare_engine(&app, engine).await;
    emit_download(
        &app,
        match &outcome {
            Ok(_) => DownloadProgress::done(format!("{} 已就绪", engine.name)),
            Err(error) => DownloadProgress::from_error(error, &CANCEL_FLAG),
        },
    );
    // 取消时返回以「已取消」开头的错误：前端据此不再接着启动超分
    outcome.map(String::from).map_err(|error| {
        if CANCEL_FLAG.load(Ordering::SeqCst) {
            DownloadError::Cancelled.to_string()
        } else {
            error.to_string()
        }
    })
}

/// 下载并部署引擎，返回 "already_ready" 或 "done"
async fn prepare_engine(
    app: &tauri::AppHandle,
    engine: &EngineDef,
) -> Result<&'static str, DownloadError> {
    if is_engine_ready(engine).await {
        ensure_not_cancelled()?;
        return Ok("already_ready");
    }
    ensure_not_cancelled()?;
    if engine.use_python {
        prepare_python_engine(app).await?;
    } else {
        download_ncnn_engine(app, engine).await?;
    }
    ensure_not_cancelled()?;
    Ok("done")
}

async fn download_ncnn_engine(
    app: &tauri::AppHandle,
    engine: &EngineDef,
) -> Result<(), DownloadError> {
    let dest_dir = engine_dir(engine.id);
    std::fs::create_dir_all(&dest_dir)
        .map_err(|e| DownloadError::Other(format!("创建目录失败: {}", e)))?;
    let zip_path = dest_dir.join("_download.zip");
    let client = download_client().map_err(DownloadError::Other)?;
    emit_download(app, DownloadProgress::starting(engine.name));
    download_to_file(
        client.get(engine.download_url),
        &zip_path,
        engine.name,
        &CANCEL_FLAG,
        |p| emit_download(app, p),
    )
    .await?;
    ensure_not_cancelled()?;
    emit_download(
        app,
        DownloadProgress::new("extracting", 99.0, format!("{} — 正在解压...", engine.name)),
    );
    let archive = zip_path.clone();
    tokio::task::spawn_blocking(move || extract_zip(&archive, &dest_dir))
        .await
        .map_err(|e| DownloadError::Other(format!("解压任务失败: {}", e)))?
        .map_err(|e| DownloadError::Other(format!("解压失败: {}", e)))?;
    let _ = tokio::fs::remove_file(&zip_path).await;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(
            engine_binary(engine),
            std::fs::Permissions::from_mode(0o755),
        )
        .map_err(|e| DownloadError::Other(format!("设置引擎权限失败: {}", e)))?;
    }
    Ok(())
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

async fn prepare_python_engine(app: &tauri::AppHandle) -> Result<(), DownloadError> {
    let other = DownloadError::Other;
    emit_download(
        app,
        DownloadProgress::new("downloading", 5.0, "正在检查 Python 环境..."),
    );
    let python = super::python_env::setup_python_env(app, "upscale")
        .await
        .map_err(other)?;
    ensure_not_cancelled()?;
    super::python_env::ensure_onnx_gpu_runtime(app, &python, "upscale")
        .await
        .map_err(other)?;
    ensure_not_cancelled()?;
    if super::python_env::probe_python(&python, "import cv2")
        .await
        .is_none()
    {
        ensure_not_cancelled()?;
        emit_download(
            app,
            DownloadProgress::new("downloading", 15.0, "正在安装 OpenCV..."),
        );
        super::python_env::pip_install_for(app, &python, &[OPENCV_PACKAGE], "upscale")
            .await
            .map_err(other)?;
    }
    ensure_not_cancelled()?;
    let client = download_client().map_err(other)?;
    let weights_dir = realesrgan_weights_dir();
    let files = ESRGAN_MODELS
        .iter()
        .map(|(_, filename, url)| {
            DownloadFile::new(client.get(*url), weights_dir.join(filename), *filename)
        })
        .collect();
    let options = DownloadFilesOptions {
        skip_existing: true,
        percent_range: Some((30.0, 98.0)),
        ..Default::default()
    };
    download_files(files, options, &CANCEL_FLAG, |p| emit_download(app, p)).await?;
    Ok(())
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
    super::python_env::clear_pending_cancel("upscale");
    let engine = find_engine(&options.engine_id)?;
    tokio::task::spawn_blocking(move || run_upscale(&app, engine, &options))
        .await
        .map_err(|e| format!("任务执行失败: {}", e))?
}

fn run_upscale<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    engine: &'static EngineDef,
    options: &UpscaleOptions,
) -> Result<ProcessResult, String> {
    let run_id = super::begin_run(EVENT);
    let (plan, skipped) = plan_upscale(options, engine.use_python)?;
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
        .emit(app, EVENT);
    let outcome = if engine.use_python {
        run_esrgan(
            app,
            &plan,
            || esrgan_command(options),
            &PythonTempTag::for_run(run_id),
            &CANCEL_FLAG,
            &ACTIVE_CHILD,
        )
    } else {
        NcnnBatch::for_engine(engine, options)
            .and_then(|batch| run_ncnn_upscale(app, &batch, &plan, &CANCEL_FLAG, &ACTIVE_CHILD))
    };
    let cancelled = CANCEL_FLAG.load(Ordering::SeqCst);
    python_task::finish_command(app, EVENT, total, outcome, cancelled, "完成")
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

/// 处理失败的原因；输出位置原本就有文件时注明它被保留
fn note_kept_output(error: String, output: &Path) -> String {
    if output.exists() {
        format!("{}{}", error, KEPT_OUTPUT_NOTE)
    } else {
        error
    }
}

// ===== NCNN Upscale =====

struct NcnnBatch<'a> {
    bin: PathBuf,
    engine_dir: PathBuf,
    model: &'static str,
    options: &'a UpscaleOptions,
}

impl<'a> NcnnBatch<'a> {
    fn for_engine(engine: &EngineDef, options: &'a UpscaleOptions) -> Result<Self, String> {
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
        Ok(NcnnBatch {
            bin,
            engine_dir,
            model,
            options,
        })
    }
}

fn run_ncnn_upscale<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    batch: &NcnnBatch,
    plan: &[PlannedImage],
    cancel: &AtomicBool,
    child_slot: &Mutex<Option<u32>>,
) -> Result<ProcessResult, String> {
    let total = plan.len() as u32;
    let mut result = ProcessResult {
        total,
        ..Default::default()
    };
    for (i, item) in plan.iter().enumerate() {
        if cancel.load(Ordering::SeqCst) {
            break;
        }
        let filename = super::file_name_lossy(&item.input);
        let current = i as u32 + 1;
        ProgressEvent::new(
            "processing",
            format!("[{}/{}] 正在处理: {}", current, total, filename),
        )
        .at(i as u32, total)
        .file(&filename)
        .emit(app, EVENT);
        let outcome = match &item.output {
            Ok(output) => upscale_one(batch, &item.input, output, i, child_slot)
                .map_err(|error| note_kept_output(error, output)),
            Err(error) => Err(error.clone()),
        };
        if cancel.load(Ordering::SeqCst) && outcome.is_err() {
            break;
        }
        match outcome {
            Ok(depth_reduced) => {
                result.success_count += 1;
                if depth_reduced {
                    ProgressEvent::new(
                        "warning",
                        format!("{}: 源图为 16 位，NCNN 引擎只能输出 8 位", filename),
                    )
                    .at(current, total)
                    .file(&filename)
                    .emit(app, EVENT);
                }
                ProgressEvent::new("success", format!("[{}/{}] ✓ {}", current, total, filename))
                    .at(current, total)
                    .file(filename)
                    .emit(app, EVENT);
            }
            Err(error) => {
                result.fail_count += 1;
                let message = format!("{}: {}", filename, error);
                result.errors.push(message.clone());
                ProgressEvent::new("error", format!("[{}/{}] ✗ {}", current, total, message))
                    .at(current, total)
                    .file(filename)
                    .emit(app, EVENT);
            }
        }
    }
    Ok(result)
}

/// 用 NCNN 超分一张图并按源图格式写到 out_file，返回结果是否从 16 位降成了 8 位（NCNN 只输出 8 位）。
/// Err 为失败原因（不含文件名）
fn upscale_one(
    batch: &NcnnBatch,
    file_path: &Path,
    out_file: &Path,
    index: usize,
    child_slot: &Mutex<Option<u32>>,
) -> Result<bool, String> {
    let source = probe_image(file_path)?;
    // PNG 源图的文件头决定怎样写回：位深、有没有 ICC 配置文件
    let png_header = if source.format == ImageFormat::Png {
        probe_header(file_path).ok()
    } else {
        None
    };
    let options = batch.options;
    // 引擎先写临时 PNG：失败时不动已有输出，成功后再按源图格式写回
    let engine_out = std::env::temp_dir().join(format!(
        "purinbox-upscale-{}-{}.png",
        std::process::id(),
        index
    ));
    let _ = std::fs::remove_file(&engine_out);
    let mut cmd = Command::new(&batch.bin);
    python_proc::configure_python_command(&mut cmd, false);

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

    if options.tta {
        cmd.arg("-x");
    }

    // 捕获输出（必须 piped，否则 wait_with_output 拿到的 stderr 恒为空，错误信息丢失）
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let child = python_proc::spawn_exclusive(&mut cmd).map_err(|e| format!("启动失败 - {}", e))?;
    let output = {
        let _registration = PidRegistration::new(child_slot, child.id());
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
        finalize_engine_output(&engine_out, out_file, &source, png_header.as_ref())
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
        Ok(png_header.is_some_and(|h| h.high_bit_depth()))
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(stderr.lines().last().unwrap_or("未知错误").to_string())
    }
}

/// 引擎只输出 PNG：PNG 源图又没有 ICC 配置文件时直接采用引擎的文件，否则按源图格式（带 ICC）重新编码。
/// 先写到输出旁的临时文件再替换，写失败时已有的输出文件不受影响
fn finalize_engine_output(
    engine_png: &Path,
    out_file: &Path,
    source: &SourceInfo,
    png_header: Option<&ImageHeader>,
) -> Result<(), String> {
    let mut staged = out_file.as_os_str().to_os_string();
    staged.push(".purin-upscale.tmp");
    let staged = PathBuf::from(staged);
    let written = if png_header.is_some_and(|h| !h.has_icc) {
        std::fs::copy(engine_png, &staged)
            .map(drop)
            .map_err(|e| format!("写入超分结果失败: {}", e))
    } else {
        // image::open 按整图大小向 512 MiB 的分配上限预留，大图放大后读不回来；load_image 不做这项预留
        load_image(engine_png)
            .map_err(|e| format!("读取超分结果失败: {}", e))
            .and_then(|(img, _)| save_like_source(img, &staged, source))
    };
    let replaced = written.and_then(|()| {
        std::fs::rename(&staged, out_file).map_err(|e| format!("写入超分结果失败: {}", e))
    });
    if replaced.is_err() {
        let _ = std::fs::remove_file(&staged);
    }
    replaced
}

// ===== Python Upscale =====

/// Real-ESRGAN 引擎的命令（解释器、脚本、模型与参数），文件清单另加
fn esrgan_command(options: &UpscaleOptions) -> Result<PythonCommand, String> {
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
    let cmd = PythonCommand::new(python)
        .arg(script)
        .arg("--model-path")
        .arg(model)
        .arg("--scale")
        .arg(options.scale.to_string())
        .arg("--tile")
        .arg(options.tile_size.to_string())
        .arg("--device")
        .arg(if options.gpu_id < 0 { "cpu" } else { "auto" })
        .use_gpu(options.gpu_id >= 0);
    Ok(if options.tta { cmd.arg("--tta") } else { cmd })
}

/// 计划里能交给 Python 的 [输入, 输出] 对；不能处理的条目给出原因
fn esrgan_job(item: &PlannedImage) -> Result<[String; 2], String> {
    let output = item.output.as_ref().map_err(String::clone)?;
    match (item.input.to_str(), output.to_str()) {
        (Some(input), Some(output)) => Ok([input.to_owned(), output.to_owned()]),
        // 清单是 UTF-8 的 JSON，写不进这种路径
        _ => Err("路径含有非 UTF-8 字符，Real-ESRGAN 引擎无法处理".into()),
    }
}

fn run_esrgan<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    plan: &[PlannedImage],
    command: impl FnOnce() -> Result<PythonCommand, String>,
    temp_tag: &PythonTempTag,
    cancel: &AtomicBool,
    child_slot: &Mutex<Option<u32>>,
) -> Result<ProcessResult, String> {
    let mut result = ProcessResult {
        total: plan.len() as u32,
        ..Default::default()
    };
    let mut jobs = Vec::new();
    for item in plan {
        if cancel.load(Ordering::SeqCst) {
            return Ok(result);
        }
        match esrgan_job(item) {
            Ok(job) => jobs.push(job),
            Err(error) => {
                result.fail_count += 1;
                let filename = super::file_name_lossy(&item.input);
                let message = format!("{}: {}", filename, error);
                result.errors.push(message.clone());
                ProgressEvent::new("error", message)
                    .at(result.fail_count, result.total)
                    .file(filename)
                    .emit(app, EVENT);
            }
        }
    }
    if jobs.is_empty() {
        return Ok(result);
    }
    let cmd = command()?;
    let manifest = ManifestFile::write("purinbox-upscale", &jobs)?;
    let rejected = result.fail_count;
    let mut got_done = false;
    let exit = python_proc::run_json_lines_script_with(
        temp_tag.apply(cmd.arg("--files").arg(manifest.path())),
        None,
        child_slot,
        cancel,
        python_proc::stderr_warnings(app, EVENT, StderrNoise::Runtime, None),
        |msg| {
            match msg["type"].as_str().unwrap_or("") {
                "log" => ProgressEvent::python_log(&msg, 0, 0).emit(app, EVENT),
                "error" => {
                    return Err(format!(
                        "Real-ESRGAN 错误: {}",
                        msg["message"].as_str().unwrap_or("")
                    ))
                }
                "progress" => {
                    // 脚本的序号从 1 起，与清单逐项对应
                    let index = msg["current"].as_u64().unwrap_or(0) as u32;
                    let status = msg["status"].as_str().unwrap_or("processing");
                    let mut message = msg["message"].as_str().unwrap_or("").to_owned();
                    if status == "success" {
                        result.success_count += 1;
                    }
                    if status == "error" {
                        if let Some([_, output]) =
                            index.checked_sub(1).and_then(|i| jobs.get(i as usize))
                        {
                            message = note_kept_output(message, Path::new(output));
                        }
                        result.fail_count += 1;
                        result.errors.push(message.clone());
                    }
                    ProgressEvent::new(status, message)
                        .at(index + rejected, result.total)
                        .file(msg["filename"].as_str().unwrap_or(""))
                        .emit(app, EVENT);
                }
                "done" => {
                    // 计数以脚本的汇总为准；失败明细沿用逐条收到的，那里已注明保留了原有输出
                    result.success_count = msg["success_count"].as_u64().unwrap_or(0) as u32;
                    result.fail_count = rejected + msg["fail_count"].as_u64().unwrap_or(0) as u32;
                    got_done = true;
                }
                _ => {}
            }
            Ok(())
        },
    );
    if !got_done {
        // 没正常结束（被取消、超时、报错，或自己崩溃、被系统杀掉）时可能有写到一半的临时文件
        for [_, output] in &jobs {
            let _ = std::fs::remove_file(temp_tag.temp_file(Path::new(output)));
        }
    }
    let exit = exit?;
    if !got_done && !exit.cancelled {
        return Err(abnormal_exit("超分进程", &exit));
    }
    Ok(result)
}

#[tauri::command]
pub fn cancel_upscale() {
    CANCEL_FLAG.store(true, Ordering::SeqCst);
    super::python_env::cancel_setup_for("upscale");
}

#[tauri::command]
pub fn force_cancel_upscale() {
    cancel_upscale();
    python_proc::kill_registered_pid(&ACTIVE_CHILD);
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    use crate::commands::batch::capture_events;
    #[cfg(unix)]
    use crate::commands::test_support::fake_program;
    use crate::commands::test_support::TempDir;
    use image::ImageEncoder;

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

    #[cfg(unix)]
    fn fake_profile() -> Vec<u8> {
        let mut profile = vec![0u8; 128];
        profile[16..20].copy_from_slice(b"RGB ");
        profile
    }

    fn write_png(path: &Path, img: &image::DynamicImage, icc: Option<Vec<u8>>) {
        let mut bytes = Vec::new();
        let mut encoder = image::codecs::png::PngEncoder::new(&mut bytes);
        if let Some(profile) = icc {
            encoder.set_icc_profile(profile).unwrap();
        }
        img.write_with_encoder(encoder).unwrap();
        std::fs::write(path, bytes).unwrap();
    }

    fn rgb(width: u32, value: u8) -> image::DynamicImage {
        image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
            width,
            width,
            image::Rgb([value, value / 2, 90]),
        ))
    }

    /// 把 `-o` 指向的输出写成工作目录下的 engine.png；输入名带 fail 时报错退出
    #[cfg(unix)]
    const FAKE_NCNN: &str = "while [ $# -gt 0 ]; do case \"$1\" in -i) in=$2;; -o) out=$2;; esac; shift; done\ncase \"$in\" in *fail*) echo 'engine crashed' >&2; exit 1;; esac\ncp engine.png \"$out\"\n";

    #[cfg(unix)]
    fn ncnn_batch<'a>(root: &Path, opts: &'a UpscaleOptions) -> NcnnBatch<'a> {
        NcnnBatch {
            bin: fake_program(root, "fake-ncnn", FAKE_NCNN),
            engine_dir: root.to_path_buf(),
            model: "models-se",
            options: opts,
        }
    }

    #[test]
    fn readiness_requires_both_named_weights() {
        let root = TempDir::new("upscale_weights");
        std::fs::write(root.join("unrelated.onnx"), []).unwrap();
        assert!(!all_esrgan_weights_ready(&root));
        std::fs::write(root.join(ESRGAN_MODELS[0].1), []).unwrap();
        assert!(!all_esrgan_weights_ready(&root));
        std::fs::write(root.join(ESRGAN_MODELS[1].1), []).unwrap();
        assert!(all_esrgan_weights_ready(&root));
    }

    #[test]
    fn both_engines_share_collection_and_output_guards() {
        let root = TempDir::new("upscale_plan");
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

    #[cfg(unix)]
    #[test]
    fn ncnn_png_restores_icc_and_failed_engine_preserves_output() {
        use image::ImageDecoder;
        let root = TempDir::new("upscale_ncnn_icc");
        let source = root.join("source.png");
        write_png(&source, &rgb(8, 40), Some(fake_profile()));
        write_png(&root.join("engine.png"), &rgb(16, 200), None);
        let mut opts = options(&root);
        opts.gpu_id = 0;
        let batch = ncnn_batch(&root, &opts);
        let slot = Mutex::new(None);
        let out = root.join("out.png");
        assert!(!upscale_one(&batch, &source, &out, 997, &slot).unwrap());
        let mut decoder = image::ImageReader::open(&out)
            .unwrap()
            .into_decoder()
            .unwrap();
        assert_eq!(decoder.icc_profile().unwrap(), Some(fake_profile()));
        assert_eq!(decoder.dimensions(), (16, 16));
        assert!(slot.lock().unwrap().is_none());
        let good = std::fs::read(&out).unwrap();
        let failing = root.join("fail.png");
        std::fs::copy(&source, &failing).unwrap();
        assert_eq!(
            upscale_one(&batch, &failing, &out, 997, &slot).unwrap_err(),
            "engine crashed"
        );
        assert_eq!(std::fs::read(&out).unwrap(), good);
        fake_program(&root, "fake-ncnn", "while [ $# -gt 0 ]; do if [ \"$1\" = '-o' ]; then out=$2; fi; shift; done\ncp engine.png \"$out\"\necho 'vkCreateInstance failed -9' >&2\nexit 0\n");
        assert!(upscale_one(&batch, &source, &out, 997, &slot)
            .unwrap_err()
            .starts_with("Vulkan 初始化失败"));
        assert_eq!(std::fs::read(&out).unwrap(), good);
        assert!(!std::env::temp_dir()
            .join(format!("purinbox-upscale-{}-997.png", std::process::id()))
            .exists());
        assert!(!root.join("out.png.purin-upscale.tmp").exists());
    }

    #[cfg(unix)]
    #[test]
    fn ncnn_png_without_icc_takes_engine_output_as_is() {
        let root = TempDir::new("upscale_ncnn_copy");
        let source = root.join("source.png");
        write_png(&source, &rgb(8, 40), None);
        write_png(&root.join("engine.png"), &rgb(16, 200), None);
        let opts = options(&root);
        let out = root.join("out.png");
        upscale_one(
            &ncnn_batch(&root, &opts),
            &source,
            &out,
            998,
            &Mutex::new(None),
        )
        .unwrap();
        assert_eq!(
            std::fs::read(&out).unwrap(),
            std::fs::read(root.join("engine.png")).unwrap()
        );
    }

    #[cfg(unix)]
    #[test]
    fn ncnn_batch_warns_on_16_bit_sources_and_notes_kept_outputs() {
        let root = TempDir::new("upscale_ncnn_batch");
        std::fs::create_dir_all(root.join("out")).unwrap();
        write_png(&root.join("engine.png"), &rgb(16, 200), None);
        let deep = root.join("deep.png");
        image::DynamicImage::ImageRgb16(image::ImageBuffer::from_pixel(
            8,
            8,
            image::Rgb([1000u16, 2000, 3000]),
        ))
        .save(&deep)
        .unwrap();
        let kept = root.join("fail-kept.png");
        let fresh = root.join("fail-fresh.png");
        for path in [&kept, &fresh] {
            write_png(path, &rgb(8, 10), None);
        }
        std::fs::write(root.join("out/fail-kept.png"), b"previous").unwrap();
        let plan: Vec<_> = [&deep, &kept, &fresh]
            .into_iter()
            .map(|input| PlannedImage {
                input: input.clone(),
                output: Ok(root.join("out").join(input.file_name().unwrap())),
            })
            .collect();
        let opts = options(&root);
        let app = tauri::test::mock_app();
        let events = capture_events(app.handle(), EVENT);
        let result = run_ncnn_upscale(
            app.handle(),
            &ncnn_batch(&root, &opts),
            &plan,
            &AtomicBool::new(false),
            &Mutex::new(None),
        )
        .unwrap();
        assert_eq!((result.success_count, result.fail_count), (1, 2));
        let events = events.lock().unwrap();
        let warnings: Vec<_> = events.iter().filter(|e| e["status"] == "warning").collect();
        assert_eq!(warnings.len(), 1);
        assert_eq!(warnings[0]["filename"], "deep.png");
        assert!(warnings[0]["message"].as_str().unwrap().contains("8 位"));
        let errors: Vec<_> = events
            .iter()
            .filter(|e| e["status"] == "error")
            .map(|e| e["message"].as_str().unwrap().to_string())
            .collect();
        assert_eq!(errors.len(), 2);
        assert!(
            errors[0].ends_with(&format!("engine crashed{}", KEPT_OUTPUT_NOTE)),
            "{}",
            errors[0]
        );
        assert!(!errors[1].contains(KEPT_OUTPUT_NOTE), "{}", errors[1]);
        assert_eq!(
            std::fs::read(root.join("out/fail-kept.png")).unwrap(),
            b"previous"
        );
    }

    #[test]
    fn engine_png_is_rewritten_in_source_format() {
        let root = TempDir::new("upscale_keep_format");
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
        write_png(&engine_png, &rgb(64, 30), None);

        let out = root.join("out").join("a.jpg");
        std::fs::create_dir_all(out.parent().unwrap()).unwrap();
        finalize_engine_output(&engine_png, &out, &probe_image(&src).unwrap(), None).unwrap();
        let bytes = std::fs::read(&out).unwrap();
        assert_eq!(
            image::guess_format(&bytes).unwrap(),
            image::ImageFormat::Jpeg
        );
        assert_eq!(image::load_from_memory(&bytes).unwrap().width(), 64);
    }

    /// PNG 头声明 `width`×`height` 的 RGB8 图，图像数据只有开头两行
    fn truncated_png(width: u32, height: u32) -> Vec<u8> {
        fn chunk(out: &mut Vec<u8>, kind: &[u8], data: &[u8]) {
            let mut crc = flate2::Crc::new();
            crc.update(kind);
            crc.update(data);
            out.extend_from_slice(&(data.len() as u32).to_be_bytes());
            out.extend_from_slice(kind);
            out.extend_from_slice(data);
            out.extend_from_slice(&crc.sum().to_be_bytes());
        }
        let mut header = Vec::new();
        header.extend_from_slice(&width.to_be_bytes());
        header.extend_from_slice(&height.to_be_bytes());
        header.extend_from_slice(&[8, 2, 0, 0, 0]);
        let rows = vec![0u8; 2 * (1 + width as usize * 3)];
        let mut zlib = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::fast());
        std::io::Write::write_all(&mut zlib, &rows).unwrap();
        let idat = zlib.flush_finish().unwrap();
        let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
        chunk(&mut png, b"IHDR", &header);
        chunk(&mut png, b"IDAT", &idat);
        chunk(&mut png, b"IEND", &[]);
        png
    }

    #[test]
    fn engine_output_is_read_without_the_default_allocation_limit() {
        let root = TempDir::new("upscale_large_read");
        // 16384×11000 的 RGB8 约 540 MB，超过 image::open 默认的 512 MiB 预留上限
        let engine_png = root.join("engine.png");
        std::fs::write(&engine_png, truncated_png(16384, 11000)).unwrap();
        assert!(matches!(
            image::open(&engine_png),
            Err(image::ImageError::Limits(_))
        ));
        let src = root.join("a.jpg");
        rgb(8, 40).save(&src).unwrap();
        let error = finalize_engine_output(
            &engine_png,
            &root.join("out.jpg"),
            &probe_image(&src).unwrap(),
            None,
        )
        .unwrap_err();
        assert!(error.starts_with("读取超分结果失败"), "{error}");
        assert!(!error.contains("limit"), "{error}");
        assert!(!root.join("out.jpg").exists());
        assert!(!root.join("out.jpg.purin-upscale.tmp").exists());
    }

    #[cfg(unix)]
    fn esrgan_plan(paths: &[PathBuf], out_dir: &Path) -> Vec<PlannedImage> {
        paths
            .iter()
            .map(|input| PlannedImage {
                input: input.clone(),
                output: Ok(out_dir.join(input.file_name().unwrap())),
            })
            .collect()
    }

    #[cfg(unix)]
    #[test]
    fn esrgan_rejects_non_utf8_paths_one_by_one() {
        use std::os::unix::ffi::OsStrExt;
        let root = TempDir::new("upscale_non_utf8");
        let good = root.join("good.png");
        let bad = root.join(std::ffi::OsStr::from_bytes(b"bad\xff.png"));
        let python = fake_program(
            &root,
            "fake-python",
            &format!("while [ $# -gt 0 ]; do if [ \"$1\" = --files ]; then cp \"$2\" '{}'; fi; shift; done\nprintf '%s\\n' '{{\"type\":\"progress\",\"current\":1,\"total\":1,\"filename\":\"good.png\",\"status\":\"success\",\"message\":\"ok\"}}' '{{\"type\":\"done\",\"success_count\":1,\"fail_count\":0,\"errors\":[]}}'\n", root.join("manifest.json").display()),
        );
        let app = tauri::test::mock_app();
        let events = capture_events(app.handle(), EVENT);
        let result = run_esrgan(
            app.handle(),
            &esrgan_plan(&[bad, good.clone()], &root.join("out")),
            || Ok(PythonCommand::new(&python)),
            &PythonTempTag::for_run(1),
            &AtomicBool::new(false),
            &Mutex::new(None),
        )
        .unwrap();
        assert_eq!(
            (result.success_count, result.fail_count, result.total),
            (1, 1, 2)
        );
        assert!(result.errors[0].contains("UTF-8"));
        let manifest: Vec<[String; 2]> =
            serde_json::from_slice(&std::fs::read(root.join("manifest.json")).unwrap()).unwrap();
        assert_eq!(manifest.len(), 1);
        assert_eq!(manifest[0][0], good.to_str().unwrap());
        let events = events.lock().unwrap();
        assert_eq!(events.iter().filter(|e| e["status"] == "error").count(), 1);
        // Python 的进度序号排在被拒的条目之后
        assert_eq!(
            events.iter().find(|e| e["status"] == "success").unwrap()["current"],
            2
        );
    }

    #[cfg(unix)]
    #[test]
    fn esrgan_cancel_removes_the_half_written_temp_file() {
        use tauri::Listener;
        let root = TempDir::new("upscale_cancel_temp");
        let input = root.join("a.png");
        let output = root.join("out/a.png");
        std::fs::create_dir_all(root.join("out")).unwrap();
        // 按 PURIN_TEMP_TAG 建临时文件后停在写盘中途
        let python = fake_program(
            &root,
            "fake-python",
            &format!("touch '{}'.\"$PURIN_TEMP_TAG\".tmp\nprintf '%s\\n' '{{\"type\":\"progress\",\"current\":1,\"total\":1,\"filename\":\"a.png\",\"status\":\"processing\",\"message\":\"writing\"}}'\nsleep 30\n", output.display()),
        );
        let cancel = std::sync::Arc::new(AtomicBool::new(false));
        let app = tauri::test::mock_app();
        let flag = cancel.clone();
        app.listen_any(EVENT, move |event| {
            if event.payload().contains("writing") {
                flag.store(true, Ordering::SeqCst);
            }
        });
        let tag = PythonTempTag::for_run(9);
        let started = std::time::Instant::now();
        let result = run_esrgan(
            app.handle(),
            &esrgan_plan(&[input], &root.join("out")),
            || Ok(PythonCommand::new(&python)),
            &tag,
            &cancel,
            &Mutex::new(None),
        )
        .unwrap();
        assert!(started.elapsed() < std::time::Duration::from_secs(10));
        assert_eq!(result.success_count, 0);
        assert!(!tag.temp_file(&output).exists());
        assert!(!output.exists());
    }

    /// 脚本自己崩溃（没被我们终止）也会留下写到一半的临时文件
    #[cfg(unix)]
    #[test]
    fn esrgan_crash_without_done_reports_stderr_and_removes_temp_files() {
        let root = TempDir::new("upscale_crash");
        let output = root.join("out/a.png");
        std::fs::create_dir_all(root.join("out")).unwrap();
        let python = fake_program(
            &root,
            "fake-python",
            &format!(
                "touch '{}'.\"$PURIN_TEMP_TAG\".tmp\necho 'Killed: out of memory' >&2\nexit 3\n",
                output.display()
            ),
        );
        let tag = PythonTempTag::for_run(2);
        let app = tauri::test::mock_app();
        let error = run_esrgan(
            app.handle(),
            &esrgan_plan(&[root.join("a.png")], &root.join("out")),
            || Ok(PythonCommand::new(&python)),
            &tag,
            &AtomicBool::new(false),
            &Mutex::new(None),
        )
        .unwrap_err();
        assert_eq!(error, "超分进程异常退出（退出码 3）: Killed: out of memory");
        assert!(!tag.temp_file(&output).exists());
    }

    #[cfg(unix)]
    #[test]
    fn esrgan_failure_notes_the_kept_output_only_when_one_exists() {
        let root = TempDir::new("upscale_esrgan_kept");
        std::fs::create_dir_all(root.join("out")).unwrap();
        std::fs::write(root.join("out/kept.png"), b"previous").unwrap();
        let progress = |current: u32, name: &str| {
            serde_json::json!({"type": "progress", "current": current, "total": 2, "filename": name,
                "status": "error", "message": format!("[{current}/2] ✗ {name}: 无法读取图片")})
        };
        let python = fake_program(
            &root,
            "fake-python",
            &format!(
                "printf '%s\\n' '{}' '{}' '{}'\n",
                progress(1, "kept.png"),
                progress(2, "fresh.png"),
                serde_json::json!({"type": "done", "success_count": 0, "fail_count": 2, "total": 2,
                    "errors": ["[1/2] ✗ kept.png: 无法读取图片", "[2/2] ✗ fresh.png: 无法读取图片"]})
            ),
        );
        let app = tauri::test::mock_app();
        let events = capture_events(app.handle(), EVENT);
        let result = run_esrgan(
            app.handle(),
            &esrgan_plan(
                &[root.join("kept.png"), root.join("fresh.png")],
                &root.join("out"),
            ),
            || Ok(PythonCommand::new(&python)),
            &PythonTempTag::for_run(3),
            &AtomicBool::new(false),
            &Mutex::new(None),
        )
        .unwrap();
        let expected = [
            format!("[1/2] ✗ kept.png: 无法读取图片{}", KEPT_OUTPUT_NOTE),
            "[2/2] ✗ fresh.png: 无法读取图片".to_string(),
        ];
        assert_eq!((result.success_count, result.fail_count), (0, 2));
        assert_eq!(result.errors, expected);
        let events = events.lock().unwrap();
        let shown: Vec<_> = events
            .iter()
            .filter(|e| e["status"] == "error")
            .map(|e| e["message"].as_str().unwrap().to_string())
            .collect();
        assert_eq!(shown, expected);
    }
}
