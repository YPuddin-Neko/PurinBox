use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::Emitter;

use super::{collect_image_files, ProcessResult, ProgressEvent};

static CANCEL_FLAG: AtomicBool = AtomicBool::new(false);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlurNoiseOptions {
    pub input_path: String,
    pub output_path: String,
    /// 高斯模糊半径 0.0 ~ 10.0 (0 = 不模糊)
    pub blur_radius: f64,
    /// 噪点强度 0 ~ 100 (0 = 不加噪点)
    pub noise_strength: u32,
}

#[tauri::command]
pub async fn blur_noise_images(app: tauri::AppHandle, options: BlurNoiseOptions) -> Result<ProcessResult, String> {
    CANCEL_FLAG.store(false, Ordering::SeqCst);
    tokio::task::spawn_blocking(move || {
        blur_noise_sync(&app, &options)
    })
    .await
    .map_err(|e| format!("任务执行失败: {}", e))?
}

#[tauri::command]
pub fn cancel_blur_noise() {
    CANCEL_FLAG.store(true, Ordering::SeqCst);
}

fn blur_noise_sync(app: &tauri::AppHandle, options: &BlurNoiseOptions) -> Result<ProcessResult, String> {
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

    let label = match (options.blur_radius > 0.0, options.noise_strength > 0) {
        (true, true) => "模糊+噪点",
        (true, false) => "高斯模糊",
        (false, true) => "噪点",
        _ => "处理",
    };

    for (i, file_path) in files.iter().enumerate() {
        if CANCEL_FLAG.load(Ordering::SeqCst) {
            let _ = app.emit("blur-noise-progress", ProgressEvent {
                current: i as u32, total, filename: String::new(),
                status: "done".to_string(),
                message: format!("已取消: 已处理 {}, 共 {}", i, total),
            });
            break;
        }

        let filename = file_path.file_name().unwrap_or_default().to_string_lossy().to_string();

        let _ = app.emit("blur-noise-progress", ProgressEvent {
            current: i as u32 + 1,
            total,
            filename: filename.clone(),
            status: "processing".to_string(),
            message: format!("正在处理: {}", filename),
        });

        match process_blur_noise(file_path, output_dir, options.blur_radius, options.noise_strength) {
            Ok(_) => {
                success_count += 1;
                let _ = app.emit("blur-noise-progress", ProgressEvent {
                    current: i as u32 + 1,
                    total,
                    filename: filename.clone(),
                    status: "success".to_string(),
                    message: format!("[{}] {} ✓", label, filename),
                });
            }
            Err(e) => {
                fail_count += 1;
                let err_msg = format!("{}: {}", filename, e);
                errors.push(err_msg.clone());
                let _ = app.emit("blur-noise-progress", ProgressEvent {
                    current: i as u32 + 1,
                    total,
                    filename: filename.clone(),
                    status: "error".to_string(),
                    message: format!("[失败] {}", err_msg),
                });
            }
        }
    }

    let _ = app.emit("blur-noise-progress", ProgressEvent {
        current: total,
        total,
        filename: String::new(),
        status: "done".to_string(),
        message: format!("处理完成: 成功 {}, 失败 {}, 共 {}", success_count, fail_count, total),
    });

    Ok(ProcessResult { success_count, fail_count, total, errors })
}

fn process_blur_noise(file_path: &Path, output_dir: &Path, blur_radius: f64, noise_strength: u32) -> Result<(), String> {
    let mut img = image::open(file_path)
        .map_err(|e| format!("无法打开图片: {}", e))?;

    // 高斯模糊
    if blur_radius > 0.0 {
        let sigma = blur_radius.max(0.1) as f32;
        img = image::DynamicImage::ImageRgba8(
            image::imageops::blur(&img, sigma)
        );
    }

    // 高斯噪点
    if noise_strength > 0 {
        let mut rgba = img.to_rgba8();
        let (w, h) = (rgba.width(), rgba.height());
        // 使用文件名hash做伪随机种子
        let mut seed: u64 = file_path.to_string_lossy().bytes()
            .fold(0u64, |acc, b| acc.wrapping_mul(31).wrapping_add(b as u64));

        let strength = noise_strength as f64;

        for y in 0..h {
            for x in 0..w {
                let pixel = rgba.get_pixel_mut(x, y);
                for c in 0..3 {
                    // 简单的Box-Muller近似高斯噪声
                    seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
                    let u1 = (seed >> 33) as f64 / (1u64 << 31) as f64;
                    seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
                    let u2 = (seed >> 33) as f64 / (1u64 << 31) as f64;

                    let u1_clamped = u1.max(1e-10);
                    let gaussian = (-2.0 * u1_clamped.ln()).sqrt() * (2.0 * std::f64::consts::PI * u2).cos();
                    let noise = gaussian * strength;

                    let val = pixel[c] as f64 + noise;
                    pixel[c] = val.round().clamp(0.0, 255.0) as u8;
                }
                // Alpha 通道不加噪点
            }
        }
        img = image::DynamicImage::ImageRgba8(rgba);
    }

    let file_name = file_path.file_name().ok_or("无效的文件名")?.to_string_lossy();
    let output_path = output_dir.join(file_name.as_ref());
    img.save(&output_path).map_err(|e| format!("无法保存图片: {}", e))?;
    Ok(())
}
