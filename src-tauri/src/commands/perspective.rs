use image::{ImageBuffer, Pixel};
use serde::{Deserialize, Serialize};
use std::path::Path;

use super::image_io::{
    into_rgba_keeping_depth, load_image, map_pixels, save_like_source, Channel, PixelMap,
};
use super::{
    collect_image_files_with_recursive_excluding, dir_of, same_name_output, same_path,
    ProcessResult,
};

use super::batch::{BatchJob, FileBatch, FileOutcome};

static JOB: BatchJob = BatchJob::new("透视变换");

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PerspectiveOptions {
    pub input_path: String,
    pub output_path: String,
    /// 透视强度 0.0 ~ 0.5 (推荐 0.05 ~ 0.15)
    pub intensity: f64,
    #[serde(default)]
    pub recursive: bool,
}

#[tauri::command]
pub async fn perspective_transform<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    options: PerspectiveOptions,
) -> Result<ProcessResult, String> {
    JOB.run(move || perspective_sync(&app, &options)).await
}

#[tauri::command]
pub fn cancel_perspective() {
    JOB.cancel();
}

fn perspective_sync<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    options: &PerspectiveOptions,
) -> Result<ProcessResult, String> {
    let input = Path::new(&options.input_path);
    let output_dir = Path::new(&options.output_path);
    if !(0.0..=0.5).contains(&options.intensity) {
        return Err(format!(
            "透视强度 {} 超出有效范围 0.0~0.5（该值为比例，不是百分比）",
            options.intensity
        ));
    }
    std::fs::create_dir_all(output_dir).map_err(|e| format!("无法创建输出目录: {}", e))?;
    // 数据增强不能原地替换源图：输出目录就是输入所在目录时，每张图都会写回自己
    if same_path(&dir_of(input), output_dir) {
        return Err("输出目录与输入目录相同，请更换输出目录".to_string());
    }
    let files =
        collect_image_files_with_recursive_excluding(input, options.recursive, Some(output_dir))?;
    Ok(
        FileBatch::new(app, "perspective-progress", JOB.cancel_flag())
            .error_prefix("[失败] ")
            .archive_failures(input, output_dir, options.recursive)
            .run(
                &files,
                |item| {
                    process_perspective(item.path, input, output_dir, options)?;
                    Ok(FileOutcome::done(format!("[透视变换] {} ✓", item.name)))
                },
                |c| c.summary("处理完成"),
            ),
    )
}

fn process_perspective(
    file_path: &Path,
    input_root: &Path,
    output_dir: &Path,
    options: &PerspectiveOptions,
) -> Result<(), String> {
    use image::DynamicImage;

    let (img, source) = load_image(file_path)?;

    // 按路径种子选一个透视方向（4 种），同一路径结果可复现
    let variant = path_seed(file_path) % 4;

    let d = options.intensity;

    // 源四角（单位正方形）→ 目标四角 (归一化坐标)
    // 从目标像素反查源像素位置（逆映射）
    const SRC_CORNERS: [(f64, f64); 4] = [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)];
    let dst_corners = match variant {
        // 从上方俯视：顶部收窄
        0 => [(d, d * 0.5), (1.0 - d, d * 0.5), (1.0, 1.0), (0.0, 1.0)],
        // 从下方仰视：底部收窄
        1 => [
            (0.0, 0.0),
            (1.0, 0.0),
            (1.0 - d, 1.0 - d * 0.5),
            (d, 1.0 - d * 0.5),
        ],
        // 从左侧看：左边收窄
        2 => [(d * 0.5, d), (1.0, 0.0), (1.0, 1.0), (d * 0.5, 1.0 - d)],
        // 从右侧看：右边收窄
        _ => [
            (0.0, 0.0),
            (1.0 - d * 0.5, d),
            (1.0 - d * 0.5, 1.0 - d),
            (0.0, 1.0),
        ],
    };

    // 计算 3x3 透视变换矩阵 (dst→src)
    let mat = compute_perspective_matrix(&dst_corners, &SRC_CORNERS);

    // 越界区域要透明，先转成带 alpha 的同位深图再变换
    let out = map_pixels(into_rgba_keeping_depth(img), &Warp(mat));
    let output_path = same_name_output(input_root, file_path, output_dir, options.recursive)?;
    // JPEG/BMP 不保留透明边界，越界区域采用黑边。
    let out = if matches!(
        source.format,
        image::ImageFormat::Jpeg | image::ImageFormat::Bmp
    ) {
        DynamicImage::ImageRgb8(out.to_rgb8())
    } else {
        out
    };
    save_like_source(out, &output_path, &source)
}

