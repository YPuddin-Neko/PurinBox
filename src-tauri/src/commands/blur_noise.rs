use serde::{Deserialize, Serialize};
use std::path::Path;

use super::image_io::{load_image, save_like_source};
use super::{collect_image_files_with_recursive_excluding, same_name_output, ProcessResult};

use super::batch::{BatchJob, FileBatch, FileOutcome};

static JOB: BatchJob = BatchJob::new("模糊噪点");

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlurNoiseOptions {
    pub input_path: String,
    pub output_path: String,
    /// 高斯模糊半径 0.0 ~ 10.0 (0 = 不模糊)
    pub blur_radius: f64,
    /// 噪点强度 0 ~ 100 (0 = 不加噪点)
    pub noise_strength: u32,
    #[serde(default)]
    pub recursive: bool,
}

#[tauri::command]
pub async fn blur_noise_images<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    options: BlurNoiseOptions,
) -> Result<ProcessResult, String> {
    JOB.run(move || blur_noise_sync(&app, &options)).await
}

#[tauri::command]
pub fn cancel_blur_noise() {
    JOB.cancel();
}

fn blur_noise_sync<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    options: &BlurNoiseOptions,
) -> Result<ProcessResult, String> {
    let input = Path::new(&options.input_path);
    let output_dir = Path::new(&options.output_path);
    std::fs::create_dir_all(output_dir).map_err(|e| format!("无法创建输出目录: {}", e))?;
    let files =
        collect_image_files_with_recursive_excluding(input, options.recursive, Some(output_dir))?;
    let label = match (options.blur_radius > 0.0, options.noise_strength > 0) {
        (true, true) => "模糊+噪点",
        (true, false) => "高斯模糊",
        (false, true) => "噪点",
        _ => "处理",
    };
    Ok(
        FileBatch::new(app, "blur-noise-progress", JOB.cancel_flag())
            .error_prefix("[失败] ")
            .run(
                &files,
                |item| {
                    process_blur_noise(item.path, input, output_dir, options)?;
                    Ok(FileOutcome::done(format!("[{}] {} ✓", label, item.name)))
                },
                |c| c.summary("处理完成"),
            ),
    )
}

fn process_blur_noise(
    file_path: &Path,
    input_root: &Path,
    output_dir: &Path,
    options: &BlurNoiseOptions,
) -> Result<(), String> {
    let (mut img, source) = load_image(file_path)?;

    // 高斯模糊
    if options.blur_radius > 0.0 {
        let sigma = options.blur_radius.max(0.1) as f32;
        img = img.blur(sigma);
    }

    // 高斯噪点
    if options.noise_strength > 0 {
        use image::DynamicImage;
        let seed = super::perspective::path_seed(file_path);
        img = match img.color() {
            image::ColorType::L16
            | image::ColorType::La16
            | image::ColorType::Rgb16
            | image::ColorType::Rgba16 => DynamicImage::ImageRgba16(add_noise(
                img.to_rgba16(),
                options.noise_strength,
                seed,
                true,
            )),
            image::ColorType::Rgb32F | image::ColorType::Rgba32F => DynamicImage::ImageRgba32F(
                add_noise(img.to_rgba32f(), options.noise_strength, seed, false),
            ),
            _ => DynamicImage::ImageRgba8(add_noise(
                img.to_rgba8(),
                options.noise_strength,
                seed,
                true,
            )),
        };
    }
    let output_path = same_name_output(input_root, file_path, output_dir, options.recursive)?;
    save_like_source(img, &output_path, &source)
}

fn add_noise<T: image::Primitive>(
    mut rgba: image::ImageBuffer<image::Rgba<T>, Vec<T>>,
    strength: u32,
    mut seed: u64,
    quantize: bool,
) -> image::ImageBuffer<image::Rgba<T>, Vec<T>>
where
    f64: From<T>,
    image::Rgba<T>: image::Pixel<Subpixel = T>,
{
    let max: f64 = T::DEFAULT_MAX_VALUE.into();
    let strength = strength as f64 * max / 255.0;
    for pixel in rgba.pixels_mut() {
        for c in 0..3 {
            // 简单的Box-Muller近似高斯噪声
            seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            let u1 = (seed >> 33) as f64 / (1u64 << 31) as f64;
            seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            let u2 = (seed >> 33) as f64 / (1u64 << 31) as f64;

            let u1_clamped = u1.max(1e-10);
            let gaussian =
                (-2.0 * u1_clamped.ln()).sqrt() * (2.0 * std::f64::consts::PI * u2).cos();
            let noise = gaussian * strength;

            let val = f64::from(pixel[c]) + noise;
            let val = if quantize {
                val.round().clamp(0.0, max)
            } else {
                val
            };
            pixel[c] = T::from(val).unwrap();
        }
        // Alpha 通道不加噪点
    }
    rgba
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn noise_preserves_16bit_depth_and_alpha() {
        let root = super::super::image_io::test_dir("noise16");
        let output = root.join("out");
        std::fs::create_dir_all(&output).unwrap();
        for ext in ["png", "tiff"] {
            let source = root.join(format!("source.{ext}"));
            let original =
                image::ImageBuffer::from_pixel(4, 3, image::Rgba([23456u16, 34567, 45678, 12345]));
            original.save(&source).unwrap();
            let options = BlurNoiseOptions {
                input_path: root.to_string_lossy().into_owned(),
                output_path: output.to_string_lossy().into_owned(),
                blur_radius: 0.0,
                noise_strength: 1,
                recursive: false,
            };
            process_blur_noise(&source, &root, &output, &options).unwrap();
            let decoded = image::open(output.join(format!("source.{ext}"))).unwrap();
            assert_eq!(decoded.color(), image::ColorType::Rgba16);
            let output = decoded.to_rgba16();
            assert!(output.pixels().all(|p| p[3] == 12345));
            assert!(output.pixels().any(|p| p[0] % 257 != 0));
            let mut seed = super::super::perspective::path_seed(&source);
            seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            let u1 = ((seed >> 33) as f64 / (1u64 << 31) as f64).max(1e-10);
            seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            let u2 = (seed >> 33) as f64 / (1u64 << 31) as f64;
            let expected = (23456.0
                + (-2.0 * u1.ln()).sqrt() * (2.0 * std::f64::consts::PI * u2).cos() * 257.0)
                .round() as u16;
            assert_eq!(output.get_pixel(0, 0)[0], expected);
        }
        std::fs::remove_dir_all(root).unwrap();
    }
}
