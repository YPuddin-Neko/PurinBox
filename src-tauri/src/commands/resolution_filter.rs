use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::Emitter;

use super::{collect_image_files, ProcessResult, ProgressEvent};

static CANCEL_FLAG: AtomicBool = AtomicBool::new(false);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FilterOptions {
    pub input_path: String,
    pub output_path: String,
    /// "copy" | "delete"
    pub action: String,
    /// "min_width" | "min_height" | "below_resolution" | "above_resolution"
    pub condition: String,
    pub width: u32,
    pub height: u32,
}

#[tauri::command]
pub async fn filter_by_resolution(app: tauri::AppHandle, options: FilterOptions) -> Result<ProcessResult, String> {
    CANCEL_FLAG.store(false, Ordering::SeqCst);
    tokio::task::spawn_blocking(move || {
        filter_sync(&app, &options)
    })
    .await
    .map_err(|e| format!("任务执行失败: {}", e))?
}

#[tauri::command]
pub fn cancel_filter() {
    CANCEL_FLAG.store(true, Ordering::SeqCst);
}

fn filter_sync(app: &tauri::AppHandle, options: &FilterOptions) -> Result<ProcessResult, String> {
    let input = Path::new(&options.input_path);
    let output_dir = Path::new(&options.output_path);

    if !input.exists() || !input.is_dir() {
        return Err(format!("输入目录不存在: {}", options.input_path));
    }

    if options.action == "copy" && !output_dir.exists() {
        std::fs::create_dir_all(output_dir)
            .map_err(|e| format!("无法创建输出目录: {}", e))?;
    }

    let files = collect_image_files(input)?;
    let total = files.len() as u32;
    let mut success_count = 0u32;
    let mut fail_count = 0u32;
    let mut errors = Vec::new();

    let condition_label = match options.condition.as_str() {
        "min_width" => format!("宽度 < {}px", options.width),
        "min_height" => format!("高度 < {}px", options.height),
        "below_resolution" => format!("低于 {}x{}", options.width, options.height),
        "above_resolution" => format!("高于 {}x{}", options.width, options.height),
        _ => "未知条件".to_string(),
    };

    let action_label = if options.action == "copy" { "输出" } else { "删除" };

    let _ = app.emit("filter-progress", ProgressEvent {
        current: 0,
        total,
        filename: String::new(),
        status: "processing".to_string(),
        message: format!("开始筛选: 条件={}, 操作={}, 共 {} 张图片", condition_label, action_label, total),
    });

    for (i, file_path) in files.iter().enumerate() {
        if CANCEL_FLAG.load(Ordering::SeqCst) {
            let _ = app.emit("filter-progress", ProgressEvent {
                current: i as u32, total, filename: String::new(),
                status: "done".to_string(),
                message: format!("已取消: 已处理 {}, 共 {}", i, total),
            });
            break;
        }
        let filename = file_path.file_name().unwrap_or_default().to_string_lossy().to_string();

        let _ = app.emit("filter-progress", ProgressEvent {
            current: i as u32 + 1,
            total,
            filename: filename.clone(),
            status: "processing".to_string(),
            message: format!("正在检查: {}", filename),
        });

        match process_filter(file_path, output_dir, options) {
            Ok((matched, w, h)) => {
                if matched {
                    success_count += 1;
                    let _ = app.emit("filter-progress", ProgressEvent {
                        current: i as u32 + 1,
                        total,
                        filename: filename.clone(),
                        status: "success".to_string(),
                        message: format!("[匹配] {} ({}x{}) → {}", filename, w, h, action_label),
                    });
                } else {
                    let _ = app.emit("filter-progress", ProgressEvent {
                        current: i as u32 + 1,
                        total,
                        filename: filename.clone(),
                        status: "success".to_string(),
                        message: format!("[跳过] {} ({}x{}, 不匹配条件)", filename, w, h),
                    });
                }
            }
            Err(e) => {
                fail_count += 1;
                let err_msg = format!("{}: {}", filename, e);
                errors.push(err_msg.clone());
                let _ = app.emit("filter-progress", ProgressEvent {
                    current: i as u32 + 1,
                    total,
                    filename: filename.clone(),
                    status: "error".to_string(),
                    message: format!("[错误] {}", err_msg),
                });
            }
        }
    }

    let _ = app.emit("filter-progress", ProgressEvent {
        current: total,
        total,
        filename: String::new(),
        status: "done".to_string(),
        message: format!("筛选完成: 匹配并{} {} 张, 失败 {} 张, 共扫描 {} 张", action_label, success_count, fail_count, total),
    });

    Ok(ProcessResult { success_count, fail_count, total, errors })
}

fn process_filter(file_path: &Path, output_dir: &Path, options: &FilterOptions) -> Result<(bool, u32, u32), String> {
    let (w, h) = image::image_dimensions(file_path)
        .map_err(|e| format!("无法读取图片尺寸: {}", e))?;

    let matches = match options.condition.as_str() {
        "min_width" => w < options.width,
        "min_height" => h < options.height,
        "below_resolution" => w < options.width && h < options.height,
        "above_resolution" => w > options.width && h > options.height,
        _ => return Err("无效的筛选条件".to_string()),
    };

    if matches {
        match options.action.as_str() {
            "copy" => {
                let file_name = file_path.file_name().ok_or("无效的文件名")?.to_string_lossy();
                let dest = output_dir.join(file_name.as_ref());
                std::fs::copy(file_path, dest).map_err(|e| format!("复制失败: {}", e))?;
            }
            "delete" => {
                std::fs::remove_file(file_path).map_err(|e| format!("删除失败: {}", e))?;
            }
            _ => return Err("无效的操作".to_string()),
        }
    }

    Ok((matches, w, h))
}
