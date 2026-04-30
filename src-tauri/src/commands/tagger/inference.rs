use image::GenericImageView;
use ort::ep::ExecutionProvider;
use ort::session::Session;
use std::path::Path;
use tauri::Emitter;

use super::{TagCategory, TagDefinition, TaggerOptions, ProcessResult, ProgressEvent};
use crate::commands::collect_image_files;

/// 检测 CUDA 是否可用，返回 (可用, 详情)
pub fn check_cuda() -> (bool, String) {
    // 检查 ONNX Runtime 是否成功加载
    let rt_info = format!("ONNX Runtime: {}", ort::info());

    match ort::execution_providers::CUDAExecutionProvider::default().is_available() {
        Ok(true) => (true, format!("{}\nCUDA ExecutionProvider: 可用", rt_info)),
        Ok(false) => (false, format!(
            "{}\nCUDA ExecutionProvider: 不可用\n\n可能原因:\n\
            1. 当前加载的 ONNX Runtime 不含 CUDA 支持\n\
            2. 请安装 onnxruntime-gpu: pip install onnxruntime-gpu\n\
            3. 或将 onnxruntime_providers_cuda.dll 放到程序目录",
            rt_info
        )),
        Err(e) => (false, format!(
            "{}\nCUDA 检测异常: {}\n\n请确认已安装 onnxruntime-gpu (pip install onnxruntime-gpu)",
            rt_info, e
        )),
    }
}

/// 从 CSV 文件加载标签定义
pub fn load_tags(csv_path: &Path) -> Result<Vec<TagDefinition>, String> {
    let mut reader = csv::Reader::from_path(csv_path)
        .map_err(|e| format!("无法读取标签文件: {}", e))?;

    let mut tags = Vec::new();
    for result in reader.records() {
        let record = result.map_err(|e| format!("CSV 解析错误: {}", e))?;
        if record.len() >= 3 {
            let name = record.get(1).unwrap_or("").to_string();
            let cat_id: i32 = record.get(2).unwrap_or("0").parse().unwrap_or(0);
            if let Some(category) = TagCategory::from_csv_id(cat_id) {
                tags.push(TagDefinition { name, category });
            }
        }
    }
    Ok(tags)
}

/// 预处理图片: [1, size, size, 3] BGR float32 [0,255]
fn preprocess_image(img_path: &Path, target_size: u32) -> Result<Vec<f32>, String> {
    let img = image::open(img_path).map_err(|e| format!("无法打开图片: {}", e))?;
    let (w, h) = img.dimensions();
    let size = target_size;

    let scale = (size as f32 / w as f32).min(size as f32 / h as f32);
    let new_w = (w as f32 * scale) as u32;
    let new_h = (h as f32 * scale) as u32;
    let pad_x = (size - new_w) / 2;
    let pad_y = (size - new_h) / 2;

    let resized = img.resize_exact(new_w, new_h, image::imageops::FilterType::Lanczos3);
    let rgb = resized.to_rgb8();

    let total = (size * size * 3) as usize;
    let mut input = vec![255.0f32; total]; // white padding

    for y in 0..new_h {
        for x in 0..new_w {
            let px = rgb.get_pixel(x, y);
            let out_x = (x + pad_x) as usize;
            let out_y = (y + pad_y) as usize;
            let idx = (out_y * size as usize + out_x) * 3;
            input[idx] = px[2] as f32;     // B
            input[idx + 1] = px[1] as f32; // G
            input[idx + 2] = px[0] as f32; // R
        }
    }
    Ok(input)
}

