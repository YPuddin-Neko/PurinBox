use super::fingerprint::{compute_fingerprints, is_duplicate};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use super::batch::{BatchCounts, BatchJob, RunEvents};
use super::ProgressEvent;

static JOB: BatchJob = BatchJob::new("图片去重");

const EVENT: &str = "dedup_progress";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DedupOptions {
    pub folder_path: String,
    pub dhash_threshold: u32,
    pub phash_threshold: u32,
    pub color_threshold: f64,
    #[serde(default)]
    pub recursive: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DupGroup {
    pub paths: Vec<String>,
    pub method: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DedupResult {
    pub total_images: u32,
    pub duplicate_groups: Vec<DupGroup>,
    pub scan_time_ms: u64,
    /// 指纹计算失败的文件（路径 + 原因）
    pub failed_files: Vec<String>,
}

#[tauri::command]
pub async fn start_image_dedup<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    options: DedupOptions,
) -> Result<DedupResult, String> {
    JOB.run(move || dedup_sync(&app, &options, JOB.cancel_flag()))
        .await
}

#[tauri::command]
pub fn cancel_image_dedup() {
    JOB.cancel();
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
    Ok(DeleteResult {
        deleted,
        failed,
        errors,
    })
}

// ── Core logic ──

/// 取消时发带 `cancelled` 的终态 done，返回「已取消」
fn dedup_sync<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    options: &DedupOptions,
    cancel: &AtomicBool,
) -> Result<DedupResult, String> {
    let start = std::time::Instant::now();
    let folder = Path::new(&options.folder_path);
    if !folder.is_dir() {
        return Err(format!("文件夹不存在: {}", options.folder_path));
    }

    let files = super::collect_image_files_with_recursive(folder, options.recursive)?;
    let total = files.len() as u32;

    if total == 0 {
        return Ok(DedupResult {
            total_images: 0,
            duplicate_groups: vec![],
            scan_time_ms: 0,
            failed_files: vec![],
        });
    }

    let run = RunEvents::begin(app, EVENT);
    // Phase 1: compute fingerprints (parallel)
    run.emit(ProgressEvent::new("processing", "正在计算图片指纹...").at(0, total));

    let mut done = 0u32;
    let on_each = || {
        done += 1;
        run.emit(
            ProgressEvent::new("processing", format!("计算指纹 {}/{}", done, total))
                .at(done, total),
        );
    };
    let batch = compute_fingerprints(&files, cancel, on_each);
    let counts = BatchCounts {
        success: batch.fingerprints.len() as u32,
        failed: batch.failed.len() as u32,
        total,
        ..Default::default()
    };
    if batch.cancelled {
        return run.finish_cancelled(&counts);
    }
    let fingerprints = batch.fingerprints;

    // Phase 2: find duplicates by comparing fingerprints
    run.emit(ProgressEvent::new("processing", "正在比对图片...").at(total, total));

    let mut duplicate_groups: Vec<DupGroup> = Vec::new();
    let mut used: Vec<bool> = vec![false; fingerprints.len()];

    for i in 0..fingerprints.len() {
        if cancel.load(Ordering::SeqCst) {
            return run.finish_cancelled(&counts);
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
                method: best_method,
            });
        }
    }

    let elapsed = start.elapsed().as_millis() as u64;

    run.finish(&counts, false, |_| {
        format!("完成，发现 {} 组重复", duplicate_groups.len())
    });

    Ok(DedupResult {
        total_images: total,
        duplicate_groups,
        scan_time_ms: elapsed,
        failed_files: batch.failed,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::batch::capture_raw_events;
    use crate::commands::test_support::TempDir;

    fn dataset(tag: &str) -> TempDir {
        let root = TempDir::new(tag);
        for (name, shade) in [("a.png", 10u8), ("b.png", 10), ("c.png", 250)] {
            image::RgbImage::from_fn(32, 32, |x, _| {
                image::Rgb([shade, (x * 8) as u8, shade.wrapping_add(x as u8)])
            })
            .save(root.join(name))
            .unwrap();
        }
        root
    }

    fn options(root: &Path) -> DedupOptions {
        DedupOptions {
            folder_path: root.to_string_lossy().into_owned(),
            dhash_threshold: 5,
            phash_threshold: 5,
            color_threshold: 0.9,
            recursive: false,
        }
    }

    #[test]
    fn finds_duplicates_and_stamps_one_run() {
        let root = dataset("dedup_run");
        let app = tauri::test::mock_app();
        let log = capture_raw_events(app.handle(), EVENT);
        let result = dedup_sync(app.handle(), &options(&root), &AtomicBool::new(false)).unwrap();
        assert_eq!(result.total_images, 3);
        assert_eq!(result.duplicate_groups.len(), 1);
        assert_eq!(result.duplicate_groups[0].paths.len(), 2);
        let events = log.lock().unwrap();
        let run_id = events[0]["run_id"].as_u64().unwrap();
        assert!(events.iter().all(|e| e["run_id"] == run_id));
        let done = events.last().unwrap();
        assert_eq!(done["status"], "done");
        assert_eq!(done["message"], "完成，发现 1 组重复");
        assert!(done.get("cancelled").is_none());
    }

    /// 取消：终态 done 带 cancelled、用统一取消文案，返回以「已取消」开头的错误
    #[test]
    fn cancelled_run_ends_with_a_cancelled_done() {
        let root = dataset("dedup_cancel");
        let app = tauri::test::mock_app();
        let log = capture_raw_events(app.handle(), EVENT);
        let err = dedup_sync(app.handle(), &options(&root), &AtomicBool::new(true)).unwrap_err();
        assert!(err.starts_with("已取消"), "{}", err);
        let events = log.lock().unwrap();
        let done = events.last().unwrap();
        assert_eq!(done["status"], "done");
        assert_eq!(done["cancelled"], true);
        assert_eq!(done["message"], "已取消: 已处理 0/3, 成功 0, 失败 0");
        assert_eq!(events.iter().filter(|e| e["status"] == "done").count(), 1);
    }
}
