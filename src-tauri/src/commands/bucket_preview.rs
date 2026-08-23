use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::Emitter;

/// 分析与推荐各自独立的取消标志（两者可分别取消，互不影响）
static ANALYZE_CANCEL: AtomicBool = AtomicBool::new(false);
static RECOMMEND_CANCEL: AtomicBool = AtomicBool::new(false);

/// 取消正在进行的分桶分析
#[tauri::command]
pub fn cancel_bucket_analysis() {
    ANALYZE_CANCEL.store(true, Ordering::SeqCst);
}

/// 取消正在进行的分桶参数推荐
#[tauri::command]
pub fn cancel_bucket_recommend() {
    RECOMMEND_CANCEL.store(true, Ordering::SeqCst);
}

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
    /// 桶最小分辨率边长 (min_bucket_reso)
    pub min_bucket_reso: Option<u32>,
    /// 桶最大分辨率边长 (max_bucket_reso)
    pub max_bucket_reso: Option<u32>,
    /// 分桶策略: "legacy" | "nearest_only" | "diffusion_pipe"
    pub bucket_mode: Option<String>,
    /// 是否递归扫描子文件夹
    pub recursive: Option<bool>,
    /// diffusion-pipe AR 桶最小宽高比
    pub dp_min_ar: Option<f64>,
    /// diffusion-pipe AR 桶最大宽高比
    pub dp_max_ar: Option<f64>,
    /// diffusion-pipe AR 桶数量
    pub dp_num_ar_buckets: Option<u32>,
    /// 训练 batch size，用于估算有效样本
    pub batch_size: Option<u32>,
    /// 是否丢弃不足一个 batch 的桶尾样本
    pub drop_last: Option<bool>,
}

/// 分桶推荐参数
#[derive(Debug, Clone, Deserialize)]
pub struct BucketRecommendOptions {
    /// 输入图片文件夹
    pub input_path: String,
    /// 是否递归扫描子文件夹
    pub recursive: Option<bool>,
}

/// 分桶推荐结果
#[derive(Debug, Clone, Serialize)]
pub struct BucketParamRecommendation {
    /// 成功读取的图片数量
    pub total_images: u32,
    /// 读取失败的图片数量
    pub skipped_count: u32,
    /// 不同尺寸数量
    pub unique_sizes: u32,
    /// 不同 AR 数量
    pub unique_aspect_ratios: u32,
    /// 推荐训练分辨率宽
    pub res_width: u32,
    /// 推荐训练分辨率高
    pub res_height: u32,
    /// 推荐桶划分单位
    pub steps: u32,
    /// 推荐 DP 最小 AR
    pub dp_min_ar: f64,
    /// 推荐 DP 最大 AR
    pub dp_max_ar: f64,
    /// 推荐 DP AR 桶数量
    pub dp_num_ar_buckets: u32,
    /// 推荐 SD-Scripts 最小桶尺寸
    pub min_bucket_reso: u32,
    /// 推荐 SD-Scripts 最大桶尺寸
    pub max_bucket_reso: u32,
    /// 推荐 batch size（按 drop_last=true 的可用率评估）
    pub batch_size: u32,
    /// 活跃尺寸桶数量
    pub active_bucket_count: u32,
    /// 总 count
    pub total_count: u32,
    /// 有效 count
    pub effective_count: u32,
    /// 被丢弃的 count
    pub dropped_count: u32,
    /// count 可用率
    pub usable_rate: f64,
    /// 候选推荐列表
    pub candidates: Vec<BucketParamCandidate>,
}

/// 单个推荐候选
#[derive(Debug, Clone, Serialize)]
pub struct BucketParamCandidate {
    pub res_width: u32,
    pub res_height: u32,
    pub steps: u32,
    pub dp_min_ar: f64,
    pub dp_max_ar: f64,
    pub dp_num_ar_buckets: u32,
    pub batch_size: u32,
    pub active_bucket_count: u32,
    pub total_count: u32,
    pub effective_count: u32,
    pub dropped_count: u32,
    pub usable_rate: f64,
    pub mean_ar_error: f64,
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
    /// 重复次数（从父文件夹名前缀检测）
    pub repeats: u32,
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
    /// 按 batch size/drop_last 估算的有效 count
    pub effective_count: u32,
    /// 按 batch size/drop_last 估算会被丢弃的 count
    pub dropped_count: u32,
    /// 当前桶会产生的 batch 数
    pub batch_count: u32,
    /// 当前桶会产生的短 batch 数
    pub short_batch_count: u32,
    /// 宽高比
    pub aspect_ratio: f64,
    /// 当前桶内图片的平均 AR 误差
    pub mean_ar_error: f64,
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
    /// 有效 count
    pub effective_count: u32,
    /// 被丢弃 count
    pub dropped_count: u32,
    /// batch 数
    pub batch_count: u32,
    /// 短 batch 数
    pub short_batch_count: u32,
    /// count 可用率
    pub usable_rate: f64,
    /// 估算使用的 batch size
    pub batch_size: u32,
    /// 是否丢弃不足 batch size 的桶尾样本
    pub drop_last: bool,
    /// 桶数量
    pub bucket_count: u32,
    /// 读取失败的文件
    pub skipped: Vec<(String, String)>,
    /// 各桶详情
    pub buckets: Vec<BucketGroup>,
    /// 平均 AR 误差 (without repeats)
    pub mean_ar_error: f64,
    /// AR 误差计算方式: "linear" | "log"
    pub ar_error_metric: String,
}