/// 执行批量打标
pub fn run_tagging(
    app: &tauri::AppHandle,
    options: &TaggerOptions,
    model_path: &Path,
    tag_defs: &[TagDefinition],
    input_size: u32,
) -> Result<ProcessResult, String> {
    let _ = app.emit("tagger-progress", ProgressEvent {
        current: 0, total: 0, filename: String::new(),
        status: "info".to_string(),
        message: format!("正在加载模型 (GPU: {})...", if options.use_gpu { "启用" } else { "禁用" }),
    });

    let mut builder = Session::builder()
        .map_err(|e| format!("创建会话失败: {}", e))?;

    if options.use_gpu {
        let (cuda_ok, cuda_detail) = check_cuda();
        if cuda_ok {
            builder = builder
                .with_execution_providers([
                    ort::execution_providers::CUDAExecutionProvider::default().build()
                ])
                .map_err(|e| format!("CUDA 初始化失败: {}", e))?;
            let _ = app.emit("tagger-progress", ProgressEvent {
                current: 0, total: 0, filename: String::new(),
                status: "success".to_string(),
                message: "✓ CUDA 加速已启用".to_string(),
            });
        } else {
            let _ = app.emit("tagger-progress", ProgressEvent {
                current: 0, total: 0, filename: String::new(),
                status: "error".to_string(),
                message: format!("✗ CUDA 不可用，使用 CPU\n{}", cuda_detail),
            });
        }
    }

    let mut session = builder.commit_from_file(model_path)
        .map_err(|e| format!("加载模型失败: {}", e))?;

    let _ = app.emit("tagger-progress", ProgressEvent {
        current: 0, total: 0, filename: String::new(),
        status: "success".to_string(),
        message: "模型加载完成，开始打标...".to_string(),
    });

    let input_dir = Path::new(&options.input_path);
    let files = collect_image_files(input_dir)?;
    let total = files.len() as u32;
    let mut success_count = 0u32;
    let mut fail_count = 0u32;
    let mut errors = Vec::new();

    let enabled_cats: Vec<&str> = options.enabled_categories.iter().map(|s| s.as_str()).collect();

    for (i, file_path) in files.iter().enumerate() {
        let filename = file_path.file_name().unwrap_or_default().to_string_lossy().to_string();

        let _ = app.emit("tagger-progress", ProgressEvent {
            current: i as u32 + 1, total,
            filename: filename.clone(),
            status: "processing".to_string(),
            message: format!("正在处理: {} ({}/{})", filename, i + 1, total),
        });

        match tag_single_image(&mut session, file_path, tag_defs, input_size, options, &enabled_cats) {
            Ok(tag_count) => {
                success_count += 1;
                let _ = app.emit("tagger-progress", ProgressEvent {
                    current: i as u32 + 1, total,
                    filename: filename.clone(),
                    status: "success".to_string(),
                    message: format!("[完成] {} → {} 个标签", filename, tag_count),
                });
            }
            Err(e) => {
                fail_count += 1;
                let err_msg = format!("{}: {}", filename, e);
                errors.push(err_msg.clone());
                let _ = app.emit("tagger-progress", ProgressEvent {
                    current: i as u32 + 1, total,
                    filename: filename.clone(),
                    status: "error".to_string(),
                    message: format!("[错误] {}", err_msg),
                });
            }
        }
    }

    let _ = app.emit("tagger-progress", ProgressEvent {
        current: total, total, filename: String::new(),
        status: "done".to_string(),
        message: format!("打标完成: 成功 {}, 失败 {}, 共 {}", success_count, fail_count, total),
    });

    Ok(ProcessResult { success_count, fail_count, total, errors })
}

fn tag_single_image(
    session: &mut Session,
    img_path: &Path,
    tag_defs: &[TagDefinition],
    input_size: u32,
    options: &TaggerOptions,
    enabled_cats: &[&str],
) -> Result<usize, String> {
    let input_data = preprocess_image(img_path, input_size)?;
    let size = input_size as i64;

    // 使用 Tensor::from_array 创建输入 [1, size, size, 3]
    let input_tensor = ort::value::Tensor::from_array((vec![1i64, size, size, 3], input_data))
        .map_err(|e| format!("创建输入张量失败: {}", e))?;

    let outputs = session.run(ort::inputs![input_tensor])
        .map_err(|e| format!("推理失败: {}", e))?;

    // 提取第一个输出
    let (_, output_value) = outputs.iter().next().ok_or("无输出结果")?;
    let output_tensor = output_value.try_extract_tensor::<f32>()
        .map_err(|e| format!("提取输出失败: {}", e))?;

    let pred_slice = output_tensor.1;

    let mut selected_tags: Vec<String> = Vec::new();

    for (idx, &confidence) in pred_slice.iter().enumerate() {
        if idx >= tag_defs.len() { break; }
        let tag = &tag_defs[idx];
        let cat_key = tag.category.key();

        if !enabled_cats.contains(&cat_key) { continue; }

        let threshold = match tag.category {
            TagCategory::Character => options.character_threshold,
            _ => options.general_threshold,
        };

        if confidence >= threshold {
            selected_tags.push(tag.name.clone());
        }
    }

    // 保存到同名 .txt
    let stem = img_path.file_stem().ok_or("无效的文件名")?.to_string_lossy();
    let parent = img_path.parent().unwrap_or(Path::new("."));
    let txt_path = parent.join(format!("{}.txt", stem));
    let tag_text = selected_tags.join(", ");
    std::fs::write(&txt_path, &tag_text).map_err(|e| format!("写入标签失败: {}", e))?;

    Ok(selected_tags.len())
}
