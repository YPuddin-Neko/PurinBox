use image::{ImageBuffer, Pixel, Primitive};
use serde::{Deserialize, Serialize};
use std::path::Path;

use super::image_io::{load_image, map_pixels, save_like_source, Channel, PixelMap};
use super::{
    collect_image_files_with_recursive_excluding, copy_file_safe, same_name_output, ProcessResult,
};

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
        (true, true) => Some("模糊+噪点"),
        (true, false) => Some("高斯模糊"),
        (false, true) => Some("噪点"),
        (false, false) => None,
    };
    Ok(
        FileBatch::new(app, "blur-noise-progress", JOB.cancel_flag())
            .error_prefix("[失败] ")
            .archive_failures(input, output_dir, options.recursive)
            .run(
                &files,
                |item| {
                    let output_path =
                        same_name_output(input, item.path, output_dir, options.recursive)?;
                    // 工作流节点默认 0/0：像素不变就原样复制，不重新编码
                    let Some(label) = label else {
                        copy_file_safe(item.path, &output_path)?;
                        return Ok(FileOutcome::unchanged(format!(
                            "[跳过] {} (模糊和噪点均为 0)",
                            item.name
                        )));
                    };
                    process_blur_noise(item.path, &output_path, options)?;
                    Ok(FileOutcome::done(format!("[{}] {} ✓", label, item.name)))
                },
                |c| c.summary("处理完成"),
            ),
    )
}

fn process_blur_noise(
    file_path: &Path,
    output_path: &Path,
    options: &BlurNoiseOptions,
) -> Result<(), String> {
    let (mut img, source) = load_image(file_path)?;

    if options.blur_radius > 0.0 {
        let sigma = options.blur_radius.max(0.1) as f32;
        img = img.blur(sigma);
    }

    if options.noise_strength > 0 {
        img = map_pixels(
            img,
            &GaussianNoise {
                strength: options.noise_strength,
                seed: super::perspective::path_seed(file_path),
            },
        );
    }
    save_like_source(img, output_path, &source)
}

/// 高斯噪点：强度按 8 位刻度（0~255）给出，按通道取值范围换算；alpha 通道不加。
/// 灰度图只有一个颜色通道，加的就是亮度噪点
struct GaussianNoise {
    strength: u32,
    seed: u64,
}

