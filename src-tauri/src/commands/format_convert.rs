use super::{
    collect_files_matching, copy_file_safe, has_extension, is_supported_image_file, path_key_ci,
};
use image::{DynamicImage, RgbaImage};
use serde::{Deserialize, Serialize};
use std::path::Path;

use super::{output_path_for_input, ProcessResult};

use super::batch::{BatchJob, FileBatch, FileOutcome};

static JOB: BatchJob = BatchJob::new("格式转换");

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FormatConvertOptions {
    pub input_path: String,
    pub output_path: String,
    /// 目标格式: "png" | "jpg" | "jpeg" | "bmp" | "webp"
    pub target_format: String,
    #[serde(default)]
    pub recursive: bool,
}

fn open_image(file_path: &Path) -> Result<DynamicImage, String> {
    let ext = file_path
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default();

    if ext == "psd" {
        let bytes = std::fs::read(file_path).map_err(|e| format!("无法读取 PSD 文件: {}", e))?;
        let psd_file =
            psd::Psd::from_bytes(&bytes).map_err(|e| format!("无法解析 PSD 文件: {:?}", e))?;

        let width = psd_file.width();
        let height = psd_file.height();
        let rgba_data = psd_file.rgba();

        let img_buf = RgbaImage::from_raw(width, height, rgba_data).ok_or("无法创建图片缓冲区")?;

        Ok(DynamicImage::ImageRgba8(img_buf))
    } else {
        image::ImageReader::open(file_path)
            .map_err(|e| format!("无法打开图片: {}", e))
            .and_then(|r| {
                r.with_guessed_format()
                    .map_err(|e| format!("无法识别图片格式: {}", e))
            })
            .and_then(|r| r.decode().map_err(|e| format!("无法解码图片: {}", e)))
    }
}

#[tauri::command]
pub async fn convert_format<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    options: FormatConvertOptions,
) -> Result<ProcessResult, String> {
    JOB.run(move || convert_format_sync(&app, &options)).await
}

#[tauri::command]
pub fn cancel_convert() {
    JOB.cancel();
}

fn convert_format_sync<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    options: &FormatConvertOptions,
) -> Result<ProcessResult, String> {
    let input = Path::new(&options.input_path);
    let output_dir = Path::new(&options.output_path);
    if !input.exists() {
        return Err(format!("输入路径不存在: {}", input.display()));
    }
    std::fs::create_dir_all(output_dir).map_err(|e| format!("无法创建输出目录: {}", e))?;
    let files = collect_files_matching(input, options.recursive, Some(output_dir), |p| {
        is_supported_image_file(p) || has_extension(p, &["psd"])
    })?;
    let input_set: std::collections::HashSet<String> =
        files.iter().map(|p| path_key_ci(p)).collect();
    let mut used_outputs: std::collections::HashSet<String> = std::collections::HashSet::new();
    let target_ext = options.target_format.to_lowercase();
    let tgt_normalized = match target_ext.as_str() {
        "jpeg" => "jpg",
        other => other,
    };
    Ok(FileBatch::new(app, "convert-progress", JOB.cancel_flag())
        .no_processing()
        .error_prefix("[错误] ")
        .run(
            &files,
            |item| {
                let src_ext = item
                    .path
                    .extension()
                    .map(|e| e.to_string_lossy().to_lowercase())
                    .unwrap_or_default();
                let src_normalized = match src_ext.as_str() {
                    "jpeg" => "jpg",
                    other => other,
                };
                if src_normalized == tgt_normalized {
                    let dst =
                        super::same_name_output(input, item.path, output_dir, options.recursive)?;
                    let dst_key = path_key_ci(&dst);
                    let is_self = dst_key == path_key_ci(item.path);
                    if !used_outputs.insert(dst_key) && !is_self {
                        return Err("输出文件名与本批其他文件冲突，已跳过".to_string());
                    }
                    copy_file_safe(item.path, &dst)?;
                    return Ok(FileOutcome::unchanged(format!(
                        "[跳过转换] {} (已是 .{} 格式，直接复制)",
                        item.name, target_ext
                    ))
                    .with_status("skipped"));
                }
                item.emit("processing", format!("正在转换: {}", item.name));
                process_convert(
                    item.path,
                    input,
                    output_dir,
                    options.recursive,
                    &target_ext,
                    &input_set,
                    &mut used_outputs,
                )?;
                Ok(FileOutcome::done(format!(
                    "[转换] {} (.{} → .{})",
                    item.name, src_ext, target_ext
                )))
            },
            |c| c.summary("转换完成"),
        ))
}

fn process_convert(
    file_path: &Path,
    input_root: &Path,
    output_dir: &Path,
    recursive: bool,
    target_ext: &str,
    input_files: &std::collections::HashSet<String>,
    used_outputs: &mut std::collections::HashSet<String>,
) -> Result<(), String> {
    let img = open_image(file_path)?;

    let img = match target_ext {
        "jpg" | "jpeg" | "bmp" => DynamicImage::ImageRgb8(img.to_rgb8()),
        // image 0.25 的 WebP 编码器只接受 RGB8/RGBA8，灰度/16 位图需先归一化
        "webp" => {
            if img.color().has_alpha() {
                DynamicImage::ImageRgba8(img.to_rgba8())
            } else {
                DynamicImage::ImageRgb8(img.to_rgb8())
            }
        }
        _ => img,
    };

    let stem = file_path
        .file_stem()
        .ok_or("无效的文件名")?
        .to_string_lossy();
    let new_name = format!("{}.{}", stem, target_ext);
    let output_path =
        output_path_for_input(input_root, file_path, output_dir, &new_name, recursive)?;

    // 原地模式下输出名可能撞上另一张源图（a.jpg 转 png 覆盖已存在的 a.png）
    let out_key = crate::commands::path_key_ci(&output_path);
    if input_files.contains(&out_key) {
        return Err(format!(
            "输出 {} 会覆盖另一张源图，已跳过（请更换输出目录）",
            new_name
        ));
    }
    // 同 stem 不同扩展的输入映射到同一输出名：后到者报错而不是静默覆盖
    if !used_outputs.insert(out_key) {
        return Err(format!("输出 {} 与本批其他文件同名冲突，已跳过", new_name));
    }

    img.save(&output_path)
        .map_err(|e| format!("无法保存图片: {}", e))
}
