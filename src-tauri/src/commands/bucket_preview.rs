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
    /// bucket_reso_steps (对齐粒度，如 32/64/128)
    pub steps: u32,
    /// 是否禁止放大（小图不拉伸）
    pub no_upscale: bool,
    /// repeat 次数
    pub repeats: u32,
    /// 桶最小分辨率边长 (min_bucket_reso)
    pub min_bucket_reso: Option<u32>,
    /// 桶最大分辨率边长 (max_bucket_reso)
    pub max_bucket_reso: Option<u32>,
    /// 分桶策略: "legacy" | "nearest_only"
    pub bucket_mode: Option<String>,
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

/// sd-scripts round_to_steps: 先四舍五入再向下对齐到 steps
fn round_to_steps(x: f64, steps: u32) -> u32 {
    let v = (x + 0.5) as u32;
    let aligned = v - v % steps;
    aligned.max(steps)
}

/// 生成候选桶分辨率列表（对应 sd-scripts model_util.make_bucket_resolutions）
fn make_bucket_resolutions(max_reso: (u32, u32), min_size: u32, max_size: u32, divisible: u32) -> Vec<(u32, u32)> {
    let max_area = max_reso.0 as u64 * max_reso.1 as u64;
    let mut resos = std::collections::BTreeSet::new();

    // 正方形桶
    let sq = ((max_area as f64).sqrt() / divisible as f64) as u32 * divisible;
    resos.insert((sq, sq));

    // 从 min_size 到 max_size 枚举宽度
    let mut width = min_size;
    while width <= max_size {
        let height = std::cmp::min(
            max_size,
            ((max_area / width as u64) / divisible as u64) as u32 * divisible,
        );
        if height >= min_size {
            resos.insert((width, height));
            resos.insert((height, width));
        }
        width += divisible;
    }

    resos.into_iter().collect()
}

/// nearest_only 模式：根据实际图片尺寸生成最匹配的桶
/// 对应 lora-rescripts BucketManager.make_buckets_by_nearest_image_aspect
fn make_buckets_by_nearest(image_sizes: &[(u32, u32)], max_area: f64, reso_steps: u32, min_size: u32) -> Vec<(u32, u32)> {
    let min_edge = reso_steps.max(min_size);
    let mut resos = std::collections::BTreeSet::new();

    for &(w, h) in image_sizes {
        if w == 0 || h == 0 { continue; }
        let aspect = w as f64 / h as f64;
        let target_w = (max_area * aspect).sqrt();
        let target_h = max_area / target_w;

        // 方案1: 先对齐宽度
        let b_w_rounded = round_to_steps(target_w, reso_steps).max(min_edge);
        let b_h_in_wr = round_to_steps(b_w_rounded as f64 / aspect, reso_steps).max(min_edge);
        let ar_w_rounded = b_w_rounded as f64 / b_h_in_wr as f64;

        // 方案2: 先对齐高度
        let b_h_rounded = round_to_steps(target_h, reso_steps).max(min_edge);
        let b_w_in_hr = round_to_steps(b_h_rounded as f64 * aspect, reso_steps).max(min_edge);
        let ar_h_rounded = b_w_in_hr as f64 / b_h_rounded as f64;

        if (ar_w_rounded - aspect).abs() <= (ar_h_rounded - aspect).abs() {
            resos.insert((b_w_rounded, b_h_in_wr));
        } else {
            resos.insert((b_w_in_hr, b_h_rounded));
        }
    }

    resos.into_iter().collect()
}

/// 对单张图片选择最佳桶（预定义桶匹配模式）
/// 对应 sd-scripts BucketManager.select_bucket（use_predefined_buckets=true）
fn select_bucket_predefined(w: u32, h: u32, predefined_resos: &[(u32, u32)]) -> (u32, u32) {
    let aspect = w as f64 / h as f64;
    // 如果原图分辨率恰好在列表中则直接用
    if predefined_resos.contains(&(w, h)) {
        return (w, h);
    }
    let mut best_idx = 0;
    let mut best_err = f64::MAX;
    for (i, &(bw, bh)) in predefined_resos.iter().enumerate() {
        let err = (bw as f64 / bh as f64 - aspect).abs();
        if err < best_err {
            best_err = err;
            best_idx = i;
        }
    }
    predefined_resos[best_idx]
}

