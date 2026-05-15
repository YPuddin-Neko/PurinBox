use image::imageops::FilterType;
use image::GenericImageView;
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::Emitter;

use super::{collect_image_files, ProcessResult, ProgressEvent};

static CANCEL_FLAG: AtomicBool = AtomicBool::new(false);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScaleOptions {
    pub input_path: String,
    pub output_path: String,
    /// "upscale" | "downscale" | "both"
    pub mode: String,
    pub target_width: u32,
    pub target_height: u32,
    /// 下采样目标（mode="both" 时使用）
    #[serde(default)]
    pub down_target_width: u32,
    #[serde(default)]
    pub down_target_height: u32,
}

#[tauri::command]
pub async fn scale_images(app: tauri::AppHandle, options: ScaleOptions) -> Result<ProcessResult, String> {
    CANCEL_FLAG.store(false, Ordering::SeqCst);
    tokio::task::spawn_blocking(move || {
        scale_images_sync(&app, &options)
    })
    .await
    .map_err(|e| format!("任务执行失败: {}", e))?
}

#[tauri::command]
pub fn cancel_scale() {
    CANCEL_FLAG.store(true, Ordering::SeqCst);
}

fn scale_images_sync(app: &tauri::AppHandle, options: &ScaleOptions) -> Result<ProcessResult, String> {
    let input = Path::new(&options.input_path);
    let output_dir = Path::new(&options.output_path);

    if !output_dir.exists() {
        std::fs::create_dir_all(output_dir)
            .map_err(|e| format!("无法创建输出目录: {}", e))?;
    }

    let files = collect_image_files(input)?;
    let total = files.len() as u32;
    let mut success_count = 0u32;
    let mut fail_count = 0u32;
    let mut errors = Vec::new();

    for (i, file_path) in files.iter().enumerate() {
        if CANCEL_FLAG.load(Ordering::SeqCst) {
            let _ = app.emit("scale-progress", ProgressEvent {
                current: i as u32, total, filename: String::new(),
                status: "done".to_string(),
                message: format!("已取消: 已处理 {}, 共 {}", i, total),
            });
            break;
        }
        let filename = file_path.file_name().unwrap_or_default().to_string_lossy().to_string();

        let _ = app.emit("scale-progress", ProgressEvent {
            current: i as u32 + 1,
            total,
            filename: filename.clone(),
            status: "processing".to_string(),
            message: format!("正在处理: {}", filename),
        });

        match process_scale(file_path, output_dir, options) {
            Ok(msg) => {
                success_count += 1;
                let _ = app.emit("scale-progress", ProgressEvent {
                    current: i as u32 + 1,
                    total,
                    filename: filename.clone(),
                    status: "success".to_string(),
                    message: msg,
                });
            }
            Err(e) => {
                fail_count += 1;
                let err_msg = format!("{}: {}", filename, e);
                errors.push(err_msg.clone());
                let _ = app.emit("scale-progress", ProgressEvent {
                    current: i as u32 + 1,
                    total,
                    filename: filename.clone(),
                    status: "error".to_string(),
                    message: err_msg,
                });
            }
        }
    }

    let _ = app.emit("scale-progress", ProgressEvent {
        current: total,
        total,
        filename: String::new(),
        status: "done".to_string(),
        message: format!("处理完成: 成功 {}, 失败 {}, 共 {}", success_count, fail_count, total),
    });

    Ok(ProcessResult { success_count, fail_count, total, errors })
}

/// Area-based proportional scaling (preserves aspect ratio, rounds to nearest multiple of 64)
fn area_scale(img: &image::DynamicImage, target_w: u32, target_h: u32) -> image::DynamicImage {
    let (orig_w, orig_h) = img.dimensions();
    let target_area = target_w as f64 * target_h as f64;
    let orig_area = orig_w as f64 * orig_h as f64;
    let scale = (target_area / orig_area).sqrt();

    let new_w = ((orig_w as f64 * scale / 64.0).round() * 64.0).max(64.0) as u32;
    let new_h = ((orig_h as f64 * scale / 64.0).round() * 64.0).max(64.0) as u32;

    img.resize_exact(new_w, new_h, FilterType::Lanczos3)
}

fn process_scale(
    file_path: &Path,
    output_dir: &Path,
    options: &ScaleOptions,
) -> Result<String, String> {
    let img = image::open(file_path)
        .map_err(|e| format!("无法打开图片: {}", e))?;

    let (orig_w, orig_h) = img.dimensions();
    let filename = file_path.file_name().unwrap_or_default().to_string_lossy().to_string();
    let output_path = output_dir.join(&filename);

    match options.mode.as_str() {
        "upscale" => {
            let target_w = options.target_width;
            let target_h = options.target_height;
            if orig_w < target_w || orig_h < target_h {
                let resized = area_scale(&img, target_w, target_h);
                let (nw, nh) = resized.dimensions();
                resized.save(&output_path).map_err(|e| format!("无法保存图片: {}", e))?;
                Ok(format!("[上采样] {} ({}x{} → {}x{})", filename, orig_w, orig_h, nw, nh))
            } else {
                std::fs::copy(file_path, &output_path).map_err(|e| format!("无法复制图片: {}", e))?;
                Ok(format!("[跳过] {} ({}x{}, 无需上采样)", filename, orig_w, orig_h))
            }
        }
        "downscale" => {
            let target_w = options.target_width;
            let target_h = options.target_height;
            if orig_w > target_w || orig_h > target_h {
                let resized = area_scale(&img, target_w, target_h);
                let (nw, nh) = resized.dimensions();
                resized.save(&output_path).map_err(|e| format!("无法保存图片: {}", e))?;
                Ok(format!("[下采样] {} ({}x{} → {}x{})", filename, orig_w, orig_h, nw, nh))
            } else {
                std::fs::copy(file_path, &output_path).map_err(|e| format!("无法复制图片: {}", e))?;
                Ok(format!("[跳过] {} ({}x{}, 无需下采样)", filename, orig_w, orig_h))
            }
        }
        "both" => {
            // 先上采样，再下采样
            let up_w = options.target_width;
            let up_h = options.target_height;
            let down_w = if options.down_target_width > 0 { options.down_target_width } else { up_w };
            let down_h = if options.down_target_height > 0 { options.down_target_height } else { up_h };

            let mut current = img;
            let mut steps = Vec::new();

            // Step 1: 上采样（小于上采样目标的图）
            let (cw, ch) = current.dimensions();
            if cw < up_w || ch < up_h {
                current = area_scale(&current, up_w, up_h);
                let (nw, nh) = current.dimensions();
                steps.push(format!("上采样 {}x{} → {}x{}", cw, ch, nw, nh));
            }

            // Step 2: 下采样（大于下采样目标的图）
            let (cw, ch) = current.dimensions();
            if cw > down_w || ch > down_h {
                current = area_scale(&current, down_w, down_h);
                let (nw, nh) = current.dimensions();
                steps.push(format!("下采样 {}x{} → {}x{}", cw, ch, nw, nh));
            }

            if steps.is_empty() {
                std::fs::copy(file_path, &output_path).map_err(|e| format!("无法复制图片: {}", e))?;
                Ok(format!("[跳过] {} ({}x{}, 已在目标范围内)", filename, orig_w, orig_h))
            } else {
                let (final_w, final_h) = current.dimensions();
                current.save(&output_path).map_err(|e| format!("无法保存图片: {}", e))?;
                Ok(format!("[缩放] {} ({}) → {}x{}", filename, steps.join(" → "), final_w, final_h))
            }
        }
        _ => Err("无效的缩放模式".to_string()),
    }
}

