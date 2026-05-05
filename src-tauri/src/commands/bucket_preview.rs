use serde::{Deserialize, Serialize};
use std::path::Path;
use tauri::Emitter;

/// 分桶分析参数
#[derive(Debug, Clone, Deserialize)]
pub struct BucketOptions {
    /// 输入图片文件夹
    pub input_path: String,
    /// 训练分辨率宽
    pub res_width: u32,
    /// 训练分辨率高
    pub res_height: u32,
    /// bucket_reso_steps (对齐粒度，如 32/64)
    pub steps: u32,
    /// 是否禁止放大（小图不拉伸）
    pub no_upscale: bool,
    /// repeat 次数
    pub repeats: u32,
}

/// 单张图片的分桶信息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BucketImageInfo {
    /// 文件路径
    pub path: String,
    /// 文件名
    pub name: String,
    /// 原始宽
    pub orig_width: u32,
    /// 原始高
    pub orig_height: u32,
}

/// 单个桶
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BucketGroup {
    /// 桶索引
    pub index: u32,
    /// 桶宽
    pub bucket_width: u32,
    /// 桶高
    pub bucket_height: u32,
    /// 物理图片数
    pub image_count: u32,
    /// count = 物理图片数 × repeats
    pub total_count: u32,
    /// 宽高比
    pub aspect_ratio: f64,
    /// 包含的图片
    pub images: Vec<BucketImageInfo>,
}

/// 分桶分析结果
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BucketAnalysis {
    /// 总图片数
    pub total_images: u32,
    /// 总 count
    pub total_count: u32,
    /// 桶数量
    pub bucket_count: u32,
    /// 读取失败的文件
    pub skipped: Vec<(String, String)>,
    /// 各桶详情
    pub buckets: Vec<BucketGroup>,
}

/// 进度事件
#[derive(Debug, Clone, Serialize)]
struct ScanProgress {
    current: u32,
    total: u32,
    status: String,
    message: String,
}

/// 计算图片的目标桶分辨率
fn calculate_bucket_reso(w: u32, h: u32, max_area: f64, steps: u32, no_upscale: bool) -> (u32, u32) {
    let img_area = w as f64 * h as f64;
    let mut scale = (max_area / img_area).sqrt();

    if no_upscale && scale > 1.0 {
        scale = 1.0;
    }

    let new_w = (w as f64 * scale + 0.5) as u32;
    let new_h = (h as f64 * scale + 0.5) as u32;

    let bucket_w = (new_w as f64 / steps as f64).round().max(1.0) as u32 * steps;
    let bucket_h = (new_h as f64 / steps as f64).round().max(1.0) as u32 * steps;

    (bucket_w, bucket_h)
}

