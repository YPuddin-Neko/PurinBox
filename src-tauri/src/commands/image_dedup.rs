use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::Emitter;

use super::ProgressEvent;

static CANCEL_FLAG: AtomicBool = AtomicBool::new(false);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DedupOptions {
    pub folder_path: String,
    pub dhash_threshold: u32,
    pub phash_threshold: u32,
    pub color_threshold: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DupGroup {
    pub paths: Vec<String>,
    pub similarity: f64,
    pub method: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DedupResult {
    pub total_images: u32,
    pub duplicate_groups: Vec<DupGroup>,
    pub scan_time_ms: u64,
}

#[tauri::command]
pub async fn start_image_dedup(
    app: tauri::AppHandle,
    options: DedupOptions,
) -> Result<DedupResult, String> {
    CANCEL_FLAG.store(false, Ordering::SeqCst);
    tokio::task::spawn_blocking(move || dedup_sync(&app, &options))
        .await
        .map_err(|e| format!("任务执行失败: {}", e))?
}

#[tauri::command]
pub fn cancel_image_dedup() {
    CANCEL_FLAG.store(true, Ordering::SeqCst);
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeleteResult {
    pub deleted: u32,
    pub failed: u32,
    pub errors: Vec<String>,
}

#[tauri::command]
pub async fn delete_dedup_files(paths: Vec<String>) -> Result<DeleteResult, String> {
    let mut deleted = 0u32;
    let mut failed = 0u32;
    let mut errors = Vec::new();
    for p in &paths {
        match std::fs::remove_file(p) {
            Ok(_) => deleted += 1,
            Err(e) => {
                failed += 1;
                errors.push(format!("{}: {}", p, e));
            }
        }
    }
    Ok(DeleteResult { deleted, failed, errors })
}

// ── Hash types ──

struct ImageFingerprint {
    path: PathBuf,
    dhash: u64,
    phash: u64,
    color_hist: [f64; 48], // 16 bins × 3 channels, normalized
}

// ── Core logic ──

fn dedup_sync(app: &tauri::AppHandle, options: &DedupOptions) -> Result<DedupResult, String> {
    let start = std::time::Instant::now();
    let folder = Path::new(&options.folder_path);
    if !folder.exists() || !folder.is_dir() {
        return Err(format!("文件夹不存在: {}", options.folder_path));
    }

    let files = super::collect_image_files(folder)?;
    let total = files.len() as u32;

    if total == 0 {
        return Ok(DedupResult {
            total_images: 0,
            duplicate_groups: vec![],
            scan_time_ms: 0,
        });
    }

    // Phase 1: compute fingerprints (parallel)
    let _ = app.emit("dedup_progress", ProgressEvent {
        current: 0, total, filename: String::new(),
        status: "processing".into(),
        message: "正在计算图片指纹...".into(),
    });

    let num_threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).min(16);
    let counter = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0));
    let mut fingerprints: Vec<ImageFingerprint> = Vec::with_capacity(files.len());

    for chunk in files.chunks(num_threads) {
        if CANCEL_FLAG.load(Ordering::SeqCst) {
            return Err("已取消".into());
        }

        let handles: Vec<_> = chunk.iter().map(|file| {
            let path = file.clone();
            std::thread::spawn(move || compute_fingerprint(&path))
        }).collect();

        for handle in handles {
            let cnt = counter.fetch_add(1, Ordering::SeqCst) + 1;
            let _ = app.emit("dedup_progress", ProgressEvent {
                current: cnt, total,
                filename: String::new(),
                status: "processing".into(),
                message: format!("计算指纹 {}/{}", cnt, total),
            });

            if let Ok(Ok(fp)) = handle.join() {
                fingerprints.push(fp);
            }
        }
    }

    // Phase 2: find duplicates by comparing fingerprints
    let _ = app.emit("dedup_progress", ProgressEvent {
        current: total, total,
        filename: String::new(),
        status: "processing".into(),
        message: "正在比对图片...".into(),
    });

    let mut duplicate_groups: Vec<DupGroup> = Vec::new();
    let mut used: Vec<bool> = vec![false; fingerprints.len()];

    for i in 0..fingerprints.len() {
        if CANCEL_FLAG.load(Ordering::SeqCst) {
            return Err("已取消".into());
        }
        if used[i] {
            continue;
        }

        let mut group_paths = vec![fingerprints[i].path.to_string_lossy().to_string()];
        let mut best_sim = 0.0_f64;
        let mut best_method = String::new();

        for j in (i + 1)..fingerprints.len() {
            if used[j] {
                continue;
            }

            let (is_dup, sim, method) = is_duplicate(
                &fingerprints[i],
                &fingerprints[j],
                options.dhash_threshold,
                options.phash_threshold,
                options.color_threshold,
            );

            if is_dup {
                group_paths.push(fingerprints[j].path.to_string_lossy().to_string());
                used[j] = true;
                if sim > best_sim {
                    best_sim = sim;
                    best_method = method;
                }
            }
        }

        if group_paths.len() > 1 {
            used[i] = true;
            duplicate_groups.push(DupGroup {
                paths: group_paths,
                similarity: best_sim,
                method: best_method,
            });
        }
    }

    let elapsed = start.elapsed().as_millis() as u64;

    let _ = app.emit("dedup_progress", ProgressEvent {
        current: total, total,
        filename: String::new(),
        status: "done".into(),
        message: format!("完成，发现 {} 组重复", duplicate_groups.len()),
    });

    Ok(DedupResult {
        total_images: total,
        duplicate_groups,
        scan_time_ms: elapsed,
    })
}

