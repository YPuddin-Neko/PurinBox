use image::{DynamicImage, GenericImageView};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::Emitter;

use super::image_io::{load_image, save_like_source};
use super::{
    collect_image_files_with_recursive_excluding, output_path_for_input, ProcessResult,
    ProgressEvent,
};

static CANCEL_FLAG: AtomicBool = AtomicBool::new(false);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlphaConvertOptions {
    pub input_path: String,
    pub output_path: String,
    /// 替换透明区域的背景色: "white" | "black"
    pub background: String,
    #[serde(default)]
    pub recursive: bool,
}

/// 检测图片是否有透明通道（存在非完全不透明的像素）
fn has_alpha(img: &DynamicImage) -> bool {
    match img {
        DynamicImage::ImageRgba8(rgba) => rgba.pixels().any(|p| p[3] < 255),
        DynamicImage::ImageRgba16(rgba) => rgba.pixels().any(|p| p[3] < 65535),
        DynamicImage::ImageRgba32F(rgba) => rgba.pixels().any(|p| p[3] < 1.0),
        DynamicImage::ImageLumaA8(la) => la.pixels().any(|p| p[1] < 255),
        DynamicImage::ImageLumaA16(la) => la.pixels().any(|p| p[1] < 65535),
        _ => false,
    }
}

#[tauri::command]
pub async fn convert_alpha<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    options: AlphaConvertOptions,
) -> Result<ProcessResult, String> {
    // 互斥：页面与工作流节点共用全局取消标志，并发会互吞取消
    static RUNNING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    let _busy = crate::commands::BusyGuard::acquire(&RUNNING, "透明通道转换")?;

    CANCEL_FLAG.store(false, Ordering::SeqCst);
    tokio::task::spawn_blocking(move || convert_alpha_sync(&app, &options))
        .await
        .map_err(|e| format!("任务执行失败: {}", e))?
}

#[tauri::command]
pub fn cancel_alpha() {
    CANCEL_FLAG.store(true, Ordering::SeqCst);
}

