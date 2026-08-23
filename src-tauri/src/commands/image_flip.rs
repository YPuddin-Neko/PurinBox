use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::Emitter;

use super::{
    collect_image_files_with_recursive_excluding, output_path_for_input, ProcessResult,
    ProgressEvent,
};

static CANCEL_FLAG: AtomicBool = AtomicBool::new(false);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlipOptions {
    pub input_path: String,
    pub output_path: String,
    /// "horizontal" | "vertical" | "both"
    pub direction: String,
    #[serde(default)]
    pub recursive: bool,
}

#[tauri::command]
pub async fn flip_images<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    options: FlipOptions,
) -> Result<ProcessResult, String> {
    // 互斥：页面与工作流节点共用全局取消标志，并发会互吞取消
    static RUNNING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    let _busy = crate::commands::BusyGuard::acquire(&RUNNING, "翻转")?;

    CANCEL_FLAG.store(false, Ordering::SeqCst);
    tokio::task::spawn_blocking(move || flip_images_sync(&app, &options))
        .await
        .map_err(|e| format!("任务执行失败: {}", e))?
}

#[tauri::command]
pub fn cancel_flip() {
    CANCEL_FLAG.store(true, Ordering::SeqCst);
}

fn flip_images_sync<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    options: &FlipOptions,
) -> Result<ProcessResult, String> {
    let input = Path::new(&options.input_path);
    let output_dir = Path::new(&options.output_path);

    if !output_dir.exists() {
        std::fs::create_dir_all(output_dir).map_err(|e| format!("无法创建输出目录: {}", e))?;
    }

    let files =
        collect_image_files_with_recursive_excluding(input, options.recursive, Some(output_dir))?;
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
        if CANCEL_FLAG.load(Ordering::SeqCst) {
            let _ = app.emit(
                "flip-progress",
                ProgressEvent {
                    current: i as u32,
                    total,
                    filename: String::new(),
                    status: "done".to_string(),
                    message: format!("已取消: 已处理 {}, 共 {}", i, total),
                    ..Default::default()
                },
            );
            break;
        }
        let filename = file_path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();

        let _ = app.emit(
            "flip-progress",
            ProgressEvent {
                current: i as u32 + 1,
                total,
                filename: filename.clone(),
                status: "processing".to_string(),
                message: format!("正在处理: {}", filename),
                ..Default::default()
            },
        );

        match process_flip(file_path, input, output_dir, options) {
            Ok(_) => {
                success_count += 1;
                let _ = app.emit(
                    "flip-progress",
                    ProgressEvent {
                        current: i as u32 + 1,
                        total,
                        filename: filename.clone(),
                        status: "success".to_string(),
                        message: format!("[{}] {} ✓", direction_label, filename),
                        ..Default::default()
                    },
                );
            }
            Err(e) => {
                fail_count += 1;
                let err_msg = format!("{}: {}", filename, e);
                errors.push(err_msg.clone());
                let _ = app.emit(
                    "flip-progress",
                    ProgressEvent {
                        current: i as u32 + 1,
                        total,
                        filename: filename.clone(),
                        status: "error".to_string(),
                        message: format!("[失败] {}", err_msg),
                        ..Default::default()
                    },
                );
            }
        }
    }

    // 取消路径已发过"已取消"的 done 事件，这里不再发完成事件覆盖它
    if !CANCEL_FLAG.load(Ordering::SeqCst) {
        let _ = app.emit(
            "flip-progress",
            ProgressEvent {
                current: total,
                total,
                filename: String::new(),
                status: "done".to_string(),
                message: format!(
                    "处理完成: 成功 {}, 失败 {}, 共 {}",
                    success_count, fail_count, total
                ),
                ..Default::default()
            },
        );
    }

    Ok(ProcessResult {
        success_count,
        fail_count,
        total,
        errors,
    })
}

fn process_flip(
    file_path: &Path,
    input_root: &Path,
    output_dir: &Path,
    options: &FlipOptions,
) -> Result<(), String> {
    let img = image::ImageReader::open(file_path)
        .map_err(|e| format!("无法打开图片: {}", e))?
        .with_guessed_format()
        .map_err(|e| format!("无法识别图片格式: {}", e))?
        .decode()
        .map_err(|e| format!("无法解码图片: {}", e))?;

    let flipped = match options.direction.as_str() {
        "horizontal" => img.fliph(),
        "vertical" => img.flipv(),
        "both" => img.fliph().flipv(),
        _ => return Err("无效的翻转方向".to_string()),
    };

    let file_name = file_path
        .file_name()
        .ok_or("无效的文件名")?
        .to_string_lossy();
    let output_path = output_path_for_input(
        input_root,
        file_path,
        output_dir,
        file_name.as_ref(),
        options.recursive,
    )?;
    flipped
        .save(&output_path)
        .map_err(|e| format!("无法保存图片: {}", e))?;
    Ok(())
}
