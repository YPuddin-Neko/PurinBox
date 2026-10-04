use serde::{Deserialize, Serialize};
use std::path::Path;

use super::{
    collect_image_files_with_recursive_excluding, same_name_output, ProcessResult, ProgressEvent,
};

use super::batch::{BatchJob, FileBatch, FileOutcome, RunEvents};

static JOB: BatchJob = BatchJob::new("分辨率筛选");

const EVENT: &str = "filter-progress";

/// 命中条件的图片怎么处理，JSON 取值为小写名称
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FilterAction {
    /// 复制到输出目录
    Copy,
    /// 原地删除
    Delete,
}

/// 筛选条件，JSON 取值为下划线小写名称
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FilterCondition {
    /// 宽度小于 `width`
    MinWidth,
    /// 高度小于 `height`
    MinHeight,
    /// 宽高都小于目标
    BelowResolution,
    /// 宽高都大于目标
    AboveResolution,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FilterOptions {
    pub input_path: String,
    pub output_path: String,
    pub action: FilterAction,
    pub condition: FilterCondition,
    pub width: u32,
    pub height: u32,
    #[serde(default)]
    pub recursive: bool,
}

impl FilterOptions {
    fn matches(&self, w: u32, h: u32) -> bool {
        match self.condition {
            FilterCondition::MinWidth => w < self.width,
            FilterCondition::MinHeight => h < self.height,
            FilterCondition::BelowResolution => w < self.width && h < self.height,
            FilterCondition::AboveResolution => w > self.width && h > self.height,
        }
    }

    fn condition_label(&self) -> String {
        match self.condition {
            FilterCondition::MinWidth => format!("宽度 < {}px", self.width),
            FilterCondition::MinHeight => format!("高度 < {}px", self.height),
            FilterCondition::BelowResolution => format!("低于 {}x{}", self.width, self.height),
            FilterCondition::AboveResolution => format!("高于 {}x{}", self.width, self.height),
        }
    }
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
    let input = Path::new(&options.input_path);
    let output_dir = Path::new(&options.output_path);
    if !input.is_dir() {
        return Err(format!("输入目录不存在: {}", options.input_path));
    }
    let copy = options.action == FilterAction::Copy;
    if copy {
        std::fs::create_dir_all(output_dir).map_err(|e| format!("无法创建输出目录: {}", e))?;
    }
    let files = collect_image_files_with_recursive_excluding(
        input,
        options.recursive,
        copy.then_some(output_dir),
    )?;
    let total = files.len() as u32;
    let action_label = if copy { "输出" } else { "删除" };
    let run = RunEvents::begin(app, EVENT);
    run.emit(
        ProgressEvent::new(
            "processing",
            format!(
                "开始筛选: 条件={}, 操作={}, 共 {} 张图片",
                options.condition_label(),
                action_label,
                total
            ),
        )
        .at(0, total),
    );
    // 删除模式就地处理，读不出的文件归集到输入目录的 Fail/
    let archive_root = if copy { output_dir } else { input };
    Ok(FileBatch::new(app, EVENT, JOB.cancel_flag())
        .in_run(run.run_id())
        .error_prefix("[错误] ")
        .archive_failures(input, archive_root, options.recursive)
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

    let matches = options.matches(w, h);
    if matches {
        match options.action {
            FilterAction::Copy => {
                let dest = same_name_output(input_root, file_path, output_dir, options.recursive)?;
                crate::commands::copy_file_safe(file_path, &dest)?;
            }
            FilterAction::Delete => {
                std::fs::remove_file(file_path).map_err(|e| format!("删除失败: {}", e))?;
            }
        }
    }

    Ok((matches, w, h))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::batch::capture_raw_events;
    use crate::commands::test_support::TempDir;
    use serde_json::json;

    fn filter_options(input: &Path, output: &Path, action: &str) -> FilterOptions {
        serde_json::from_value(json!({
            "input_path": input.to_string_lossy(),
            "output_path": output.to_string_lossy(),
            "action": action,
            "condition": "min_width",
            "width": 20,
            "height": 20,
            "recursive": false,
        }))
        .unwrap()
    }

    #[test]
    fn mismatched_extension_filters_normally() {
        let root = TempDir::new("filter_content");
        image::RgbImage::new(17, 23)
            .save_with_format(root.join("webp.png"), image::ImageFormat::WebP)
            .unwrap();
        let output = root.join("out");
        let app = tauri::test::mock_app();
        let result = filter_sync(app.handle(), &filter_options(&root, &output, "copy")).unwrap();
        assert_eq!((result.success_count, result.fail_count), (1, 0));
        assert_eq!(
            std::fs::read(root.join("webp.png")).unwrap(),
            std::fs::read(output.join("webp.png")).unwrap()
        );
    }

    #[test]
    fn options_use_the_frontend_string_values() {
        let root = TempDir::new("filter_values");
        let mut value = serde_json::to_value(filter_options(&root, &root, "copy")).unwrap();
        for action in ["copy", "delete"] {
            value["action"] = json!(action);
            for condition in [
                "min_width",
                "min_height",
                "below_resolution",
                "above_resolution",
            ] {
                value["condition"] = json!(condition);
                let options: FilterOptions = serde_json::from_value(value.clone()).unwrap();
                assert_eq!(serde_json::to_value(options.action).unwrap(), action);
                assert_eq!(serde_json::to_value(options.condition).unwrap(), condition);
            }
        }
        value["action"] = json!("invalid");
        assert!(serde_json::from_value::<FilterOptions>(value).is_err());
    }

    /// 「开始筛选」与逐文件事件、done 属于同一轮；读不出的文件按模式归集到输出或输入目录的 Fail/
    #[test]
    fn start_event_shares_the_run_and_failures_are_archived() {
        let root = TempDir::new("filter_run");
        let (input, output) = (root.join("in"), root.join("out"));
        std::fs::create_dir_all(&input).unwrap();
        image::RgbImage::new(8, 8)
            .save(input.join("small.png"))
            .unwrap();
        std::fs::write(input.join("bad.png"), b"not an image").unwrap();
        let app = tauri::test::mock_app();
        let log = capture_raw_events(app.handle(), EVENT);

        let result = filter_sync(app.handle(), &filter_options(&input, &output, "copy")).unwrap();
        assert_eq!((result.success_count, result.fail_count), (1, 1));
        let events = log.lock().unwrap();
        assert!(events[0]["message"]
            .as_str()
            .unwrap()
            .starts_with("开始筛选"));
        let run_id = events[0]["run_id"].as_u64().unwrap();
        assert!(events.iter().all(|e| e["run_id"] == run_id));
        assert_eq!(events.last().unwrap()["status"], "done");
        assert!(output.join("Fail/bad.png").is_file());
        drop(events);

        let result = filter_sync(app.handle(), &filter_options(&input, &output, "delete")).unwrap();
        assert_eq!((result.success_count, result.fail_count), (1, 1));
        assert!(!input.join("small.png").exists());
        assert!(input.join("Fail/bad.png").is_file());
    }
}
