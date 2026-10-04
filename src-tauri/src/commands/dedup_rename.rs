use super::fingerprint::{compute_fingerprints, is_duplicate};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use super::batch::{BatchCounts, BatchJob, RunEvents};
use super::{same_path, tag_sidecars, unique_destination, NameSuffix, ProgressEvent};

static SCAN_JOB: BatchJob = BatchJob::new("查重扫描");

const EVENT: &str = "dedup-rename-progress";

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
pub async fn scan_dedup_rename<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    options: DedupRenameOptions,
) -> Result<DedupRenameScanResult, String> {
    SCAN_JOB
        .run(move || scan_sync(&app, &options, SCAN_JOB.cancel_flag()))
        .await
}

#[tauri::command]
pub fn cancel_dedup_rename() {
    SCAN_JOB.cancel();
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

/// 导出未匹配文件到目标文件夹。目标里已有同名文件时不覆盖，记为失败
#[tauri::command]
pub async fn export_unmatched_files(
    source_folder: String,
    filenames: Vec<String>,
    dest_folder: String,
) -> Result<DedupRenameResult, String> {
    tokio::task::spawn_blocking(move || {
        let src = Path::new(&source_folder);
        let dst = Path::new(&dest_folder);
        // 先比再建：拒绝时不留空目录（还不存在的目标文件夹也不会是源文件夹）
        if same_path(src, dst) {
            return Err("目标文件夹不能与源文件夹相同".to_string());
        }
        std::fs::create_dir_all(dst).map_err(|e| format!("创建目标文件夹失败: {}", e))?;
        let mut success_count = 0u32;
        let mut fail_count = 0u32;
        let mut errors = Vec::new();
        for name in &filenames {
            let src_path = src.join(name);
            let dst_path = dst.join(name);
            let copied = if dst_path.exists() {
                Err("目标已存在".to_string())
            } else {
                super::copy_file_safe(&src_path, &dst_path)
            };
            match copied {
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
    tokio::task::spawn_blocking(move || execute_sync(&app, &actions))
        .await
        .map_err(|e| format!("任务执行失败: {}", e))
}

fn execute_sync<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    actions: &[RenameAction],
) -> DedupRenameResult {
    let run = RunEvents::begin(app, EVENT);
    let total = actions.len() as u32;
    let mut counts = BatchCounts {
        total,
        ..Default::default()
    };
    let mut errors = Vec::new();

    for (i, action) in actions.iter().enumerate() {
        match plan_moves(action).and_then(|moves| apply_moves(&moves)) {
            Ok(()) => counts.success += 1,
            Err(e) => {
                counts.failed += 1;
                errors.push(e);
            }
        }
        run.emit(
            ProgressEvent::new(
                "processing",
                format!("[{}/{}] {}", i + 1, total, action.target_name),
            )
            .at(i as u32 + 1, total)
            .file(action.target_name.clone()),
        );
    }

    run.finish(&counts, false, |c| {
        format!("完成: 成功 {}, 失败 {}", c.success, c.failed)
    });

    DedupRenameResult {
        success_count: counts.success,
        fail_count: counts.failed,
        errors,
    }
}

/// 存在的同名标签文件 `(扩展名, 路径)`
fn existing_sidecars(path: &Path) -> Vec<(&'static str, PathBuf)> {
    tag_sidecars(path)
        .filter(|(_, sidecar)| sidecar.is_file())
        .collect()
}

/// 一个动作要做的全部改名（按执行顺序）：目标名被占用时先让占着的文件连同标签文件改成
/// `{stem}_rename`（已占用时追加序号），再把源文件连同标签文件改成目标名。
/// 执行前检查完所有目标：任何一个标签文件的目标被别的文件占着，整个动作失败、一个文件都不动——
/// 只改了图片而标签被挡住，标签就会和别的图片对上
fn plan_moves(action: &RenameAction) -> Result<Vec<(PathBuf, PathBuf)>, String> {
    let src = Path::new(&action.src_path);
    if !src.exists() {
        return Err(format!("源文件不存在: {}", action.src_path));
    }
    let src_dir = src.parent().unwrap_or(Path::new("."));
    let target_path = src_dir.join(&action.target_name);
    if target_path == src {
        return Ok(Vec::new());
    }

    let mut moves = Vec::new();
    if target_path.exists() {
        // 只为前端指明的、恰好占着目标名的文件让位，否则拒绝覆盖
        let Some(conflict) = action
            .conflict_path
            .as_deref()
            .map(Path::new)
            .filter(|p| *p == target_path)
        else {
            return Err(format!(
                "目标文件已存在，跳过以免覆盖: {}",
                target_path.display()
            ));
        };
        let sidecars = existing_sidecars(conflict);
        let parent = conflict.parent().unwrap_or(Path::new("."));
        let yielded = unique_destination(
            parent,
            &super::file_name_lossy(conflict),
            NameSuffix::Rename,
            |candidate| {
                candidate.exists()
                    || sidecars
                        .iter()
                        .any(|(ext, _)| candidate.with_extension(ext).exists())
            },
        );
        moves.push((conflict.to_path_buf(), yielded.clone()));
        for (ext, sidecar) in sidecars {
            moves.push((sidecar, yielded.with_extension(ext)));
        }
    }

    let vacated: Vec<PathBuf> = moves.iter().map(|(from, _)| from.clone()).collect();
    moves.push((src.to_path_buf(), target_path.clone()));
    for (ext, sidecar) in existing_sidecars(src) {
        let dest = target_path.with_extension(ext);
        if dest == sidecar {
            continue;
        }
        if dest.exists() && !vacated.contains(&dest) {
            return Err(format!(
                "标签文件的目标已存在，未改名: {} → {}",
                sidecar.display(),
                dest.display()
            ));
        }
        moves.push((sidecar, dest));
    }
    Ok(moves)
}

/// 依次改名；中途失败时把已改的按相反顺序改回去
fn apply_moves(moves: &[(PathBuf, PathBuf)]) -> Result<(), String> {
    for (done, (from, to)) in moves.iter().enumerate() {
        if let Err(e) = std::fs::rename(from, to) {
            let mut message = format!("{} → {}: {}", from.display(), to.display(), e);
            let rollback_failed: Vec<String> = moves[..done]
                .iter()
                .rev()
                .filter_map(|(from, to)| {
                    std::fs::rename(to, from)
                        .err()
                        .map(|e| format!("{} → {}: {}", to.display(), from.display(), e))
                })
                .collect();
            if !rollback_failed.is_empty() {
                message.push_str(&format!("（撤回失败: {}）", rollback_failed.join("; ")));
            }
            return Err(message);
        }
    }
    Ok(())
}

/// 取消时发带 `cancelled` 的终态 done，返回「已取消」
fn scan_sync<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    options: &DedupRenameOptions,
    cancel: &AtomicBool,
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

    let run = RunEvents::begin(app, EVENT);
    // Phase 1: compute fingerprints
    run.emit(ProgressEvent::new("processing", "正在计算图片指纹...").at(0, total_all));

    // A、B 两侧共用一个进度计数
    let mut done = 0u32;
    let mut on_each = || {
        done += 1;
        run.emit(
            ProgressEvent::new("processing", format!("计算指纹 {}/{}", done, total_all))
                .at(done, total_all),
        );
    };
    let batch_a = compute_fingerprints(&files_a, cancel, &mut on_each);
    let batch_b = if batch_a.cancelled {
        Default::default()
    } else {
        compute_fingerprints(&files_b, cancel, &mut on_each)
    };
    let counts = BatchCounts {
        success: (batch_a.fingerprints.len() + batch_b.fingerprints.len()) as u32,
        failed: (batch_a.failed.len() + batch_b.failed.len()) as u32,
        total: total_all,
        ..Default::default()
    };
    if cancel.load(Ordering::SeqCst) {
        return run.finish_cancelled(&counts);
    }
    let (fps_a, fps_b) = (batch_a.fingerprints, batch_b.fingerprints);
    let mut failed_files = batch_a.failed;
    failed_files.extend(batch_b.failed);

    // Phase 2: cross-compare A vs B
    run.emit(ProgressEvent::new("processing", "正在比对图片...").at(total_all, total_all));

    let mut pairs: Vec<DedupPair> = Vec::new();
    let mut unmatched_a: Vec<String> = Vec::new();
    let mut used_b: Vec<bool> = vec![false; fps_b.len()];

    for fp_a in &fps_a {
        if cancel.load(Ordering::SeqCst) {
            return run.finish_cancelled(&counts);
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

    run.finish(&counts, false, |_| {
        format!("完成，找到 {} 对匹配", pairs.len())
    });

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::batch::capture_raw_events;
    use crate::commands::test_support::TempDir;

    fn action(src: &Path, target: &str, conflict: Option<&Path>) -> RenameAction {
        RenameAction {
            src_path: src.to_string_lossy().into_owned(),
            target_name: target.into(),
            conflict_path: conflict.map(|p| p.to_string_lossy().into_owned()),
        }
    }

    fn execute(actions: Vec<RenameAction>) -> DedupRenameResult {
        execute_sync(tauri::test::mock_app().handle(), &actions)
    }

    fn read(path: &Path) -> Vec<u8> {
        std::fs::read(path).unwrap()
    }

    #[tokio::test]
    async fn same_directory_export_never_truncates_sources() {
        let root = TempDir::new("export_self");
        std::fs::write(root.join("a.png"), b"source bytes").unwrap();
        // 只差大小写：大小写不敏感的文件系统上就是源文件夹
        let upper = root.with_file_name(crate::commands::file_name_lossy(&root).to_uppercase());
        for destination in [root.to_path_buf(), root.join("."), upper] {
            let result = export_unmatched_files(
                root.to_string_lossy().into_owned(),
                vec!["a.png".into()],
                destination.to_string_lossy().into_owned(),
            )
            .await;
            assert!(result.unwrap_err().contains("不能与源文件夹相同"));
            assert_eq!(read(&root.join("a.png")), b"source bytes");
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
        assert_eq!(read(&output.join("a.png")), b"source bytes");
    }

    /// 目标文件夹里已有同名文件：不覆盖，记为失败「目标已存在」，其余照常导出
    #[tokio::test]
    async fn export_keeps_existing_files_in_the_destination() {
        let root = TempDir::new("export_existing");
        let (source, dest) = (root.join("src"), root.join("dest"));
        std::fs::create_dir_all(&source).unwrap();
        std::fs::create_dir_all(&dest).unwrap();
        std::fs::write(source.join("a.png"), b"new a").unwrap();
        std::fs::write(source.join("b.png"), b"new b").unwrap();
        std::fs::write(dest.join("a.png"), b"old a").unwrap();
        let result = export_unmatched_files(
            source.to_string_lossy().into_owned(),
            vec!["a.png".into(), "b.png".into()],
            dest.to_string_lossy().into_owned(),
        )
        .await
        .unwrap();
        assert_eq!((result.success_count, result.fail_count), (1, 1));
        assert_eq!(result.errors, vec!["a.png: 目标已存在".to_string()]);
        assert_eq!(read(&dest.join("a.png")), b"old a");
        assert_eq!(read(&dest.join("b.png")), b"new b");
    }

    #[test]
    fn conflict_must_be_the_exact_destination() {
        let root = TempDir::new("rename_conflict");
        for name in ["source.png", "target.png", "unrelated.png"] {
            std::fs::write(root.join(name), name.as_bytes()).unwrap();
        }
        std::fs::write(root.join("target.caption"), b"caption").unwrap();
        let result = execute(vec![action(
            &root.join("source.png"),
            "target.png",
            Some(&root.join("unrelated.png")),
        )]);
        assert_eq!((result.success_count, result.fail_count), (0, 1));
        for name in ["source.png", "target.png", "unrelated.png"] {
            assert_eq!(read(&root.join(name)), name.as_bytes());
        }
        assert!(!root.join("unrelated_rename.png").exists());
        let result = execute(vec![action(
            &root.join("source.png"),
            "target.png",
            Some(&root.join("target.png")),
        )]);
        assert_eq!((result.success_count, result.fail_count), (1, 0));
        assert_eq!(read(&root.join("target.png")), b"source.png");
        assert_eq!(read(&root.join("target_rename.png")), b"target.png");
        assert_eq!(read(&root.join("target_rename.caption")), b"caption");
    }

    /// 源图标签文件的目标被别的文件占着：整条动作失败，图片和标签都不动
    #[test]
    fn occupied_sidecar_target_fails_the_whole_action() {
        let root = TempDir::new("rename_sidecar_blocked");
        std::fs::write(root.join("source.png"), b"source").unwrap();
        std::fs::write(root.join("source.txt"), b"source tags").unwrap();
        // 同名另一张图的标签占着 target.txt
        std::fs::write(root.join("target.jpg"), b"other").unwrap();
        std::fs::write(root.join("target.txt"), b"other tags").unwrap();
        let app = tauri::test::mock_app();
        let log = capture_raw_events(app.handle(), EVENT);
        let result = execute_sync(
            app.handle(),
            &[action(&root.join("source.png"), "target.png", None)],
        );
        assert_eq!((result.success_count, result.fail_count), (0, 1));
        assert!(
            result.errors[0].starts_with("标签文件的目标已存在"),
            "{:?}",
            result.errors
        );
        assert_eq!(read(&root.join("source.png")), b"source");
        assert_eq!(read(&root.join("source.txt")), b"source tags");
        assert_eq!(read(&root.join("target.txt")), b"other tags");
        assert!(!root.join("target.png").exists());
        let events = log.lock().unwrap();
        let done = events.last().unwrap();
        assert_eq!(done["message"], "完成: 成功 0, 失败 1");
        let run_id = done["run_id"].as_u64().unwrap();
        assert!(events.iter().all(|e| e["run_id"] == run_id));
    }

    /// 让位的文件连同标签一起改名，挑的名字要让图片和标签都空闲；源图标签随后占用原目标名
    #[test]
    fn yielding_moves_sidecars_to_a_name_free_for_all_of_them() {
        let root = TempDir::new("rename_yield");
        std::fs::write(root.join("source.png"), b"source").unwrap();
        std::fs::write(root.join("source.txt"), b"source tags").unwrap();
        std::fs::write(root.join("target.png"), b"target").unwrap();
        std::fs::write(root.join("target.txt"), b"target tags").unwrap();
        std::fs::write(root.join("target_rename.txt"), b"unrelated").unwrap();
        let result = execute(vec![action(
            &root.join("source.png"),
            "target.png",
            Some(&root.join("target.png")),
        )]);
        assert_eq!(
            (result.success_count, result.fail_count),
            (1, 0),
            "{:?}",
            result.errors
        );
        assert_eq!(read(&root.join("target.png")), b"source");
        assert_eq!(read(&root.join("target.txt")), b"source tags");
        assert_eq!(read(&root.join("target_rename_1.png")), b"target");
        assert_eq!(read(&root.join("target_rename_1.txt")), b"target tags");
        assert_eq!(read(&root.join("target_rename.txt")), b"unrelated");
        assert!(!root.join("source.png").exists() && !root.join("source.txt").exists());
    }

    /// 中途改名失败时撤回已做的改名
    #[test]
    fn failed_move_rolls_back_earlier_moves() {
        let root = TempDir::new("rename_rollback");
        std::fs::write(root.join("a.png"), b"a").unwrap();
        let moves = vec![
            (root.join("a.png"), root.join("b.png")),
            (root.join("missing.txt"), root.join("b.txt")),
        ];
        assert!(apply_moves(&moves).is_err());
        assert_eq!(read(&root.join("a.png")), b"a");
        assert!(!root.join("b.png").exists());
    }

    #[test]
    fn cancelled_scan_ends_with_a_cancelled_done() {
        let root = TempDir::new("dedup_rename_cancel");
        let (a, b) = (root.join("a"), root.join("b"));
        for dir in [&a, &b] {
            std::fs::create_dir_all(dir).unwrap();
            image::RgbImage::new(8, 8).save(dir.join("x.png")).unwrap();
        }
        let options = DedupRenameOptions {
            folder_a: a.to_string_lossy().into_owned(),
            folder_b: b.to_string_lossy().into_owned(),
            dhash_threshold: 5,
            phash_threshold: 5,
            color_threshold: 0.9,
        };
        let app = tauri::test::mock_app();
        let log = capture_raw_events(app.handle(), EVENT);
        let err = scan_sync(app.handle(), &options, &AtomicBool::new(true)).unwrap_err();
        assert!(err.starts_with("已取消"));
        let events = log.lock().unwrap();
        let done = events.last().unwrap();
        assert_eq!(done["cancelled"], true);
        assert_eq!(done["message"], "已取消: 已处理 0/2, 成功 0, 失败 0");

        let app = tauri::test::mock_app();
        let log = capture_raw_events(app.handle(), EVENT);
        let result = scan_sync(app.handle(), &options, &AtomicBool::new(false)).unwrap();
        assert_eq!(result.pairs.len(), 1);
        let events = log.lock().unwrap();
        assert_eq!(events.last().unwrap()["message"], "完成，找到 1 对匹配");
    }
}
