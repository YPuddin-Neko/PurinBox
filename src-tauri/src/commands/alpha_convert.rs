use image::DynamicImage;
use serde::{Deserialize, Serialize};
use std::path::Path;

use super::image_io::{
    flatten_preserving_depth, load_image, probe_has_alpha_channel, save_like_source,
};
use super::{collect_image_files_with_recursive_excluding, same_name_output, ProcessResult};

use super::batch::{BatchJob, FileBatch, FileOutcome};

static JOB: BatchJob = BatchJob::new("透明通道转换");

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
    JOB.run(move || convert_alpha_sync(&app, &options)).await
}

#[tauri::command]
pub fn cancel_alpha() {
    JOB.cancel();
}

fn convert_alpha_sync<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    options: &AlphaConvertOptions,
) -> Result<ProcessResult, String> {
    let input = Path::new(&options.input_path);
    let output_dir = Path::new(&options.output_path);
    std::fs::create_dir_all(output_dir).map_err(|e| format!("无法创建输出目录: {}", e))?;
    let files =
        collect_image_files_with_recursive_excluding(input, options.recursive, Some(output_dir))?;
    let bg_color: [u8; 3] = match options.background.as_str() {
        "black" => [0, 0, 0],
        _ => [255, 255, 255],
    };
    Ok(FileBatch::new(app, "alpha-progress", JOB.cancel_flag())
        .error_prefix("[错误] ")
        .archive_failures(input, output_dir, options.recursive)
        .run(
            &files,
            |item| {
                Ok(
                    if process_alpha(item.path, input, output_dir, options, &bg_color)? {
                        FileOutcome::done(format!("[转换] {} (检测到透明通道, 已转换)", item.name))
                    } else {
                        FileOutcome::unchanged(format!("[跳过] {} (无透明通道)", item.name))
                    },
                )
            },
            |c| {
                format!(
                    "完成: 转换 {}, 跳过 {}, 失败 {}, 共 {}",
                    c.success - c.unchanged,
                    c.unchanged,
                    c.failed,
                    c.total
                )
            },
        ))
}

fn process_alpha(
    file_path: &Path,
    input_root: &Path,
    output_dir: &Path,
    options: &AlphaConvertOptions,
    bg_color: &[u8; 3],
) -> Result<bool, String> {
    let output_path = same_name_output(input_root, file_path, output_dir, options.recursive)?;
    if !probe_has_alpha_channel(file_path)? {
        crate::commands::copy_file_safe(file_path, &output_path)?;
        return Ok(false);
    }
    let (img, source) = load_image(file_path)?;

    if !has_alpha(&img) {
        crate::commands::copy_file_safe(file_path, &output_path)?;
        return Ok(false);
    }
    save_like_source(
        flatten_preserving_depth(img, *bg_color),
        &output_path,
        &source,
    )?;

    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::test_support::TempDir;

    #[test]
    fn no_alpha_channel_copies_without_pixel_decode() {
        let root = TempDir::new("alpha_header");
        let path = root.join("broken.png");
        super::super::image_io::write_broken_pixels(&path);
        let output = root.join("out");
        std::fs::create_dir_all(&output).unwrap();
        let options = AlphaConvertOptions {
            input_path: root.to_string_lossy().into_owned(),
            output_path: output.to_string_lossy().into_owned(),
            background: "white".into(),
            recursive: false,
        };
        assert!(!process_alpha(&path, &root, &output, &options, &[255, 255, 255]).unwrap());
        assert_eq!(
            std::fs::read(&path).unwrap(),
            std::fs::read(output.join("broken.png")).unwrap()
        );
    }

    #[test]
    fn flatten_preserves_16bit_values_and_counts_unchanged() {
        let root = TempDir::new("alpha16");
        let input = root.join("in");
        let output = root.join("out");
        std::fs::create_dir_all(&input).unwrap();
        let pixel = [12345u16, 23456, 34567, 32768];
        for ext in ["png", "tiff"] {
            image::ImageBuffer::from_pixel(3, 2, image::Rgba(pixel))
                .save(input.join(format!("a.{ext}")))
                .unwrap();
        }
        image::RgbImage::new(3, 2)
            .save(input.join("opaque.png"))
            .unwrap();
        let options = AlphaConvertOptions {
            input_path: input.to_string_lossy().into_owned(),
            output_path: output.to_string_lossy().into_owned(),
            background: "white".into(),
            recursive: false,
        };
        let app = tauri::test::mock_app();
        let result = convert_alpha_sync(app.handle(), &options).unwrap();
        assert_eq!(
            (result.success_count, result.fail_count, result.total),
            (3, 0, 3)
        );
        let expected: [u16; 3] =
            std::array::from_fn(|c| ((u64::from(pixel[c]) * 32768 + 65535 * 32767) / 65535) as u16);
        for ext in ["png", "tiff"] {
            let image = image::open(output.join(format!("a.{ext}"))).unwrap();
            assert_eq!(image.color(), image::ColorType::Rgb16);
            assert_eq!(image.to_rgb16().get_pixel(0, 0).0, expected);
        }
        assert_eq!(
            std::fs::read(input.join("opaque.png")).unwrap(),
            std::fs::read(output.join("opaque.png")).unwrap()
        );
    }

    #[test]
    fn keeps_source_name_and_format() {
        let root = TempDir::new("alpha_keep");
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
        assert_eq!(
            image::guess_format(&bytes).unwrap(),
            image::ImageFormat::WebP
        );
        let img = image::load_from_memory(&bytes).unwrap();
        assert!(!img.color().has_alpha());
        assert_eq!(img.to_rgb8().get_pixel(8, 8).0, [255, 255, 255]);
        assert!(!out.join("w.png").exists());
    }

    /// 读不出的文件复制进输出目录的 Fail/，递归时保留子目录
    #[test]
    fn failed_files_are_archived_into_output_fail() {
        let root = TempDir::new("alpha_fail");
        let (input, output) = (root.join("in"), root.join("out"));
        std::fs::create_dir_all(input.join("sub")).unwrap();
        image::RgbaImage::from_pixel(4, 4, image::Rgba([1, 2, 3, 4]))
            .save(input.join("ok.png"))
            .unwrap();
        std::fs::write(input.join("sub/bad.png"), b"not an image").unwrap();
        let options = AlphaConvertOptions {
            input_path: input.to_string_lossy().into_owned(),
            output_path: output.to_string_lossy().into_owned(),
            background: "black".into(),
            recursive: true,
        };
        let result = convert_alpha_sync(tauri::test::mock_app().handle(), &options).unwrap();
        assert_eq!((result.success_count, result.fail_count), (1, 1));
        assert_eq!(
            std::fs::read(output.join("Fail/sub/bad.png")).unwrap(),
            b"not an image"
        );
        assert!(!output.join("Fail/ok.png").exists());
    }
}