/// 进度事件
#[derive(Debug, Clone, Serialize)]
struct ScanProgress {
    current: u32,
    total: u32,
    status: String,
    message: String,
}

#[derive(Debug, Clone, Copy)]
enum ArErrorMetric {
    Linear,
    Log,
}

#[derive(Debug, Clone, Copy)]
struct RecommendSample {
    width: u32,
    height: u32,
    repeats: u32,
}

#[derive(Debug, Clone, Copy)]
struct BucketBatchStats {
    effective_count: u32,
    dropped_count: u32,
    batch_count: u32,
    short_batch_count: u32,
}

#[derive(Debug, Clone, Copy)]
struct CandidateEval {
    active_bucket_count: u32,
    total_count: u32,
    effective_count: u32,
    dropped_count: u32,
    usable_rate: f64,
    mean_ar_error: f64,
}

fn compute_bucket_batch_stats(
    total_count: u32,
    batch_size: u32,
    drop_last: bool,
) -> BucketBatchStats {
    let batch_size = batch_size.max(1);
    if total_count == 0 {
        return BucketBatchStats {
            effective_count: 0,
            dropped_count: 0,
            batch_count: 0,
            short_batch_count: 0,
        };
    }

    let full_batches = total_count / batch_size;
    let remainder = total_count % batch_size;
    if drop_last {
        let effective_count = full_batches * batch_size;
        BucketBatchStats {
            effective_count,
            dropped_count: total_count - effective_count,
            batch_count: full_batches,
            short_batch_count: 0,
        }
    } else {
        BucketBatchStats {
            effective_count: total_count,
            dropped_count: 0,
            batch_count: full_batches + u32::from(remainder > 0),
            short_batch_count: u32::from(remainder > 0),
        }
    }
}

/// SD-Scripts round_to_steps: 先四舍五入再向下对齐到 steps
fn round_to_steps(x: f64, steps: u32) -> u32 {
    let v = (x + 0.5) as u32;
    let aligned = v - v % steps;
    aligned.max(steps)
}

/// 从文件夹名提取 repeats 前缀（如 "10_character" → 10，无前缀则返回 1）
fn extract_repeats(folder_name: &str) -> u32 {
    if let Some(pos) = folder_name.find('_') {
        if let Ok(n) = folder_name[..pos].parse::<u32>() {
            return n.max(1);
        }
    }
    1
}

/// 读取图片尺寸时按文件头猜测格式，避免 WebP 内容使用 .png 后缀时被当作 PNG 解析失败。
fn read_image_dimensions(path: &Path) -> image::ImageResult<(u32, u32)> {
    image::ImageReader::open(path)?
        .with_guessed_format()?
        .into_dimensions()
}

fn collect_supported_image_files(input_path: &Path, recursive: bool) -> Vec<PathBuf> {
    let supported_exts = ["png", "jpg", "jpeg", "webp", "bmp", "tiff", "tif", "gif"];
    let walk_depth = if recursive { usize::MAX } else { 1 };
    let mut image_files = Vec::new();

    for entry in walkdir::WalkDir::new(input_path)
        .max_depth(walk_depth)
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

    image_files.sort();
    image_files
}

fn round_up_to_multiple(value: u32, multiple: u32) -> u32 {
    if value == 0 {
        multiple
    } else {
        value.div_ceil(multiple) * multiple
    }
}

