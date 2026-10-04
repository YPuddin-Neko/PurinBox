use super::image_io::{decode_image, sniff_format};
use super::{
    collect_files_matching, copy_file_safe, has_extension, is_supported_image_file,
    output_path_for_input, path_key_ci, ProcessResult,
};
use image::{DynamicImage, ImageFormat, RgbaImage};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::Path;

use super::batch::{BatchJob, FileBatch, FileOutcome};

static JOB: BatchJob = BatchJob::new("格式转换");

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FormatConvertOptions {
    pub input_path: String,
    pub output_path: String,
    /// 目标格式: "png" | "jpg" | "jpeg" | "bmp" | "webp"
    pub target_format: String,
    #[serde(default)]
    pub recursive: bool,
}

fn open_image(file_path: &Path) -> Result<DynamicImage, String> {
    if has_extension(file_path, &["psd"]) {
        let bytes = std::fs::read(file_path).map_err(|e| format!("无法读取 PSD 文件: {}", e))?;
        let psd_file =
            psd::Psd::from_bytes(&bytes).map_err(|e| format!("无法解析 PSD 文件: {:?}", e))?;

        let width = psd_file.width();
        let height = psd_file.height();
        let rgba_data = psd_file.rgba();

        let img_buf = RgbaImage::from_raw(width, height, rgba_data).ok_or("无法创建图片缓冲区")?;

        Ok(DynamicImage::ImageRgba8(img_buf))
    } else {
        decode_image(file_path)
    }
}

#[tauri::command]
pub async fn convert_format<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    options: FormatConvertOptions,
) -> Result<ProcessResult, String> {
    JOB.run(move || convert_format_sync(&app, &options)).await
}

#[tauri::command]
pub fn cancel_convert() {
    JOB.cancel();
}

/// 一批转换共用的输出冲突检查
struct OutputGuard {
    /// 本批全部源文件（`path_key_ci`），原地模式下输出不能盖掉它们
    inputs: HashSet<String>,
    /// 本批已经占用的输出路径
    used: HashSet<String>,
}

impl OutputGuard {
    /// 占用 `output`；会覆盖源文件或与本批其他输出同名时报错
    fn claim(&mut self, source: &Path, output: &Path) -> Result<(), String> {
        let name = super::file_name_lossy(output);
        let key = path_key_ci(output);
        if key == path_key_ci(source) {
            return Err(format!(
                "输出 {} 会覆盖源文件，已跳过（请更换输出目录）",
                name
            ));
        }
        if self.inputs.contains(&key) {
            return Err(format!(
                "输出 {} 会覆盖另一张源图，已跳过（请更换输出目录）",
                name
            ));
        }
        // 同 stem 不同扩展的输入映射到同一输出名：后到者报错而不是静默覆盖
        if !self.used.insert(key) {
            return Err(format!("输出 {} 与本批其他文件同名冲突，已跳过", name));
        }
        Ok(())
    }
}

fn convert_format_sync<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    options: &FormatConvertOptions,
) -> Result<ProcessResult, String> {
    let input = Path::new(&options.input_path);
    let output_dir = Path::new(&options.output_path);
    std::fs::create_dir_all(output_dir).map_err(|e| format!("无法创建输出目录: {}", e))?;
    let files = collect_files_matching(input, options.recursive, Some(output_dir), |p| {
        is_supported_image_file(p) || has_extension(p, &["psd"])
    })?;
    let target_ext = options.target_format.to_lowercase();
    let target_format = ImageFormat::from_extension(&target_ext);
    let mut guard = OutputGuard {
        inputs: files.iter().map(|p| path_key_ci(p)).collect(),
        used: HashSet::new(),
    };
    Ok(FileBatch::new(app, "convert-progress", JOB.cancel_flag())
        .error_prefix("[错误] ")
        .archive_failures(input, output_dir, options.recursive)
        .run(
            &files,
            |item| {
                let src_ext = item
                    .path
                    .extension()
                    .map(|e| e.to_string_lossy().to_lowercase())
                    .unwrap_or_default();
                // 按文件内容判断是否已是目标格式：扩展名可能与内容不符
                if target_format.is_some() && sniff_format(item.path) == target_format {
                    // 扩展名也对得上就保持原名；对不上就只改扩展名，字节原样复制
                    let dst = if ImageFormat::from_extension(&src_ext) == target_format {
                        super::same_name_output(input, item.path, output_dir, options.recursive)?
                    } else {
                        converted_output(
                            item.path,
                            input,
                            output_dir,
                            options.recursive,
                            &target_ext,
                        )?
                    };
                    if path_key_ci(&dst) != path_key_ci(item.path) {
                        guard.claim(item.path, &dst)?;
                    }
                    copy_file_safe(item.path, &dst)?;
                    return Ok(FileOutcome::unchanged(format!(
                        "[跳过转换] {} (已是 .{} 格式，直接复制)",
                        item.name, target_ext
                    )));
                }
                let output_path =
                    converted_output(item.path, input, output_dir, options.recursive, &target_ext)?;
                guard.claim(item.path, &output_path)?;
                convert_to(item.path, &output_path, &target_ext)?;
                Ok(FileOutcome::done(format!(
                    "[转换] {} (.{} → .{})",
                    item.name, src_ext, target_ext
                )))
            },
            |c| c.summary("转换完成"),
        ))
}

