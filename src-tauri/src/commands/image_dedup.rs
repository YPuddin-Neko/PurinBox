use serde::{Deserialize, Serialize};
use super::fingerprint::{compute_fingerprint, is_duplicate, ImageFingerprint};
use std::path::Path;
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
    #[serde(default)]
    pub recursive: bool,
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
    /// 指纹计算失败的文件（路径 + 原因）
    pub failed_files: Vec<String>,
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
    Ok(DeleteResult {
        deleted,
        failed,
        errors,
    })
}

// ── Hash types ──


// ── Core logic ──

fn dedup_sync(app: &tauri::AppHandle, options: &DedupOptions) -> Result<DedupResult, String> {
    let start = std::time::Instant::now();
    let folder = Path::new(&options.folder_path);
    if !folder.exists() || !folder.is_dir() {
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

    // Phase 1: compute fingerprints (parallel)
    let _ = app.emit(
        "dedup_progress",
        ProgressEvent {
            current: 0,
            total,
            filename: String::new(),
            status: "processing".into(),
            message: "正在计算图片指纹...".into(),
            ..Default::default()
        },
    );

    let num_threads = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
        .min(16);
    let counter = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0));
    let mut fingerprints: Vec<ImageFingerprint> = Vec::with_capacity(files.len());
    let mut failed_files: Vec<String> = Vec::new();

    for chunk in files.chunks(num_threads) {
        if CANCEL_FLAG.load(Ordering::SeqCst) {
            return Err("已取消".into());
        }

        let handles: Vec<_> = chunk
            .iter()
            .map(|file| {
                let path = file.clone();
                std::thread::spawn(move || compute_fingerprint(&path))
            })
            .collect();

        for (file, handle) in chunk.iter().zip(handles) {
            let cnt = counter.fetch_add(1, Ordering::SeqCst) + 1;
            let _ = app.emit(
                "dedup_progress",
                ProgressEvent {
                    current: cnt,
                    total,
                    filename: String::new(),
                    status: "processing".into(),
                    message: format!("计算指纹 {}/{}", cnt, total),
                    ..Default::default()
                },
            );

            match handle.join() {
                Ok(Ok(fp)) => fingerprints.push(fp),
                Ok(Err(e)) => failed_files.push(format!("{}: {}", file.display(), e)),
                Err(_) => failed_files.push(format!("{}: 指纹计算线程异常退出", file.display())),
            }
        }
    }

    // Phase 2: find duplicates by comparing fingerprints
    let _ = app.emit(
        "dedup_progress",
        ProgressEvent {
            current: total,
            total,
            filename: String::new(),
            status: "processing".into(),
            message: "正在比对图片...".into(),
            ..Default::default()
        },
    );

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

    let _ = app.emit(
        "dedup_progress",
        ProgressEvent {
            current: total,
            total,
            filename: String::new(),
            status: "done".into(),
            message: format!("完成，发现 {} 组重复", duplicate_groups.len()),
            ..Default::default()
        },
    );

    Ok(DedupResult {
        total_images: total,
        duplicate_groups,
        scan_time_ms: elapsed,
        failed_files,
    })
}







