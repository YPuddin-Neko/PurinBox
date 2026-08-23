use image::{DynamicImage, RgbaImage};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::Emitter;

use super::{output_path_for_input, ProcessResult, ProgressEvent};

static CANCEL_FLAG: AtomicBool = AtomicBool::new(false);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FormatConvertOptions {
    pub input_path: String,
    pub output_path: String,
    /// 目标格式: "png" | "jpg" | "jpeg" | "bmp" | "webp"
    pub target_format: String,
    #[serde(default)]
    pub recursive: bool,
}

fn is_supported_source(ext: &str) -> bool {
    matches!(
        ext,
        "png" | "jpg" | "jpeg" | "webp" | "bmp" | "tiff" | "tif" | "gif" | "psd"
    )
}

fn collect_convertible_files(
    input: &Path,
    recursive: bool,
    excluded_dir: Option<&Path>,
) -> Result<Vec<std::path::PathBuf>, String> {
    let mut files = Vec::new();

    if input.is_file() {
        files.push(input.to_path_buf());
    } else if input.is_dir() {
        let input_canonical = std::fs::canonicalize(input).ok();
        let excluded = excluded_dir
            .filter(|dir| dir.exists())
            .and_then(|dir| std::fs::canonicalize(dir).ok());
        // 只在输出目录是输入目录的子目录时才排除，两者相同则不排除
        let should_exclude_output = match (input_canonical.as_ref(), excluded.as_ref()) {
            (Some(input_c), Some(excluded_c)) => {
                excluded_c != input_c && excluded_c.starts_with(input_c)
            }
            _ => false,
        };
        let walker = if recursive {
            walkdir::WalkDir::new(input)
        } else {
            walkdir::WalkDir::new(input).max_depth(1)
        };
        for entry in walker.into_iter().filter_map(|e| e.ok()) {
            let p = entry.path();
            if p.is_file() {
                if should_exclude_output {
                    let excluded = excluded.as_ref().expect("checked by should_exclude_output");
                    let normalized =
                        std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
                    if normalized.starts_with(excluded) {
                        continue;
                    }
                }
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

    if recursive {
        files.sort_by(|a, b| a.to_string_lossy().cmp(&b.to_string_lossy()));
    } else {
        files.sort_by(|a, b| a.file_name().cmp(&b.file_name()));
    }
    Ok(files)
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
    // 互斥：页面与工作流节点共用全局取消标志，并发会互吞取消
    static RUNNING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    let _busy = crate::commands::BusyGuard::acquire(&RUNNING, "格式转换")?;

    CANCEL_FLAG.store(false, Ordering::SeqCst);
    tokio::task::spawn_blocking(move || convert_format_sync(&app, &options))
        .await
        .map_err(|e| format!("任务执行失败: {}", e))?
}

#[tauri::command]
pub fn cancel_convert() {
    CANCEL_FLAG.store(true, Ordering::SeqCst);
}

fn convert_format_sync<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    options: &FormatConvertOptions,
) -> Result<ProcessResult, String> {
    let input = Path::new(&options.input_path);
    let output_dir = Path::new(&options.output_path);

    // 前置校验：输入不存在时给出明确原因。工作流中最常见的成因是
    // 上游节点为原地操作、并未产出该目录
    if !input.exists() {
        return Err(format!(
            "输入路径不存在: {}（若在工作流中使用，请检查上游节点是否实际产出了该目录）",
            input.display()
        ));
    }

    if !output_dir.exists() {
        std::fs::create_dir_all(output_dir).map_err(|e| format!("无法创建输出目录: {}", e))?;
    }

    let files = collect_convertible_files(input, options.recursive, Some(output_dir))?;
    let total = files.len() as u32;
    let mut success_count = 0u32;
    let mut fail_count = 0u32;
    let mut errors = Vec::new();

    // 同 stem 不同扩展（a.jpg 与 a.png）会映射到同一输出名，
    // 且原地模式下转换结果可能覆盖另一张源图——两种覆盖都要拦下
    let input_set: std::collections::HashSet<String> =
        files.iter().map(|p| crate::commands::path_key_ci(p)).collect();
    let mut used_outputs: std::collections::HashSet<String> = std::collections::HashSet::new();

    let target_ext = options.target_format.to_lowercase();

    for (i, file_path) in files.iter().enumerate() {
        if CANCEL_FLAG.load(Ordering::SeqCst) {
            let _ = app.emit(
                "convert-progress",
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
        let src_ext = file_path
            .extension()
            .map(|e| e.to_string_lossy().to_lowercase())
            .unwrap_or_default();

        // 跳过已经是目标格式的文件（jpg 和 jpeg 视为同一格式）
        let src_normalized = match src_ext.as_str() {
            "jpeg" => "jpg",
            other => other,
        };
        let tgt_normalized = match target_ext.as_str() {
            "jpeg" => "jpg",
            other => other,
        };
        if src_normalized == tgt_normalized {
            // 已是目标格式：直接复制到输出目录。
            // 工作流中输出目录是下游节点的输入，单纯跳过会造成数据集缺文件。
            let copy_result = crate::commands::output_path_for_input(
                input,
                file_path,
                output_dir,
                &filename,
                options.recursive,
            )
            .and_then(|dst| {
                // 输出目录==输入目录时 dst 就是源文件自身（大小写不敏感判定）：
                // fs::copy 自拷贝会先截断目标，把源文件清成 0 字节
                let dst_key = crate::commands::path_key_ci(&dst);
                if dst_key == crate::commands::path_key_ci(file_path) {
                    used_outputs.insert(dst_key);
                    return Ok(());
                }
                if !used_outputs.insert(dst_key) {
                    return Err("输出文件名与本批其他文件冲突，已跳过".to_string());
                }
                std::fs::copy(file_path, &dst)
                    .map(|_| ())
                    .map_err(|e| format!("复制失败: {}", e))
            });
            match copy_result {
                Ok(_) => {
                    success_count += 1;
                    let _ = app.emit(
                        "convert-progress",
                        ProgressEvent {
                            current: i as u32 + 1,
                            total,
                            filename: filename.clone(),
                            status: "skipped".to_string(),
                            message: format!("[跳过转换] {} (已是 .{} 格式，直接复制)", filename, target_ext),
                            ..Default::default()
                        },
                    );
                }
                Err(e) => {
                    fail_count += 1;
                    let err_msg = format!("{}: {}", filename, e);
                    errors.push(err_msg.clone());
                    let _ = app.emit(
                        "convert-progress",
                        ProgressEvent {
                            current: i as u32 + 1,
                            total,
                            filename: filename.clone(),
                            status: "error".to_string(),
                            message: format!("[错误] {}", err_msg),
                            ..Default::default()
                        },
                    );
                }
            }
            continue;
        }

        let _ = app.emit(
            "convert-progress",
            ProgressEvent {
                current: i as u32 + 1,
                total,
                filename: filename.clone(),
                status: "processing".to_string(),
                message: format!("正在转换: {}", filename),
                ..Default::default()
            },
        );

        match process_convert(
            file_path,
            input,
            output_dir,
            options,
            &target_ext,
            &input_set,
            &mut used_outputs,
        ) {
            Ok(_) => {
                success_count += 1;
                let _ = app.emit(
                    "convert-progress",
                    ProgressEvent {
                        current: i as u32 + 1,
                        total,
                        filename: filename.clone(),
                        status: "success".to_string(),
                        message: format!("[转换] {} (.{} → .{})", filename, src_ext, target_ext),
                        ..Default::default()
                    },
                );
            }
            Err(e) => {
                fail_count += 1;
                let err_msg = format!("{}: {}", filename, e);
                errors.push(err_msg.clone());
                let _ = app.emit(
                    "convert-progress",
                    ProgressEvent {
                        current: i as u32 + 1,
                        total,
                        filename: filename.clone(),
                        status: "error".to_string(),
                        message: format!("[错误] {}", err_msg),
                        ..Default::default()
                    },
                );
            }
        }
    }

    // 取消路径已发过"已取消"的 done 事件，这里不再发完成事件覆盖它
    if !CANCEL_FLAG.load(Ordering::SeqCst) {
        let _ = app.emit(
            "convert-progress",
            ProgressEvent {
                current: total,
                total,
                filename: String::new(),
                status: "done".to_string(),
                message: format!(
                    "转换完成: 成功 {}, 失败 {}, 共 {}",
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

#[allow(clippy::too_many_arguments)]
fn process_convert(
    file_path: &Path,
    input_root: &Path,
    output_dir: &Path,
    options: &FormatConvertOptions,
    target_ext: &str,
    input_files: &std::collections::HashSet<String>,
    used_outputs: &mut std::collections::HashSet<String>,
) -> Result<String, String> {
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
    let output_path = output_path_for_input(
        input_root,
        file_path,
        output_dir,
        &new_name,
        options.recursive,
    )?;

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
        .map_err(|e| format!("无法保存图片: {}", e))?;
    Ok(new_name)
}
