use rand::seq::SliceRandom;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tauri::Emitter;

use super::{collect_image_files_with_recursive, ProcessResult, ProgressEvent, TAG_SIDECAR_EXTS};

#[cfg(test)]
mod regression_tests {
    use super::*;

    #[test]
    fn caption_preview_execution_and_error_totals_match() {
        let root = super::super::image_io::test_dir("rename_caption");
        image::RgbImage::new(2, 2).save(root.join("a.png")).unwrap();
        std::fs::write(root.join("a.caption"), b"original caption").unwrap();
        std::fs::write(root.join("new1.caption"), b"existing caption").unwrap();
        let options = RenameOptions {
            input_path: root.to_string_lossy().into_owned(),
            prefix: "new".into(),
            start_number: 1,
            digit_count: 1,
            shuffle: false,
            shuffle_seed: None,
            rename_tags: true,
        };
        let preview = preview_rename_sync(&options).unwrap();
        assert_eq!(preview.len(), 2);
        assert_eq!(preview[1].renamed, "new1.caption");
        let app = tauri::test::mock_app();
        let events = super::super::batch::capture_events(app.handle(), "rename-progress");
        let result = execute_rename_sync(app.handle(), &options).unwrap();
        assert_eq!(
            (result.success_count, result.fail_count, result.total),
            (1, 1, 2)
        );
        assert!(events
            .lock()
            .unwrap()
            .iter()
            .all(|event| event["total"] == 2));
        assert_eq!(
            std::fs::read(root.join("a.caption")).unwrap(),
            b"original caption"
        );
        assert_eq!(
            std::fs::read(root.join("new1.caption")).unwrap(),
            b"existing caption"
        );
        std::fs::remove_dir_all(root).unwrap();
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RenameOptions {
    pub input_path: String,
    /// 文件名前缀
    pub prefix: String,
    /// 起始编号
    pub start_number: u32,
    /// 编号位数（例如 4 → 0001）
    pub digit_count: u32,
    /// 是否打乱顺序
    pub shuffle: bool,
    /// 打乱用的随机种子。预览与执行传同一种子时顺序一致；
    /// 不传（如工作流直接执行）则随机
    #[serde(default)]
    pub shuffle_seed: Option<u64>,
    /// 是否同步重命名标签文件（.txt, .json, .caption）
    #[serde(default)]
    pub rename_tags: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RenamePreviewItem {
    pub original: String,
    pub renamed: String,
}

/// 生成重命名预览（不实际执行）
#[tauri::command]
pub async fn preview_rename(options: RenameOptions) -> Result<Vec<RenamePreviewItem>, String> {
    tokio::task::spawn_blocking(move || preview_rename_sync(&options))
        .await
        .map_err(|e| format!("任务执行失败: {}", e))?
}

fn preview_rename_sync(options: &RenameOptions) -> Result<Vec<RenamePreviewItem>, String> {
    let input = Path::new(&options.input_path);
    let mut files = collect_image_files_with_recursive(input, false)?;

    if options.shuffle {
        apply_shuffle(&mut files, options.shuffle_seed);
    }

    Ok(plan_renames(&files, options)
        .into_iter()
        .map(|(path, renamed)| RenamePreviewItem {
            original: super::file_name_lossy(&path),
            renamed,
        })
        .collect())
}

/// 预览与执行共用的命名计划：(源路径, 新文件名)，标签文件紧跟在所属图片之后
fn plan_renames(files: &[PathBuf], options: &RenameOptions) -> Vec<(PathBuf, String)> {
    let mut plan = Vec::new();
    for (i, file_path) in files.iter().enumerate() {
        let ext = file_path
            .extension()
            .map(|e| e.to_string_lossy().to_lowercase())
            .unwrap_or_else(|| "png".into());
        let number = options.start_number + i as u32;
        let new_stem = format!(
            "{}{:0>width$}",
            options.prefix,
            number,
            width = options.digit_count as usize
        );
        plan.push((file_path.clone(), format!("{}.{}", new_stem, ext)));

        if options.rename_tags {
            let stem = file_path.file_stem().unwrap_or_default().to_string_lossy();
            let parent = file_path.parent().unwrap_or(Path::new("."));
            for tag_ext in TAG_SIDECAR_EXTS {
                let tag_path = parent.join(format!("{}.{}", stem, tag_ext));
                if tag_path.exists() {
                    plan.push((tag_path, format!("{}.{}", new_stem, tag_ext)));
                }
            }
        }
    }
    plan
}

/// 执行批量重命名
#[tauri::command]
pub async fn execute_rename<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    options: RenameOptions,
) -> Result<ProcessResult, String> {
    tokio::task::spawn_blocking(move || execute_rename_sync(&app, &options))
        .await
        .map_err(|e| format!("任务执行失败: {}", e))?
}

fn execute_rename_sync<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    options: &RenameOptions,
) -> Result<ProcessResult, String> {
    let input = Path::new(&options.input_path);
    let mut files = collect_image_files_with_recursive(input, false)?;

    if options.shuffle {
        apply_shuffle(&mut files, options.shuffle_seed);
    }

    let mut success_count = 0u32;
    let mut fail_count = 0u32;
    let mut errors = Vec::new();

    // Step 1: Build (original_path, temp_path, final_name) mappings.
    // 先整批改成临时名再改成最终名，避免新名撞上批内尚未改名的文件；
    // 临时名里的下标保证批内不重名，批次 ID 避免与目录里已有的文件同名
    let batch_id = uuid_simple();
    let temp_mappings: Vec<(PathBuf, PathBuf, String)> = plan_renames(&files, options)
        .into_iter()
        .enumerate()
        .map(|(idx, (original, final_name))| {
            let parent = original.parent().unwrap_or(Path::new("."));
            let temp_path = parent.join(format!("__rename_temp_{}_{}", idx, batch_id));
            (original, temp_path, final_name)
        })
        .collect();

    // Step 2: Rename to temp names
    for (idx, (original, temp, _)) in temp_mappings.iter().enumerate() {
        if let Err(e) = std::fs::rename(original, temp) {
            // 失败时尽力回滚：把已改为临时名的文件恢复为原名，避免文件滞留在临时名
            let mut rollback_fails: Vec<String> = Vec::new();
            for (orig, tmp, _) in temp_mappings.iter().take(idx) {
                if let Err(re) = std::fs::rename(tmp, orig) {
                    rollback_fails.push(format!("{} → {}: {}", tmp.display(), orig.display(), re));
                }
            }
            let mut msg = format!("临时重命名失败 {}: {}", original.display(), e);
            if rollback_fails.is_empty() {
                if idx > 0 {
                    msg.push_str(&format!("（已回滚 {} 个文件）", idx));
                }
            } else {
                msg.push_str(&format!(
                    "（已回滚 {} 个文件，{} 个回滚失败: {}）",
                    idx - rollback_fails.len(),
                    rollback_fails.len(),
                    rollback_fails.join("; ")
                ));
            }
            return Err(msg);
        }
    }

    // Step 3: Rename to final names
    let total_mappings = temp_mappings.len() as u32;
    for (i, (original, temp, final_name)) in temp_mappings.iter().enumerate() {
        let original_name = super::file_name_lossy(original);
        let parent = temp.parent().unwrap_or(Path::new("."));
        let final_path = parent.join(final_name);

        let _ = app.emit(
            "rename-progress",
            ProgressEvent::new(
                "processing",
                format!("正在重命名: {} → {}", original_name, final_name),
            )
            .at(i as u32 + 1, total_mappings)
            .file(original_name.clone()),
        );

        // 最终名被批外文件占用时拒绝覆盖（批内文件此时都已移到临时名）
        let rename_result = if final_path.exists() {
            Err(format!("目标文件已存在，跳过以免覆盖: {}", final_name))
        } else {
            std::fs::rename(temp, &final_path).map_err(|e| e.to_string())
        };

        match rename_result {
            Ok(_) => {
                success_count += 1;
                let _ = app.emit(
                    "rename-progress",
                    ProgressEvent::new(
                        "success",
                        format!("[重命名] {} → {}", original_name, final_name),
                    )
                    .at(i as u32 + 1, total_mappings)
                    .file(original_name.clone()),
                );
            }
            Err(e) => {
                fail_count += 1;
                // 把临时名滚回原名，避免文件滞留无扩展名的临时名
                let mut err_msg = format!("{} → {}: {}", original_name, final_name, e);
                if let Err(re) = std::fs::rename(temp, original) {
                    err_msg.push_str(&format!(
                        "（且回滚原名失败，文件滞留临时名 {}: {}）",
                        temp.display(),
                        re
                    ));
                }
                errors.push(err_msg.clone());
                let _ = app.emit(
                    "rename-progress",
                    ProgressEvent::new("error", format!("[错误] {}", err_msg))
                        .at(i as u32 + 1, total_mappings)
                        .file(original_name.clone()),
                );
            }
        }
    }

    let _ = app.emit(
        "rename-progress",
        ProgressEvent::new(
            "done",
            format!(
                "重命名完成: 图片 {} 张, 共处理 {} 个文件, 失败 {}",
                files.len(),
                total_mappings,
                fail_count
            ),
        )
        .at(total_mappings, total_mappings),
    );

    Ok(ProcessResult {
        success_count,
        fail_count,
        total: total_mappings,
        errors,
    })
}

/// 打乱文件列表。种子相同时顺序确定，用于让预览与执行的映射一致；
/// 无种子（工作流等直接执行场景）时完全随机
fn apply_shuffle(files: &mut [PathBuf], seed: Option<u64>) {
    match seed {
        Some(s) => {
            use rand::SeedableRng;
            let mut rng = rand::rngs::StdRng::seed_from_u64(s);
            files.shuffle(&mut rng);
        }
        None => files.shuffle(&mut rand::rng()),
    }
}

/// 生成简易唯一 ID（避免引入 uuid 库）
fn uuid_simple() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("{:x}", ts)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shuffle_with_same_seed_is_deterministic() {
        let files: Vec<std::path::PathBuf> = (0..10)
            .map(|i| std::path::PathBuf::from(format!("img_{}.png", i)))
            .collect();
        let mut a = files.clone();
        let mut b = files.clone();
        apply_shuffle(&mut a, Some(42));
        apply_shuffle(&mut b, Some(42));
        assert_eq!(a, b);

        // 只是重排，不增删文件
        let mut sorted = a.clone();
        sorted.sort();
        assert_eq!(sorted, files);
    }

    #[test]
    fn shuffle_without_seed_keeps_all_files() {
        let files: Vec<std::path::PathBuf> = (0..5)
            .map(|i| std::path::PathBuf::from(format!("img_{}.png", i)))
            .collect();
        let mut a = files.clone();
        apply_shuffle(&mut a, None);
        a.sort();
        assert_eq!(a, files);
    }
}
