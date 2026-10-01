use serde::{Deserialize, Serialize};
use std::path::Path;

use super::image_io::{load_image, save_like_source};
use super::{collect_image_files_with_recursive_excluding, same_name_output, ProcessResult};

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
    let files =
        collect_image_files_with_recursive_excluding(input, options.recursive, Some(output_dir))?;
    Ok(
        FileBatch::new(app, "perspective-progress", JOB.cancel_flag())
            .error_prefix("[失败] ")
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

    let out = match img.color() {
        image::ColorType::L16
        | image::ColorType::La16
        | image::ColorType::Rgb16
        | image::ColorType::Rgba16 => {
            DynamicImage::ImageRgba16(warp_rgba(&img.to_rgba16(), &mat, true))
        }
        image::ColorType::Rgb32F | image::ColorType::Rgba32F => {
            DynamicImage::ImageRgba32F(warp_rgba(&img.to_rgba32f(), &mat, false))
        }
        _ => DynamicImage::ImageRgba8(warp_rgba(&img.to_rgba8(), &mat, true)),
    };
    let output_path = same_name_output(input_root, file_path, output_dir, options.recursive)?;
    // 数据增强不能原地替换源图。
    if crate::commands::path_key_ci(&output_path) == crate::commands::path_key_ci(file_path) {
        return Err("输出与输入为同一文件，已跳过（请更换输出目录）".to_string());
    }
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

fn warp_rgba<T: image::Primitive>(
    rgba: &image::ImageBuffer<image::Rgba<T>, Vec<T>>,
    mat: &[f64; 9],
    quantize: bool,
) -> image::ImageBuffer<image::Rgba<T>, Vec<T>>
where
    f64: From<T>,
    image::Rgba<T>: image::Pixel<Subpixel = T>,
{
    let (w, h) = rgba.dimensions();
    let (fw, fh) = (w as f64, h as f64);

    let mut out = image::ImageBuffer::new(w, h);
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
                let pixel = bilinear_sample(rgba, src_x, src_y, w, h, quantize);
                out.put_pixel(px, py, pixel);
            }
            // 超出范围的像素保持透明/黑色
        }
    }

    out
}

/// 由完整路径算出的确定性伪随机种子：同一路径种子不变，换目录后种子随之改变
pub(super) fn path_seed(path: &Path) -> u64 {
    path.to_string_lossy()
        .bytes()
        .fold(0u64, |acc, b| acc.wrapping_mul(31).wrapping_add(b as u64))
}

fn bilinear_sample<T: image::Primitive>(
    img: &image::ImageBuffer<image::Rgba<T>, Vec<T>>,
    x: f64,
    y: f64,
    w: u32,
    h: u32,
    quantize: bool,
) -> image::Rgba<T>
where
    f64: From<T>,
    image::Rgba<T>: image::Pixel<Subpixel = T>,
{
    let x0 = x.floor() as u32;
    let y0 = y.floor() as u32;
    let x1 = (x0 + 1).min(w - 1);
    let y1 = (y0 + 1).min(h - 1);
    let fx = x - x0 as f64;
    let fy = y - y0 as f64;

    let p00 = img.get_pixel(x0, y0);
    let p10 = img.get_pixel(x1, y0);
    let p01 = img.get_pixel(x0, y1);
    let p11 = img.get_pixel(x1, y1);

    let lerp = |a: T, b: T, c: T, d: T| -> T {
        let v = f64::from(a) * (1.0 - fx) * (1.0 - fy)
            + f64::from(b) * fx * (1.0 - fy)
            + f64::from(c) * (1.0 - fx) * fy
            + f64::from(d) * fx * fy;
        let v = if quantize {
            v.round().clamp(0.0, T::DEFAULT_MAX_VALUE.into())
        } else {
            v
        };
        T::from(v).unwrap()
    };

    image::Rgba([
        lerp(p00[0], p10[0], p01[0], p11[0]),
        lerp(p00[1], p10[1], p01[1], p11[1]),
        lerp(p00[2], p10[2], p01[2], p11[2]),
        lerp(p00[3], p10[3], p01[3], p11[3]),
    ])
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

    #[test]
    fn perspective_preserves_16bit_pixels() {
        let root = super::super::image_io::test_dir("perspective16");
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
        assert_eq!(bilinear_sample(&pixels, 0.5, 0.5, 2, 2, true)[0], 2501);
        std::fs::remove_dir_all(root).unwrap();
    }
}
