use image::GenericImageView;
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::Emitter;

use super::{collect_image_files, ProcessResult, ProgressEvent};

static CANCEL_FLAG: AtomicBool = AtomicBool::new(false);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CropOptions {
    pub input_path: String,
    pub output_path: String,
    /// "center" | "aspect" | "edges"
    pub mode: String,
    /// 中心裁切: 目标宽度
    pub target_width: u32,
    /// 中心裁切: 目标高度
    pub target_height: u32,
    /// 宽高比裁切: 宽高比（如 1.0 = 1:1, 0.75 = 3:4）
    pub aspect_ratio: f64,
    /// 边缘裁切: 上下左右像素
    pub crop_top: u32,
    pub crop_bottom: u32,
    pub crop_left: u32,
    pub crop_right: u32,
}

#[tauri::command]
pub async fn crop_images(app: tauri::AppHandle, options: CropOptions) -> Result<ProcessResult, String> {
    CANCEL_FLAG.store(false, Ordering::SeqCst);
    tokio::task::spawn_blocking(move || {
        crop_images_sync(&app, &options)
    })
    .await
    .map_err(|e| format!("任务执行失败: {}", e))?
}

#[tauri::command]
pub fn cancel_crop() {
    CANCEL_FLAG.store(true, Ordering::SeqCst);
}

fn crop_images_sync(app: &tauri::AppHandle, options: &CropOptions) -> Result<ProcessResult, String> {
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
            let _ = app.emit("crop-progress", ProgressEvent {
                current: i as u32, total, filename: String::new(),
                status: "done".to_string(),
                message: format!("已取消: 已处理 {}, 共 {}", i, total),
            });
            break;
        }
        let filename = file_path.file_name().unwrap_or_default().to_string_lossy().to_string();

        let _ = app.emit("crop-progress", ProgressEvent {
            current: i as u32 + 1,
            total,
            filename: filename.clone(),
            status: "processing".to_string(),
            message: format!("正在处理: {}", filename),
        });

        match process_crop(file_path, output_dir, options) {
            Ok(msg) => {
                success_count += 1;
                let _ = app.emit("crop-progress", ProgressEvent {
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
                let _ = app.emit("crop-progress", ProgressEvent {
                    current: i as u32 + 1,
                    total,
                    filename: filename.clone(),
                    status: "error".to_string(),
                    message: err_msg,
                });
            }
        }
    }

    let _ = app.emit("crop-progress", ProgressEvent {
        current: total,
        total,
        filename: String::new(),
        status: "done".to_string(),
        message: format!("处理完成: 成功 {}, 失败 {}, 共 {}", success_count, fail_count, total),
    });

    Ok(ProcessResult { success_count, fail_count, total, errors })
}

fn process_crop(
    file_path: &Path,
    output_dir: &Path,
    options: &CropOptions,
) -> Result<String, String> {
    let img = image::open(file_path)
        .map_err(|e| format!("无法打开图片: {}", e))?;

    let (orig_w, orig_h) = img.dimensions();
    let filename = file_path.file_name().unwrap_or_default().to_string_lossy().to_string();
    let output_path = output_dir.join(&filename);

    match options.mode.as_str() {
        "center" => {
            let tw = options.target_width.min(orig_w);
            let th = options.target_height.min(orig_h);
            if tw == orig_w && th == orig_h {
                // 无需裁切
                std::fs::copy(file_path, &output_path)
                    .map_err(|e| format!("无法复制图片: {}", e))?;
                return Ok(format!("[跳过] {} ({}x{}, 无需裁切)", filename, orig_w, orig_h));
            }
            let x = (orig_w - tw) / 2;
            let y = (orig_h - th) / 2;
            let cropped = img.crop_imm(x, y, tw, th);
            cropped.save(&output_path)
                .map_err(|e| format!("无法保存图片: {}", e))?;
            Ok(format!("[中心裁切] {} ({}x{} → {}x{})", filename, orig_w, orig_h, tw, th))
        }
        "aspect" => {
            let target_ratio = options.aspect_ratio;
            if target_ratio <= 0.0 {
                return Err("无效的宽高比".to_string());
            }
            let current_ratio = orig_w as f64 / orig_h as f64;

            if (current_ratio - target_ratio).abs() < 0.01 {
                std::fs::copy(file_path, &output_path)
                    .map_err(|e| format!("无法复制图片: {}", e))?;
                return Ok(format!("[跳过] {} ({}x{}, 比例已匹配)", filename, orig_w, orig_h));
            }

            let (tw, th) = if current_ratio > target_ratio {
                // 图片太宽 → 裁宽度
                let new_w = (orig_h as f64 * target_ratio) as u32;
                (new_w.min(orig_w), orig_h)
            } else {
                // 图片太高 → 裁高度
                let new_h = (orig_w as f64 / target_ratio) as u32;
                (orig_w, new_h.min(orig_h))
            };

            let x = (orig_w - tw) / 2;
            let y = (orig_h - th) / 2;
            let cropped = img.crop_imm(x, y, tw, th);
            cropped.save(&output_path)
                .map_err(|e| format!("无法保存图片: {}", e))?;
            Ok(format!("[比例裁切] {} ({}x{} → {}x{}, 比例 {:.2})", filename, orig_w, orig_h, tw, th, target_ratio))
        }
        "edges" => {
            let ct = options.crop_top;
            let cb = options.crop_bottom;
            let cl = options.crop_left;
            let cr = options.crop_right;

            if ct + cb >= orig_h || cl + cr >= orig_w {
                return Err(format!("裁切边距超过图片尺寸 ({}x{})", orig_w, orig_h));
            }

            if ct == 0 && cb == 0 && cl == 0 && cr == 0 {
                std::fs::copy(file_path, &output_path)
                    .map_err(|e| format!("无法复制图片: {}", e))?;
                return Ok(format!("[跳过] {} ({}x{}, 无需裁切)", filename, orig_w, orig_h));
            }

            let tw = orig_w - cl - cr;
            let th = orig_h - ct - cb;
            let cropped = img.crop_imm(cl, ct, tw, th);
            cropped.save(&output_path)
                .map_err(|e| format!("无法保存图片: {}", e))?;
            Ok(format!("[边缘裁切] {} ({}x{} → {}x{}, 上{}下{}左{}右{})", filename, orig_w, orig_h, tw, th, ct, cb, cl, cr))
        }
        _ => Err("无效的裁切模式".to_string()),
    }
}
