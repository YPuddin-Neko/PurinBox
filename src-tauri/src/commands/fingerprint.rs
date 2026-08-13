//! 感知指纹：dHash / pHash / 颜色直方图
//!
//! 图片去重（image_dedup）与去重重命名（dedup_rename）共用的指纹计算与比较逻辑。

use std::path::{Path, PathBuf};

pub struct ImageFingerprint {
    pub path: PathBuf,
    pub dhash: u64,
    pub phash: u64,
    pub color_hist: [f64; 48], // 16 bins × 3 channels, normalized
}

pub fn compute_fingerprint(path: &Path) -> Result<ImageFingerprint, String> {
    let img = image::ImageReader::open(path)
        .map_err(|e| e.to_string())?
        .with_guessed_format()
        .map_err(|e| e.to_string())?
        .decode()
        .map_err(|e| e.to_string())?;
    let gray = img.to_luma8();
    let rgb = img.to_rgb8();

    Ok(ImageFingerprint {
        path: path.to_path_buf(),
        dhash: compute_dhash(&gray),
        phash: compute_phash(&gray),
        color_hist: compute_color_histogram(&rgb),
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

#[allow(clippy::needless_range_loop)]
fn compute_phash(gray: &image::GrayImage) -> u64 {
    let size = 32usize;
    let resized = image::imageops::resize(
        gray,
        size as u32,
        size as u32,
        image::imageops::FilterType::Lanczos3,
    );
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
            if u == 0 && v == 0 {
                continue;
            }
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

// ── Color histogram (16 bins × RGB) ──

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

// ── Duplicate check: all three metrics must pass ──

pub fn is_duplicate(
    a: &ImageFingerprint,
    b: &ImageFingerprint,
    dhash_thresh: u32,
    phash_thresh: u32,
    color_thresh: f64,
) -> (bool, f64, String) {
    let dhash_dist = hamming_distance(a.dhash, b.dhash);
    let phash_dist = hamming_distance(a.phash, b.phash);
    let color_sim = color_similarity(&a.color_hist, &b.color_hist);

    if dhash_dist <= dhash_thresh && phash_dist <= phash_thresh && color_sim >= color_thresh {
        let method = format!(
            "dHash:{} pHash:{} color:{:.2}",
            dhash_dist, phash_dist, color_sim
        );
        (true, color_sim, method)
    } else {
        (false, 0.0, String::new())
    }
}
