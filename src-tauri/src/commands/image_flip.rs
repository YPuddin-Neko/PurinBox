use serde::{Deserialize, Serialize};
use std::path::Path;
use tauri::Emitter;

use super::{collect_image_files, ProcessResult, ProgressEvent};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlipOptions {
    pub input_path: String,
    pub output_path: String,
    /// "horizontal" | "vertical" | "both"
    pub direction: String,
}

#[tauri::command]
pub async fn flip_images(app: tauri::AppHandle, options: FlipOptions) -> Result<ProcessResult, String> {
    tokio::task::spawn_blocking(move || {
        flip_images_sync(&app, &options)
    })
    .await
    .map_err(|e| format!("任务执行失败: {}", e))?
}

fn flip_images_sync(app: &tauri::AppHandle, options: &FlipOptions) -> Result<ProcessResult, String> {
    let input = Path::new(&options.input_path);
    let output_dir = Path::new(&options.output_path);

    if !output_dir.exists() {
        std::fs::create_dir_all(output_dir)
            .map_err(|e| format!("无法创建输出目录: {}", e))?;
    }

    let files = collect_image_files(input)?;
    let total = files.len() as u32;
    let mut success_count = 0u32;
    let mut fail_count = 0u32;
    let mut errors = Vec::new();

    let direction_label = match options.direction.as_str() {
        "horizontal" => "水平翻转",
        "vertical" => "垂直翻转",
        "both" => "双向翻转",
        _ => "翻转",
    };

    for (i, file_path) in files.iter().enumerate() {
        let filename = file_path.file_name().unwrap_or_default().to_string_lossy().to_string();

        let _ = app.emit("flip-progress", ProgressEvent {
            current: i as u32 + 1,
            total,
            filename: filename.clone(),
            status: "processing".to_string(),
            message: format!("正在处理: {}", filename),
        });

        match process_flip(file_path, output_dir, &options.direction) {
            Ok(_) => {
                success_count += 1;
                let _ = app.emit("flip-progress", ProgressEvent {
                    current: i as u32 + 1,
                    total,
                    filename: filename.clone(),
                    status: "success".to_string(),
                    message: format!("[{}] {} ✓", direction_label, filename),
                });
            }
            Err(e) => {
                fail_count += 1;
                let err_msg = format!("{}: {}", filename, e);
                errors.push(err_msg.clone());
                let _ = app.emit("flip-progress", ProgressEvent {
                    current: i as u32 + 1,
                    total,
                    filename: filename.clone(),
                    status: "error".to_string(),
                    message: format!("[失败] {}", err_msg),
                });
            }
        }
    }

    let _ = app.emit("flip-progress", ProgressEvent {
        current: total,
        total,
        filename: String::new(),
        status: "done".to_string(),
        message: format!("处理完成: 成功 {}, 失败 {}, 共 {}", success_count, fail_count, total),
    });

    Ok(ProcessResult { success_count, fail_count, total, errors })
}

fn process_flip(file_path: &Path, output_dir: &Path, direction: &str) -> Result<(), String> {
    let img = image::open(file_path)
        .map_err(|e| format!("无法打开图片: {}", e))?;

    let flipped = match direction {
        "horizontal" => img.fliph(),
        "vertical" => img.flipv(),
        "both" => img.fliph().flipv(),
        _ => return Err("无效的翻转方向".to_string()),
    };

    let file_name = file_path.file_name().ok_or("无效的文件名")?.to_string_lossy();
    let output_path = output_dir.join(file_name.as_ref());
    flipped.save(&output_path).map_err(|e| format!("无法保存图片: {}", e))?;
    Ok(())
}