fn convert_alpha_sync<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    options: &AlphaConvertOptions,
) -> Result<ProcessResult, String> {
    let input = Path::new(&options.input_path);
    let output_dir = Path::new(&options.output_path);

    if !output_dir.exists() {
        std::fs::create_dir_all(output_dir).map_err(|e| format!("无法创建输出目录: {}", e))?;
    }

    let files =
        collect_image_files_with_recursive_excluding(input, options.recursive, Some(output_dir))?;
    let total = files.len() as u32;
    let mut success_count = 0u32;
    let mut fail_count = 0u32;
    let mut skipped = 0u32;
    let mut errors = Vec::new();

    let bg_color: [u8; 3] = match options.background.as_str() {
        "black" => [0, 0, 0],
        _ => [255, 255, 255],
    };

    for (i, file_path) in files.iter().enumerate() {
        if CANCEL_FLAG.load(Ordering::SeqCst) {
            let _ = app.emit(
                "alpha-progress",
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
            "alpha-progress",
            ProgressEvent {
                current: i as u32 + 1,
                total,
                filename: filename.clone(),
                status: "processing".to_string(),
                message: format!("正在检测: {}", filename),
                ..Default::default()
            },
        );

        match process_alpha(file_path, input, output_dir, options, &bg_color) {
            Ok(converted) => {
                if converted {
                    success_count += 1;
                    let _ = app.emit(
                        "alpha-progress",
                        ProgressEvent {
                            current: i as u32 + 1,
                            total,
                            filename: filename.clone(),
                            status: "success".to_string(),
                            message: format!("[转换] {} (检测到透明通道, 已转换)", filename),
                            ..Default::default()
                        },
                    );
                } else {
                    skipped += 1;
                    let _ = app.emit(
                        "alpha-progress",
                        ProgressEvent {
                            current: i as u32 + 1,
                            total,
                            filename: filename.clone(),
                            status: "success".to_string(),
                            message: format!("[跳过] {} (无透明通道)", filename),
                            ..Default::default()
                        },
                    );
                }
            }
            Err(e) => {
                fail_count += 1;
                let err_msg = format!("{}: {}", filename, e);
                errors.push(err_msg.clone());
                let _ = app.emit(
                    "alpha-progress",
                    ProgressEvent {
                        current: i as u32 + 1,
                        total,
                        filename: filename.clone(),
                        status: "error".to_string(),
                        message: format!("[错误] {}", err_msg),
                        ..Default::default()
                    },
                );
            }
        }
    }

    // 取消路径已发过"已取消"的 done 事件，这里不再发完成事件覆盖它
    if !CANCEL_FLAG.load(Ordering::SeqCst) {
        let _ = app.emit(
            "alpha-progress",
            ProgressEvent {
                current: total,
                total,
                filename: String::new(),
                status: "done".to_string(),
                message: format!(
                    "完成: 转换 {}, 跳过 {}, 失败 {}, 共 {}",
                    success_count, skipped, fail_count, total
                ),
                ..Default::default()
            },
        );
    }

    Ok(ProcessResult {
        success_count,
        fail_count,
        total,
        errors,
    })
}

fn process_alpha(
    file_path: &Path,
    input_root: &Path,
    output_dir: &Path,
    options: &AlphaConvertOptions,
    bg_color: &[u8; 3],
) -> Result<bool, String> {
    let (img, source) = load_image(file_path)?;

    if !has_alpha(&img) {
        let filename = file_path
            .file_name()
            .ok_or("无效的文件名")?
            .to_string_lossy();
        let dest = output_path_for_input(
            input_root,
            file_path,
            output_dir,
            filename.as_ref(),
            options.recursive,
        )?;
        crate::commands::copy_file_safe(file_path, &dest)?;
        return Ok(false);
    }

    let (width, height) = img.dimensions();
    let rgba = img.to_rgba8();
    let mut rgb = image::RgbImage::new(width, height);

    for (x, y, pixel) in rgba.enumerate_pixels() {
        let alpha = pixel[3] as f32 / 255.0;
        let r = (pixel[0] as f32 * alpha + bg_color[0] as f32 * (1.0 - alpha)) as u8;
        let g = (pixel[1] as f32 * alpha + bg_color[1] as f32 * (1.0 - alpha)) as u8;
        let b = (pixel[2] as f32 * alpha + bg_color[2] as f32 * (1.0 - alpha)) as u8;
        rgb.put_pixel(x, y, image::Rgb([r, g, b]));
    }

    let file_name = file_path
        .file_name()
        .ok_or("无效的文件名")?
        .to_string_lossy();
    let output_path = output_path_for_input(
        input_root,
        file_path,
        output_dir,
        file_name.as_ref(),
        options.recursive,
    )?;

    save_like_source(DynamicImage::ImageRgb8(rgb), &output_path, &source)?;

    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_source_name_and_format() {
        let root = std::env::temp_dir().join(format!("purinbox_alpha_keep_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let (input, out) = (root.join("in"), root.join("out"));
        std::fs::create_dir_all(&input).unwrap();
        std::fs::create_dir_all(&out).unwrap();
        let src = input.join("w.webp");
        image::RgbaImage::from_pixel(16, 16, image::Rgba([0, 0, 0, 0]))
            .save(&src)
            .unwrap();
        let options: AlphaConvertOptions = serde_json::from_value(serde_json::json!({
            "input_path": input.to_string_lossy(),
            "output_path": out.to_string_lossy(),
            "background": "white",
        }))
        .unwrap();

        assert!(process_alpha(&src, &input, &out, &options, &[255, 255, 255]).unwrap());
        let bytes = std::fs::read(out.join("w.webp")).unwrap();
        assert_eq!(image::guess_format(&bytes).unwrap(), image::ImageFormat::WebP);
        let img = image::load_from_memory(&bytes).unwrap();
        assert!(!img.color().has_alpha());
        assert_eq!(img.to_rgb8().get_pixel(8, 8).0, [255, 255, 255]);
        assert!(!out.join("w.png").exists());
        let _ = std::fs::remove_dir_all(&root);
    }
}
