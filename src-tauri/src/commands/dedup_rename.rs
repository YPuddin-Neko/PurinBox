use super::fingerprint::{compute_fingerprints, is_duplicate};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::Emitter;

use super::ProgressEvent;

static CANCEL_FLAG: AtomicBool = AtomicBool::new(false);

#[cfg(test)]
mod safety_tests {
    use super::*;

    #[tokio::test]
    async fn same_directory_export_never_truncates_sources() {
        let root = super::super::image_io::test_dir("export_self");
        std::fs::write(root.join("a.png"), b"source bytes").unwrap();
        for destination in [root.clone(), root.join(".")] {
            let result = export_unmatched_files(
                root.to_string_lossy().into_owned(),
                vec!["a.png".into()],
                destination.to_string_lossy().into_owned(),
            )
            .await;
            assert!(result.unwrap_err().contains("不能与源文件夹相同"));
            assert_eq!(std::fs::read(root.join("a.png")).unwrap(), b"source bytes");
        }
        let output = root.join("export");
        let result = export_unmatched_files(
            root.to_string_lossy().into_owned(),
            vec!["a.png".into()],
            output.to_string_lossy().into_owned(),
        )
        .await
        .unwrap();
        assert_eq!(result.success_count, 1);
        assert_eq!(
            std::fs::read(output.join("a.png")).unwrap(),
            b"source bytes"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn conflict_must_be_the_exact_destination() {
        let root = super::super::image_io::test_dir("rename_conflict");
        let app = tauri::test::mock_app();
        for name in ["source.png", "target.png", "unrelated.png"] {
            std::fs::write(root.join(name), name.as_bytes()).unwrap();
        }
        std::fs::write(root.join("target.caption"), b"caption").unwrap();
        let mut action = RenameAction {
            src_path: root.join("source.png").to_string_lossy().into_owned(),
            target_name: "target.png".into(),
            conflict_path: Some(root.join("unrelated.png").to_string_lossy().into_owned()),
        };
        let result = execute_dedup_rename(app.handle().clone(), vec![action.clone()])
            .await
            .unwrap();
        assert_eq!((result.success_count, result.fail_count), (0, 1));
        for name in ["source.png", "target.png", "unrelated.png"] {
            assert_eq!(std::fs::read(root.join(name)).unwrap(), name.as_bytes());
        }
        assert!(!root.join("unrelated_rename.png").exists());
        action.conflict_path = Some(root.join("target.png").to_string_lossy().into_owned());
        let result = execute_dedup_rename(app.handle().clone(), vec![action])
            .await
            .unwrap();
        assert_eq!((result.success_count, result.fail_count), (1, 0));
        assert_eq!(
            std::fs::read(root.join("target.png")).unwrap(),
            b"source.png"
        );
        assert_eq!(
            std::fs::read(root.join("target_rename.png")).unwrap(),
            b"target.png"
        );
        assert_eq!(
            std::fs::read(root.join("target_rename.caption")).unwrap(),
            b"caption"
        );
        std::fs::remove_dir_all(root).unwrap();
    }
}

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
    /// 目标名被占用时需先让位的文件：改名为 `{stem}_rename`（已占用时追加序号）后再执行重命名
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
        if super::path_key_ci(src) == super::path_key_ci(dst) {
            return Err("目标文件夹不能与源文件夹相同".to_string());
        }
        std::fs::create_dir_all(dst).map_err(|e| format!("创建目标文件夹失败: {}", e))?;
        if let (Ok(source), Ok(destination)) = (src.canonicalize(), dst.canonicalize()) {
            if source == destination {
                return Err("目标文件夹不能与源文件夹相同".to_string());
            }
        }
        let mut success_count = 0u32;
        let mut fail_count = 0u32;
        let mut errors = Vec::new();
        for name in &filenames {
            let src_path = src.join(name);
            let dst_path = dst.join(name);
            match super::copy_file_safe(&src_path, &dst_path) {
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
pub async fn execute_dedup_rename<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
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
            if let Some(conflict) = action
                .conflict_path
                .as_ref()
                .filter(|p| Path::new(p) == target_path)
            {
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
            ProgressEvent::new(
                "processing",
                format!("[{}/{}] {}", i + 1, total, action.target_name),
            )
            .at(i as u32 + 1, total)
            .file(action.target_name.clone()),
        );
    }

    let _ = app.emit(
        "dedup-rename-progress",
        ProgressEvent::new(
            "done",
            format!("完成: 成功 {}, 失败 {}", success_count, fail_count),
        )
        .at(total, total),
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

    for ext in super::TAG_SIDECAR_EXTS {
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

fn scan_sync(
    app: &tauri::AppHandle,
    options: &DedupRenameOptions,
) -> Result<DedupRenameScanResult, String> {
    let start = std::time::Instant::now();

    let folder_a = Path::new(&options.folder_a);
    let folder_b = Path::new(&options.folder_b);

    if !folder_a.is_dir() {
        return Err(format!("文件夹A不存在: {}", options.folder_a));
    }
    if !folder_b.is_dir() {
        return Err(format!("文件夹B不存在: {}", options.folder_b));
    }

    let files_a = super::collect_image_files_with_recursive(folder_a, false)?;
    let files_b = super::collect_image_files_with_recursive(folder_b, false)?;
    let total_a = files_a.len() as u32;
    let total_b = files_b.len() as u32;
    let total_all = total_a + total_b;

    if total_a == 0 || total_b == 0 {
        return Ok(DedupRenameScanResult {
            pairs: vec![],
            total_a,
            total_b,
            unmatched_a: files_a.iter().map(|f| super::file_name_lossy(f)).collect(),
            unmatched_b: files_b.iter().map(|f| super::file_name_lossy(f)).collect(),
            scan_time_ms: 0,
            failed_files: vec![],
        });
    }

    // Phase 1: compute fingerprints
    let _ = app.emit(
        "dedup-rename-progress",
        ProgressEvent::new("processing", "正在计算图片指纹...").at(0, total_all),
    );

    // A、B 两侧共用一个进度计数
    let mut done = 0u32;
    let mut on_each = || {
        done += 1;
        let _ = app.emit(
            "dedup-rename-progress",
            ProgressEvent::new("processing", format!("计算指纹 {}/{}", done, total_all))
                .at(done, total_all),
        );
    };
    let Some((fps_a, mut failed_files)) =
        compute_fingerprints(&files_a, &CANCEL_FLAG, &mut on_each)
    else {
        return Err("已取消".into());
    };
    if CANCEL_FLAG.load(Ordering::SeqCst) {
        return Err("已取消".into());
    }
    let Some((fps_b, failed_b)) = compute_fingerprints(&files_b, &CANCEL_FLAG, &mut on_each) else {
        return Err("已取消".into());
    };
    failed_files.extend(failed_b);
    if CANCEL_FLAG.load(Ordering::SeqCst) {
        return Err("已取消".into());
    }

    // Phase 2: cross-compare A vs B
    let _ = app.emit(
        "dedup-rename-progress",
        ProgressEvent::new("processing", "正在比对图片...").at(total_all, total_all),
    );

    let mut pairs: Vec<DedupPair> = Vec::new();
    let mut unmatched_a: Vec<String> = Vec::new();
    let mut used_b: Vec<bool> = vec![false; fps_b.len()];

    for fp_a in &fps_a {
        if CANCEL_FLAG.load(Ordering::SeqCst) {
            return Err("已取消".into());
        }

        let mut best_j: Option<usize> = None;
        let mut best_sim = 0.0_f64;

        for (j, fp_b) in fps_b.iter().enumerate() {
            if used_b[j] {
                continue;
            }

            let (is_dup, sim, _) = is_duplicate(
                fp_a,
                fp_b,
                options.dhash_threshold,
                options.phash_threshold,
                options.color_threshold,
            );

            if is_dup && sim > best_sim {
                best_sim = sim;
                best_j = Some(j);
            }
        }

        let name_a = super::file_name_lossy(&fp_a.path);
        if let Some(j) = best_j {
            used_b[j] = true;
            let name_b = super::file_name_lossy(&fps_b[j].path);
            pairs.push(DedupPair {
                path_a: fp_a.path.to_string_lossy().to_string(),
                name_a,
                path_b: fps_b[j].path.to_string_lossy().to_string(),
                name_b,
            });
        } else {
            unmatched_a.push(name_a);
        }
    }

    let unmatched_b: Vec<String> = fps_b
        .iter()
        .enumerate()
        .filter(|(j, _)| !used_b[*j])
        .map(|(_, fp)| super::file_name_lossy(&fp.path))
        .collect();

    let elapsed = start.elapsed().as_millis() as u64;

    let _ = app.emit(
        "dedup-rename-progress",
        ProgressEvent::new("done", format!("完成，找到 {} 对匹配", pairs.len()))
            .at(total_all, total_all),
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