/// 转换产物的路径：与源文件同名、扩展名换成目标格式，递归时保留子目录
fn converted_output(
    file_path: &Path,
    input_root: &Path,
    output_dir: &Path,
    recursive: bool,
    target_ext: &str,
) -> Result<std::path::PathBuf, String> {
    let stem = file_path
        .file_stem()
        .ok_or("无效的文件名")?
        .to_string_lossy();
    let new_name = format!("{}.{}", stem, target_ext);
    output_path_for_input(input_root, file_path, output_dir, &new_name, recursive)
}

fn convert_to(file_path: &Path, output_path: &Path, target_ext: &str) -> Result<(), String> {
    let img = open_image(file_path)?;

    let img = match target_ext {
        "jpg" | "jpeg" | "bmp" => DynamicImage::ImageRgb8(img.to_rgb8()),
        // image 0.25 的 WebP 编码器只接受 RGB8/RGBA8，灰度/16 位图需先归一化
        "webp" => {
            if img.color().has_alpha() {
                DynamicImage::ImageRgba8(img.to_rgba8())
            } else {
                DynamicImage::ImageRgb8(img.to_rgb8())
            }
        }
        _ => img,
    };

    img.save(output_path)
        .map_err(|e| format!("无法保存图片: {}", e))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::test_support::TempDir;

    fn convert(input: &Path, output: &Path, target: &str) -> ProcessResult {
        let options = FormatConvertOptions {
            input_path: input.to_string_lossy().into_owned(),
            output_path: output.to_string_lossy().into_owned(),
            target_format: target.into(),
            recursive: false,
        };
        convert_format_sync(tauri::test::mock_app().handle(), &options).unwrap()
    }

    fn rgb() -> DynamicImage {
        DynamicImage::ImageRgb8(image::RgbImage::from_fn(8, 8, |x, y| {
            image::Rgb([(x * 30) as u8, (y * 30) as u8, 90])
        }))
    }

    fn real_format(path: &Path) -> ImageFormat {
        image::guess_format(&std::fs::read(path).unwrap()).unwrap()
    }

    /// 是否已是目标格式看内容不看扩展名：内容已是目标格式的原样复制（扩展名不对就只改名），
    /// 扩展名像目标格式、内容却不是的照常转换
    #[test]
    fn target_format_is_detected_by_content() {
        let root = TempDir::new("convert_content");
        let (input, output) = (root.join("in"), root.join("out"));
        std::fs::create_dir_all(&input).unwrap();
        rgb()
            .save_with_format(input.join("real.png"), ImageFormat::Png)
            .unwrap();
        rgb()
            .save_with_format(input.join("png_named.jpg"), ImageFormat::Png)
            .unwrap();
        rgb()
            .save_with_format(input.join("jpeg_named.png"), ImageFormat::Jpeg)
            .unwrap();

        let result = convert(&input, &output, "png");
        assert_eq!((result.success_count, result.fail_count), (3, 0));
        for (source, output_name) in [("real.png", "real.png"), ("png_named.jpg", "png_named.png")]
        {
            assert_eq!(
                std::fs::read(input.join(source)).unwrap(),
                std::fs::read(output.join(output_name)).unwrap(),
                "{}",
                source
            );
        }
        assert!(!output.join("png_named.jpg").exists());
        assert_eq!(
            real_format(&output.join("jpeg_named.png")),
            ImageFormat::Png
        );
    }

    /// 原地转换不能写回源文件本身，也不能盖掉另一张源图；读不出的文件进 Fail/
    #[test]
    fn in_place_conversion_never_overwrites_sources() {
        let root = TempDir::new("convert_in_place");
        rgb()
            .save_with_format(root.join("a.png"), ImageFormat::Jpeg)
            .unwrap();
        rgb()
            .save_with_format(root.join("b.jpg"), ImageFormat::Jpeg)
            .unwrap();
        rgb()
            .save_with_format(root.join("b.png"), ImageFormat::Png)
            .unwrap();
        std::fs::write(root.join("c.png"), b"not an image").unwrap();
        let sources: Vec<_> = ["a.png", "b.jpg", "b.png", "c.png"]
            .iter()
            .map(|name| std::fs::read(root.join(name)).unwrap())
            .collect();

        let result = convert(&root, &root, "png");
        assert_eq!((result.success_count, result.fail_count), (1, 3));
        assert!(result
            .errors
            .iter()
            .any(|e| e == "a.png: 输出 a.png 会覆盖源文件，已跳过（请更换输出目录）"));
        assert!(result
            .errors
            .iter()
            .any(|e| e == "b.jpg: 输出 b.png 会覆盖另一张源图，已跳过（请更换输出目录）"));
        for (name, bytes) in ["a.png", "b.jpg", "b.png", "c.png"].iter().zip(&sources) {
            assert_eq!(&std::fs::read(root.join(name)).unwrap(), bytes, "{}", name);
        }
        assert!(root.join("Fail/c.png").is_file());
    }
}
