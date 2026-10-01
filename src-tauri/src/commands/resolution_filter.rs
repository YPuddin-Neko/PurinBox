use serde::{Deserialize, Serialize};
use std::path::Path;

use super::{
    collect_image_files_with_recursive_excluding, same_name_output, ProcessResult, ProgressEvent,
};

use super::batch::{BatchJob, FileBatch, FileOutcome};

static JOB: BatchJob = BatchJob::new("分辨率筛选");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mismatched_extension_filters_normally_and_validates_once() {
        let root = super::super::image_io::test_dir("filter_content");
        image::RgbImage::new(17, 23)
            .save_with_format(root.join("webp.png"), image::ImageFormat::WebP)
            .unwrap();
        let output = root.join("out");
        let mut options = FilterOptions {
            input_path: root.to_string_lossy().into_owned(),
            output_path: output.to_string_lossy().into_owned(),
            action: "copy".into(),
            condition: "min_width".into(),
            width: 20,
            height: 20,
            recursive: false,
        };
        let app = tauri::test::mock_app();
        let result = filter_sync(app.handle(), &options).unwrap();
        assert_eq!((result.success_count, result.fail_count), (1, 0));
        assert_eq!(
            std::fs::read(root.join("webp.png")).unwrap(),
            std::fs::read(output.join("webp.png")).unwrap()
        );
        options.action = "invalid".into();
        assert_eq!(
            filter_sync(app.handle(), &options).unwrap_err(),
            "无效的操作"
        );
        std::fs::remove_dir_all(root).unwrap();
    }
}

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
    #[serde(default)]
    pub recursive: bool,
}

#[tauri::command]
pub async fn filter_by_resolution<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    options: FilterOptions,
) -> Result<ProcessResult, String> {
    JOB.run(move || filter_sync(&app, &options)).await
}

#[tauri::command]
pub fn cancel_filter() {
    JOB.cancel();
}

fn filter_sync<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    options: &FilterOptions,
) -> Result<ProcessResult, String> {
    if !matches!(
        options.condition.as_str(),
        "min_width" | "min_height" | "below_resolution" | "above_resolution"
    ) {
        return Err("无效的筛选条件".to_string());
    }
    if !matches!(options.action.as_str(), "copy" | "delete") {
        return Err("无效的操作".to_string());
    }
    let input = Path::new(&options.input_path);
    let output_dir = Path::new(&options.output_path);
    if !input.is_dir() {
        return Err(format!("输入目录不存在: {}", options.input_path));
    }
    if options.action == "copy" {
        std::fs::create_dir_all(output_dir).map_err(|e| format!("无法创建输出目录: {}", e))?;
    }
    let files = collect_image_files_with_recursive_excluding(
        input,
        options.recursive,
        if options.action == "copy" {
            Some(output_dir)
        } else {
            None
        },
    )?;
    let total = files.len() as u32;
    let condition_label = match options.condition.as_str() {
        "min_width" => format!("宽度 < {}px", options.width),
        "min_height" => format!("高度 < {}px", options.height),
        "below_resolution" => format!("低于 {}x{}", options.width, options.height),
        "above_resolution" => format!("高于 {}x{}", options.width, options.height),
        _ => unreachable!("validated condition"),
    };
    let action_label = if options.action == "copy" {
        "输出"
    } else {
        "删除"
    };
    ProgressEvent::new(
        "processing",
        format!(
            "开始筛选: 条件={}, 操作={}, 共 {} 张图片",
            condition_label, action_label, total
        ),
    )
    .at(0, total)
    .emit(app, "filter-progress");
    Ok(FileBatch::new(app, "filter-progress", JOB.cancel_flag())
        .processing("正在检查")
        .error_prefix("[错误] ")
        .run(
            &files,
            |item| {
                let (matched, w, h) = process_filter(item.path, input, output_dir, options)?;
                Ok(if matched {
                    FileOutcome::done(format!(
                        "[匹配] {} ({}x{}) → {}",
                        item.name, w, h, action_label
                    ))
                } else {
                    FileOutcome::skipped(format!("[跳过] {} ({}x{}, 不匹配条件)", item.name, w, h))
                })
            },
            |c| {
                format!(
                    "筛选完成: 匹配并{} {} 张, 失败 {} 张, 共扫描 {} 张",
                    action_label, c.success, c.failed, c.total
                )
            },
        ))
}

fn process_filter(
    file_path: &Path,
    input_root: &Path,
    output_dir: &Path,
    options: &FilterOptions,
) -> Result<(bool, u32, u32), String> {
    let (w, h) = super::image_io::read_dimensions(file_path)
        .map_err(|e| format!("无法读取图片尺寸: {}", e))?;

    let matches = match options.condition.as_str() {
        "min_width" => w < options.width,
        "min_height" => h < options.height,
        "below_resolution" => w < options.width && h < options.height,
        "above_resolution" => w > options.width && h > options.height,
        _ => unreachable!("validated condition"),
    };

    if matches {
        match options.action.as_str() {
            "copy" => {
                let dest = same_name_output(input_root, file_path, output_dir, options.recursive)?;
                crate::commands::copy_file_safe(file_path, &dest)?;
            }
            "delete" => {
                std::fs::remove_file(file_path).map_err(|e| format!("删除失败: {}", e))?;
            }
            _ => unreachable!("validated action"),
        }
    }

    Ok((matches, w, h))
}