/// 按 3x3 透视矩阵（目标归一化坐标 → 源归一化坐标）逆映射重采样
struct Warp([f64; 9]);

impl PixelMap for Warp {
    fn map<P>(&self, src: ImageBuffer<P, Vec<P::Subpixel>>) -> ImageBuffer<P, Vec<P::Subpixel>>
    where
        P: Pixel,
        P::Subpixel: Channel,
    {
        let mat = &self.0;
        let (w, h) = src.dimensions();
        let (fw, fh) = (w as f64, h as f64);

        let mut out = ImageBuffer::new(w, h);
        for py in 0..h {
            for px in 0..w {
                let nx = px as f64 / fw;
                let ny = py as f64 / fh;

                // 应用透视变换得到源坐标
                let denom = mat[6] * nx + mat[7] * ny + mat[8];
                if denom.abs() < 1e-10 {
                    continue;
                }
                let sx = (mat[0] * nx + mat[1] * ny + mat[2]) / denom;
                let sy = (mat[3] * nx + mat[4] * ny + mat[5]) / denom;

                let src_x = sx * fw;
                let src_y = sy * fh;

                // 双线性插值
                if src_x >= 0.0 && src_x < fw - 1.0 && src_y >= 0.0 && src_y < fh - 1.0 {
                    out.put_pixel(px, py, bilinear_sample(&src, src_x, src_y));
                }
                // 超出范围的像素保持透明/黑色
            }
        }

        out
    }
}

/// 由完整路径算出的确定性伪随机种子：同一路径种子不变，换目录后种子随之改变
pub(super) fn path_seed(path: &Path) -> u64 {
    path.to_string_lossy()
        .bytes()
        .fold(0u64, |acc, b| acc.wrapping_mul(31).wrapping_add(b as u64))
}

fn bilinear_sample<P>(img: &ImageBuffer<P, Vec<P::Subpixel>>, x: f64, y: f64) -> P
where
    P: Pixel,
    P::Subpixel: Channel,
{
    let (w, h) = img.dimensions();
    let x0 = x.floor() as u32;
    let y0 = y.floor() as u32;
    let x1 = (x0 + 1).min(w - 1);
    let y1 = (y0 + 1).min(h - 1);
    let fx = x - x0 as f64;
    let fy = y - y0 as f64;

    let corners = [
        (img.get_pixel(x0, y0), (1.0 - fx) * (1.0 - fy)),
        (img.get_pixel(x1, y0), fx * (1.0 - fy)),
        (img.get_pixel(x0, y1), (1.0 - fx) * fy),
        (img.get_pixel(x1, y1), fx * fy),
    ];
    let mut out = *corners[0].0;
    for (c, channel) in out.channels_mut().iter_mut().enumerate() {
        let value: f64 = corners
            .iter()
            .map(|(p, weight)| p.channels()[c].into() * weight)
            .sum();
        *channel = P::Subpixel::from_f64(value);
    }
    out
}

