use image::imageops::FilterType;
use serde::{Deserialize, Serialize};
use std::path::Path;

use super::image_io::{load_image, read_dimensions, save_like_source};
use super::{
    collect_image_files_with_recursive_excluding, file_name_lossy, same_name_output, ProcessResult,
};

use super::batch::{BatchJob, FileBatch, FileOutcome};

static JOB: BatchJob = BatchJob::new("裁剪");

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CropOptions {
    pub input_path: String,
    pub output_path: String,
    /// "center" | "cover" | "aspect" | "edges"
    pub mode: String,
    /// 填满裁切保留方向: "center" | "top" | "bottom" | "left" | "right"
    #[serde(default = "default_crop_anchor")]
    pub crop_anchor: String,
    /// 中心裁切/填满裁切: 目标宽度
    pub target_width: u32,
    /// 中心裁切/填满裁切: 目标高度
    pub target_height: u32,
    /// 宽高比裁切: 宽高比（如 1.0 = 1:1, 0.75 = 3:4）
    pub aspect_ratio: f64,
    /// 边缘裁切: 上下左右像素
    pub crop_top: u32,
    pub crop_bottom: u32,
    pub crop_left: u32,
    pub crop_right: u32,
    #[serde(default)]
    pub recursive: bool,
}

fn default_crop_anchor() -> String {
    "center".to_string()
}

fn anchored_offset(
    outer: u32,
    inner: u32,
    anchor: &str,
    start_anchor: &str,
    end_anchor: &str,
) -> u32 {
    if outer <= inner {
        return 0;
    }

    if anchor == start_anchor {
        0
    } else if anchor == end_anchor {
        outer - inner
    } else {
        (outer - inner) / 2
    }
}

fn crop_anchor_label(anchor: &str) -> &'static str {
    match anchor {
        "top" => "上方",
        "bottom" => "下方",
        "left" => "左侧",
        "right" => "右侧",
        _ => "居中",
    }
}

#[tauri::command]
pub async fn crop_images<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    options: CropOptions,
) -> Result<ProcessResult, String> {
    JOB.run(move || crop_images_sync(&app, &options)).await
}

#[tauri::command]
pub fn cancel_crop() {
    JOB.cancel();
}

fn crop_images_sync<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    options: &CropOptions,
) -> Result<ProcessResult, String> {
    match options.mode.as_str() {
        "center" | "cover" if options.target_width == 0 || options.target_height == 0 => {
            return Err("目标尺寸必须大于 0".to_string())
        }
        "aspect" if !options.aspect_ratio.is_finite() || options.aspect_ratio <= 0.0 => {
            return Err("无效的宽高比".to_string())
        }
        "center" | "cover" | "aspect" | "edges" => {}
        _ => return Err("无效的裁切模式".to_string()),
    }
    let input = Path::new(&options.input_path);
    let output_dir = Path::new(&options.output_path);
    std::fs::create_dir_all(output_dir).map_err(|e| format!("无法创建输出目录: {}", e))?;
    let files =
        collect_image_files_with_recursive_excluding(input, options.recursive, Some(output_dir))?;
    Ok(FileBatch::new(app, "crop-progress", JOB.cancel_flag()).run(
        &files,
        |item| process_crop(item.path, input, output_dir, options).map(FileOutcome::done),
        |c| c.summary("处理完成"),
    ))
}

