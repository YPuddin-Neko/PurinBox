//! 缩略图生成与缓存
//!
//! 前端网格/预览直接经 asset 协议加载原图时，WebView 需解码全尺寸图片
//! （例如 8K 原图渲染进 100px 格子），大分辨率数据集下明显卡顿。
//! 此模块按最长边生成缩略图并缓存到系统缓存目录，命令返回缓存文件路径，
//! 前端仍经 asset 协议加载缩略图文件。
//!
//! 缓存键包含原图路径、mtime、文件大小与目标边长：原图被修改后键自然失效，
//! 旧条目由容量修剪回收，无需主动失效。

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

/// 缓存容量上限（条数）。超过后按 mtime 修剪到 PRUNE_TARGET。
const PRUNE_LIMIT: usize = 8000;
const PRUNE_TARGET: usize = 6000;

/// 修剪进行中标志：并发生成缩略图时只允许一个线程执行修剪
static PRUNING: AtomicBool = AtomicBool::new(false);

/// 缓存目录：{系统缓存目录}/PurinBox/thumbnails（取不到系统缓存目录时退回临时目录）
pub(crate) fn thumb_cache_dir() -> PathBuf {
    dirs::cache_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("PurinBox")
        .join("thumbnails")
}

/// 缓存键：规范路径 + mtime + 文件大小 + 目标边长 的 MD5（仅作缓存键，无安全用途）
fn cache_key(path: &Path, meta: &fs::Metadata, max_edge: u32) -> String {
    let canonical = path
        .canonicalize()
        .unwrap_or_else(|_| path.to_path_buf());
    let mtime = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let raw = format!(
        "{}|{}|{}|{}",
        canonical.to_string_lossy(),
        mtime,
        meta.len(),
        max_edge
    );
    format!("{:x}", md5::compute(raw.as_bytes()))
}

/// 获取图片缩略图（返回缩略图文件路径）
///
/// - 原图最长边不超过 max_edge 时直接返回原图路径，避免无谓的重编码
/// - 解码/编码是 CPU 密集操作，放入 spawn_blocking
#[tauri::command]
pub async fn get_image_thumbnail(path: String, max_edge: Option<u32>) -> Result<String, String> {
    let max_edge = max_edge.unwrap_or(384).clamp(64, 2048);
    tokio::task::spawn_blocking(move || get_image_thumbnail_sync(&path, max_edge))
        .await
        .map_err(|e| format!("缩略图任务执行失败: {}", e))?
}

fn get_image_thumbnail_sync(path: &str, max_edge: u32) -> Result<String, String> {
    let src = PathBuf::from(path);
    let meta = fs::metadata(&src).map_err(|e| format!("读取文件信息失败 {}: {}", path, e))?;

    // 无需完整解码即可拿到尺寸；小图直接用原图
    let reader = image::ImageReader::open(&src)
        .map_err(|e| format!("打开图片失败 {}: {}", path, e))?
        .with_guessed_format()
        .map_err(|e| format!("识别图片格式失败 {}: {}", path, e))?;
    let (w, h) = reader
        .into_dimensions()
        .map_err(|e| format!("读取图片尺寸失败 {}: {}", path, e))?;
    if w.max(h) <= max_edge {
        return Ok(path.to_string());
    }

    let dir = thumb_cache_dir();
    let key = cache_key(&src, &meta, max_edge);
    // 输出格式取决于是否带透明通道（解码后才知道），命中检查两种后缀
    let jpg_path = dir.join(format!("{}.jpg", key));
    if jpg_path.exists() {
        return Ok(jpg_path.to_string_lossy().to_string());
    }
    let png_path = dir.join(format!("{}.png", key));
    if png_path.exists() {
        return Ok(png_path.to_string_lossy().to_string());
    }

    fs::create_dir_all(&dir).map_err(|e| format!("创建缩略图缓存目录失败: {}", e))?;

    let img = image::ImageReader::open(&src)
        .map_err(|e| format!("打开图片失败 {}: {}", path, e))?
        .with_guessed_format()
        .map_err(|e| format!("识别图片格式失败 {}: {}", path, e))?
        .decode()
        .map_err(|e| format!("解码图片失败 {}: {}", path, e))?;

    let thumb = img.thumbnail(max_edge, max_edge);
    let has_alpha = thumb.color().has_alpha();
    let dest = if has_alpha { png_path } else { jpg_path };

    // 先写临时文件再改名，避免并发/中断产生半截缓存文件
    let tmp = dir.join(format!("{}.tmp-{}", key, std::process::id()));
    let write_result = (|| -> Result<(), String> {
        let file = fs::File::create(&tmp).map_err(|e| format!("创建缩略图失败: {}", e))?;
        let mut writer = std::io::BufWriter::new(file);
        if has_alpha {
            thumb
                .to_rgba8()
                .write_to(&mut writer, image::ImageFormat::Png)
                .map_err(|e| format!("编码缩略图失败: {}", e))?;
        } else {
            let encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut writer, 85);
            thumb
                .to_rgb8()
                .write_with_encoder(encoder)
                .map_err(|e| format!("编码缩略图失败: {}", e))?;
        }
        Ok(())
    })();
    if let Err(e) = write_result {
        let _ = fs::remove_file(&tmp);
        return Err(e);
    }

    if let Err(rename_err) = fs::rename(&tmp, &dest) {
        let _ = fs::remove_file(&tmp);
        // Windows 上并发生成同一缩略图时 rename 可能因目标已存在失败，视为命中
        if !dest.exists() {
            return Err(format!("写入缩略图失败: {}", rename_err));
        }
    }

    prune_cache_if_needed(&dir);
    Ok(dest.to_string_lossy().to_string())
}

/// 缓存超过上限时按 mtime 删除最旧的条目（仅一个线程执行，失败静默）
fn prune_cache_if_needed(dir: &Path) {
    if PRUNING
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        return;
    }
    let result = std::panic::catch_unwind(|| {
        let Ok(entries) = fs::read_dir(dir) else {
            return;
        };
        let mut files: Vec<(PathBuf, std::time::SystemTime)> = entries
            .flatten()
            .filter_map(|entry| {
                let path = entry.path();
                if !path.is_file() {
                    return None;
                }
                let mtime = entry.metadata().ok()?.modified().ok()?;
                Some((path, mtime))
            })
            .collect();
        if files.len() <= PRUNE_LIMIT {
            return;
        }
        files.sort_by_key(|(_, mtime)| *mtime);
        let remove_count = files.len().saturating_sub(PRUNE_TARGET);
        for (path, _) in files.into_iter().take(remove_count) {
            let _ = fs::remove_file(path);
        }
    });
    PRUNING.store(false, Ordering::SeqCst);
    if result.is_err() {
        // catch_unwind 仅为保证 PRUNING 标志一定被复位
    }
}
