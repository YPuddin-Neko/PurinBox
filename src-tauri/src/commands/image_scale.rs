use image::imageops::FilterType;
use image::GenericImageView;
use serde::{Deserialize, Serialize};
use std::path::Path;

use super::image_io::{load_image, read_dimensions, save_like_source};
use super::{
    collect_image_files_with_recursive_excluding, file_name_lossy, same_name_output, ProcessResult,
};

use super::batch::{BatchJob, FileBatch, FileOutcome};

static JOB: BatchJob = BatchJob::new("缩放");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skipped_modes_copy_without_pixel_decode_and_keep_messages() {
        let root = super::super::image_io::test_dir("scale_header");
        let path = root.join("broken.png");
        super::super::image_io::write_broken_pixels(&path);
        let output = root.join("out");
        std::fs::create_dir_all(&output).unwrap();
        let mut options = ScaleOptions {
            input_path: root.to_string_lossy().into_owned(),
            output_path: output.to_string_lossy().into_owned(),
            mode: "both".into(),
            target_width: 32,
            target_height: 32,
            down_target_width: 32,
            down_target_height: 32,
            recursive: false,
        };
        for (mode, message) in [
            ("upscale", "无需上采样"),
            ("downscale", "无需下采样"),
            ("both", "已在目标范围内"),
        ] {
            options.mode = mode.into();
            assert_eq!(
                process_scale(&path, &root, &output, &options).unwrap(),
                format!("[跳过] broken.png (32x32, {})", message)
            );
            assert_eq!(
                std::fs::read(&path).unwrap(),
                std::fs::read(output.join("broken.png")).unwrap()
            );
        }
        options.mode = "invalid".into();
        assert_eq!(
            scale_images_sync(tauri::test::mock_app().handle(), &options).unwrap_err(),
            "无效的缩放模式"
        );
        std::fs::remove_dir_all(root).unwrap();
    }
}

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
    #[serde(default)]
    pub recursive: bool,
}

#[tauri::command]
pub async fn scale_images<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    options: ScaleOptions,
) -> Result<ProcessResult, String> {
    JOB.run(move || scale_images_sync(&app, &options)).await
}

#[tauri::command]
pub fn cancel_scale() {
    JOB.cancel();
}

fn scale_images_sync<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    options: &ScaleOptions,
) -> Result<ProcessResult, String> {
    if !matches!(options.mode.as_str(), "upscale" | "downscale" | "both") {
        return Err("无效的缩放模式".to_string());
    }
    let input = Path::new(&options.input_path);
    let output_dir = Path::new(&options.output_path);
    std::fs::create_dir_all(output_dir).map_err(|e| format!("无法创建输出目录: {}", e))?;
    let files =
        collect_image_files_with_recursive_excluding(input, options.recursive, Some(output_dir))?;
    Ok(
        FileBatch::new(app, "scale-progress", JOB.cancel_flag()).run(
            &files,
            |item| process_scale(item.path, input, output_dir, options).map(FileOutcome::done),
            |c| c.summary("处理完成"),
        ),
    )
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
    input_root: &Path,
    output_dir: &Path,
    options: &ScaleOptions,
) -> Result<String, String> {
    let (orig_w, orig_h) =
        read_dimensions(file_path).map_err(|e| format!("无法读取图片尺寸: {}", e))?;
    let filename = file_name_lossy(file_path);
    let output_path = same_name_output(input_root, file_path, output_dir, options.recursive)?;
    let up = (options.mode != "downscale").then_some((options.target_width, options.target_height));
    let down = match options.mode.as_str() {
        "downscale" => Some((options.target_width, options.target_height)),
        "both" => Some((
            if options.down_target_width > 0 {
                options.down_target_width
            } else {
                options.target_width
            },
            if options.down_target_height > 0 {
                options.down_target_height
            } else {
                options.target_height
            },
        )),
        _ => None,
    };
    let needs_up = up.is_some_and(|(w, h)| orig_w < w || orig_h < h);
    let needs_down = down.is_some_and(|(w, h)| orig_w > w || orig_h > h);
    if !needs_up && !needs_down {
        crate::commands::copy_file_safe(file_path, &output_path)?;
        let reason = match options.mode.as_str() {
            "upscale" => "无需上采样",
            "downscale" => "无需下采样",
            _ => "已在目标范围内",
        };
        return Ok(format!(
            "[跳过] {} ({}x{}, {})",
            filename, orig_w, orig_h, reason
        ));
    }
    let (mut current, source) = load_image(file_path)?;
    let mut steps = Vec::new();
    for (target, upscale, label) in [(up, true, "上采样"), (down, false, "下采样")] {
        let Some((w, h)) = target else { continue };
        let (cw, ch) = current.dimensions();
        if (upscale && (cw < w || ch < h)) || (!upscale && (cw > w || ch > h)) {
            current = area_scale(&current, w, h);
            let (nw, nh) = current.dimensions();
            steps.push(format!("{} {}x{} → {}x{}", label, cw, ch, nw, nh));
        }
    }
    let (final_w, final_h) = current.dimensions();
    save_like_source(current, &output_path, &source)?;
    Ok(if options.mode == "both" {
        format!(
            "[缩放] {} ({}) → {}x{}",
            filename,
            steps.join(" → "),
            final_w,
            final_h
        )
    } else {
        let label = if options.mode == "upscale" {
            "上采样"
        } else {
            "下采样"
        };
        format!(
            "[{}] {} ({}x{} → {}x{})",
            label, filename, orig_w, orig_h, final_w, final_h
        )
    })
}
