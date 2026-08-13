use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use tauri::Emitter;

use super::{
    collect_image_files_with_recursive_excluding, finalize_part_file, output_path_for_input,
    prepare_part_file, ProcessResult, ProgressEvent,
};

/// 超分子进程句柄（Python 或 NCNN），用于强制取消
static ACTIVE_CHILD: Mutex<Option<u32>> = Mutex::new(None);

// ===== Upscale Engine Definitions =====

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpscaleEngineInfo {
    pub id: String,
    pub name: String,
    pub description: String,
    pub downloaded: bool,
    pub size_mb: f64,
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
    pub dir_name: String,
}

struct EngineDef {
    id: &'static str,
    name: &'static str,
    description: &'static str,
    bin_name: &'static str,
    size_mb: f64,
    scales: &'static [u32],
    supports_denoise: bool,
    denoise_range: (i32, i32),
    supports_cpu: bool,
    use_python: bool,
    models: &'static [(&'static str, &'static str, &'static str)], // (id, name, dir)
    #[cfg(target_os = "macos")]
    download_url: &'static str,
    #[cfg(target_os = "windows")]
    download_url: &'static str,
    #[cfg(target_os = "linux")]
    download_url: &'static str,
}

const ENGINES: &[EngineDef] = &[
    EngineDef {
        id: "realcugan",
        name: "Real-CUGAN",
        description: "常用的动漫超分，线条保留最好",
        bin_name: "realcugan-ncnn-vulkan",
        size_mb: 55.0,
        use_python: false,
        scales: &[2, 3, 4],
        supports_denoise: true,
        denoise_range: (-1, 3),
        supports_cpu: true,
        models: &[
            ("models-se", "标准版 (SE)", "models-se"),
            ("models-pro", "Pro版", "models-pro"),
            ("models-nose", "无降噪版", "models-nose"),
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
        description: "通用超分，真人/动漫/风景 (ONNX 推理)",
        bin_name: "",
        size_mb: 0.0,
        scales: &[2, 4],
        supports_denoise: false,
        denoise_range: (0, 0),
        supports_cpu: true,
        use_python: true,
        models: &[
            ("realesrgan-x4plus", "通用 (x4plus)", "realesrgan-x4plus"),
            ("realesrgan-x4plus-anime", "动漫 (x4plus-anime)", "realesrgan-x4plus-anime"),
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
        description: "经典动漫超分去噪",
        bin_name: "waifu2x-ncnn-vulkan",
        size_mb: 35.0,
        use_python: false,
        scales: &[1, 2, 4, 8],
        supports_denoise: true,
        denoise_range: (-1, 3),
        supports_cpu: false,
        models: &[
            ("models-cunet", "动漫 (CUNet)", "models-cunet"),
            ("models-upconv_7_anime_style_art_rgb", "动漫 (轻量)", "models-upconv_7_anime_style_art_rgb"),
            ("models-upconv_7_photo", "真实照片", "models-upconv_7_photo"),
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
    base.join("models").join("upscale_engines")
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

fn is_engine_ready(engine: &EngineDef) -> bool {
    if engine.use_python {
        // Python engine: need deps (torch+cv2) AND at least one model weight
        return is_python_deps_ready() && has_any_esrgan_weight();
    }
    engine_binary(engine).exists()
}

/// Real-ESRGAN 模型权重目录 — 统一到 models/upscale_engines/realesrgan/ 下
fn realesrgan_weights_dir() -> PathBuf {
    engine_dir("realesrgan")
}

/// 检查是否有至少一个 ESRGAN 模型权重文件 (.onnx)
fn has_any_esrgan_weight() -> bool {
    let dir = realesrgan_weights_dir();
    if !dir.exists() {
        return false;
    }
    dir.read_dir()
        .ok()
        .map(|mut entries| {
            entries.any(|e| {
                e.ok()
                    .map(|e| {
                        e.path()
                            .extension()
                            .map(|ext| ext == "onnx")
                            .unwrap_or(false)
                    })
                    .unwrap_or(false)
            })
        })
        .unwrap_or(false)
}

/// 检查 Python + onnxruntime + cv2 是否已安装
fn is_python_deps_ready() -> bool {
    let python = match super::python_env::get_python_exe() {
        Some(p) => p,
        None => return false,
    };
    let mut cmd = std::process::Command::new(&python);
    cmd.args(["-c", "import onnxruntime; import cv2"]);
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000);
    }
    cmd.output().map(|o| o.status.success()).unwrap_or(false)
}

// ===== Commands =====

#[tauri::command]
pub fn get_upscale_engines() -> Result<Vec<UpscaleEngineInfo>, String> {
    Ok(ENGINES
        .iter()
        .map(|e| UpscaleEngineInfo {
            id: e.id.to_string(),
            name: e.name.to_string(),
            description: e.description.to_string(),
            downloaded: is_engine_ready(e),
            size_mb: e.size_mb,
            scales: e.scales.to_vec(),
            models: e
                .models
                .iter()
                .map(|(id, name, dir)| UpscaleModelChoice {
                    id: id.to_string(),
                    name: name.to_string(),
                    dir_name: dir.to_string(),
                })
                .collect(),
            supports_denoise: e.supports_denoise,
            denoise_range: e.denoise_range,
            supports_cpu: e.supports_cpu,
            use_python: e.use_python,
        })
        .collect())
}

// ===== Download =====

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpscaleDownloadProgress {
    pub downloaded: u64,
    pub total: u64,
    pub percent: f32,
    pub speed_mbps: f64,
    pub status: String,
    pub message: String,
}

static DOWNLOAD_CANCEL: AtomicBool = AtomicBool::new(false);

#[tauri::command]
pub async fn download_upscale_engine(
    app: tauri::AppHandle,
    engine_id: String,
) -> Result<String, String> {
    DOWNLOAD_CANCEL.store(false, Ordering::SeqCst);

    let engine = ENGINES
        .iter()
        .find(|e| e.id == engine_id)
        .ok_or_else(|| format!("未知引擎: {}", engine_id))?;

    if is_engine_ready(engine) {
        let _ = app.emit(
            "upscale-download",
            UpscaleDownloadProgress {
                downloaded: 0,
                total: 0,
                percent: 100.0,
                speed_mbps: 0.0,
                status: "done".into(),
                message: format!("{} 已就绪", engine.name),
            },
        );
        return Ok("already_ready".into());
    }

    // Python engine: install deps + download model weights
    if engine.use_python {
        return download_python_engine(&app, engine).await;
    }

    let dest_dir = engine_dir(engine.id);
    if !dest_dir.exists() {
        std::fs::create_dir_all(&dest_dir).map_err(|e| format!("创建目录失败: {}", e))?;
    }

    let url = engine.download_url;
    let zip_path = dest_dir.join("_download.zip");

    let client = crate::commands::proxy_config::build_http_client()
        .user_agent("PurinBox/0.1.7")
        .timeout(std::time::Duration::from_secs(1200))
        .build()
        .map_err(|e| format!("HTTP 客户端失败: {}", e))?;

    let _ = app.emit(
        "upscale-download",
        UpscaleDownloadProgress {
            downloaded: 0,
            total: 0,
            percent: 0.0,
            speed_mbps: 0.0,
            status: "downloading".into(),
            message: format!("正在下载 {} ...", engine.name),
        },
    );

    let resp = client
        .get(url)
        .send()
        .await
        .map_err(|e| format!("请求失败: {}", e))?;
    if !resp.status().is_success() {
        return Err(format!("HTTP {}: {}", resp.status(), url));
    }

    let total_size = resp.content_length().unwrap_or(0);
    let mut stream = resp.bytes_stream();
    // 先写入 .part 临时文件，校验完成后再原子替换，避免中断残件
    let part_path = prepare_part_file(&zip_path);
    let mut file = tokio::fs::File::create(&part_path)
        .await
        .map_err(|e| format!("创建文件失败: {}", e))?;
    let mut downloaded: u64 = 0;
    let mut last_t = std::time::Instant::now();
    let mut last_b: u64 = 0;
    let start = std::time::Instant::now();

    // 下载循环结果 — 任何错误（含取消）统一在循环外清理 .part 残件
    let mut result: Result<(), String> = Ok(());

    while let Some(chunk) = stream.next().await {
        if DOWNLOAD_CANCEL.load(Ordering::SeqCst) {
            result = Err("下载已取消".into());
            break;
        }
        let chunk = match chunk {
            Ok(c) => c,
            Err(e) => {
                result = Err(format!("下载失败: {}", e));
                break;
            }
        };
        if let Err(e) = tokio::io::AsyncWriteExt::write_all(&mut file, &chunk).await {
            result = Err(format!("写入失败: {}", e));
            break;
        }
        downloaded += chunk.len() as u64;

        let now = std::time::Instant::now();
        let elapsed_ms = now.duration_since(last_t).as_millis();
        if elapsed_ms >= 500 || (total_size > 0 && downloaded >= total_size) {
            let speed = if elapsed_ms > 0 {
                (downloaded - last_b) as f64 / elapsed_ms as f64 * 1000.0 / 1_048_576.0
            } else {
                0.0
            };
            last_t = now;
            last_b = downloaded;
            let pct = if total_size > 0 {
                (downloaded as f64 / total_size as f64 * 100.0) as f32
            } else {
                0.0
            };
            let avg = {
                let t = start.elapsed().as_secs_f64();
                if t > 0.0 {
                    downloaded as f64 / t / 1_048_576.0
                } else {
                    0.0
                }
            };
            let mb_done = downloaded as f64 / 1_048_576.0;
            let msg = if total_size > 0 {
                format!(
                    "{} — {:.1}/{:.1} MB ({:.1} MB/s)",
                    engine.name,
                    mb_done,
                    total_size as f64 / 1_048_576.0,
                    avg
                )
            } else {
                format!("{} — {:.1} MB ({:.1} MB/s)", engine.name, mb_done, avg)
            };
            let _ = app.emit(
                "upscale-download",
                UpscaleDownloadProgress {
                    downloaded,
                    total: total_size,
                    percent: pct,
                    speed_mbps: speed,
                    status: "downloading".into(),
                    message: msg,
                },
            );
        }
    }

    // 确保缓冲数据全部落盘
    if result.is_ok() {
        if let Err(e) = tokio::io::AsyncWriteExt::flush(&mut file).await {
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
    finalize_part_file(&part_path, &zip_path, downloaded, total_size)?;

    // Extracting zip
    let _ = app.emit(
        "upscale-download",
        UpscaleDownloadProgress {
            downloaded,
            total: total_size,
            percent: 99.0,
            speed_mbps: 0.0,
            status: "extracting".into(),
            message: format!("{} — 正在解压...", engine.name),
        },
    );

    // Extract zip in blocking task
    let dest_dir_clone = dest_dir.clone();
    let zip_path_clone = zip_path.clone();
    let engine_name = engine.name.to_string();
    let bin_name = engine.bin_name.to_string();
    tokio::task::spawn_blocking(move || extract_zip(&zip_path_clone, &dest_dir_clone, &bin_name))
        .await
        .map_err(|e| format!("解压任务失败: {}", e))?
        .map_err(|e| format!("解压失败: {}", e))?;

    // Clean up zip
    let _ = tokio::fs::remove_file(&zip_path).await;

    // Set executable permission on Unix
    #[cfg(unix)]
    {
        let bin = engine_binary(engine);
        if bin.exists() {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755));
        }
    }

    let _ = app.emit(
        "upscale-download",
        UpscaleDownloadProgress {
            downloaded,
            total: total_size,
            percent: 100.0,
            speed_mbps: 0.0,
            status: "done".into(),
            message: format!("{} — 下载完成 ✓", engine_name),
        },
    );

    Ok("done".into())
}

fn extract_zip(zip_path: &Path, dest_dir: &Path, _bin_name: &str) -> Result<(), String> {
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

        // Strip the top-level directory prefix
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

#[tauri::command]
pub fn cancel_upscale_download() {
    DOWNLOAD_CANCEL.store(true, Ordering::SeqCst);
}

/// Real-ESRGAN Python 引擎下载: 安装 Python + PyTorch + OpenCV 依赖
/// 模型权重会在首次运行时由 Python 脚本自动下载
async fn download_python_engine(
    app: &tauri::AppHandle,
    engine: &EngineDef,
) -> Result<String, String> {
    let emit = |pct: f32, status: &str, msg: String| {
        let _ = app.emit(
            "upscale-download",
            UpscaleDownloadProgress {
                downloaded: 0,
                total: 0,
                percent: pct,
                speed_mbps: 0.0,
                status: status.into(),
                message: msg,
            },
        );
    };

    // Step 1: Ensure Python environment + onnxruntime GPU 运行时
    emit(5.0, "downloading", "正在检查 Python 环境...".into());
    let python = super::python_env::setup_python_env(app).await?;
    let _has_gpu = super::python_env::ensure_onnx_gpu_runtime(app, &python).await?;

    if DOWNLOAD_CANCEL.load(Ordering::SeqCst) {
        return Err("下载已取消".into());
    }

    // Step 2: Check and install OpenCV (only additional dep needed)
    let has_cv2 = {
        let p = python.clone();
        tokio::task::spawn_blocking(move || {
            let mut cmd = std::process::Command::new(&p);
            cmd.args(["-c", "import cv2"]);
            #[cfg(target_os = "windows")]
            {
                use std::os::windows::process::CommandExt;
                cmd.creation_flags(0x08000000);
            }
            cmd.output().map(|o| o.status.success()).unwrap_or(false)
        })
        .await
        .unwrap_or(false)
    };

    if !has_cv2 {
        emit(15.0, "downloading", "正在安装 OpenCV...".into());
        let p = python.clone();
        let app2 = app.clone();
        tokio::task::spawn_blocking(move || {
            super::python_env::pip_install_with_python(
                &app2,
                &p,
                &["opencv-python-headless==4.11.0.86"],
            )
        })
        .await
        .map_err(|e| format!("安装线程异常: {}", e))??;
        emit(30.0, "downloading", "OpenCV 安装完成".into());
    } else {
        emit(30.0, "downloading", "OpenCV 已就绪".into());
    }

    if DOWNLOAD_CANCEL.load(Ordering::SeqCst) {
        return Err("下载已取消".into());
    }

    // Step 3: Download ONNX model weights
    let weights_dir = realesrgan_weights_dir();
    std::fs::create_dir_all(&weights_dir).ok();

    // ONNX models (single file, weights embedded)
    let onnx_models: &[(&str, &str)] = &[
        ("RealESRGAN_x4plus.onnx", "https://github.com/YPuddin-Neko/PurinBox/releases/download/models/RealESRGAN_x4plus.onnx"),
        ("RealESRGAN_x4plus_anime_6B.onnx", "https://github.com/YPuddin-Neko/PurinBox/releases/download/models/RealESRGAN_x4plus_anime_6B.onnx"),
    ];

    let client = crate::commands::proxy_config::build_http_client()
        .user_agent("PurinBox/0.3.7")
        .timeout(std::time::Duration::from_secs(600))
        .build()
        .map_err(|e| format!("HTTP 客户端失败: {}", e))?;

    let total_files = onnx_models.len();
    for (fi, (filename, url)) in onnx_models.iter().enumerate() {
        let dest = weights_dir.join(filename);
        if dest.exists() {
            let base_pct = 30.0 + ((fi + 1) as f32 / total_files as f32) * 68.0;
            emit(
                base_pct,
                "downloading",
                format!("{} 已存在，跳过", filename),
            );
            continue;
        }

        let resp = client
            .get(*url)
            .send()
            .await
            .map_err(|e| format!("{} 下载请求失败: {}", filename, e))?;
        if !resp.status().is_success() {
            return Err(format!("{} 下载失败 HTTP {}", filename, resp.status()));
        }

        let total_size = resp.content_length().unwrap_or(0);
        let mut stream = resp.bytes_stream();
        // 先写入 .part 临时文件，校验完成后再原子替换到最终路径，避免中断残件被当作完整文件
        let part_path = prepare_part_file(&dest);
        let mut file = tokio::fs::File::create(&part_path)
            .await
            .map_err(|e| format!("创建文件失败: {}", e))?;
        let mut downloaded: u64 = 0;
        let start = std::time::Instant::now();
        let mut last_t = std::time::Instant::now();

        // 下载循环结果 — 任何错误（含取消）统一在循环外清理 .part 残件
        let mut result: Result<(), String> = Ok(());

        while let Some(chunk) = stream.next().await {
            if DOWNLOAD_CANCEL.load(Ordering::SeqCst) {
                result = Err("下载已取消".into());
                break;
            }
            let chunk = match chunk {
                Ok(c) => c,
                Err(e) => {
                    result = Err(format!("下载失败: {}", e));
                    break;
                }
            };
            if let Err(e) = tokio::io::AsyncWriteExt::write_all(&mut file, &chunk).await {
                result = Err(format!("写入失败: {}", e));
                break;
            }
            downloaded += chunk.len() as u64;

            let now = std::time::Instant::now();
            if now.duration_since(last_t).as_millis() >= 500
                || (total_size > 0 && downloaded >= total_size)
            {
                last_t = now;
                let avg = {
                    let t = start.elapsed().as_secs_f64();
                    if t > 0.0 {
                        downloaded as f64 / t / 1_048_576.0
                    } else {
                        0.0
                    }
                };
                let mb_done = downloaded as f64 / 1_048_576.0;
                let base_pct = 30.0 + (fi as f32 / total_files as f32) * 68.0;
                let file_pct = if total_size > 0 {
                    (downloaded as f32 / total_size as f32) * (68.0 / total_files as f32)
                } else {
                    0.0
                };
                let pct = base_pct + file_pct;
                let msg = if total_size > 0 {
                    format!(
                        "{} — {:.1}/{:.1} MB ({:.1} MB/s)",
                        filename,
                        mb_done,
                        total_size as f64 / 1_048_576.0,
                        avg
                    )
                } else {
                    format!("{} — {:.1} MB ({:.1} MB/s)", filename, mb_done, avg)
                };
                let _ = app.emit(
                    "upscale-download",
                    UpscaleDownloadProgress {
                        downloaded,
                        total: total_size,
                        percent: pct,
                        speed_mbps: avg,
                        status: "downloading".into(),
                        message: msg,
                    },
                );
            }
        }

        // 确保缓冲数据全部落盘
        if result.is_ok() {
            if let Err(e) = tokio::io::AsyncWriteExt::flush(&mut file).await {
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
        finalize_part_file(&part_path, &dest, downloaded, total_size)?;
    }

    emit(100.0, "done", format!("{} 准备完成 ✓", engine.name));

    Ok("done".into())
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

#[tauri::command]
pub async fn start_upscale(
    app: tauri::AppHandle,
    options: UpscaleOptions,
) -> Result<ProcessResult, String> {
    CANCEL_FLAG.store(false, Ordering::SeqCst);

    let engine = ENGINES
        .iter()
        .find(|e| e.id == options.engine_id)
        .ok_or_else(|| format!("未知引擎: {}", options.engine_id))?;

    // Python 引擎: 依赖已在 download_upscale_engine 中安装完成
    if engine.use_python {
        return run_python_upscale(&app, &options).await;
    }

    // NCNN 引擎: 检查二进制是否存在
    if !engine_binary(engine).exists() {
        return Err(format!("{} 尚未下载，请先下载", engine.name));
    }

    // NCNN 逐图阻塞循环放入 blocking 线程，避免占死 tokio worker
    let app_clone = app.clone();
    tokio::task::spawn_blocking(move || run_ncnn_upscale(&app_clone, engine, &options))
        .await
        .map_err(|e| format!("任务执行失败: {}", e))?
}

/// NCNN 引擎逐图超分（阻塞循环，需在 spawn_blocking 中运行）
fn run_ncnn_upscale(
    app: &tauri::AppHandle,
    engine: &EngineDef,
    options: &UpscaleOptions,
) -> Result<ProcessResult, String> {
    let bin = engine_binary(engine);
    let input = Path::new(&options.input_path);
    let output_dir = Path::new(&options.output_path);

    if !output_dir.exists() {
        std::fs::create_dir_all(output_dir).map_err(|e| format!("创建输出目录失败: {}", e))?;
    }

    let files =
        collect_image_files_with_recursive_excluding(input, options.recursive, Some(output_dir))?;
    if files.is_empty() {
        return Err("未找到任何图片".into());
    }

    let total = files.len() as u32;
    let mut success_count = 0u32;
    let mut fail_count = 0u32;
    let mut errors = Vec::new();

    let _ = app.emit(
        "upscale-progress",
        ProgressEvent {
            current: 0,
            total,
            filename: String::new(),
            status: "processing".to_string(),
            message: format!(
                "开始超分: 共 {} 张, 引擎: {}, 倍率: {}x",
                total, engine.name, options.scale
            ),
            ..Default::default()
        },
    );

    let engine_dir = engine_dir(engine.id);

    // For Real-ESRGAN, the -n flag is model name, not denoise
    // For Real-CUGAN and Waifu2x, -n is noise-level and -m is model path
    let model_choice = engine
        .models
        .iter()
        .find(|m| m.0 == options.model_id)
        .unwrap_or(&engine.models[0]);

    for (i, file_path) in files.iter().enumerate() {
        if CANCEL_FLAG.load(Ordering::SeqCst) {
            let _ = app.emit(
                "upscale-progress",
                ProgressEvent {
                    current: i as u32,
                    total,
                    filename: String::new(),
                    status: "done".to_string(),
                    message: format!("已取消: 已处理 {}, 共 {}", i, total),
                    ..Default::default()
                },
            );
            break;
        }

        let filename = file_path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();

        let _ = app.emit(
            "upscale-progress",
            ProgressEvent {
                current: i as u32,
                total,
                filename: filename.clone(),
                status: "processing".to_string(),
                message: format!("[{}/{}] 正在处理: {}", i + 1, total, filename),
                ..Default::default()
            },
        );

        // Build output filename — keep name, force png output
        let stem = file_path.file_stem().unwrap_or_default().to_string_lossy();
        let out_name = format!("{}.png", stem);
        let out_file =
            output_path_for_input(input, file_path, output_dir, &out_name, options.recursive)?;

        // Build command
        let mut cmd = std::process::Command::new(&bin);

        // Common args
        cmd.arg("-i")
            .arg(file_path)
            .arg("-o")
            .arg(&out_file)
            .arg("-s")
            .arg(options.scale.to_string())
            .arg("-t")
            .arg(if options.tile_size < 0 {
                "0".to_string()
            } else {
                options.tile_size.to_string()
            });

        // GPU selection: macOS NCNN builds don't support -g -1 (CPU mode)
        if options.gpu_id >= 0 {
            cmd.arg("-g").arg(options.gpu_id.to_string());
        }

        match engine.id {
            "realcugan" => {
                cmd.arg("-n").arg(options.denoise_level.to_string());
                cmd.arg("-m").arg(engine_dir.join(model_choice.2));
            }
            "realesrgan" => {
                // Real-ESRGAN uses -n for model name, -m for model dir
                cmd.arg("-n").arg(model_choice.2);
                cmd.arg("-m").arg(engine_dir.join("models"));
            }
            "waifu2x" => {
                cmd.arg("-n").arg(options.denoise_level.to_string());
                cmd.arg("-m").arg(engine_dir.join(model_choice.2));
            }
            _ => {}
        }

        // TTA mode: -x flag (all engines support it)
        if options.tta {
            cmd.arg("-x");
        }

        // 捕获输出（必须 piped，否则 wait_with_output 拿到的 stderr 恒为空，错误信息丢失）
        cmd.stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());

        // Quiet: hide console window on Windows
        #[cfg(target_os = "windows")]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
        }

        let child = cmd.spawn().map_err(|e| {
            fail_count += 1;
            let err_msg = format!("{}: 启动失败 - {}", filename, e);
            errors.push(err_msg.clone());
            let _ = app.emit(
                "upscale-progress",
                ProgressEvent {
                    current: i as u32 + 1,
                    total,
                    filename: filename.clone(),
                    status: "error".to_string(),
                    message: format!("[{}/{}] ✗ {}", i + 1, total, err_msg),
                    ..Default::default()
                },
            );
            err_msg
        });

        let child = match child {
            Ok(c) => c,
            Err(_) => continue,
        };

        // 存储 PID 以便强制取消
        if let Ok(mut guard) = ACTIVE_CHILD.lock() {
            *guard = Some(child.id());
        }

        let output = child.wait_with_output();

        // 清除 PID
        if let Ok(mut guard) = ACTIVE_CHILD.lock() {
            *guard = None;
        }

        match output {
            Ok(output) => {
                if output.status.success() && out_file.exists() {
                    success_count += 1;
                    let _ = app.emit(
                        "upscale-progress",
                        ProgressEvent {
                            current: i as u32 + 1,
                            total,
                            filename: filename.clone(),
                            status: "success".to_string(),
                            message: format!("[{}/{}] ✓ {}", i + 1, total, filename),
                            ..Default::default()
                        },
                    );
                } else {
                    fail_count += 1;
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    let err_msg = format!(
                        "{}: {}",
                        filename,
                        stderr.lines().last().unwrap_or("未知错误")
                    );
                    errors.push(err_msg.clone());
                    let _ = app.emit(
                        "upscale-progress",
                        ProgressEvent {
                            current: i as u32 + 1,
                            total,
                            filename: filename.clone(),
                            status: "error".to_string(),
                            message: format!("[{}/{}] ✗ {}", i + 1, total, err_msg),
                            ..Default::default()
                        },
                    );
                }
            }
            Err(e) => {
                fail_count += 1;
                let err_msg = format!("{}: 执行失败 - {}", filename, e);
                errors.push(err_msg.clone());
                let _ = app.emit(
                    "upscale-progress",
                    ProgressEvent {
                        current: i as u32 + 1,
                        total,
                        filename: filename.clone(),
                        status: "error".to_string(),
                        message: format!("[{}/{}] ✗ {}", i + 1, total, err_msg),
                        ..Default::default()
                    },
                );
            }
        }
    }

    let _ = app.emit(
        "upscale-progress",
        ProgressEvent {
            current: total,
            total,
            filename: String::new(),
            status: "done".to_string(),
            message: format!(
                "完成: 成功 {}, 失败 {}, 共 {}",
                success_count, fail_count, total
            ),
            ..Default::default()
        },
    );

    Ok(ProcessResult {
        success_count,
        fail_count,
        total,
        errors,
    })
}

// ===== Python Upscale =====

/// 获取 Python 超分脚本路径
fn get_upscale_script() -> Result<PathBuf, String> {
    super::python_proc::find_script("realesrgan_upscale.py")
}

/// 通过 Python 脚本执行 Real-ESRGAN 超分
async fn run_python_upscale(
    app: &tauri::AppHandle,
    options: &UpscaleOptions,
) -> Result<ProcessResult, String> {
    use std::io::BufRead;

    let python = super::python_env::get_python_exe().ok_or("Python 环境未就绪，请先安装")?;
    let script = get_upscale_script()?;

    // 确定设备参数
    let device = if options.gpu_id < 0 {
        "cpu".to_string()
    } else {
        "auto".to_string()
    };

    let input = options.input_path.clone();
    let output = options.output_path.clone();
    let model = options.model_id.clone();
    let scale = options.scale;
    let tile = options.tile_size;
    let tta = options.tta;
    let recursive = options.recursive;
    let app_clone = app.clone();

    let weights_dir = realesrgan_weights_dir();

    tokio::task::spawn_blocking(move || {
        let mut cmd = std::process::Command::new(&python);
        cmd.arg(script.to_string_lossy().as_ref())
            .arg("--input")
            .arg(&input)
            .arg("--output")
            .arg(&output)
            .arg("--model")
            .arg(&model)
            .arg("--scale")
            .arg(scale.to_string())
            .arg("--tile")
            .arg(if tile < 0 {
                "0".to_string()
            } else {
                tile.to_string()
            })
            .arg("--device")
            .arg(&device)
            .arg("--weights-dir")
            .arg(&weights_dir)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .env("PYTHONUNBUFFERED", "1")
            .env("PYTHONIOENCODING", "utf-8");

        if tta {
            cmd.arg("--tta");
        }
        if recursive {
            cmd.arg("--recursive");
        }

        // Windows: 无窗口 + GPU 模式注入 CUDA/cuDNN DLL 路径（共享实现见 python_proc）
        super::python_proc::configure_python_command(&mut cmd, device != "cpu");

        let mut child = cmd
            .spawn()
            .map_err(|e| format!("启动 Python 失败: {}", e))?;

        // 存储子进程 PID 以便强制取消
        if let Ok(mut guard) = ACTIVE_CHILD.lock() {
            *guard = Some(child.id());
        }

        let (stdout, stderr) = match (child.stdout.take(), child.stderr.take()) {
            (Some(o), Some(e)) => (o, e),
            _ => {
                // 取管道失败：杀掉并回收子进程，清空 PID 记录
                let _ = child.kill();
                let _ = child.wait();
                if let Ok(mut guard) = ACTIVE_CHILD.lock() {
                    *guard = None;
                }
                return Err("无法获取 Python 进程管道".into());
            }
        };

        // stderr 线程: 输出 Python 警告到日志（过滤 onnxruntime 内部告警）
        let app_err = app_clone.clone();
        std::thread::spawn(move || {
            let mut reader = std::io::BufReader::new(stderr);
            let mut buf = Vec::new();
            use std::io::Read;
            let mut byte = [0u8; 1];
            loop {
                match reader.read(&mut byte) {
                    Ok(0) => break,
                    Ok(_) => {
                        if byte[0] == b'\n' {
                            let line = String::from_utf8(buf.clone()).unwrap_or_else(|_| {
                                let (s, _, _) = encoding_rs::GBK.decode(&buf);
                                s.to_string()
                            });
                            buf.clear();
                            let clean = line.trim();
                            if clean.is_empty() {
                                continue;
                            }
                            // 去除 ANSI 转义序列
                            let clean = clean
                                .replace('\x1b', "")
                                .replace("[0m", "")
                                .replace("[1m", "")
                                .replace("[31m", "")
                                .replace("[33m", "");
                            let clean = clean.trim();
                            if clean.is_empty() {
                                continue;
                            }
                            // 过滤 onnxruntime 内部日志
                            if clean.contains("[W:onnxruntime:")
                                || clean.contains("[I:onnxruntime:")
                            {
                                continue;
                            }
                            // 过滤 cuDNN/CUDA 加载警告和编码乱码
                            let lower = clean.to_lowercase();
                            if lower.contains("cudnn")
                                || lower.contains("cuda_path")
                                || lower.contains("could not load")
                                || lower.contains("loaded library")
                            {
                                continue;
                            }

                            let _ = app_err.emit(
                                "upscale-progress",
                                ProgressEvent {
                                    current: 0,
                                    total: 0,
                                    filename: String::new(),
                                    status: "warning".to_string(),
                                    message: format!("[Python] {}", clean),
                                    ..Default::default()
                                },
                            );
                        } else if byte[0] != b'\r' {
                            buf.push(byte[0]);
                        }
                    }
                    Err(_) => break,
                }
            }
        });

        // 解析 stdout JSON 行
        let reader = std::io::BufReader::new(stdout);
        let mut success_count = 0u32;
        let mut fail_count = 0u32;
        let mut total = 0u32;
        let mut errors = Vec::new();

        for line in reader.lines().map_while(Result::ok) {
            if CANCEL_FLAG.load(Ordering::SeqCst) {
                let _ = child.kill();
                break;
            }

            if let Ok(msg) = serde_json::from_str::<serde_json::Value>(&line) {
                let msg_type = msg.get("type").and_then(|v| v.as_str()).unwrap_or("");
                match msg_type {
                    "log" => {
                        let text = msg
                            .get("message")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                        let i18n_key = msg
                            .get("i18n_key")
                            .and_then(|v| v.as_str())
                            .map(|s| s.to_string());
                        let i18n_params = msg.get("i18n_params").cloned();
                        let _ = app_clone.emit(
                            "upscale-progress",
                            ProgressEvent {
                                current: 0,
                                total: 0,
                                filename: String::new(),
                                status: "info".to_string(),
                                message: text,
                                i18n_key,
                                i18n_params,
                            },
                        );
                    }
                    "error" => {
                        let text = msg.get("message").and_then(|v| v.as_str()).unwrap_or("");
                        // 出错时先杀掉并回收子进程，再清空 PID 记录，
                        // 避免僵尸进程以及之后"强制取消"对陈旧 PID 误杀无关进程
                        let _ = child.kill();
                        let _ = child.wait();
                        if let Ok(mut guard) = ACTIVE_CHILD.lock() {
                            *guard = None;
                        }
                        return Err(format!("Real-ESRGAN 错误: {}", text));
                    }
                    "progress" => {
                        let cur = msg.get("current").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
                        let tot = msg.get("total").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
                        let fname = msg.get("filename").and_then(|v| v.as_str()).unwrap_or("");
                        let status = msg
                            .get("status")
                            .and_then(|v| v.as_str())
                            .unwrap_or("processing");
                        let message = msg.get("message").and_then(|v| v.as_str()).unwrap_or("");
                        total = tot;

                        if status == "success" {
                            success_count += 1;
                        } else if status == "error" {
                            fail_count += 1;
                            errors.push(message.to_string());
                        }

                        let _ = app_clone.emit(
                            "upscale-progress",
                            ProgressEvent {
                                current: cur,
                                total: tot,
                                filename: fname.to_string(),
                                status: status.to_string(),
                                message: message.to_string(),
                                ..Default::default()
                            },
                        );
                    }
                    "done" => {
                        success_count =
                            msg.get("success").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
                        fail_count = msg.get("fail").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
                        total = msg.get("total").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
                    }
                    _ => {}
                }
            }
        }

        let _ = child.wait();
        // 清除子进程 PID
        if let Ok(mut guard) = ACTIVE_CHILD.lock() {
            *guard = None;
        }

        let _ = app_clone.emit(
            "upscale-progress",
            ProgressEvent {
                current: total,
                total,
                filename: String::new(),
                status: "done".to_string(),
                message: format!(
                    "完成: 成功 {}, 失败 {}, 共 {}",
                    success_count, fail_count, total
                ),
                ..Default::default()
            },
        );

        Ok(ProcessResult {
            success_count,
            fail_count,
            total,
            errors,
        })
    })
    .await
    .map_err(|e| format!("任务执行失败: {}", e))?
}

#[tauri::command]
pub fn cancel_upscale() {
    CANCEL_FLAG.store(true, Ordering::SeqCst);
}

/// 强制取消超分（终止整个子进程树）
#[tauri::command]
pub fn force_cancel_upscale() {
    CANCEL_FLAG.store(true, Ordering::SeqCst);
    if let Ok(mut guard) = ACTIVE_CHILD.lock() {
        if let Some(pid) = guard.take() {
            super::kill_process_tree(pid);
        }
    }
}