/// 计算 3x3 透视变换矩阵，将 src 四点映射到 dst 四点
#[allow(clippy::needless_range_loop)]
fn compute_perspective_matrix(src: &[(f64, f64); 4], dst: &[(f64, f64); 4]) -> [f64; 9] {
    // 使用 DLT (Direct Linear Transform) 算法
    // 构建 8x8 线性方程组 A * h = b，直接写成增广矩阵 [A | b]
    let mut aug = [[0.0f64; 9]; 8];
    for (i, (&(sx, sy), &(dx, dy))) in src.iter().zip(dst).enumerate() {
        aug[i * 2] = [sx, sy, 1.0, 0.0, 0.0, 0.0, -dx * sx, -dx * sy, dx];
        aug[i * 2 + 1] = [0.0, 0.0, 0.0, sx, sy, 1.0, -dy * sx, -dy * sy, dy];
    }

    // 高斯消元
    for col in 0..8 {
        // 选主元
        let mut max_row = col;
        for row in (col + 1)..8 {
            if aug[row][col].abs() > aug[max_row][col].abs() {
                max_row = row;
            }
        }
        aug.swap(col, max_row);

        let pivot = aug[col][col];
        if pivot.abs() < 1e-12 {
            return [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0];
        }

        for j in col..9 {
            aug[col][j] /= pivot;
        }
        for row in 0..8 {
            if row == col {
                continue;
            }
            let factor = aug[row][col];
            for j in col..9 {
                aug[row][j] -= factor * aug[col][j];
            }
        }
    }

    std::array::from_fn(|i| if i < 8 { aug[i][8] } else { 1.0 })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::test_support::TempDir;

    #[test]
    fn perspective_preserves_16bit_pixels() {
        let root = TempDir::new("perspective16");
        let output = root.join("out");
        std::fs::create_dir_all(&output).unwrap();
        for ext in ["png", "tiff"] {
            let source = root.join(format!("source.{ext}"));
            let pixels = image::ImageBuffer::from_fn(5, 5, |x, y| {
                image::Rgba([12345u16 + x as u16, 23456 + y as u16, 34567, 45678])
            });
            pixels.save(&source).unwrap();
            let options = PerspectiveOptions {
                input_path: root.to_string_lossy().into_owned(),
                output_path: output.to_string_lossy().into_owned(),
                intensity: 0.0,
                recursive: false,
            };
            process_perspective(&source, &root, &output, &options).unwrap();
            let image = image::open(output.join(format!("source.{ext}"))).unwrap();
            assert_eq!(image.color(), image::ColorType::Rgba16);
            assert_eq!(image.to_rgba16().get_pixel(2, 2), pixels.get_pixel(2, 2));
        }
        let pixels = image::ImageBuffer::from_fn(2, 2, |x, y| {
            image::Rgba([1001u16 + (x + y * 2) as u16 * 1000, 0, 0, 65535])
        });
        assert_eq!(bilinear_sample(&pixels, 0.5, 0.5)[0], 2501);
        let floats =
            image::ImageBuffer::from_fn(2, 2, |x, _| image::Rgba([x as f32, 0.25, 0.0, 1.0]));
        assert_eq!(
            bilinear_sample(&floats, 0.25, 0.0),
            image::Rgba([0.25, 0.25, 0.0, 1.0])
        );
    }

    fn options(input: &Path, output: &Path) -> PerspectiveOptions {
        PerspectiveOptions {
            input_path: input.to_string_lossy().into_owned(),
            output_path: output.to_string_lossy().into_owned(),
            intensity: 0.1,
            recursive: true,
        }
    }

    /// 输出目录就是输入目录（单张图片时是它所在的目录）：开始前报一次错，不碰源图
    #[test]
    fn same_output_dir_is_rejected_before_processing() {
        let root = TempDir::new("perspective_same_dir");
        let source = root.join("a.png");
        image::RgbImage::from_pixel(8, 8, image::Rgb([9, 9, 9]))
            .save(&source)
            .unwrap();
        let before = std::fs::read(&source).unwrap();
        let app = tauri::test::mock_app();
        let log = super::super::batch::capture_events(app.handle(), "perspective-progress");
        for input in [root.to_path_buf(), source.clone()] {
            let err = perspective_sync(app.handle(), &options(&input, &root)).unwrap_err();
            assert_eq!(err, "输出目录与输入目录相同，请更换输出目录");
        }
        assert!(log.lock().unwrap().is_empty());
        assert_eq!(std::fs::read(&source).unwrap(), before);
    }

    #[test]
    fn failed_files_are_archived_into_output_fail() {
        let root = TempDir::new("perspective_fail");
        let (input, output) = (root.join("in"), root.join("out"));
        std::fs::create_dir_all(&input).unwrap();
        image::RgbImage::from_pixel(8, 8, image::Rgb([9, 9, 9]))
            .save(input.join("ok.png"))
            .unwrap();
        std::fs::write(input.join("bad.png"), b"not an image").unwrap();
        let result =
            perspective_sync(tauri::test::mock_app().handle(), &options(&input, &output)).unwrap();
        assert_eq!((result.success_count, result.fail_count), (1, 1));
        assert!(output.join("ok.png").is_file());
        assert!(output.join("Fail/bad.png").is_file());
    }
}