fn compute_fingerprint(path: &Path) -> Result<ImageFingerprint, String> {
    let img = image::open(path).map_err(|e| e.to_string())?;
    let gray = img.to_luma8();
    let rgb = img.to_rgb8();

    let dhash = compute_dhash(&gray);
    let phash = compute_phash(&gray);
    let color_hist = compute_color_histogram(&rgb);

    Ok(ImageFingerprint {
        path: path.to_path_buf(),
        dhash,
        phash,
        color_hist,
    })
}

// ── dHash: difference hash (8x8 → 64-bit) ──

fn compute_dhash(gray: &image::GrayImage) -> u64 {
    let resized = image::imageops::resize(gray, 9, 8, image::imageops::FilterType::Lanczos3);
    let mut hash: u64 = 0;
    for y in 0..8 {
        for x in 0..8 {
            let left = resized.get_pixel(x, y)[0];
            let right = resized.get_pixel(x + 1, y)[0];
            if left > right {
                hash |= 1 << (y * 8 + x);
            }
        }
    }
    hash
}

// ── pHash: perceptual hash using separable DCT (32x32 → 64-bit) ──

fn compute_phash(gray: &image::GrayImage) -> u64 {
    let size = 32usize;
    let resized = image::imageops::resize(gray, size as u32, size as u32, image::imageops::FilterType::Lanczos3);
    let pi = std::f64::consts::PI;
    let n = size as f64;

    // Precompute cosine table for rows: cos((2x+1)*u*pi/(2N)) for u=0..7, x=0..31
    let mut cos_table = vec![0.0f64; 8 * size];
    for u in 0..8 {
        for x in 0..size {
            cos_table[u * size + x] = ((2.0 * x as f64 + 1.0) * u as f64 * pi / (2.0 * n)).cos();
        }
    }

    // Step 1: DCT on rows — only compute first 8 frequency components per row
    let mut row_dct = vec![0.0f64; size * 8]; // [y][u] for y=0..31, u=0..7
    for y in 0..size {
        for u in 0..8 {
            let mut sum = 0.0;
            for x in 0..size {
                sum += resized.get_pixel(x as u32, y as u32)[0] as f64 * cos_table[u * size + x];
            }
            row_dct[y * 8 + u] = sum;
        }
    }

    // Step 2: DCT on columns of the row_dct result — only first 8 rows
    // result[v][u] for v=0..7, u=0..7
    let mut dct_8x8 = [[0.0f64; 8]; 8];
    for v in 0..8 {
        for u in 0..8 {
            let mut sum = 0.0;
            for y in 0..size {
                sum += row_dct[y * 8 + u] * cos_table[v * size + y];
            }
            dct_8x8[v][u] = sum;
        }
    }

    // Collect low-frequency components, excluding DC
    let mut low_freq: Vec<f64> = Vec::with_capacity(63);
    for v in 0..8 {
        for u in 0..8 {
            if u == 0 && v == 0 { continue; }
            low_freq.push(dct_8x8[v][u]);
        }
    }

    // Median
    let mut sorted = low_freq.clone();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let median = sorted[sorted.len() / 2];

    // Build hash
    let mut hash: u64 = 0;
    for (i, val) in low_freq.iter().enumerate() {
        if *val > median {
            hash |= 1 << i;
        }
    }
    hash
}

// ── Color histogram (16 bins × 3 channels, normalized) ──

fn compute_color_histogram(rgb: &image::RgbImage) -> [f64; 48] {
    let mut hist = [0u64; 48]; // 16 bins × 3 channels
    let total_pixels = (rgb.width() * rgb.height()) as f64;

    for pixel in rgb.pixels() {
        let r_bin = (pixel[0] as usize) >> 4; // 0..15
        let g_bin = (pixel[1] as usize) >> 4;
        let b_bin = (pixel[2] as usize) >> 4;
        hist[r_bin] += 1;
        hist[16 + g_bin] += 1;
        hist[32 + b_bin] += 1;
    }

    let mut normalized = [0.0f64; 48];
    for i in 0..48 {
        normalized[i] = hist[i] as f64 / total_pixels;
    }
    normalized
}

// ── Hamming distance ──

fn hamming_distance(a: u64, b: u64) -> u32 {
    (a ^ b).count_ones()
}

// ── Color histogram similarity (Bhattacharyya coefficient) ──

fn color_similarity(a: &[f64; 48], b: &[f64; 48]) -> f64 {
    let mut bc = 0.0;
    for i in 0..48 {
        bc += (a[i] * b[i]).sqrt();
    }
    bc // 0.0 = completely different, 1.0 = identical
}

// ── Duplicate check: pass if ANY hash method detects similarity ──

fn is_duplicate(
    a: &ImageFingerprint,
    b: &ImageFingerprint,
    dhash_thresh: u32,
    phash_thresh: u32,
    color_thresh: f64,
) -> (bool, f64, String) {
    let dhash_dist = hamming_distance(a.dhash, b.dhash);
    let phash_dist = hamming_distance(a.phash, b.phash);
    let color_sim = color_similarity(&a.color_hist, &b.color_hist);

    // All three conditions must pass for a match
    let dhash_pass = dhash_dist <= dhash_thresh;
    let phash_pass = phash_dist <= phash_thresh;
    let color_pass = color_sim >= color_thresh;

    if dhash_pass && phash_pass && color_pass {
        let sim = color_sim;
        let method = format!(
            "dHash:{} pHash:{} color:{:.2}",
            dhash_dist, phash_dist, color_sim
        );
        (true, sim, method)
    } else {
        (false, 0.0, String::new())
    }
}
