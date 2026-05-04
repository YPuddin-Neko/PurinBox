use image::{DynamicImage, RgbaImage};
use serde::{Deserialize, Serialize};
use std::path::Path;
use tauri::Emitter;

use super::{ProcessResult, ProgressEvent};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FormatConvertOptions {
    pub input_path: String,
    pub output_path: String,
    /// 目标格式: "png" | "jpg" | "jpeg" | "bmp" | "webp"
    pub target_format: String,
}

fn is_supported_source(ext: &str) -> bool {
    matches!(ext, "png" | "jpg" | "jpeg" | "webp" | "bmp" | "tiff" | "tif" | "gif" | "psd")
}

fn collect_convertible_files(input: &Path) -> Result<Vec<std::path::PathBuf>, String> {
    let mut files = Vec::new();

    if input.is_file() {
        files.push(input.to_path_buf());
    } else if input.is_dir() {
        for entry in walkdir::WalkDir::new(input)
            .max_depth(1)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            let p = entry.path();
            if p.is_file() {
                if let Some(ext) = p.extension() {
                    let ext_lower = ext.to_string_lossy().to_lowercase();
                    if is_supported_source(&ext_lower) {
                        files.push(p.to_path_buf());
                    }
                }
            }
        }
    } else {
        return Err(format!("输入路径无效: {}", input.display()));
    }

    files.sort_by(|a, b| a.file_name().cmp(&b.file_name()));
    Ok(files)
}

fn open_image(file_path: &Path) -> Result<DynamicImage, String> {
    let ext = file_path.extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default();

    if ext == "psd" {
        let bytes = std::fs::read(file_path)
            .map_err(|e| format!("无法读取 PSD 文件: {}", e))?;
        let psd_file = psd::Psd::from_bytes(&bytes)
            .map_err(|e| format!("无法解析 PSD 文件: {:?}", e))?;

        let width = psd_file.width();
        let height = psd_file.height();
        let rgba_data = psd_file.rgba();

        let img_buf = RgbaImage::from_raw(width, height, rgba_data)
            .ok_or("无法创建图片缓冲区")?;

        Ok(DynamicImage::ImageRgba8(img_buf))
    } else {
        image::open(file_path)
            .map_err(|e| format!("无法打开图片: {}", e))
    }
}

#[tauri::command]
pub async fn convert_format(app: tauri::AppHandle, options: FormatConvertOptions) -> Result<ProcessResult, String> {
    tokio::task::spawn_blocking(move || {
        convert_format_sync(&app, &options)
    })
    .await
    .map_err(|e| format!("任务执行失败: {}", e))?
}

fn convert_format_sync(app: &tauri::AppHandle, options: &FormatConvertOptions) -> Result<ProcessResult, String> {
    let input = Path::new(&options.input_path);
    let output_dir = Path::new(&options.output_path);

    if !output_dir.exists() {
        std::fs::create_dir_all(output_dir)
            .map_err(|e| format!("无法创建输出目录: {}", e))?;
    }

    let files = collect_convertible_files(input)?;
    let total = files.len() as u32;
    let mut success_count = 0u32;
    let mut fail_count = 0u32;
    let mut errors = Vec::new();

    let target_ext = options.target_format.to_lowercase();

    for (i, file_path) in files.iter().enumerate() {
        let filename = file_path.file_name().unwrap_or_default().to_string_lossy().to_string();
        let src_ext = file_path.extension()
            .map(|e| e.to_string_lossy().to_lowercase())
            .unwrap_or_default();

        // 跳过已经是目标格式的文件（jpg 和 jpeg 视为同一格式）
        let src_normalized = match src_ext.as_str() { "jpeg" => "jpg", other => other };
        let tgt_normalized = match target_ext.as_str() { "jpeg" => "jpg", other => other };
        if src_normalized == tgt_normalized {
            let _ = app.emit("convert-progress", ProgressEvent {
                current: i as u32 + 1,
                total,
                filename: filename.clone(),
                status: "skipped".to_string(),
                message: format!("[跳过] {} (已是 .{} 格式)", filename, target_ext),
            });
            continue;
        }

        let _ = app.emit("convert-progress", ProgressEvent {
            current: i as u32 + 1,
            total,
            filename: filename.clone(),
            status: "processing".to_string(),
            message: format!("正在转换: {}", filename),
        });

        match process_convert(file_path, output_dir, &target_ext) {
            Ok(_) => {
                success_count += 1;
                let _ = app.emit("convert-progress", ProgressEvent {
                    current: i as u32 + 1,
                    total,
                    filename: filename.clone(),
                    status: "success".to_string(),
                    message: format!("[转换] {} (.{} → .{})", filename, src_ext, target_ext),
                });
            }
            Err(e) => {
                fail_count += 1;
                let err_msg = format!("{}: {}", filename, e);
                errors.push(err_msg.clone());
                let _ = app.emit("convert-progress", ProgressEvent {
                    current: i as u32 + 1,
                    total,
                    filename: filename.clone(),
                    status: "error".to_string(),
                    message: format!("[错误] {}", err_msg),
                });
            }
        }
    }

    let _ = app.emit("convert-progress", ProgressEvent {
        current: total,
        total,
        filename: String::new(),
        status: "done".to_string(),
        message: format!("转换完成: 成功 {}, 失败 {}, 共 {}", success_count, fail_count, total),
    });

    Ok(ProcessResult { success_count, fail_count, total, errors })
}

fn process_convert(file_path: &Path, output_dir: &Path, target_ext: &str) -> Result<String, String> {
    let img = open_image(file_path)?;

    let img = match target_ext {
        "jpg" | "jpeg" | "bmp" => {
            DynamicImage::ImageRgb8(img.to_rgb8())
        }
        _ => img,
    };

    let stem = file_path.file_stem().ok_or("无效的文件名")?.to_string_lossy();
    let new_name = format!("{}.{}", stem, target_ext);
    let output_path = output_dir.join(&new_name);

    img.save(&output_path).map_err(|e| format!("无法保存图片: {}", e))?;
    Ok(new_name)
}