fn process_crop(
    file_path: &Path,
    input_root: &Path,
    output_dir: &Path,
    options: &CropOptions,
) -> Result<String, String> {
    let (orig_w, orig_h) =
        read_dimensions(file_path).map_err(|e| format!("无法读取图片尺寸: {}", e))?;
    let filename = file_name_lossy(file_path);
    let output_path = same_name_output(input_root, file_path, output_dir, options.recursive)?;

    match options.mode.as_str() {
        "center" => {
            let tw = options.target_width.min(orig_w);
            let th = options.target_height.min(orig_h);
            if tw == orig_w && th == orig_h {
                crate::commands::copy_file_safe(file_path, &output_path)?;
                return Ok(format!(
                    "[跳过] {} ({}x{}, 无需裁切)",
                    filename, orig_w, orig_h
                ));
            }
            let x = (orig_w - tw) / 2;
            let y = (orig_h - th) / 2;
            let (img, source) = load_image(file_path)?;
            let cropped = img.crop_imm(x, y, tw, th);
            save_like_source(cropped, &output_path, &source)?;
            Ok(format!(
                "[中心裁切] {} ({}x{} → {}x{})",
                filename, orig_w, orig_h, tw, th
            ))
        }
        "cover" => {
            let tw = options.target_width;
            let th = options.target_height;
            let scale = (tw as f64 / orig_w as f64).max(th as f64 / orig_h as f64);
            let scaled_w = ((orig_w as f64 * scale).ceil() as u32).max(tw);
            let scaled_h = ((orig_h as f64 * scale).ceil() as u32).max(th);
            let (img, source) = load_image(file_path)?;
            let resized = img.resize_exact(scaled_w, scaled_h, FilterType::Lanczos3);
            let anchor = options.crop_anchor.as_str();
            let x = anchored_offset(scaled_w, tw, anchor, "left", "right");
            let y = anchored_offset(scaled_h, th, anchor, "top", "bottom");
            let cropped = resized.crop_imm(x, y, tw, th);
            save_like_source(cropped, &output_path, &source)?;
            Ok(format!(
                "[填满裁切] {} ({}x{} → 缩放 {}x{} → 裁切 {}x{}, 保留 {})",
                filename,
                orig_w,
                orig_h,
                scaled_w,
                scaled_h,
                tw,
                th,
                crop_anchor_label(anchor)
            ))
        }
        "aspect" => {
            let target_ratio = options.aspect_ratio;
            let current_ratio = orig_w as f64 / orig_h as f64;

            if (current_ratio - target_ratio).abs() < 0.01 {
                crate::commands::copy_file_safe(file_path, &output_path)?;
                return Ok(format!(
                    "[跳过] {} ({}x{}, 比例已匹配)",
                    filename, orig_w, orig_h
                ));
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
            let (img, source) = load_image(file_path)?;
            let cropped = img.crop_imm(x, y, tw, th);
            save_like_source(cropped, &output_path, &source)?;
            Ok(format!(
                "[比例裁切] {} ({}x{} → {}x{}, 比例 {:.2})",
                filename, orig_w, orig_h, tw, th, target_ratio
            ))
        }
        "edges" => {
            let ct = options.crop_top;
            let cb = options.crop_bottom;
            let cl = options.crop_left;
            let cr = options.crop_right;

            if ct.saturating_add(cb) >= orig_h || cl.saturating_add(cr) >= orig_w {
                return Err(format!("裁切边距超过图片尺寸 ({}x{})", orig_w, orig_h));
            }

            if ct == 0 && cb == 0 && cl == 0 && cr == 0 {
                crate::commands::copy_file_safe(file_path, &output_path)?;
                return Ok(format!(
                    "[跳过] {} ({}x{}, 无需裁切)",
                    filename, orig_w, orig_h
                ));
            }

            let tw = orig_w - cl - cr;
            let th = orig_h - ct - cb;
            let (img, source) = load_image(file_path)?;
            let cropped = img.crop_imm(cl, ct, tw, th);
            save_like_source(cropped, &output_path, &source)?;
            Ok(format!(
                "[边缘裁切] {} ({}x{} → {}x{}, 上{}下{}左{}右{})",
                filename, orig_w, orig_h, tw, th, ct, cb, cl, cr
            ))
        }
        _ => Err("无效的裁切模式".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::GenericImageView;
    use std::path::PathBuf;

    #[test]
    fn skip_uses_dimensions_without_decoding_pixels() {
        let root = super::super::image_io::test_dir("crop_header");
        let path = root.join("broken.png");
        super::super::image_io::write_broken_pixels(&path);
        let output = root.join("out");
        std::fs::create_dir_all(&output).unwrap();
        let mut options: CropOptions = serde_json::from_value(serde_json::json!({
            "input_path": root.to_string_lossy(), "output_path": output.to_string_lossy(), "mode": "center",
            "target_width": 32, "target_height": 32, "aspect_ratio": 1.0,
            "crop_top": 0, "crop_bottom": 0, "crop_left": 0, "crop_right": 0
        })).unwrap();
        for mode in ["center", "aspect", "edges"] {
            options.mode = mode.into();
            assert!(process_crop(&path, &root, &output, &options)
                .unwrap()
                .starts_with("[跳过]"));
            assert_eq!(
                std::fs::read(&path).unwrap(),
                std::fs::read(output.join("broken.png")).unwrap()
            );
        }
        options.mode = "invalid".into();
        assert_eq!(
            crop_images_sync(tauri::test::mock_app().handle(), &options).unwrap_err(),
            "无效的裁切模式"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    fn fixture(tag: &str) -> (PathBuf, PathBuf, PathBuf) {
        let root =
            std::env::temp_dir().join(format!("purinbox_crop_keep_{}_{}", tag, std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let (input, output) = (root.join("in"), root.join("out"));
        std::fs::create_dir_all(&input).unwrap();
        std::fs::create_dir_all(&output).unwrap();
        (root, input, output)
    }

    fn run(input: &Path, output: &Path) -> Vec<Result<String, String>> {
        let options: CropOptions = serde_json::from_value(serde_json::json!({
            "input_path": input.to_string_lossy(),
            "output_path": output.to_string_lossy(),
            "mode": "center",
            "target_width": 32,
            "target_height": 32,
            "aspect_ratio": 1.0,
            "crop_top": 0, "crop_bottom": 0, "crop_left": 0, "crop_right": 0,
        }))
        .unwrap();
        collect_image_files_with_recursive_excluding(input, false, Some(output))
            .unwrap()
            .iter()
            .map(|f| process_crop(f, input, output, &options))
            .collect()
    }

    fn real_format(p: &Path) -> image::ImageFormat {
        image::guess_format(&std::fs::read(p).unwrap()).unwrap()
    }

    fn rgb(w: u32, h: u32, c: [u8; 3]) -> image::DynamicImage {
        image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(w, h, image::Rgb(c)))
    }

    #[test]
    fn output_keeps_actual_format_and_name() {
        let (root, input, out) = fixture("format");
        rgb(64, 64, [10, 20, 30])
            .save_with_format(input.join("png_inside.jpg"), image::ImageFormat::Png)
            .unwrap();
        rgb(64, 64, [30, 20, 10])
            .save_with_format(input.join("jpeg_inside.png"), image::ImageFormat::Jpeg)
            .unwrap();
        rgb(64, 64, [60, 90, 120])
            .save(input.join("c.webp"))
            .unwrap();

        let results = run(&input, &out);
        assert!(results.iter().all(|r| r.is_ok()), "{:?}", results);
        assert_eq!(
            real_format(&out.join("png_inside.jpg")),
            image::ImageFormat::Png
        );
        assert_eq!(
            real_format(&out.join("jpeg_inside.png")),
            image::ImageFormat::Jpeg
        );
        assert_eq!(real_format(&out.join("c.webp")), image::ImageFormat::WebP);
        let cropped =
            image::load_from_memory(&std::fs::read(out.join("png_inside.jpg")).unwrap()).unwrap();
        assert_eq!(cropped.dimensions(), (32, 32));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn uncropped_files_are_copied_unchanged() {
        let (root, input, out) = fixture("copy");
        // 小于目标尺寸，走"无需裁切"分支
        rgb(16, 16, [0, 120, 0])
            .save(input.join("small.jpg"))
            .unwrap();

        let results = run(&input, &out);
        assert!(results.iter().all(|r| r.is_ok()), "{:?}", results);
        assert_eq!(
            std::fs::read(out.join("small.jpg")).unwrap(),
            std::fs::read(input.join("small.jpg")).unwrap()
        );
        let _ = std::fs::remove_dir_all(&root);
    }
}