impl PixelMap for GaussianNoise {
    fn map<P>(&self, mut img: ImageBuffer<P, Vec<P::Subpixel>>) -> ImageBuffer<P, Vec<P::Subpixel>>
    where
        P: Pixel,
        P::Subpixel: Channel,
    {
        let max: f64 = P::Subpixel::DEFAULT_MAX_VALUE.into();
        let sigma = f64::from(self.strength) * max / 255.0;
        let color_channels = usize::from(P::CHANNEL_COUNT) - usize::from(P::HAS_ALPHA);
        let mut seed = self.seed;
        let mut uniform = || {
            seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (seed >> 33) as f64 / (1u64 << 31) as f64
        };
        for pixel in img.pixels_mut() {
            for channel in &mut pixel.channels_mut()[..color_channels] {
                // Box-Muller 变换
                let u1 = uniform().max(1e-10);
                let u2 = uniform();
                let gaussian = (-2.0 * u1.ln()).sqrt() * (2.0 * std::f64::consts::PI * u2).cos();
                // 浮点图也钳位到 [0, 1]，不写出负值或超过 1 的值
                let value = ((*channel).into() + gaussian * sigma).clamp(0.0, max);
                *channel = P::Subpixel::from_f64(value);
            }
        }
        img
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::test_support::TempDir;
    use image::{ColorType, DynamicImage, GenericImageView};

    fn options(input: &Path, output: &Path, blur: f64, noise: u32) -> BlurNoiseOptions {
        BlurNoiseOptions {
            input_path: input.to_string_lossy().into_owned(),
            output_path: output.to_string_lossy().into_owned(),
            blur_radius: blur,
            noise_strength: noise,
            recursive: false,
        }
    }

    /// 第一个像素第一个通道的噪声值（8 位刻度）
    fn first_gaussian(path: &Path) -> f64 {
        let mut seed = super::super::perspective::path_seed(path);
        let mut uniform = || {
            seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (seed >> 33) as f64 / (1u64 << 31) as f64
        };
        let u1 = uniform().max(1e-10);
        let u2 = uniform();
        (-2.0 * u1.ln()).sqrt() * (2.0 * std::f64::consts::PI * u2).cos()
    }

    #[test]
    fn noise_preserves_16bit_depth_and_alpha() {
        let root = TempDir::new("noise16");
        let output = root.join("out");
        std::fs::create_dir_all(&output).unwrap();
        for ext in ["png", "tiff"] {
            let source = root.join(format!("source.{ext}"));
            let original =
                image::ImageBuffer::from_pixel(4, 3, image::Rgba([23456u16, 34567, 45678, 12345]));
            original.save(&source).unwrap();
            let target = output.join(format!("source.{ext}"));
            process_blur_noise(&source, &target, &options(&root, &output, 0.0, 1)).unwrap();
            let decoded = image::open(&target).unwrap();
            assert_eq!(decoded.color(), ColorType::Rgba16);
            let output = decoded.to_rgba16();
            assert!(output.pixels().all(|p| p[3] == 12345));
            assert!(output.pixels().any(|p| p[0] % 257 != 0));
            let expected = (23456.0 + first_gaussian(&source) * 257.0).round() as u16;
            assert_eq!(output.get_pixel(0, 0)[0], expected);
        }
    }

    /// 灰度图保持灰度、只加亮度噪点；没有 alpha 的图不会多出 alpha
    #[test]
    fn noise_keeps_grayscale_and_does_not_add_alpha() {
        let root = TempDir::new("noise_color_types");
        let output = root.join("out");
        std::fs::create_dir_all(&output).unwrap();
        let cases: [(&str, DynamicImage); 4] = [
            (
                "gray8.png",
                DynamicImage::ImageLuma8(image::GrayImage::from_pixel(6, 6, image::Luma([128]))),
            ),
            (
                "gray16.png",
                DynamicImage::ImageLuma16(ImageBuffer::from_pixel(6, 6, image::Luma([30000u16]))),
            ),
            (
                "gray_alpha.png",
                DynamicImage::ImageLumaA8(image::GrayAlphaImage::from_pixel(
                    6,
                    6,
                    image::LumaA([128, 77]),
                )),
            ),
            (
                "rgb.png",
                DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
                    6,
                    6,
                    image::Rgb([100, 120, 140]),
                )),
            ),
        ];
        for (name, img) in cases {
            let color = img.color();
            let source = root.join(name);
            img.save(&source).unwrap();
            let target = output.join(name);
            process_blur_noise(&source, &target, &options(&root, &output, 0.0, 40)).unwrap();
            let decoded = image::open(&target).unwrap();
            assert_eq!(decoded.color(), color, "{}", name);
            assert_ne!(
                decoded,
                image::open(&source).unwrap(),
                "{} 应加上噪点",
                name
            );
        }
        let gray_alpha = image::open(output.join("gray_alpha.png"))
            .unwrap()
            .to_luma_alpha8();
        assert!(gray_alpha.pixels().all(|p| p[1] == 77));
        let gray = image::open(output.join("gray8.png")).unwrap().to_luma8();
        let expected = (128.0 + first_gaussian(&root.join("gray8.png")) * 40.0)
            .round()
            .clamp(0.0, 255.0) as u8;
        assert_eq!(gray.get_pixel(0, 0)[0], expected);
    }

    /// 32 位浮点图加噪后钳位到 [0, 1]
    #[test]
    fn float_noise_is_clamped_to_unit_range() {
        let noisy = map_pixels(
            DynamicImage::ImageRgb32F(ImageBuffer::from_fn(16, 16, |x, _| {
                image::Rgb([if x % 2 == 0 { 0.0 } else { 1.0 }, 0.5, 1.0])
            })),
            &GaussianNoise {
                strength: 100,
                seed: 42,
            },
        );
        assert_eq!(noisy.color(), ColorType::Rgb32F);
        let values: Vec<f32> = noisy.to_rgb32f().into_raw();
        assert!(values.iter().all(|v| (0.0..=1.0).contains(v)), "{values:?}");
        assert!(values.iter().any(|v| *v > 0.0 && *v < 1.0));
    }

    /// 模糊和噪点都为 0（工作流节点默认值）时原样复制、计为 unchanged，不解码像素
    #[test]
    fn zero_blur_and_noise_copies_without_reencoding() {
        let root = TempDir::new("noise_zero");
        let (input, output) = (root.join("in"), root.join("out"));
        std::fs::create_dir_all(&input).unwrap();
        super::super::image_io::write_broken_pixels(&input.join("broken.png"));
        std::fs::write(input.join("photo.jpg"), b"jpeg bytes kept as-is").unwrap();
        let app = tauri::test::mock_app();
        let log = super::super::batch::capture_events(app.handle(), "blur-noise-progress");
        let result = blur_noise_sync(app.handle(), &options(&input, &output, 0.0, 0)).unwrap();
        assert_eq!((result.success_count, result.fail_count), (2, 0));
        for name in ["broken.png", "photo.jpg"] {
            assert_eq!(
                std::fs::read(input.join(name)).unwrap(),
                std::fs::read(output.join(name)).unwrap()
            );
        }
        let events = log.lock().unwrap();
        assert!(events
            .iter()
            .any(|e| e["message"] == "[跳过] broken.png (模糊和噪点均为 0)"));
    }

    #[test]
    fn blur_keeps_dimensions_and_failed_files_go_to_fail() {
        let root = TempDir::new("blur_fail");
        let (input, output) = (root.join("in"), root.join("out"));
        std::fs::create_dir_all(&input).unwrap();
        image::RgbImage::from_fn(8, 8, |x, _| image::Rgb([(x * 30) as u8, 0, 0]))
            .save(input.join("ok.png"))
            .unwrap();
        std::fs::write(input.join("bad.png"), b"not an image").unwrap();
        let result = blur_noise_sync(
            tauri::test::mock_app().handle(),
            &options(&input, &output, 1.5, 0),
        )
        .unwrap();
        assert_eq!((result.success_count, result.fail_count), (1, 1));
        let blurred = image::open(output.join("ok.png")).unwrap();
        assert_eq!(
            (blurred.dimensions(), blurred.color()),
            ((8, 8), ColorType::Rgb8)
        );
        assert_eq!(
            std::fs::read(output.join("Fail/bad.png")).unwrap(),
            b"not an image"
        );
    }
}