/// 分析分桶（不复制文件，仅计算）
#[tauri::command]
pub async fn analyze_buckets(
    app: tauri::AppHandle,
    options: BucketOptions,
) -> Result<BucketAnalysis, String> {
    let input_path = std::path::PathBuf::from(&options.input_path);
    if !input_path.exists() || !input_path.is_dir() {
        return Err(format!("目录不存在: {}", options.input_path));
    }

    let max_area = options.res_width as f64 * options.res_height as f64;
    let steps = options.steps.max(1);
    let repeats = options.repeats.max(1);

    // 收集图片文件
    let supported_exts = ["png", "jpg", "jpeg", "webp", "bmp", "tiff", "tif", "gif"];
    let mut image_files: Vec<std::path::PathBuf> = Vec::new();

    for entry in walkdir::WalkDir::new(&input_path)
        .max_depth(1)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        let p = entry.path();
        if p.is_file() {
            if let Some(ext) = p.extension() {
                let ext_lower = ext.to_string_lossy().to_lowercase();
                if supported_exts.contains(&ext_lower.as_str()) {
                    image_files.push(p.to_path_buf());
                }
            }
        }
    }

    image_files.sort_by(|a, b| a.file_name().cmp(&b.file_name()));
    let file_count = image_files.len() as u32;

    let _ = app.emit("bucket-progress", ScanProgress {
        current: 0, total: file_count,
        status: "info".to_string(),
        message: format!("正在扫描 {} 张图片...", file_count),
    });

    // 分桶
    let mut bucket_map: std::collections::BTreeMap<(u32, u32), Vec<BucketImageInfo>> =
        std::collections::BTreeMap::new();
    let mut skipped: Vec<(String, String)> = Vec::new();
    let mut processed = 0u32;

    for file_path in &image_files {
        let name = file_path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();

        match image::image_dimensions(file_path) {
            Ok((w, h)) => {
                let (bw, bh) = calculate_bucket_reso(w, h, max_area, steps, options.no_upscale);
                let img_info = BucketImageInfo {
                    path: file_path.to_string_lossy().to_string(),
                    name,
                    orig_width: w,
                    orig_height: h,
                };
                bucket_map.entry((bw, bh)).or_default().push(img_info);
            }
            Err(e) => {
                skipped.push((name, e.to_string()));
            }
        }

        processed += 1;
        if processed.is_multiple_of(50) || processed == file_count {
            let _ = app.emit("bucket-progress", ScanProgress {
                current: processed,
                total: file_count,
                status: "processing".to_string(),
                message: format!("已分析 {}/{}", processed, file_count),
            });
        }
    }

    // 构建结果
    let mut buckets: Vec<BucketGroup> = Vec::new();
    let mut total_images = 0u32;

    for (idx, ((bw, bh), images)) in bucket_map.iter().enumerate() {
        let count = images.len() as u32;
        total_images += count;
        buckets.push(BucketGroup {
            index: idx as u32,
            bucket_width: *bw,
            bucket_height: *bh,
            image_count: count,
            total_count: count * repeats,
            aspect_ratio: (*bw as f64 / *bh as f64 * 100.0).round() / 100.0,
            images: images.clone(),
        });
    }

    let total_count = total_images * repeats;

    let _ = app.emit("bucket-progress", ScanProgress {
        current: file_count, total: file_count,
        status: "done".to_string(),
        message: format!("分析完成: {} 张图片 → {} 个桶, 总 count {}",
            total_images, buckets.len(), total_count),
    });

    Ok(BucketAnalysis {
        total_images,
        total_count,
        bucket_count: buckets.len() as u32,
        skipped,
        buckets,
    })
}

/// 导出分桶结果（将图片按桶复制到子文件夹）
#[tauri::command]
pub async fn export_buckets(
    app: tauri::AppHandle,
    analysis: BucketAnalysis,
    output_path: String,
    repeats: u32,
) -> Result<String, String> {
    let out_dir = Path::new(&output_path);
    if !out_dir.exists() {
        std::fs::create_dir_all(out_dir)
            .map_err(|e| format!("创建输出目录失败: {}", e))?;
    }

    let total_files: u32 = analysis.buckets.iter().map(|b| b.image_count).sum();
    let mut copied = 0u32;

    for bucket in &analysis.buckets {
        let folder_name = format!(
            "Bucket {} - {}x{} (count {})",
            bucket.index, bucket.bucket_width, bucket.bucket_height,
            bucket.image_count * repeats
        );
        let bucket_dir = out_dir.join(&folder_name);
        std::fs::create_dir_all(&bucket_dir)
            .map_err(|e| format!("创建桶目录失败: {}", e))?;

        for img in &bucket.images {
            let src = Path::new(&img.path);
            let mut dst = bucket_dir.join(&img.name);

            // 处理同名文件
            let mut counter = 1;
            while dst.exists() {
                let stem = Path::new(&img.name)
                    .file_stem()
                    .unwrap_or_default()
                    .to_string_lossy();
                let ext = Path::new(&img.name)
                    .extension()
                    .map(|e| format!(".{}", e.to_string_lossy()))
                    .unwrap_or_default();
                dst = bucket_dir.join(format!("{}_{}{}", stem, counter, ext));
                counter += 1;
            }

            std::fs::copy(src, &dst)
                .map_err(|e| format!("复制文件失败 {}: {}", img.name, e))?;

            copied += 1;
            if copied.is_multiple_of(20) || copied == total_files {
                let _ = app.emit("bucket-export-progress", ScanProgress {
                    current: copied,
                    total: total_files,
                    status: if copied == total_files { "done" } else { "processing" }.to_string(),
                    message: format!("已导出 {}/{}", copied, total_files),
                });
            }
        }
    }

    Ok(format!("导出完成: {} 张图片已复制到 {} 个桶", copied, analysis.bucket_count))
}
