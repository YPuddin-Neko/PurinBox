use serde::{Deserialize, Serialize};
use std::path::Path;

use super::batch::{BatchJob, FileBatch, FileOutcome};
use super::image_io::{load_image, save_like_source};
use super::{collect_image_files_with_recursive_excluding, same_name_output, ProcessResult};

static JOB: BatchJob = BatchJob::new("翻转");

/// 翻转方向，JSON 取值为小写名称
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FlipDirection {
    Horizontal,
    Vertical,
    Both,
}

impl FlipDirection {
    fn label(self) -> &'static str {
        match self {
            FlipDirection::Horizontal => "水平翻转",
            FlipDirection::Vertical => "垂直翻转",
            FlipDirection::Both => "双向翻转",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlipOptions {
    pub input_path: String,
    pub output_path: String,
    pub direction: FlipDirection,
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
    let input = Path::new(&options.input_path);
    let output_dir = Path::new(&options.output_path);

    std::fs::create_dir_all(output_dir).map_err(|e| format!("无法创建输出目录: {}", e))?;

    let files =
        collect_image_files_with_recursive_excluding(input, options.recursive, Some(output_dir))?;

    Ok(FileBatch::new(app, "flip-progress", JOB.cancel_flag())
        .error_prefix("[失败] ")
        .archive_failures(input, output_dir, options.recursive)
        .run(
            &files,
            |item| {
                process_flip(item.path, input, output_dir, options)?;
                Ok(FileOutcome::done(format!(
                    "[{}] {} ✓",
                    options.direction.label(),
                    item.name
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

    let flipped = match options.direction {
        FlipDirection::Horizontal => img.fliph(),
        FlipDirection::Vertical => img.flipv(),
        FlipDirection::Both => img.fliph().flipv(),
    };

    let output_path = same_name_output(input_root, file_path, output_dir, options.recursive)?;
    save_like_source(flipped, &output_path, &source)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::batch::capture_events;
    use crate::commands::test_support::TempDir;
    use serde_json::json;

    fn flip_options(input: &Path, output: &Path, direction: &str) -> FlipOptions {
        serde_json::from_value(json!({
            "input_path": input.to_string_lossy(),
            "output_path": output.to_string_lossy(),
            "direction": direction,
        }))
        .unwrap()
    }

    /// 翻转命令的完整事件序列与返回值；读不出的文件复制进输出目录的 Fail/
    #[tokio::test]
    async fn flip_events_and_failure_archive() {
        let root = TempDir::new("flip_events");
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

        let result = flip_images(
            app.handle().clone(),
            flip_options(&input, &out, "horizontal"),
        )
        .await
        .unwrap();

        fn ev(
            current: u32,
            total: u32,
            filename: &str,
            status: &str,
            message: String,
        ) -> serde_json::Value {
            json!({
                "current": current,
                "total": total,
                "filename": filename,
                "status": status,
                "message": message,
            })
        }
        assert_eq!(
            *events.lock().unwrap(),
            vec![
                ev(1, 3, "a.png", "processing", "正在处理: a.png".into()),
                ev(1, 3, "a.png", "success", "[水平翻转] a.png ✓".into()),
                ev(2, 3, "b.jpg", "processing", "正在处理: b.jpg".into()),
                ev(2, 3, "b.jpg", "success", "[水平翻转] b.jpg ✓".into()),
                ev(
                    3,
                    3,
                    "c_bad.png",
                    "processing",
                    "正在处理: c_bad.png".into()
                ),
                ev(
                    3,
                    3,
                    "c_bad.png",
                    "error",
                    format!("[失败] c_bad.png: {}", decode_err)
                ),
                ev(
                    0,
                    0,
                    "",
                    "info",
                    "已将 1 个失败文件复制到 Fail/ 文件夹".into()
                ),
                ev(3, 3, "", "done", "处理完成: 成功 2, 失败 1, 共 3".into()),
            ]
        );
        assert_eq!(
            (result.success_count, result.fail_count, result.total),
            (2, 1, 3)
        );
        assert_eq!(result.errors, vec![format!("c_bad.png: {}", decode_err)]);
        assert!(out.join("a.png").exists() && out.join("b.jpg").exists());
        assert_eq!(
            std::fs::read(out.join("Fail/c_bad.png")).unwrap(),
            b"not an image"
        );
    }

    /// 单选一个扩展名不在列表里的文件（.jfif）照常处理，按实际格式写回
    #[tokio::test]
    async fn single_file_with_unlisted_extension_is_processed() {
        let root = TempDir::new("flip_jfif");
        let source = root.join("photo.jfif");
        image::RgbImage::from_fn(6, 4, |x, _| image::Rgb([(x * 40) as u8, 0, 0]))
            .save_with_format(&source, image::ImageFormat::Jpeg)
            .unwrap();
        let out = root.join("out");
        let app = tauri::test::mock_app();
        let result = flip_images(
            app.handle().clone(),
            flip_options(&source, &out, "vertical"),
        )
        .await
        .unwrap();
        assert_eq!((result.success_count, result.fail_count), (1, 0));
        let bytes = std::fs::read(out.join("photo.jfif")).unwrap();
        assert_eq!(
            image::guess_format(&bytes).unwrap(),
            image::ImageFormat::Jpeg
        );
    }

    #[test]
    fn directions_use_the_frontend_string_values() {
        let root = TempDir::new("flip_values");
        for direction in ["horizontal", "vertical", "both"] {
            let options = flip_options(&root, &root, direction);
            assert_eq!(serde_json::to_value(options.direction).unwrap(), direction);
        }
        assert!(serde_json::from_value::<FlipOptions>(json!({
            "input_path": "/i", "output_path": "/o", "direction": "diagonal",
        }))
        .is_err());
    }
}