/// 生成候选桶分辨率列表（对应 SD-Scripts model_util.make_bucket_resolutions）
fn make_bucket_resolutions(
    max_reso: (u32, u32),
    min_size: u32,
    max_size: u32,
    divisible: u32,
) -> Vec<(u32, u32)> {
    let max_area = max_reso.0 as u64 * max_reso.1 as u64;
    let mut resos = std::collections::BTreeSet::new();

    // 正方形桶（下限保护：极小训练分辨率会算出 0，(0,0) 桶让 aspect_ratio 变 NaN 崩前端）
    let sq = (((max_area as f64).sqrt() / divisible as f64) as u32 * divisible).max(divisible);
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
fn make_buckets_by_nearest(
    image_sizes: &[(u32, u32)],
    max_area: f64,
    reso_steps: u32,
    min_size: u32,
) -> Vec<(u32, u32)> {
    let min_edge = reso_steps.max(min_size);
    let mut resos = std::collections::BTreeSet::new();

    for &(w, h) in image_sizes {
        if w == 0 || h == 0 {
            continue;
        }
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

/// diffusion-pipe 使用 numpy.geomspace(min_ar, max_ar, num_ar_buckets)，再按 3 位小数去重排序。
fn make_diffusion_pipe_ar_buckets(
    min_ar: f64,
    max_ar: f64,
    num_ar_buckets: u32,
) -> Result<Vec<f64>, String> {
    if !min_ar.is_finite() || !max_ar.is_finite() || min_ar <= 0.0 || max_ar <= min_ar {
        return Err("DP AR 范围无效，需要满足 0 < min_ar < max_ar".to_string());
    }

    let count = num_ar_buckets.max(1);
    let mut ars = Vec::with_capacity(count as usize);
    if count == 1 {
        ars.push(round_ar_to_3(min_ar));
    } else {
        let log_min = min_ar.ln();
        let log_max = max_ar.ln();
        for i in 0..count {
            let t = i as f64 / (count - 1) as f64;
            let ar = (log_min + (log_max - log_min) * t).exp();
            ars.push(round_ar_to_3(ar));
        }
    }

    ars.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    ars.dedup_by(|a, b| (*a - *b).abs() < f64::EPSILON);
    Ok(ars)
}

fn round_ar_to_3(ar: f64) -> f64 {
    round_half_to_even(ar * 1000.0) / 1000.0
}

fn round_half_to_even(x: f64) -> f64 {
    let floor = x.floor();
    let frac = x - floor;
    if (frac - 0.5).abs() < 1e-12 {
        if (floor as u64).is_multiple_of(2) {
            floor
        } else {
            floor + 1.0
        }
    } else {
        x.round()
    }
}

/// diffusion-pipe utils.common.round_to_nearest_multiple
fn round_to_nearest_multiple_dp(x: f64, multiple: u32) -> u32 {
    (round_half_to_even(x / multiple as f64) as u32 * multiple).max(multiple)
}

/// diffusion-pipe ARBucketDataset.cache_latents 的 size_bucket 生成逻辑。
fn make_diffusion_pipe_size_bucket(ar: f64, max_area: f64, round_to_multiple: u32) -> (u32, u32) {
    let w = (max_area * ar).sqrt();
    let h = max_area / w;
    (
        round_to_nearest_multiple_dp(w, round_to_multiple),
        round_to_nearest_multiple_dp(h, round_to_multiple),
    )
}

/// diffusion-pipe DirectoryDataset._find_closest_ar_bucket：在 log 空间选择最近 AR。
fn select_diffusion_pipe_ar_bucket(w: u32, h: u32, ar_buckets: &[f64]) -> f64 {
    let image_log_ar = (w as f64 / h as f64).ln();
    ar_buckets
        .iter()
        .copied()
        .min_by(|a, b| {
            let da = (image_log_ar - a.ln()).abs();
            let db = (image_log_ar - b.ln()).abs();
            da.partial_cmp(&db).unwrap_or(std::cmp::Ordering::Equal)
        })
        .unwrap_or(1.0)
}

/// 对单张图片选择最佳桶（预定义桶匹配模式）
/// 对应 SD-Scripts BucketManager.select_bucket（use_predefined_buckets=true）
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
///
/// 扫描与计算是同步 CPU/IO 密集操作，必须放入 spawn_blocking，
/// 否则会占死 tokio worker 导致其他命令与事件全部卡住。
#[tauri::command]
pub async fn analyze_buckets<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    options: BucketOptions,
) -> Result<BucketAnalysis, String> {
    // 互斥：页面与工作流节点共用全局取消标志，并发会互吞取消
    static RUNNING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    let _busy = crate::commands::BusyGuard::acquire(&RUNNING, "分桶分析")?;

    ANALYZE_CANCEL.store(false, Ordering::SeqCst);
    tokio::task::spawn_blocking(move || analyze_buckets_sync(app, options))
        .await
        .map_err(|e| format!("分桶分析任务执行失败: {}", e))?
}

fn analyze_buckets_sync<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    options: BucketOptions,
) -> Result<BucketAnalysis, String> {
    let input_path = std::path::PathBuf::from(&options.input_path);
    if !input_path.exists() || !input_path.is_dir() {
        return Err(format!("目录不存在: {}", options.input_path));
    }

    let max_area = options.res_width as f64 * options.res_height as f64;
    let bucket_mode = options.bucket_mode.as_deref().unwrap_or("legacy");
    let steps = options.steps.max(1);
    let recursive = options.recursive.unwrap_or(false);
    let batch_size = options.batch_size.unwrap_or(1).max(1);
    let drop_last = options.drop_last.unwrap_or(bucket_mode == "diffusion_pipe");

    // min/max bucket reso（仅 no_upscale=false 且 legacy 模式时有效）
    let min_size = options.min_bucket_reso.unwrap_or(256).max(steps);
    let max_size = options
        .max_bucket_reso
        .unwrap_or(std::cmp::max(options.res_width, options.res_height))
        .max(std::cmp::max(options.res_width, options.res_height));

    let image_files = collect_supported_image_files(&input_path, recursive);
    let file_count = image_files.len() as u32;

    let _ = app.emit(
        "bucket-progress",
        ScanProgress {
            current: 0,
            total: file_count,
            status: "info".to_string(),
            message: format!("正在扫描 {} 张图片...", file_count),
        },
    );

    // diffusion-pipe 模式：按几何间隔 AR 桶 + log AR 最近距离分配，尺寸按目标面积和 32 对齐。
    if bucket_mode == "diffusion_pipe" {
        let min_ar = options.dp_min_ar.unwrap_or(0.5);
        let max_ar = options.dp_max_ar.unwrap_or(2.0);
        let num_ar_buckets = options.dp_num_ar_buckets.unwrap_or(7);
        let ar_buckets = make_diffusion_pipe_ar_buckets(min_ar, max_ar, num_ar_buckets)?;

        let mut bucket_map: std::collections::BTreeMap<(u32, u32), Vec<BucketImageInfo>> =
            std::collections::BTreeMap::new();
        let mut skipped: Vec<(String, String)> = Vec::new();
        let mut processed = 0u32;

        for file_path in &image_files {
            if ANALYZE_CANCEL.load(Ordering::SeqCst) {
                return Err("已取消".to_string());
            }
            let name = file_path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();
            let img_repeats = file_path
                .parent()
                .and_then(|p| p.file_name())
                .map(|n| extract_repeats(&n.to_string_lossy()))
                .unwrap_or(1);
            match read_image_dimensions(file_path) {
                Ok((w, h)) => {
                    let ar_bucket = select_diffusion_pipe_ar_bucket(w, h, &ar_buckets);
                    let (bw, bh) = make_diffusion_pipe_size_bucket(ar_bucket, max_area, steps);
                    bucket_map
                        .entry((bw, bh))
                        .or_default()
                        .push(BucketImageInfo {
                            path: file_path.to_string_lossy().to_string(),
                            name,
                            orig_width: w,
                            orig_height: h,
                            repeats: img_repeats,
                        });
                }
                Err(e) => {
                    skipped.push((name, e.to_string()));
                }
            }
            processed += 1;
            if processed.is_multiple_of(50) || processed == file_count {
                let _ = app.emit(
                    "bucket-progress",
                    ScanProgress {
                        current: processed,
                        total: file_count,
                        status: "processing".to_string(),
                        message: format!("已分析 {}/{}", processed, file_count),
                    },
                );
            }
        }

        return build_analysis_result(
            &app,
            bucket_map,
            skipped,
            file_count,
            ArErrorMetric::Log,
            batch_size,
            drop_last,
        );
    }

    // nearest_only 需要先读取所有图片尺寸
    if bucket_mode == "nearest_only" {
        let mut image_sizes: Vec<(u32, u32)> = Vec::new();
        let mut image_data: Vec<(std::path::PathBuf, String, u32, u32, u32)> = Vec::new();
        let mut skipped: Vec<(String, String)> = Vec::new();
        let mut processed = 0u32;

        for file_path in &image_files {
            if ANALYZE_CANCEL.load(Ordering::SeqCst) {
                return Err("已取消".to_string());
            }
            let name = file_path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();
            let img_repeats = file_path
                .parent()
                .and_then(|p| p.file_name())
                .map(|n| extract_repeats(&n.to_string_lossy()))
                .unwrap_or(1);
            match read_image_dimensions(file_path) {
                Ok((w, h)) => {
                    image_sizes.push((w, h));
                    image_data.push((file_path.clone(), name, w, h, img_repeats));
                }
                Err(e) => {
                    skipped.push((name, e.to_string()));
                }
            }
            processed += 1;
            if processed.is_multiple_of(50) || processed == file_count {
                let _ = app.emit(
                    "bucket-progress",
                    ScanProgress {
                        current: processed,
                        total: file_count,
                        status: "processing".to_string(),
                        message: format!("已分析 {}/{}", processed, file_count),
                    },
                );
            }
        }

        // 根据实际图片尺寸生成桶列表
        let predefined_resos = make_buckets_by_nearest(&image_sizes, max_area, steps, min_size);

        // 分配图片到桶
        let mut bucket_map: std::collections::BTreeMap<(u32, u32), Vec<BucketImageInfo>> =
            std::collections::BTreeMap::new();
        for (file_path, name, w, h, img_repeats) in &image_data {
            let (bw, bh) = select_bucket_predefined(*w, *h, &predefined_resos);
            bucket_map
                .entry((bw, bh))
                .or_default()
                .push(BucketImageInfo {
                    path: file_path.to_string_lossy().to_string(),
                    name: name.clone(),
                    orig_width: *w,
                    orig_height: *h,
                    repeats: *img_repeats,
                });
        }

        return build_analysis_result(
            &app,
            bucket_map,
            skipped,
            file_count,
            ArErrorMetric::Linear,
            batch_size,
            drop_last,
        );
    }

    // legacy 模式
    // 决定是否使用预定义桶
    let use_predefined = !options.no_upscale;
    let predefined_resos = if use_predefined {
        make_bucket_resolutions(
            (options.res_width, options.res_height),
            min_size,
            max_size,
            steps,
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
        if ANALYZE_CANCEL.load(Ordering::SeqCst) {
            return Err("已取消".to_string());
        }
        let name = file_path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        let img_repeats = file_path
            .parent()
            .and_then(|p| p.file_name())
            .map(|n| extract_repeats(&n.to_string_lossy()))
            .unwrap_or(1);
        match read_image_dimensions(file_path) {
            Ok((w, h)) => {
                let (bw, bh) = if use_predefined {
                    select_bucket_predefined(w, h, &predefined_resos)
                } else {
                    select_bucket_no_upscale(w, h, max_area, steps)
                };
                bucket_map
                    .entry((bw, bh))
                    .or_default()
                    .push(BucketImageInfo {
                        path: file_path.to_string_lossy().to_string(),
                        name,
                        orig_width: w,
                        orig_height: h,
                        repeats: img_repeats,
                    });
            }
            Err(e) => {
                skipped.push((name, e.to_string()));
            }
        }
        processed += 1;
        if processed.is_multiple_of(50) || processed == file_count {
            let _ = app.emit(
                "bucket-progress",
                ScanProgress {
                    current: processed,
                    total: file_count,
                    status: "processing".to_string(),
                    message: format!("已分析 {}/{}", processed, file_count),
                },
            );
        }
    }

    build_analysis_result(
        &app,
        bucket_map,
        skipped,
        file_count,
        ArErrorMetric::Linear,
        batch_size,
        drop_last,
    )
}

fn evaluate_diffusion_pipe_candidate(
    samples: &[RecommendSample],
    max_area: f64,
    ar_buckets: &[f64],
    steps: u32,
    batch_size: u32,
    drop_last: bool,
) -> CandidateEval {
    let mut bucket_counts: std::collections::BTreeMap<(u32, u32), u32> =
        std::collections::BTreeMap::new();
    let mut error_sum = 0.0f64;
    let mut total_count = 0u32;

    for sample in samples {
        let image_ar = sample.width as f64 / sample.height as f64;
        let ar_bucket = select_diffusion_pipe_ar_bucket(sample.width, sample.height, ar_buckets);
        let (bw, bh) = make_diffusion_pipe_size_bucket(ar_bucket, max_area, steps);
        let bucket_ar = bw as f64 / bh as f64;
        error_sum += (image_ar.ln() - bucket_ar.ln()).abs();
        total_count += sample.repeats;
        *bucket_counts.entry((bw, bh)).or_insert(0) += sample.repeats;
    }

    let mut effective_count = 0u32;
    let mut dropped_count = 0u32;
    for count in bucket_counts.values() {
        let stats = compute_bucket_batch_stats(*count, batch_size, drop_last);
        effective_count += stats.effective_count;
        dropped_count += stats.dropped_count;
    }

    CandidateEval {
        active_bucket_count: bucket_counts.len() as u32,
        total_count,
        effective_count,
        dropped_count,
        usable_rate: if total_count > 0 {
            effective_count as f64 / total_count as f64
        } else {
            0.0
        },
        mean_ar_error: if samples.is_empty() {
            0.0
        } else {
            error_sum / samples.len() as f64
        },
    }
}

/// 根据数据集尺寸分布推荐分桶参数
///
/// 网格搜索是纯 CPU 密集操作（最坏约 2.5 万次全数据集评估），
/// 必须放入 spawn_blocking，否则会长时间占死 tokio worker。
#[tauri::command]
pub async fn recommend_bucket_params(
    options: BucketRecommendOptions,
) -> Result<BucketParamRecommendation, String> {
    RECOMMEND_CANCEL.store(false, Ordering::SeqCst);
    tokio::task::spawn_blocking(move || recommend_bucket_params_sync(options))
        .await
        .map_err(|e| format!("参数推荐任务执行失败: {}", e))?
}

fn recommend_bucket_params_sync(
    options: BucketRecommendOptions,
) -> Result<BucketParamRecommendation, String> {
    let input_path = PathBuf::from(&options.input_path);
    if !input_path.exists() || !input_path.is_dir() {
        return Err(format!("目录不存在: {}", options.input_path));
    }

    let recursive = options.recursive.unwrap_or(false);
    let image_files = collect_supported_image_files(&input_path, recursive);

    let mut samples: Vec<RecommendSample> = Vec::new();
    let mut skipped_count = 0u32;

    for file_path in &image_files {
        if RECOMMEND_CANCEL.load(Ordering::SeqCst) {
            return Err("已取消".to_string());
        }
        let img_repeats = file_path
            .parent()
            .and_then(|p| p.file_name())
            .map(|n| extract_repeats(&n.to_string_lossy()))
            .unwrap_or(1);
        match read_image_dimensions(file_path) {
            Ok((w, h)) if w > 0 && h > 0 => samples.push(RecommendSample {
                width: w,
                height: h,
                repeats: img_repeats,
            }),
            Ok(_) => skipped_count += 1,
            Err(_) => skipped_count += 1,
        }
    }

    if samples.is_empty() {
        return Err("没有可读取的图片，无法推荐参数".to_string());
    }

    let dimensions: Vec<(u32, u32)> = samples
        .iter()
        .map(|sample| (sample.width, sample.height))
        .collect();
    let total_repeated_count: u32 = samples.iter().map(|sample| sample.repeats).sum();

    let mut areas: Vec<u64> = samples
        .iter()
        .map(|sample| sample.width as u64 * sample.height as u64)
        .collect();
    areas.sort_unstable();
    let median_area_index = (areas.len() - 1) / 2;
    let median_area = areas[median_area_index] as f64;
    let median_side = round_to_nearest_multiple_dp(median_area.sqrt(), 64).max(512);

    let mut ars: Vec<f64> = samples
        .iter()
        .map(|sample| sample.width as f64 / sample.height as f64)
        .collect();
    ars.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

    let mut dp_min_ar = round_ar_to_3(ars[0]);
    let mut dp_max_ar = round_ar_to_3(*ars.last().unwrap_or(&ars[0]));
    if dp_max_ar <= dp_min_ar {
        let ar = ars[0];
        dp_min_ar = round_ar_to_3((ar * 0.95).max(0.05));
        dp_max_ar = round_ar_to_3(ar * 1.05);
        if dp_max_ar <= dp_min_ar {
            dp_max_ar = dp_min_ar + 0.001;
        }
    }

    let unique_sizes = dimensions
        .iter()
        .copied()
        .collect::<std::collections::BTreeSet<_>>()
        .len() as u32;
    let unique_aspect_ratios = ars
        .iter()
        .map(|ar| (round_ar_to_3(*ar) * 1000.0).round() as u32)
        .collect::<std::collections::BTreeSet<_>>()
        .len() as u32;
    let mut side_candidates = std::collections::BTreeSet::new();
    for index in [
        0usize,
        areas.len() / 4,
        median_area_index,
        areas.len() * 3 / 4,
        areas.len() - 1,
    ] {
        side_candidates
            .insert(round_to_nearest_multiple_dp((areas[index] as f64).sqrt(), 64).max(512));
    }
    let min_candidate_side = (areas[0] as f64).sqrt() * 0.9;
    let max_candidate_side = (*areas.last().unwrap_or(&areas[0]) as f64).sqrt() * 1.1;
    for common_side in [512, 768, 1024, 1280, 1536, 2048] {
        let common_side_f64 = common_side as f64;
        if common_side_f64 >= min_candidate_side && common_side_f64 <= max_candidate_side {
            side_candidates.insert(common_side);
        }
    }

    let mut range_candidates = Vec::new();
    let mut range_keys = std::collections::BTreeSet::new();
    for padding in [0.0, 0.02, 0.05, 0.10] {
        let min_ar = round_ar_to_3((dp_min_ar * (1.0 - padding)).max(0.05));
        let mut max_ar = round_ar_to_3(dp_max_ar * (1.0 + padding));
        if max_ar <= min_ar {
            max_ar = round_ar_to_3(min_ar * 1.05).max(min_ar + 0.001);
        }
        let key = (
            (min_ar * 1000.0).round() as u32,
            (max_ar * 1000.0).round() as u32,
        );
        if range_keys.insert(key) {
            range_candidates.push((min_ar, max_ar));
        }
    }

    let max_batch_candidate = total_repeated_count.clamp(1, 16);
    let mut scored_candidates: Vec<(f64, BucketParamCandidate)> = Vec::new();
    for side in &side_candidates {
        for steps in [32u32, 64u32] {
            for (min_ar, max_ar) in &range_candidates {
                for count in 2u32..=40u32 {
                    if RECOMMEND_CANCEL.load(Ordering::SeqCst) {
                        return Err("已取消".to_string());
                    }
                    let ar_buckets = make_diffusion_pipe_ar_buckets(*min_ar, *max_ar, count)?;
                    let max_area = (*side as f64) * (*side as f64);
                    for batch_size in 1u32..=max_batch_candidate {
                        let eval = evaluate_diffusion_pipe_candidate(
                            &samples,
                            max_area,
                            &ar_buckets,
                            steps,
                            batch_size,
                            true,
                        );
                        let side_delta_penalty =
                            side.abs_diff(median_side) as f64 / median_side.max(1) as f64;
                        let score = eval.usable_rate * 100.0 + (batch_size as f64).ln() * 4.0
                            - eval.mean_ar_error * 35.0
                            - side_delta_penalty * 2.0
                            - eval.active_bucket_count as f64 * 0.025
                            - count as f64 * 0.006;
                        scored_candidates.push((
                            score,
                            BucketParamCandidate {
                                res_width: *side,
                                res_height: *side,
                                steps,
                                dp_min_ar: *min_ar,
                                dp_max_ar: *max_ar,
                                dp_num_ar_buckets: count,
                                batch_size,
                                active_bucket_count: eval.active_bucket_count,
                                total_count: eval.total_count,
                                effective_count: eval.effective_count,
                                dropped_count: eval.dropped_count,
                                usable_rate: eval.usable_rate,
                                mean_ar_error: eval.mean_ar_error,
                            },
                        ));
                    }
                }
            }
        }
    }

    scored_candidates.sort_by(|a, b| {
        b.0.partial_cmp(&a.0)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| {
                a.1.mean_ar_error
                    .partial_cmp(&b.1.mean_ar_error)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .then_with(|| b.1.batch_size.cmp(&a.1.batch_size))
    });

    let mut candidates = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    for (_, candidate) in scored_candidates {
        let key = (
            candidate.res_width,
            candidate.steps,
            (candidate.dp_min_ar * 1000.0).round() as u32,
            (candidate.dp_max_ar * 1000.0).round() as u32,
            candidate.dp_num_ar_buckets,
            candidate.batch_size,
        );
        if seen.insert(key) {
            candidates.push(candidate);
        }
        if candidates.len() >= 6 {
            break;
        }
    }

    if candidates.is_empty() {
        return Err("没有生成可用的推荐参数".to_string());
    }
    let best = candidates[0].clone();

    let max_side = dimensions
        .iter()
        .map(|(w, h)| std::cmp::max(*w, *h))
        .max()
        .unwrap_or(best.res_width)
        .max(best.res_width);

    Ok(BucketParamRecommendation {
        total_images: dimensions.len() as u32,
        skipped_count,
        unique_sizes,
        unique_aspect_ratios,
        res_width: best.res_width,
        res_height: best.res_height,
        steps: best.steps,
        dp_min_ar: best.dp_min_ar,
        dp_max_ar: best.dp_max_ar,
        dp_num_ar_buckets: best.dp_num_ar_buckets,
        min_bucket_reso: 256,
        max_bucket_reso: round_up_to_multiple(max_side, 64),
        batch_size: best.batch_size,
        active_bucket_count: best.active_bucket_count,
        total_count: best.total_count,
        effective_count: best.effective_count,
        dropped_count: best.dropped_count,
        usable_rate: best.usable_rate,
        candidates,
    })
}

/// 构建分桶分析结果
fn build_analysis_result<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    bucket_map: std::collections::BTreeMap<(u32, u32), Vec<BucketImageInfo>>,
    skipped: Vec<(String, String)>,
    file_count: u32,
    ar_error_metric: ArErrorMetric,
    batch_size: u32,
    drop_last: bool,
) -> Result<BucketAnalysis, String> {
    let batch_size = batch_size.max(1);
    let mut buckets: Vec<BucketGroup> = Vec::new();
    let mut total_images = 0u32;
    let mut total_count = 0u32;
    let mut effective_count = 0u32;
    let mut dropped_count = 0u32;
    let mut batch_count = 0u32;
    let mut short_batch_count = 0u32;
    let mut ar_error_sum = 0.0f64;

    for (idx, ((bw, bh), images)) in bucket_map.iter().enumerate() {
        let count = images.len() as u32;
        total_images += count;
        let bucket_ar = *bw as f64 / *bh as f64;
        let mut bucket_total_count = 0u32;
        let mut bucket_ar_error_sum = 0.0f64;
        // 计算每张图片的 AR 误差 和 count
        for img in images {
            let image_ar = img.orig_width as f64 / img.orig_height as f64;
            let ar_error = match ar_error_metric {
                ArErrorMetric::Linear => (image_ar - bucket_ar).abs(),
                ArErrorMetric::Log => (image_ar.ln() - bucket_ar.ln()).abs(),
            };
            ar_error_sum += ar_error;
            bucket_ar_error_sum += ar_error;
            bucket_total_count += img.repeats;
        }
        total_count += bucket_total_count;
        let bucket_mean_ar_error = if count > 0 {
            bucket_ar_error_sum / count as f64
        } else {
            0.0
        };
        let bucket_batch_stats =
            compute_bucket_batch_stats(bucket_total_count, batch_size, drop_last);
        effective_count += bucket_batch_stats.effective_count;
        dropped_count += bucket_batch_stats.dropped_count;
        batch_count += bucket_batch_stats.batch_count;
        short_batch_count += bucket_batch_stats.short_batch_count;
        buckets.push(BucketGroup {
            index: idx as u32,
            bucket_width: *bw,
            bucket_height: *bh,
            image_count: count,
            total_count: bucket_total_count,
            effective_count: bucket_batch_stats.effective_count,
            dropped_count: bucket_batch_stats.dropped_count,
            batch_count: bucket_batch_stats.batch_count,
            short_batch_count: bucket_batch_stats.short_batch_count,
            aspect_ratio: (bucket_ar * 100.0).round() / 100.0,
            mean_ar_error: bucket_mean_ar_error,
            images: images.clone(),
        });
    }

    let mean_ar_error = if total_images > 0 {
        ar_error_sum / total_images as f64
    } else {
        0.0
    };
    let usable_rate = if total_count > 0 {
        effective_count as f64 / total_count as f64
    } else {
        0.0
    };

    let _ = app.emit(
        "bucket-progress",
        ScanProgress {
            current: file_count,
            total: file_count,
            status: "done".to_string(),
            message: format!(
                "分析完成: {} 张图片 → {} 个桶, 总 count {}, 有效 count {}, AR误差 {:.10}",
                total_images,
                buckets.len(),
                total_count,
                effective_count,
                mean_ar_error
            ),
        },
    );

    Ok(BucketAnalysis {
        total_images,
        total_count,
        effective_count,
        dropped_count,
        batch_count,
        short_batch_count,
        usable_rate,
        batch_size,
        drop_last,
        bucket_count: buckets.len() as u32,
        skipped,
        buckets,
        mean_ar_error,
        ar_error_metric: match ar_error_metric {
            ArErrorMetric::Linear => "linear",
            ArErrorMetric::Log => "log",
        }
        .to_string(),
    })
}

/// 生成不与已有文件冲突的复制目标路径（同名时追加 _1/_2 …）。
/// 分辨率聚合导出（resolution_analyze）复用此逻辑。
pub(crate) fn unique_copy_destination(dir: &Path, filename: &str) -> PathBuf {
    let mut dst = dir.join(filename);
    let mut counter = 1;
    while dst.exists() {
        let stem = Path::new(filename)
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy();
        let ext = Path::new(filename)
            .extension()
            .map(|e| format!(".{}", e.to_string_lossy()))
            .unwrap_or_default();
        dst = dir.join(format!("{}_{}{}", stem, counter, ext));
        counter += 1;
    }
    dst
}

fn dropped_images_for_bucket(bucket: &BucketGroup) -> Vec<(&BucketImageInfo, u32)> {
    let mut remaining = bucket.dropped_count;
    let mut items = Vec::new();

    for img in bucket.images.iter().rev() {
        if remaining == 0 {
            break;
        }
        let dropped_repeats = img.repeats.min(remaining);
        if dropped_repeats > 0 {
            items.push((img, dropped_repeats));
            remaining -= dropped_repeats;
        }
    }

    items.reverse();
    items
}

/// 导出分桶结果（将图片按桶复制到子文件夹）
///
/// 大量文件复制是同步 IO，放入 spawn_blocking 避免占用 tokio worker。
#[tauri::command]
pub async fn export_buckets(
    app: tauri::AppHandle,
    analysis: BucketAnalysis,
    output_path: String,
) -> Result<String, String> {
    tokio::task::spawn_blocking(move || export_buckets_sync(app, analysis, output_path))
        .await
        .map_err(|e| format!("导出任务执行失败: {}", e))?
}

fn export_buckets_sync(
    app: tauri::AppHandle,
    analysis: BucketAnalysis,
    output_path: String,
) -> Result<String, String> {
    let out_dir = Path::new(&output_path);
    if !out_dir.exists() {
        std::fs::create_dir_all(out_dir).map_err(|e| format!("创建输出目录失败: {}", e))?;
    }

    let total_files: u32 = analysis.buckets.iter().map(|b| b.image_count).sum();
    let total_dropped_files: u32 = analysis
        .buckets
        .iter()
        .map(|bucket| dropped_images_for_bucket(bucket).len() as u32)
        .sum();
    let total_copy_ops = total_files + total_dropped_files;
    let mut copied = 0u32;
    let mut dropped_copied = 0u32;
    let mut completed_ops = 0u32;

    for bucket in &analysis.buckets {
        let folder_name = format!(
            "Bucket {} - {}x{} (count {})",
            bucket.index, bucket.bucket_width, bucket.bucket_height, bucket.total_count
        );
        let bucket_dir = out_dir.join(&folder_name);
        std::fs::create_dir_all(&bucket_dir).map_err(|e| format!("创建桶目录失败: {}", e))?;

        for img in &bucket.images {
            let src = Path::new(&img.path);
            let dst = unique_copy_destination(&bucket_dir, &img.name);

            std::fs::copy(src, &dst).map_err(|e| format!("复制文件失败 {}: {}", img.name, e))?;

            copied += 1;
            completed_ops += 1;
            if completed_ops.is_multiple_of(20) || completed_ops == total_copy_ops {
                let _ = app.emit(
                    "bucket-export-progress",
                    ScanProgress {
                        current: completed_ops,
                        total: total_copy_ops,
                        status: if completed_ops == total_copy_ops {
                            "done"
                        } else {
                            "processing"
                        }
                        .to_string(),
                        message: format!("已导出 {}/{}", completed_ops, total_copy_ops),
                    },
                );
            }
        }
    }

    if total_dropped_files > 0 {
        let dropped_root = out_dir.join("Dropped Materials");
        std::fs::create_dir_all(&dropped_root)
            .map_err(|e| format!("创建丢弃素材目录失败: {}", e))?;

        for bucket in &analysis.buckets {
            let dropped_items = dropped_images_for_bucket(bucket);
            if dropped_items.is_empty() {
                continue;
            }

            let folder_name = format!(
                "Bucket {} - {}x{} (dropped {})",
                bucket.index, bucket.bucket_width, bucket.bucket_height, bucket.dropped_count
            );
            let dropped_bucket_dir = dropped_root.join(folder_name);
            std::fs::create_dir_all(&dropped_bucket_dir)
                .map_err(|e| format!("创建丢弃素材桶目录失败: {}", e))?;

            for (img, _dropped_repeats) in dropped_items {
                let src = Path::new(&img.path);
                let dst = unique_copy_destination(&dropped_bucket_dir, &img.name);
                std::fs::copy(src, &dst)
                    .map_err(|e| format!("复制丢弃素材失败 {}: {}", img.name, e))?;

                dropped_copied += 1;
                completed_ops += 1;
                if completed_ops.is_multiple_of(20) || completed_ops == total_copy_ops {
                    let _ = app.emit(
                        "bucket-export-progress",
                        ScanProgress {
                            current: completed_ops,
                            total: total_copy_ops,
                            status: if completed_ops == total_copy_ops {
                                "done"
                            } else {
                                "processing"
                            }
                            .to_string(),
                            message: format!("已导出 {}/{}", completed_ops, total_copy_ops),
                        },
                    );
                }
            }
        }
    }

    if dropped_copied > 0 {
        Ok(format!(
            "导出完成: {} 张图片已复制到 {} 个桶，另复制 {} 张丢弃素材到 Dropped Materials",
            copied, analysis.bucket_count, dropped_copied
        ))
    } else {
        Ok(format!(
            "导出完成: {} 张图片已复制到 {} 个桶",
            copied, analysis.bucket_count
        ))
    }
}
