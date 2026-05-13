use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::Emitter;

use super::{collect_image_files, ProcessResult, ProgressEvent};

static CANCEL_FLAG: AtomicBool = AtomicBool::new(false);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PerspectiveOptions {
    pub input_path: String,
    pub output_path: String,
    /// 透视强度 0.0 ~ 0.5 (推荐 0.05 ~ 0.15)
    pub intensity: f64,
}

#[tauri::command]
pub async fn perspective_transform(app: tauri::AppHandle, options: PerspectiveOptions) -> Result<ProcessResult, String> {
    CANCEL_FLAG.store(false, Ordering::SeqCst);
    tokio::task::spawn_blocking(move || {
        perspective_sync(&app, &options)
    })
    .await
    .map_err(|e| format!("任务执行失败: {}", e))?
}

#[tauri::command]
pub fn cancel_perspective() {
    CANCEL_FLAG.store(true, Ordering::SeqCst);
}

fn perspective_sync(app: &tauri::AppHandle, options: &PerspectiveOptions) -> Result<ProcessResult, String> {
    let input = Path::new(&options.input_path);
    let output_dir = Path::new(&options.output_path);

    if !output_dir.exists() {
        std::fs::create_dir_all(output_dir)
            .map_err(|e| format!("无法创建输出目录: {}", e))?;
    }

    let files = collect_image_files(input)?;
    let total = files.len() as u32;
    let mut success_count = 0u32;
    let mut fail_count = 0u32;
    let mut errors = Vec::new();

    for (i, file_path) in files.iter().enumerate() {
        if CANCEL_FLAG.load(Ordering::SeqCst) {
            let _ = app.emit("perspective-progress", ProgressEvent {
                current: i as u32, total, filename: String::new(),
                status: "done".to_string(),
                message: format!("已取消: 已处理 {}, 共 {}", i, total),
            });
            break;
        }

        let filename = file_path.file_name().unwrap_or_default().to_string_lossy().to_string();

        let _ = app.emit("perspective-progress", ProgressEvent {
            current: i as u32 + 1,
            total,
            filename: filename.clone(),
            status: "processing".to_string(),
            message: format!("正在处理: {}", filename),
        });

        match process_perspective(file_path, output_dir, options.intensity) {
            Ok(_) => {
                success_count += 1;
                let _ = app.emit("perspective-progress", ProgressEvent {
                    current: i as u32 + 1,
                    total,
                    filename: filename.clone(),
                    status: "success".to_string(),
                    message: format!("[透视变换] {} ✓", filename),
                });
            }
            Err(e) => {
                fail_count += 1;
                let err_msg = format!("{}: {}", filename, e);
                errors.push(err_msg.clone());
                let _ = app.emit("perspective-progress", ProgressEvent {
                    current: i as u32 + 1,
                    total,
                    filename: filename.clone(),
                    status: "error".to_string(),
                    message: format!("[失败] {}", err_msg),
                });
            }
        }
    }

    let _ = app.emit("perspective-progress", ProgressEvent {
        current: total,
        total,
        filename: String::new(),
        status: "done".to_string(),
        message: format!("处理完成: 成功 {}, 失败 {}, 共 {}", success_count, fail_count, total),
    });

    Ok(ProcessResult { success_count, fail_count, total, errors })
}

