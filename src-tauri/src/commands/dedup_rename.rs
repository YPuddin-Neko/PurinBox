use serde::{Deserialize, Serialize};
use super::fingerprint::{compute_fingerprint, is_duplicate, ImageFingerprint};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::Emitter;

use super::ProgressEvent;

static CANCEL_FLAG: AtomicBool = AtomicBool::new(false);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DedupRenameOptions {
    pub folder_a: String,
    pub folder_b: String,
    pub dhash_threshold: u32,
    pub phash_threshold: u32,
    pub color_threshold: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DedupPair {
    pub path_a: String,
    pub name_a: String,
    pub path_b: String,
    pub name_b: String,
    pub similarity: f64,
    pub method: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DedupRenameScanResult {
    pub pairs: Vec<DedupPair>,
    pub total_a: u32,
    pub total_b: u32,
    pub unmatched_a: Vec<String>,
    pub unmatched_b: Vec<String>,
    pub scan_time_ms: u64,
    /// 指纹计算失败的文件（路径 + 原因）
    pub failed_files: Vec<String>,
}

#[tauri::command]
pub async fn scan_dedup_rename(
    app: tauri::AppHandle,
    options: DedupRenameOptions,
) -> Result<DedupRenameScanResult, String> {
    CANCEL_FLAG.store(false, Ordering::SeqCst);
    tokio::task::spawn_blocking(move || scan_sync(&app, &options))
        .await
        .map_err(|e| format!("任务执行失败: {}", e))?
}

#[tauri::command]
pub fn cancel_dedup_rename() {
    CANCEL_FLAG.store(true, Ordering::SeqCst);
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RenameAction {
    /// 源文件路径（要被重命名的文件）
    pub src_path: String,
    /// 目标文件名（不含路径，只是文件名）
    pub target_name: String,
    /// 如果目标位置已有同名文件，是否给旧文件加 _rename 后缀
    pub conflict_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DedupRenameResult {
    pub success_count: u32,
    pub fail_count: u32,
    pub errors: Vec<String>,
}

/// 导出未匹配文件到目标文件夹
#[tauri::command]
pub async fn export_unmatched_files(
    source_folder: String,
    filenames: Vec<String>,
    dest_folder: String,
) -> Result<DedupRenameResult, String> {
    tokio::task::spawn_blocking(move || {
        let src = Path::new(&source_folder);
        let dst = Path::new(&dest_folder);
        if !dst.exists() {
            std::fs::create_dir_all(dst).map_err(|e| format!("创建目标文件夹失败: {}", e))?;
        }
        let mut success_count = 0u32;
        let mut fail_count = 0u32;
        let mut errors = Vec::new();
        for name in &filenames {
            let src_path = src.join(name);
            let dst_path = dst.join(name);
            match std::fs::copy(&src_path, &dst_path) {
                Ok(_) => success_count += 1,
                Err(e) => {
                    fail_count += 1;
                    errors.push(format!("{}: {}", name, e));
                }
            }
        }
        Ok(DedupRenameResult {
            success_count,
            fail_count,
            errors,
        })
    })
    .await
    .map_err(|e| format!("任务执行失败: {}", e))?
}

#[tauri::command]
pub async fn execute_dedup_rename(
    app: tauri::AppHandle,
    actions: Vec<RenameAction>,
) -> Result<DedupRenameResult, String> {
    let total = actions.len() as u32;
    let mut success_count = 0u32;
    let mut fail_count = 0u32;
    let mut errors = Vec::new();

    for (i, action) in actions.iter().enumerate() {
        let src = Path::new(&action.src_path);
        if !src.exists() {
            fail_count += 1;
            errors.push(format!("源文件不存在: {}", action.src_path));
            continue;
        }

        let src_dir = src.parent().unwrap_or(Path::new("."));
        let target_path = src_dir.join(&action.target_name);

        // 如果目标位置已有文件（冲突），先给它加 _rename 后缀
        if target_path.exists() && target_path != src {
            if let Some(conflict) = &action.conflict_path {
                let conflict_p = Path::new(conflict);
                // 给冲突文件加 _rename 后缀，若该名已被占用则追加序号直到唯一
                let stem = conflict_p
                    .file_stem()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string();
                let ext = conflict_p
                    .extension()
                    .map(|e| format!(".{}", e.to_string_lossy()))
                    .unwrap_or_default();
                let parent = conflict_p.parent().unwrap_or(Path::new("."));
                let mut rename_path = parent.join(format!("{}_rename{}", stem, ext));
                let mut suffix = 1u32;
                while rename_path.exists() {
                    rename_path = parent.join(format!("{}_rename_{}{}", stem, suffix, ext));
                    suffix += 1;
                }

                if let Err(e) = std::fs::rename(conflict_p, &rename_path) {
                    fail_count += 1;
                    errors.push(format!("重命名冲突文件失败 {}: {}", conflict, e));
                    continue;
                }

                // 同时处理关联的标签文件
                errors.extend(rename_associated_files(conflict_p, &rename_path));
            }

            // 后端自查：冲突未被避让（前端未传 conflict_path 或避让后目标仍存在），拒绝覆盖
            if target_path.exists() {
                fail_count += 1;
                errors.push(format!(
                    "目标文件已存在，跳过以免覆盖: {}",
                    target_path.display()
                ));
                continue;
            }
        }

        // 执行重命名
        match std::fs::rename(src, &target_path) {
            Ok(_) => {
                // 同时处理关联的标签文件
                errors.extend(rename_associated_files(src, &target_path));
                success_count += 1;
            }
            Err(e) => {
                fail_count += 1;
                errors.push(format!(
                    "{} → {}: {}",
                    action.src_path, action.target_name, e
                ));
            }
        }

        let _ = app.emit(
            "dedup-rename-progress",
            ProgressEvent {
                current: i as u32 + 1,
                total,
                filename: action.target_name.clone(),
                status: "processing".to_string(),
                message: format!("[{}/{}] {}", i + 1, total, action.target_name),
                ..Default::default()
            },
        );
    }

    let _ = app.emit(
        "dedup-rename-progress",
        ProgressEvent {
            current: total,
            total,
            filename: String::new(),
            status: "done".to_string(),
            message: format!("完成: 成功 {}, 失败 {}", success_count, fail_count),
            ..Default::default()
        },
    );

    Ok(DedupRenameResult {
        success_count,
        fail_count,
        errors,
    })
}

/// 重命名关联文件（.txt, .json, .caption），返回错误信息列表（目标已存在则跳过并记录，绝不覆盖）
fn rename_associated_files(old_path: &Path, new_path: &Path) -> Vec<String> {
    let mut errors = Vec::new();
    let old_stem = old_path.file_stem().unwrap_or_default();
    let new_stem = new_path.file_stem().unwrap_or_default();
    let old_dir = old_path.parent().unwrap_or(Path::new("."));
    let new_dir = new_path.parent().unwrap_or(Path::new("."));

    for ext in &["txt", "json", "caption"] {
        let old_assoc = old_dir.join(format!("{}.{}", old_stem.to_string_lossy(), ext));
        if old_assoc.exists() {
            let new_assoc = new_dir.join(format!("{}.{}", new_stem.to_string_lossy(), ext));
            if new_assoc.exists() {
                errors.push(format!(
                    "关联文件目标已存在，跳过以免覆盖: {} → {}",
                    old_assoc.display(),
                    new_assoc.display()
                ));
                continue;
            }
            if let Err(e) = std::fs::rename(&old_assoc, &new_assoc) {
                errors.push(format!("关联文件重命名失败 {}: {}", old_assoc.display(), e));
            }
        }
    }
    errors
}

// ── Fingerprint (reuse dedup logic) ──


fn scan_sync(
    app: &tauri::AppHandle,
    options: &DedupRenameOptions,
) -> Result<DedupRenameScanResult, String> {
    let start = std::time::Instant::now();

    let folder_a = Path::new(&options.folder_a);
    let folder_b = Path::new(&options.folder_b);

    if !folder_a.exists() || !folder_a.is_dir() {
        return Err(format!("文件夹A不存在: {}", options.folder_a));
    }
    if !folder_b.exists() || !folder_b.is_dir() {
        return Err(format!("文件夹B不存在: {}", options.folder_b));
    }

    let files_a = super::collect_image_files(folder_a)?;
    let files_b = super::collect_image_files(folder_b)?;
    let total_a = files_a.len() as u32;
    let total_b = files_b.len() as u32;
    let total_all = total_a + total_b;

    if total_a == 0 || total_b == 0 {
        return Ok(DedupRenameScanResult {
            pairs: vec![],
            total_a,
            total_b,
            unmatched_a: files_a
                .iter()
                .map(|f| {
                    f.file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .to_string()
                })
                .collect(),
            unmatched_b: files_b
                .iter()
                .map(|f| {
                    f.file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .to_string()
                })
                .collect(),
            scan_time_ms: 0,
            failed_files: vec![],
        });
    }

    // Phase 1: compute fingerprints
    let _ = app.emit(
        "dedup-rename-progress",
        ProgressEvent {
            current: 0,
            total: total_all,
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

    let compute_batch = |files: &[PathBuf],
                         app: &tauri::AppHandle,
                         counter: &std::sync::Arc<std::sync::atomic::AtomicU32>,
                         total: u32|
     -> (Vec<ImageFingerprint>, Vec<String>) {
        let mut fps = Vec::with_capacity(files.len());
        let mut failed: Vec<String> = Vec::new();
        for chunk in files.chunks(num_threads) {
            if CANCEL_FLAG.load(Ordering::SeqCst) {
                break;
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
                    "dedup-rename-progress",
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
                    Ok(Ok(fp)) => fps.push(fp),
                    Ok(Err(e)) => failed.push(format!("{}: {}", file.display(), e)),
                    Err(_) => failed.push(format!("{}: 指纹计算线程异常退出", file.display())),
                }
            }
        }
        (fps, failed)
    };

    let (fps_a, mut failed_files) = compute_batch(&files_a, app, &counter, total_all);
    if CANCEL_FLAG.load(Ordering::SeqCst) {
        return Err("已取消".into());
    }
    let (fps_b, failed_b) = compute_batch(&files_b, app, &counter, total_all);
    failed_files.extend(failed_b);
    if CANCEL_FLAG.load(Ordering::SeqCst) {
        return Err("已取消".into());
    }

    // Phase 2: cross-compare A vs B
    let _ = app.emit(
        "dedup-rename-progress",
        ProgressEvent {
            current: total_all,
            total: total_all,
            filename: String::new(),
            status: "processing".into(),
            message: "正在比对图片...".into(),
            ..Default::default()
        },
    );

    let mut pairs: Vec<DedupPair> = Vec::new();
    let mut used_b: Vec<bool> = vec![false; fps_b.len()];

    for fp_a in &fps_a {
        if CANCEL_FLAG.load(Ordering::SeqCst) {
            return Err("已取消".into());
        }

        let mut best_j: Option<usize> = None;
        let mut best_sim = 0.0_f64;
        let mut best_method = String::new();

        for (j, fp_b) in fps_b.iter().enumerate() {
            if used_b[j] {
                continue;
            }

            let (is_dup, sim, method) = is_duplicate(
                fp_a,
                fp_b,
                options.dhash_threshold,
                options.phash_threshold,
                options.color_threshold,
            );

            if is_dup && sim > best_sim {
                best_sim = sim;
                best_method = method;
                best_j = Some(j);
            }
        }

        if let Some(j) = best_j {
            used_b[j] = true;
            let name_a = fp_a
                .path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();
            let name_b = fps_b[j]
                .path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();
            pairs.push(DedupPair {
                path_a: fp_a.path.to_string_lossy().to_string(),
                name_a,
                path_b: fps_b[j].path.to_string_lossy().to_string(),
                name_b,
                similarity: best_sim,
                method: best_method,
            });
        }
    }

    // Collect unmatched
    let matched_a_paths: std::collections::HashSet<String> =
        pairs.iter().map(|p| p.path_a.clone()).collect();
    let unmatched_a: Vec<String> = fps_a
        .iter()
        .filter(|fp| !matched_a_paths.contains(&fp.path.to_string_lossy().to_string()))
        .map(|fp| {
            fp.path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string()
        })
        .collect();
    let unmatched_b: Vec<String> = fps_b
        .iter()
        .enumerate()
        .filter(|(j, _)| !used_b[*j])
        .map(|(_, fp)| {
            fp.path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string()
        })
        .collect();

    let elapsed = start.elapsed().as_millis() as u64;

    let _ = app.emit(
        "dedup-rename-progress",
        ProgressEvent {
            current: total_all,
            total: total_all,
            filename: String::new(),
            status: "done".into(),
            message: format!("完成，找到 {} 对匹配", pairs.len()),
            ..Default::default()
        },
    );

    Ok(DedupRenameScanResult {
        pairs,
        total_a,
        total_b,
        unmatched_a,
        unmatched_b,
        scan_time_ms: elapsed,
        failed_files,
    })
}







