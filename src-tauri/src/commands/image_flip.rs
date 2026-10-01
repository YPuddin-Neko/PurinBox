use serde::{Deserialize, Serialize};
use std::path::Path;

use super::batch::{BatchJob, FileBatch, FileOutcome};
use super::image_io::{load_image, save_like_source};
use super::{collect_image_files_with_recursive_excluding, same_name_output, ProcessResult};

static JOB: BatchJob = BatchJob::new("翻转");

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
    JOB.run(move || flip_images_sync(&app, &options)).await
}

#[tauri::command]
pub fn cancel_flip() {
    JOB.cancel();
}

fn flip_images_sync<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    options: &FlipOptions,
) -> Result<ProcessResult, String> {
    let direction_label = match options.direction.as_str() {
        "horizontal" => "水平翻转",
        "vertical" => "垂直翻转",
        "both" => "双向翻转",
        _ => return Err("无效的翻转方向".to_string()),
    };
    let input = Path::new(&options.input_path);
    let output_dir = Path::new(&options.output_path);

    std::fs::create_dir_all(output_dir).map_err(|e| format!("无法创建输出目录: {}", e))?;

    let files =
        collect_image_files_with_recursive_excluding(input, options.recursive, Some(output_dir))?;

    Ok(FileBatch::new(app, "flip-progress", JOB.cancel_flag())
        .error_prefix("[失败] ")
        .run(
            &files,
            |item| {
                process_flip(item.path, input, output_dir, options)?;
                Ok(FileOutcome::done(format!(
                    "[{}] {} ✓",
                    direction_label, item.name
                )))
            },
            |c| c.summary("处理完成"),
        ))
}

fn process_flip(
    file_path: &Path,
    input_root: &Path,
    output_dir: &Path,
    options: &FlipOptions,
) -> Result<(), String> {
    let (img, source) = load_image(file_path)?;

    let flipped = match options.direction.as_str() {
        "horizontal" => img.fliph(),
        "vertical" => img.flipv(),
        "both" => img.fliph().flipv(),
        _ => unreachable!("validated direction"),
    };

    let output_path = same_name_output(input_root, file_path, output_dir, options.recursive)?;
    save_like_source(flipped, &output_path, &source)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::batch::capture_events;
    use serde_json::json;

    /// 迁移到批处理驱动前后，翻转命令发出的事件序列与返回值必须逐字一致
    #[tokio::test]
    async fn flip_events_are_unchanged() {
        let root =
            std::env::temp_dir().join(format!("purinbox_flip_events_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let (input, out) = (root.join("in"), root.join("out"));
        std::fs::create_dir_all(&input).unwrap();
        image::RgbImage::from_pixel(8, 8, image::Rgb([1, 2, 3]))
            .save(input.join("a.png"))
            .unwrap();
        image::RgbImage::from_pixel(8, 4, image::Rgb([4, 5, 6]))
            .save(input.join("b.jpg"))
            .unwrap();
        std::fs::write(input.join("c_bad.png"), b"not an image").unwrap();
        let decode_err = load_image(&input.join("c_bad.png")).err().unwrap();

        let app = tauri::test::mock_app();
        let events = capture_events(app.handle(), "flip-progress");

        let options: FlipOptions = serde_json::from_value(json!({
            "input_path": input.to_string_lossy(),
            "output_path": out.to_string_lossy(),
            "direction": "horizontal",
        }))
        .unwrap();
        let result = flip_images(app.handle().clone(), options).await.unwrap();

        fn ev(current: u32, filename: &str, status: &str, message: String) -> serde_json::Value {
            json!({
                "current": current,
                "total": 3,
                "filename": filename,
                "status": status,
                "message": message,
            })
        }
        assert_eq!(
            *events.lock().unwrap(),
            vec![
                ev(1, "a.png", "processing", "正在处理: a.png".into()),
                ev(1, "a.png", "success", "[水平翻转] a.png ✓".into()),
                ev(2, "b.jpg", "processing", "正在处理: b.jpg".into()),
                ev(2, "b.jpg", "success", "[水平翻转] b.jpg ✓".into()),
                ev(3, "c_bad.png", "processing", "正在处理: c_bad.png".into()),
                ev(
                    3,
                    "c_bad.png",
                    "error",
                    format!("[失败] c_bad.png: {}", decode_err)
                ),
                ev(3, "", "done", "处理完成: 成功 2, 失败 1, 共 3".into()),
            ]
        );
        assert_eq!(
            (result.success_count, result.fail_count, result.total),
            (2, 1, 3)
        );
        assert_eq!(result.errors, vec![format!("c_bad.png: {}", decode_err)]);
        assert!(out.join("a.png").exists() && out.join("b.jpg").exists());
        let _ = std::fs::remove_dir_all(&root);
    }
}
