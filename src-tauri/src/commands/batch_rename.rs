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
}

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
        previews.push(RenamePreviewItem { original, renamed });
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

    // Step 1: Rename all files to temporary names to avoid conflicts
    let mut temp_mappings: Vec<(std::path::PathBuf, std::path::PathBuf, String)> = Vec::new();

    for (i, file_path) in files.iter().enumerate() {
        let ext = file_path.extension()
            .map(|e| e.to_string_lossy().to_lowercase())
            .unwrap_or_else(|| "png".into());
        let number = options.start_number + i as u32;
        let formatted_num = format!("{:0>width$}", number, width = options.digit_count as usize);
        let final_name = format!("{}{}.{}", options.prefix, formatted_num, ext);

        // Temp name to avoid collisions
        let temp_name = format!("__rename_temp_{}_{}", i, uuid_simple());
        let parent = file_path.parent().unwrap_or(Path::new("."));
        let temp_path = parent.join(&temp_name);

        temp_mappings.push((file_path.clone(), temp_path, final_name));
    }

    // Step 2: Rename to temp names
    for (original, temp, _) in &temp_mappings {
        if let Err(e) = std::fs::rename(original, temp) {
            return Err(format!("临时重命名失败 {}: {}", original.display(), e));
        }
    }

    // Step 3: Rename to final names
    for (i, (_original, temp, final_name)) in temp_mappings.iter().enumerate() {
        let original_name = files[i].file_name().unwrap_or_default().to_string_lossy().to_string();
        let parent = temp.parent().unwrap_or(Path::new("."));
        let final_path = parent.join(final_name);

        let _ = app.emit("rename-progress", ProgressEvent {
            current: i as u32 + 1,
            total,
            filename: original_name.clone(),
            status: "processing".to_string(),
            message: format!("正在重命名: {} → {}", original_name, final_name),
        });

        match std::fs::rename(temp, &final_path) {
            Ok(_) => {
                success_count += 1;
                let _ = app.emit("rename-progress", ProgressEvent {
                    current: i as u32 + 1,
                    total,
                    filename: original_name.clone(),
                    status: "success".to_string(),
                    message: format!("[重命名] {} → {}", original_name, final_name),
                });
            }
            Err(e) => {
                fail_count += 1;
                let err_msg = format!("{} → {}: {}", original_name, final_name, e);
                errors.push(err_msg.clone());
                let _ = app.emit("rename-progress", ProgressEvent {
                    current: i as u32 + 1,
                    total,
                    filename: original_name.clone(),
                    status: "error".to_string(),
                    message: format!("[错误] {}", err_msg),
                });
            }
        }
    }

    let _ = app.emit("rename-progress", ProgressEvent {
        current: total,
        total,
        filename: String::new(),
        status: "done".to_string(),
        message: format!("重命名完成: 成功 {}, 失败 {}, 共 {}", success_count, fail_count, total),
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