/// legacy + no_upscale 模式的桶选择（直接 round 对齐）
fn select_bucket_no_upscale(w: u32, h: u32, max_area: f64, reso_steps: u32) -> (u32, u32) {
    let aspect = w as f64 / h as f64;

    if (w as f64 * h as f64) > max_area {
        // 图片太大，按面积等比缩小后选最佳对齐方案
        let resized_w = (max_area * aspect).sqrt();
        let resized_h = max_area / resized_w;

        let b_w_rounded = round_to_steps(resized_w, reso_steps);
        let b_h_in_wr = round_to_steps(b_w_rounded as f64 / aspect, reso_steps);
        let ar_w_rounded = b_w_rounded as f64 / b_h_in_wr as f64;

        let b_h_rounded = round_to_steps(resized_h, reso_steps);
        let b_w_in_hr = round_to_steps(b_h_rounded as f64 * aspect, reso_steps);
        let ar_h_rounded = b_w_in_hr as f64 / b_h_rounded as f64;

        let resized_size = if (ar_w_rounded - aspect).abs() < (ar_h_rounded - aspect).abs() {
            (b_w_rounded, (b_w_rounded as f64 / aspect + 0.5) as u32)
        } else {
            ((b_h_rounded as f64 * aspect + 0.5) as u32, b_h_rounded)
        };
        let bw = resized_size.0 - resized_size.0 % reso_steps;
        let bh = resized_size.1 - resized_size.1 % reso_steps;
        (bw.max(reso_steps), bh.max(reso_steps))
    } else {
        // 图片不需要缩小，直接向下对齐到 reso_steps
        let bw = w - w % reso_steps;
        let bh = h - h % reso_steps;
        (bw.max(reso_steps), bh.max(reso_steps))
    }
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
    let bucket_mode = options.bucket_mode.as_deref().unwrap_or("legacy");

    // min/max bucket reso（仅 no_upscale=false 且 legacy 模式时有效）
    let min_size = options.min_bucket_reso.unwrap_or(256).max(steps);
    let max_size = options.max_bucket_reso
        .unwrap_or(std::cmp::max(options.res_width, options.res_height))
        .max(std::cmp::max(options.res_width, options.res_height));

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

    // nearest_only 需要先读取所有图片尺寸
    if bucket_mode == "nearest_only" {
        let mut image_sizes: Vec<(u32, u32)> = Vec::new();
        let mut image_data: Vec<(std::path::PathBuf, String, u32, u32)> = Vec::new();
        let mut skipped: Vec<(String, String)> = Vec::new();
        let mut processed = 0u32;

        for file_path in &image_files {
            let name = file_path.file_name().unwrap_or_default().to_string_lossy().to_string();
            match image::image_dimensions(file_path) {
                Ok((w, h)) => {
                    image_sizes.push((w, h));
                    image_data.push((file_path.clone(), name, w, h));
                }
                Err(e) => { skipped.push((name, e.to_string())); }
            }
            processed += 1;
            if processed.is_multiple_of(50) || processed == file_count {
                let _ = app.emit("bucket-progress", ScanProgress {
                    current: processed, total: file_count,
                    status: "processing".to_string(),
                    message: format!("已分析 {}/{}", processed, file_count),
                });
            }
        }

        // 根据实际图片尺寸生成桶列表
        let predefined_resos = make_buckets_by_nearest(&image_sizes, max_area, steps, min_size);

        // 分配图片到桶
        let mut bucket_map: std::collections::BTreeMap<(u32, u32), Vec<BucketImageInfo>> =
            std::collections::BTreeMap::new();
        for (file_path, name, w, h) in &image_data {
            let (bw, bh) = select_bucket_predefined(*w, *h, &predefined_resos);
            bucket_map.entry((bw, bh)).or_default().push(BucketImageInfo {
                path: file_path.to_string_lossy().to_string(),
                name: name.clone(),
                orig_width: *w,
                orig_height: *h,
            });
        }

        return build_analysis_result(&app, bucket_map, skipped, repeats, file_count);
    }

    // legacy 模式
    // 决定是否使用预定义桶
    let use_predefined = !options.no_upscale;
    let predefined_resos = if use_predefined {
        make_bucket_resolutions(
            (options.res_width, options.res_height),
            min_size, max_size, steps,
        )
    } else {
        vec![] // no_upscale legacy 不需要
    };

    // 分桶
    let mut bucket_map: std::collections::BTreeMap<(u32, u32), Vec<BucketImageInfo>> =
        std::collections::BTreeMap::new();
    let mut skipped: Vec<(String, String)> = Vec::new();
    let mut processed = 0u32;

    for file_path in &image_files {
        let name = file_path.file_name().unwrap_or_default().to_string_lossy().to_string();
        match image::image_dimensions(file_path) {
            Ok((w, h)) => {
                let (bw, bh) = if use_predefined {
                    select_bucket_predefined(w, h, &predefined_resos)
                } else {
                    select_bucket_no_upscale(w, h, max_area, steps)
                };
                bucket_map.entry((bw, bh)).or_default().push(BucketImageInfo {
                    path: file_path.to_string_lossy().to_string(),
                    name,
                    orig_width: w,
                    orig_height: h,
                });
            }
            Err(e) => { skipped.push((name, e.to_string())); }
        }
        processed += 1;
        if processed.is_multiple_of(50) || processed == file_count {
            let _ = app.emit("bucket-progress", ScanProgress {
                current: processed, total: file_count,
                status: "processing".to_string(),
                message: format!("已分析 {}/{}", processed, file_count),
            });
        }
    }

    build_analysis_result(&app, bucket_map, skipped, repeats, file_count)
}

/// 构建分桶分析结果
fn build_analysis_result(
    app: &tauri::AppHandle,
    bucket_map: std::collections::BTreeMap<(u32, u32), Vec<BucketImageInfo>>,
    skipped: Vec<(String, String)>,
    repeats: u32,
    file_count: u32,
) -> Result<BucketAnalysis, String> {
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
