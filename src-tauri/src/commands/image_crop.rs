use image::imageops::FilterType;
use serde::{Deserialize, Serialize};
use std::path::Path;

use super::image_io::{load_image, read_dimensions, save_like_source};
use super::{
    collect_image_files_with_recursive_excluding, file_name_lossy, same_name_output, ProcessResult,
};

use super::batch::{BatchJob, FileBatch, FileOutcome};

static JOB: BatchJob = BatchJob::new("裁剪");

/// 裁切方式，JSON 取值为小写名称
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CropMode {
    /// 中心裁切到目标尺寸
    Center,
    /// 等比缩放填满目标尺寸后裁掉多余部分
    Cover,
    /// 按宽高比居中裁切
    Aspect,
    /// 按上下左右边距裁切
    Edges,
}

/// 填满裁切保留的方向
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CropAnchor {
    #[default]
    Center,
    Top,
    Bottom,
    Left,
    Right,
}

impl CropAnchor {
    fn label(self) -> &'static str {
        match self {
            CropAnchor::Center => "居中",
            CropAnchor::Top => "上方",
            CropAnchor::Bottom => "下方",
            CropAnchor::Left => "左侧",
            CropAnchor::Right => "右侧",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CropOptions {
    pub input_path: String,
    pub output_path: String,
    pub mode: CropMode,
    #[serde(default)]
    pub crop_anchor: CropAnchor,
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

/// 一条边上的裁切起点：`anchor` 是这条轴的起始方向时贴起点，是结束方向时贴终点，否则居中
fn anchored_offset(
    outer: u32,
    inner: u32,
    anchor: CropAnchor,
    start: CropAnchor,
    end: CropAnchor,
) -> u32 {
    if outer <= inner {
        return 0;
    }

    if anchor == start {
        0
    } else if anchor == end {
        outer - inner
    } else {
        (outer - inner) / 2
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
    match options.mode {
        CropMode::Center | CropMode::Cover
            if options.target_width == 0 || options.target_height == 0 =>
        {
            return Err("目标尺寸必须大于 0".to_string())
        }
        CropMode::Aspect if !options.aspect_ratio.is_finite() || options.aspect_ratio <= 0.0 => {
            return Err("无效的宽高比".to_string())
        }
        _ => {}
    }
    let input = Path::new(&options.input_path);
    let output_dir = Path::new(&options.output_path);
    std::fs::create_dir_all(output_dir).map_err(|e| format!("无法创建输出目录: {}", e))?;
    let files =
        collect_image_files_with_recursive_excluding(input, options.recursive, Some(output_dir))?;
    Ok(FileBatch::new(app, "crop-progress", JOB.cancel_flag())
        .archive_failures(input, output_dir, options.recursive)
        .run(
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

    match options.mode {
        CropMode::Center => {
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
        CropMode::Cover => {
            let tw = options.target_width;
            let th = options.target_height;
            let scale = (tw as f64 / orig_w as f64).max(th as f64 / orig_h as f64);
            let scaled_w = ((orig_w as f64 * scale).ceil() as u32).max(tw);
            let scaled_h = ((orig_h as f64 * scale).ceil() as u32).max(th);
            if (scaled_w, scaled_h) == (orig_w, orig_h) && (tw, th) == (orig_w, orig_h) {
                crate::commands::copy_file_safe(file_path, &output_path)?;
                return Ok(format!(
                    "[跳过] {} ({}x{}, 无需裁切)",
                    filename, orig_w, orig_h
                ));
            }
            let (img, source) = load_image(file_path)?;
            let resized = img.resize_exact(scaled_w, scaled_h, FilterType::Lanczos3);
            let anchor = options.crop_anchor;
            let x = anchored_offset(scaled_w, tw, anchor, CropAnchor::Left, CropAnchor::Right);
            let y = anchored_offset(scaled_h, th, anchor, CropAnchor::Top, CropAnchor::Bottom);
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
                anchor.label()
            ))
        }
        CropMode::Aspect => {
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
        CropMode::Edges => {
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
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::test_support::TempDir;
    use image::GenericImageView;
    use serde_json::json;

    fn crop_options(input: &Path, output: &Path, mode: &str, w: u32, h: u32) -> CropOptions {
        serde_json::from_value(json!({
            "input_path": input.to_string_lossy(),
            "output_path": output.to_string_lossy(),
            "mode": mode,
            "target_width": w,
            "target_height": h,
            "aspect_ratio": 1.0,
            "crop_top": 0, "crop_bottom": 0, "crop_left": 0, "crop_right": 0,
        }))
        .unwrap()
    }

    #[test]
    fn skip_uses_dimensions_without_decoding_pixels() {
        let root = TempDir::new("crop_header");
        let path = root.join("broken.png");
        super::super::image_io::write_broken_pixels(&path);
        let output = root.join("out");
        std::fs::create_dir_all(&output).unwrap();
        // cover 的目标尺寸等于原图时缩放与裁切都不改变像素
        for mode in ["center", "aspect", "edges", "cover"] {
            let options = crop_options(&root, &output, mode, 32, 32);
            assert!(process_crop(&path, &root, &output, &options)
                .unwrap()
                .starts_with("[跳过]"));
            assert_eq!(
                std::fs::read(&path).unwrap(),
                std::fs::read(output.join("broken.png")).unwrap()
            );
        }
    }

    /// 模式、方向的 JSON 取值与前端一致，未知取值在反序列化时就被拒绝
    #[test]
    fn options_use_the_frontend_string_values() {
        let root = TempDir::new("crop_values");
        for mode in ["center", "cover", "aspect", "edges"] {
            let options = crop_options(&root, &root, mode, 8, 8);
            assert_eq!(serde_json::to_value(options.mode).unwrap(), mode);
            assert_eq!(options.crop_anchor, CropAnchor::Center);
        }
        let mut value = serde_json::to_value(crop_options(&root, &root, "cover", 8, 8)).unwrap();
        for anchor in ["center", "top", "bottom", "left", "right"] {
            value["crop_anchor"] = json!(anchor);
            let options: CropOptions = serde_json::from_value(value.clone()).unwrap();
            assert_eq!(serde_json::to_value(options.crop_anchor).unwrap(), anchor);
        }
        value["mode"] = json!("invalid");
        assert!(serde_json::from_value::<CropOptions>(value).is_err());
    }

    fn run(input: &Path, output: &Path) -> Vec<Result<String, String>> {
        let options = crop_options(input, output, "center", 32, 32);
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
        let root = TempDir::new("crop_keep_format");
        let (input, out) = (root.join("in"), root.join("out"));
        std::fs::create_dir_all(&input).unwrap();
        std::fs::create_dir_all(&out).unwrap();
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
    }

    #[test]
    fn uncropped_files_are_copied_unchanged() {
        let root = TempDir::new("crop_keep_copy");
        let (input, out) = (root.join("in"), root.join("out"));
        std::fs::create_dir_all(&input).unwrap();
        std::fs::create_dir_all(&out).unwrap();
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
    }

    /// cover：目标比原图小时照常缩放裁切，读不出的文件复制进 Fail/
    #[test]
    fn cover_crops_and_archives_failures() {
        let root = TempDir::new("crop_cover");
        let (input, out) = (root.join("in"), root.join("out"));
        std::fs::create_dir_all(&input).unwrap();
        rgb(64, 32, [200, 10, 10])
            .save(input.join("wide.png"))
            .unwrap();
        std::fs::write(input.join("bad.png"), b"not an image").unwrap();
        let options = crop_options(&input, &out, "cover", 16, 16);
        let result = crop_images_sync(tauri::test::mock_app().handle(), &options).unwrap();
        assert_eq!((result.success_count, result.fail_count), (1, 1));
        assert_eq!(
            image::open(out.join("wide.png")).unwrap().dimensions(),
            (16, 16)
        );
        assert_eq!(
            std::fs::read(out.join("Fail/bad.png")).unwrap(),
            b"not an image"
        );
    }
}