fn process_perspective(file_path: &Path, output_dir: &Path, intensity: f64) -> Result<(), String> {
    use image::{GenericImageView, RgbaImage};

    let img = image::open(file_path)
        .map_err(|e| format!("无法打开图片: {}", e))?;

    let (w, h) = img.dimensions();
    let rgba = img.to_rgba8();

    // 随机选择一个透视方向（使用文件名hash作为伪随机种子，确保可复现）
    let seed: u64 = file_path.to_string_lossy().bytes().fold(0u64, |acc, b| acc.wrapping_mul(31).wrapping_add(b as u64));
    let variant = seed % 4; // 4种透视方向

    let fw = w as f64;
    let fh = h as f64;
    let d = intensity;

    // 源四角 → 目标四角 (归一化坐标)
    // 从目标像素反查源像素位置（逆映射）
    let (src_corners, dst_corners) = match variant {
        0 => {
            // 从上方俯视：顶部收窄
            ([(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)],
             [(d, d * 0.5), (1.0 - d, d * 0.5), (1.0, 1.0), (0.0, 1.0)])
        }
        1 => {
            // 从下方仰视：底部收窄
            ([(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)],
             [(0.0, 0.0), (1.0, 0.0), (1.0 - d, 1.0 - d * 0.5), (d, 1.0 - d * 0.5)])
        }
        2 => {
            // 从左侧看：左边收窄
            ([(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)],
             [(d * 0.5, d), (1.0, 0.0), (1.0, 1.0), (d * 0.5, 1.0 - d)])
        }
        _ => {
            // 从右侧看：右边收窄
            ([(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)],
             [(0.0, 0.0), (1.0 - d * 0.5, d), (1.0 - d * 0.5, 1.0 - d), (0.0, 1.0)])
        }
    };

    // 计算 3x3 透视变换矩阵 (dst→src)
    let mat = compute_perspective_matrix(&dst_corners, &src_corners);

    let mut out = RgbaImage::new(w, h);
    for py in 0..h {
        for px in 0..w {
            let nx = px as f64 / fw;
            let ny = py as f64 / fh;

            // 应用透视变换得到源坐标
            let denom = mat[6] * nx + mat[7] * ny + mat[8];
            if denom.abs() < 1e-10 { continue; }
            let sx = (mat[0] * nx + mat[1] * ny + mat[2]) / denom;
            let sy = (mat[3] * nx + mat[4] * ny + mat[5]) / denom;

            let src_x = sx * fw;
            let src_y = sy * fh;

            // 双线性插值
            if src_x >= 0.0 && src_x < fw - 1.0 && src_y >= 0.0 && src_y < fh - 1.0 {
                let pixel = bilinear_sample(&rgba, src_x, src_y, w, h);
                out.put_pixel(px, py, pixel);
            }
            // 超出范围的像素保持透明/黑色
        }
    }

    let file_name = file_path.file_name().ok_or("无效的文件名")?.to_string_lossy();
    let output_path = output_dir.join(file_name.as_ref());
    out.save(&output_path).map_err(|e| format!("无法保存图片: {}", e))?;
    Ok(())
}

fn bilinear_sample(img: &image::RgbaImage, x: f64, y: f64, w: u32, h: u32) -> image::Rgba<u8> {
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

    let lerp = |a: u8, b: u8, c: u8, d: u8| -> u8 {
        let v = (a as f64) * (1.0 - fx) * (1.0 - fy)
            + (b as f64) * fx * (1.0 - fy)
            + (c as f64) * (1.0 - fx) * fy
            + (d as f64) * fx * fy;
        v.round().clamp(0.0, 255.0) as u8
    };

    image::Rgba([
        lerp(p00[0], p10[0], p01[0], p11[0]),
        lerp(p00[1], p10[1], p01[1], p11[1]),
        lerp(p00[2], p10[2], p01[2], p11[2]),
        lerp(p00[3], p10[3], p01[3], p11[3]),
    ])
}

/// 计算 3x3 透视变换矩阵，将 src 四点映射到 dst 四点
fn compute_perspective_matrix(
    src: &[(f64, f64); 4],
    dst: &[(f64, f64); 4],
) -> [f64; 9] {
    // 使用 DLT (Direct Linear Transform) 算法
    // 构建 8x8 线性方程组 A * h = b
    let mut a_mat = [[0.0f64; 8]; 8];
    let mut b_vec = [0.0f64; 8];

    for i in 0..4 {
        let (sx, sy) = src[i];
        let (dx, dy) = dst[i];
        let row1 = i * 2;
        let row2 = i * 2 + 1;

        a_mat[row1] = [sx, sy, 1.0, 0.0, 0.0, 0.0, -dx * sx, -dx * sy];
        b_vec[row1] = dx;

        a_mat[row2] = [0.0, 0.0, 0.0, sx, sy, 1.0, -dy * sx, -dy * sy];
        b_vec[row2] = dy;
    }

    // 高斯消元
    let mut aug = [[0.0f64; 9]; 8];
    for i in 0..8 {
        for j in 0..8 { aug[i][j] = a_mat[i][j]; }
        aug[i][8] = b_vec[i];
    }

    for col in 0..8 {
        // 选主元
        let mut max_row = col;
        for row in (col + 1)..8 {
            if aug[row][col].abs() > aug[max_row][col].abs() { max_row = row; }
        }
        aug.swap(col, max_row);

        let pivot = aug[col][col];
        if pivot.abs() < 1e-12 { return [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0]; }

        for j in col..9 { aug[col][j] /= pivot; }
        for row in 0..8 {
            if row == col { continue; }
            let factor = aug[row][col];
            for j in col..9 { aug[row][j] -= factor * aug[col][j]; }
        }
    }

    let h: Vec<f64> = (0..8).map(|i| aug[i][8]).collect();
    [h[0], h[1], h[2], h[3], h[4], h[5], h[6], h[7], 1.0]
}
