use serde::{Deserialize, Serialize};
use std::path::Path;
use tauri::Emitter;
use rand::seq::SliceRandom;

use super::{collect_image_files, ProcessResult, ProgressEvent};

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
    /// 是否同步重命名标签文件（.txt, .json）
    #[serde(default)]
    pub rename_tags: bool,
}

/// 标签文件扩展名
const TAG_EXTS: &[&str] = &["txt", "json"];

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RenamePreviewItem {
    pub original: String,
    pub renamed: String,
}

/// 生成重命名预览（不实际执行）
#[tauri::command]
pub async fn preview_rename(options: RenameOptions) -> Result<Vec<RenamePreviewItem>, String> {
    tokio::task::spawn_blocking(move || {
        preview_rename_sync(&options)
    })
    .await
    .map_err(|e| format!("任务执行失败: {}", e))?
}

fn preview_rename_sync(options: &RenameOptions) -> Result<Vec<RenamePreviewItem>, String> {
    let input = Path::new(&options.input_path);
    let mut files = collect_image_files(input)?;

    if options.shuffle {
        let mut rng = rand::rng();
        files.shuffle(&mut rng);
    }

    let mut previews = Vec::new();
    for (i, file_path) in files.iter().enumerate() {
        let original = file_path.file_name().unwrap_or_default().to_string_lossy().to_string();
        let ext = file_path.extension()
            .map(|e| e.to_string_lossy().to_lowercase())
            .unwrap_or_else(|| "png".into());
        let number = options.start_number + i as u32;
        let formatted_num = format!("{:0>width$}", number, width = options.digit_count as usize);
        let renamed = format!("{}{}.{}", options.prefix, formatted_num, ext);
        previews.push(RenamePreviewItem { original: original.clone(), renamed: renamed.clone() });

        // 同步预览标签文件
        if options.rename_tags {
            let stem = file_path.file_stem().unwrap_or_default().to_string_lossy();
            let new_stem = format!("{}{}", options.prefix, formatted_num);
            for tag_ext in TAG_EXTS {
                let tag_file = file_path.parent().unwrap_or(Path::new(".")).join(format!("{}.{}", stem, tag_ext));
                if tag_file.exists() {
                    previews.push(RenamePreviewItem {
                        original: format!("{}.{}", stem, tag_ext),
                        renamed: format!("{}.{}", new_stem, tag_ext),
                    });
                }
            }
        }
    }

    Ok(previews)
}

/// 执行批量重命名
#[tauri::command]
pub async fn execute_rename(app: tauri::AppHandle, options: RenameOptions) -> Result<ProcessResult, String> {
    tokio::task::spawn_blocking(move || {
        execute_rename_sync(&app, &options)
    })
    .await
    .map_err(|e| format!("任务执行失败: {}", e))?
}

fn execute_rename_sync(app: &tauri::AppHandle, options: &RenameOptions) -> Result<ProcessResult, String> {
    let input = Path::new(&options.input_path);
    let mut files = collect_image_files(input)?;

    if options.shuffle {
        let mut rng = rand::rng();
        files.shuffle(&mut rng);
    }

    let total = files.len() as u32;
    let mut success_count = 0u32;
    let mut fail_count = 0u32;
    let mut errors = Vec::new();
    let image_count = files.len() as u32;

    // Step 1: Rename all files to temporary names to avoid conflicts
    // Each entry: (original_path, temp_path, final_name)
    let mut temp_mappings: Vec<(std::path::PathBuf, std::path::PathBuf, String)> = Vec::new();

    for (i, file_path) in files.iter().enumerate() {
        let ext = file_path.extension()
            .map(|e| e.to_string_lossy().to_lowercase())
            .unwrap_or_else(|| "png".into());
        let number = options.start_number + i as u32;
        let formatted_num = format!("{:0>width$}", number, width = options.digit_count as usize);
        let final_name = format!("{}{}.{}", options.prefix, formatted_num, ext);
        let parent = file_path.parent().unwrap_or(Path::new("."));

        // Temp name to avoid collisions
        let temp_name = format!("__rename_temp_{}_{}", i, uuid_simple());
        let temp_path = parent.join(&temp_name);
        temp_mappings.push((file_path.clone(), temp_path, final_name));

        // 标签文件也加入临时映射
        if options.rename_tags {
            let stem = file_path.file_stem().unwrap_or_default().to_string_lossy();
            let new_stem = format!("{}{}", options.prefix, formatted_num);
            for tag_ext in TAG_EXTS {
                let tag_path = parent.join(format!("{}.{}", stem, tag_ext));
                if tag_path.exists() {
                    let tag_temp = parent.join(format!("__rename_temp_{}_tag_{}_{}", i, tag_ext, uuid_simple()));
                    let tag_final = format!("{}.{}", new_stem, tag_ext);
                    temp_mappings.push((tag_path, tag_temp, tag_final));
                }
            }
        }
    }

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
        let original_name = original.file_name().unwrap_or_default().to_string_lossy().to_string();
        let parent = temp.parent().unwrap_or(Path::new("."));
        let final_path = parent.join(final_name);

        let _ = app.emit("rename-progress", ProgressEvent {
            current: i as u32 + 1,
            total: total_mappings,
            filename: original_name.clone(),
            status: "processing".to_string(),
            message: format!("正在重命名: {} → {}", original_name, final_name),
        ..Default::default()
        });

        // 最终名被批外文件占用时拒绝覆盖（批内文件此时都已移到临时名）
        let rename_result = if final_path.exists() {
            Err(format!("目标文件已存在，跳过以免覆盖: {}", final_name))
        } else {
            std::fs::rename(temp, &final_path).map_err(|e| e.to_string())
        };

        match rename_result {
            Ok(_) => {
                success_count += 1;
                let _ = app.emit("rename-progress", ProgressEvent {
                    current: i as u32 + 1,
                    total: total_mappings,
                    filename: original_name.clone(),
                    status: "success".to_string(),
                    message: format!("[重命名] {} → {}", original_name, final_name),
                ..Default::default()
                });
            }
            Err(e) => {
                fail_count += 1;
                // 把临时名滚回原名，避免文件滞留无扩展名的临时名
                let mut err_msg = format!("{} → {}: {}", original_name, final_name, e);
                if let Err(re) = std::fs::rename(temp, original) {
                    err_msg.push_str(&format!("（且回滚原名失败，文件滞留临时名 {}: {}）", temp.display(), re));
                }
                errors.push(err_msg.clone());
                let _ = app.emit("rename-progress", ProgressEvent {
                    current: i as u32 + 1,
                    total,
                    filename: original_name.clone(),
                    status: "error".to_string(),
                    message: format!("[错误] {}", err_msg),
                ..Default::default()
                });
            }
        }
    }

    let _ = app.emit("rename-progress", ProgressEvent {
        current: total_mappings,
        total: total_mappings,
        filename: String::new(),
        status: "done".to_string(),
        message: format!("重命名完成: 图片 {} 张, 共处理 {} 个文件, 失败 {}", image_count, total_mappings, fail_count),
    ..Default::default()
    });

    Ok(ProcessResult { success_count, fail_count, total, errors })
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
