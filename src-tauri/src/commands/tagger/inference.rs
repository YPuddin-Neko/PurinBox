use image::GenericImageView;
use ort::session::Session;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::Emitter;

use super::{TagCategory, TagDefinition, TaggerOptions, ProcessResult, ProgressEvent, OnnxModelInfo};
use crate::commands::collect_image_files;

/// 全局打标取消标志
static TAGGING_CANCELLED: AtomicBool = AtomicBool::new(false);

/// 取消打标
pub fn cancel_tagging() {
    TAGGING_CANCELLED.store(true, Ordering::SeqCst);
}

/// 重置取消标志（开始新任务前调用）
pub fn reset_tagging_cancel() {
    TAGGING_CANCELLED.store(false, Ordering::SeqCst);
}

/// 检查是否已取消
pub fn is_tagging_cancelled() -> bool {
    TAGGING_CANCELLED.load(Ordering::SeqCst)
}


/// 在 Windows 上获取 nvidia-smi 的完整路径
fn get_nvidia_smi_path() -> String {
    #[cfg(target_os = "windows")]
    {
        // 常见的 nvidia-smi 路径
        let candidates = [
            r"C:\Windows\System32\nvidia-smi.exe",
            r"C:\Program Files\NVIDIA Corporation\NVSMI\nvidia-smi.exe",
        ];
        for path in &candidates {
            if std::path::Path::new(path).exists() {
                return path.to_string();
            }
        }
        // 如果从 PATH 环境变量也能找到就用 PATH
        if let Ok(output) = run_hidden_cmd("where", &["nvidia-smi"]) {
            if let Some(first_line) = output.lines().next() {
                let trimmed = first_line.trim();
                if !trimmed.is_empty() && std::path::Path::new(trimmed).exists() {
                    return trimmed.to_string();
                }
            }
        }
    }
    "nvidia-smi".to_string()
}

/// 运行命令并隐藏 Windows 控制台窗口，返回 stdout 或错误信息
fn run_hidden_cmd(program: &str, args: &[&str]) -> Result<String, String> {
    let mut cmd = std::process::Command::new(program);
    cmd.args(args);

    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
    }

    match cmd.output() {
        Ok(output) if output.status.success() => {
            Ok(String::from_utf8_lossy(&output.stdout).to_string())
        }
        Ok(output) => {
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();
            Err(format!("exit code: {:?}, stderr: {}", output.status.code(), stderr))
        }
        Err(e) => Err(format!("{}", e)),
    }
}

/// 公开接口：检测 NVIDIA 环境
pub fn detect_nvidia_env_pub(lines: &mut Vec<String>) -> bool {
    detect_nvidia_env(lines)
}

/// 公开接口：检测 CUDA Toolkit
pub fn detect_cuda_toolkit_pub(lines: &mut Vec<String>) {
    detect_cuda_toolkit(lines)
}

/// 检测 NVIDIA 驱动和 GPU 信息
fn detect_nvidia_env(lines: &mut Vec<String>) -> bool {
    let smi_path = get_nvidia_smi_path();
    lines.push(format!("nvidia-smi 路径: {}", smi_path));

    // 查询 GPU 名称和驱动版本
    match run_hidden_cmd(&smi_path, &["--query-gpu=name,driver_version", "--format=csv,noheader,nounits"]) {
        Ok(stdout) => {
            if let Some(line) = stdout.lines().next() {
                let parts: Vec<&str> = line.split(',').map(|s| s.trim()).collect();
                let gpu_name = parts.first().unwrap_or(&"Unknown");
                let driver_ver = parts.get(1).unwrap_or(&"Unknown");
                lines.push(format!("NVIDIA GPU: {} (驱动 v{})", gpu_name, driver_ver));
            }

            // 查 CUDA 版本（在 nvidia-smi 的完整输出里）
            if let Ok(full_output) = run_hidden_cmd(&smi_path, &[]) {
                if let Some(cuda_pos) = full_output.find("CUDA Version:") {
                    let rest = &full_output[cuda_pos + 14..];
                    let cuda_ver = rest.split_whitespace().next().unwrap_or("?");
                    lines.push(format!("CUDA 驱动版本: {}", cuda_ver));
                }
            }
            true
        }
        Err(err) => {
            lines.push(format!("NVIDIA GPU: 未检测到 ({})", err));
            false
        }
    }
}

/// 检测 CUDA Toolkit (nvcc)
fn detect_cuda_toolkit(lines: &mut Vec<String>) {
    // 尝试 nvcc
    if let Ok(output) = run_hidden_cmd("nvcc", &["--version"]) {
        // 解析 "release X.Y" 字样
        if let Some(pos) = output.find("release ") {
            let ver = output[pos + 8..].split(',').next().unwrap_or("?").trim();
            lines.push(format!("CUDA Toolkit: {} (nvcc)", ver));
            return;
        }
    }

    // Windows: 检查常见的 CUDA 安装路径
    #[cfg(target_os = "windows")]
    {
        let cuda_path = std::env::var("CUDA_PATH").ok();
        if let Some(ref cp) = cuda_path {
            if std::path::Path::new(cp).exists() {
                // 尝试从目录名解析版本
                let ver = std::path::Path::new(cp)
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("unknown");
                lines.push(format!("CUDA Toolkit: {} (CUDA_PATH={})", ver, cp));
                return;
            }
        }

        // 扫描 Program Files
        if let Ok(entries) = std::fs::read_dir(r"C:\Program Files\NVIDIA GPU Computing Toolkit\CUDA") {
            let mut versions: Vec<String> = entries
                .filter_map(|e| e.ok())
                .filter(|e| e.path().is_dir())
                .filter_map(|e| e.file_name().into_string().ok())
                .collect();
            versions.sort();
            if let Some(latest) = versions.last() {
                lines.push(format!("CUDA Toolkit: {} (已安装)", latest));
                return;
            }
        }
    }

    lines.push("CUDA Toolkit: 未检测到".into());
}

/// 自动检测 ONNX 模型的输入信息
pub fn detect_model_info(model_path: &str) -> Result<OnnxModelInfo, String> {
    let session = Session::builder()
        .map_err(|e| format!("创建会话失败: {}", e))?
        .commit_from_file(model_path)
        .map_err(|e| format!("加载模型失败: {}\n\n请确认文件是有效的 ONNX 模型", e))?;

    let inputs = session.inputs();
    if inputs.is_empty() {
        return Err("模型没有输入节点".into());
    }

    let input = &inputs[0];
    let dtype = input.dtype();

    // 从 ValueType::Tensor 提取 shape
    let shape_ref = dtype.tensor_shape()
        .ok_or("输入不是 Tensor 类型")?;

    // Shape 是 SmallVec<i64>
    let shape: Vec<i64> = shape_ref.iter().copied().collect();

    if shape.len() != 4 {
        return Err(format!(
            "不支持的输入形状: {:?}\n预期 4 维 [Batch, H, W, C] 或 [Batch, C, H, W]",
            shape
        ));
    }

    // 判断 NHWC 还是 NCHW
    // NHWC: [1, H, W, 3]  -> shape[3] == 3
    // NCHW: [1, 3, H, W]  -> shape[1] == 3
    let (input_format, input_size, channels) = if shape[3] == 3 || shape[3] == 1 {
        // NHWC
        ("NHWC".to_string(), shape[1] as u32, shape[3])
    } else if shape[1] == 3 || shape[1] == 1 {
        // NCHW
        ("NCHW".to_string(), shape[2] as u32, shape[1])
    } else {
        // 猜测：取较小维度作为 channels
        if shape[1] < shape[3] {
            ("NCHW".to_string(), shape[2] as u32, shape[1])
        } else {
            ("NHWC".to_string(), shape[1] as u32, shape[3])
        }
    };

    Ok(OnnxModelInfo {
        input_size,
        input_format,
        input_shape: shape,
        channels,
    })
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

/// 从 JSON 文件加载标签定义 (CL Tagger 格式)
pub fn load_tags_json(json_path: &Path) -> Result<Vec<TagDefinition>, String> {
    let content = std::fs::read_to_string(json_path)
        .map_err(|e| format!("无法读取标签文件: {}", e))?;

    let map: std::collections::BTreeMap<String, serde_json::Value> =
        serde_json::from_str(&content)
            .map_err(|e| format!("JSON 解析错误: {}", e))?;

    let mut tags: Vec<(usize, TagDefinition)> = Vec::new();
    for (idx_str, val) in &map {
        let idx: usize = idx_str.parse().unwrap_or(0);
        let tag_name = val.get("tag").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let cat_str = val.get("category").and_then(|v| v.as_str()).unwrap_or("General");

        let category = match cat_str {
            "General" => TagCategory::General,
            "Artist" => TagCategory::Artist,
            "Copyright" => TagCategory::Copyright,
            "Character" => TagCategory::Character,
            "Meta" => TagCategory::Meta,
            "Rating" => TagCategory::Rating,
            _ => TagCategory::General,
        };

        tags.push((idx, TagDefinition { name: tag_name, category }));
    }

    tags.sort_by_key(|(idx, _)| *idx);
    Ok(tags.into_iter().map(|(_, td)| td).collect())
}

/// 预处理图片 NHWC: [1, size, size, 3] BGR float32 [0,255]
fn preprocess_image_nhwc(img_path: &Path, target_size: u32) -> Result<(Vec<i64>, Vec<f32>), String> {
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
    let s = size as i64;
    Ok((vec![1, s, s, 3], input))
}

/// 预处理图片 NCHW: [1, 3, size, size] BGR float32 [0,255]
fn preprocess_image_nchw(img_path: &Path, target_size: u32) -> Result<(Vec<i64>, Vec<f32>), String> {
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

    let plane = (size * size) as usize;
    let mut input = vec![255.0f32; plane * 3]; // white padding (3 channels)

    for y in 0..new_h {
        for x in 0..new_w {
            let px = rgb.get_pixel(x, y);
            let out_x = (x + pad_x) as usize;
            let out_y = (y + pad_y) as usize;
            let pixel_idx = out_y * size as usize + out_x;
            input[pixel_idx] = px[2] as f32;             // B channel (plane 0)
            input[plane + pixel_idx] = px[1] as f32;     // G channel (plane 1)
            input[plane * 2 + pixel_idx] = px[0] as f32; // R channel (plane 2)
        }
    }
    let s = size as i64;
    Ok((vec![1, 3, s, s], input))
}

/// 执行批量打标
pub fn run_tagging(
    app: &tauri::AppHandle,
    options: &TaggerOptions,
    model_path: &Path,
    tag_defs: &[TagDefinition],
    input_size: u32,
    is_nchw: bool,
) -> Result<ProcessResult, String> {
    let fmt_label = if is_nchw { "NCHW" } else { "NHWC" };
    let _ = app.emit("tagger-progress", ProgressEvent {
        current: 0, total: 0, filename: String::new(),
        status: "info".to_string(),
        message: format!("正在加载模型 (GPU: {}, 格式: {})...",
            if options.use_gpu { "启用" } else { "禁用" }, fmt_label),
    });
    // 模型加载放子线程，主线程轮询取消标志
    let use_gpu = options.use_gpu;
    let model_path_owned = model_path.to_path_buf();
    let app_for_load = app.clone();

    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let result = (|| -> Result<Session, String> {
            let mut builder = Session::builder()
                .map_err(|e| format!("创建会话失败: {}", e))?;

            if use_gpu {
                // 尝试注册 CUDA EP
                let cuda_builder = builder.with_execution_providers([
                    ort::execution_providers::CUDAExecutionProvider::default().build()
                ]);
                match cuda_builder {
                    Ok(b) => {
                        builder = b;
                        let _ = app_for_load.emit("tagger-progress", ProgressEvent {
                            current: 0, total: 0, filename: String::new(),
                            status: "success".to_string(),
                            message: "✓ 已注册 CUDA ExecutionProvider".to_string(),
                        });
                    }
                    Err(e) => {
                        let _ = app_for_load.emit("tagger-progress", ProgressEvent {
                            current: 0, total: 0, filename: String::new(),
                            status: "error".to_string(),
                            message: format!("✗ CUDA 注册失败，回退 CPU: {}", e),
                        });
                        builder = Session::builder()
                            .map_err(|e2| format!("创建会话失败: {}", e2))?;
                    }
                }
            }

            let _ = app_for_load.emit("tagger-progress", ProgressEvent {
                current: 0, total: 0, filename: String::new(),
                status: "info".to_string(),
                message: "正在加载 ONNX 模型文件...".to_string(),
            });

            builder.commit_from_file(&model_path_owned)
                .map_err(|e| format!("加载模型失败: {}", e))
        })();
        let _ = tx.send(result);
    });

    // 轮询等待模型加载完成，期间可取消
    let mut session = loop {
        if is_tagging_cancelled() {
            let _ = app.emit("tagger-progress", ProgressEvent {
                current: 0, total: 0, filename: String::new(),
                status: "error".to_string(),
                message: "打标已取消（模型加载阶段）".to_string(),
            });
            let _ = app.emit("tagger-progress", ProgressEvent {
                current: 0, total: 0, filename: String::new(),
                status: "done".to_string(),
                message: "打标已取消".to_string(),
            });
            return Ok(ProcessResult { success_count: 0, fail_count: 0, total: 0, errors: vec![] });
        }
        match rx.try_recv() {
            Ok(result) => break result?,
            Err(std::sync::mpsc::TryRecvError::Empty) => {
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                return Err("模型加载线程异常退出".into());
            }
        }
    };

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
        // 检查取消
        if is_tagging_cancelled() {
            let _ = app.emit("tagger-progress", ProgressEvent {
                current: i as u32, total,
                filename: String::new(),
                status: "error".to_string(),
                message: format!("打标已取消（已完成 {}/{}）", i, total),
            });
            let _ = app.emit("tagger-progress", ProgressEvent {
                current: i as u32, total,
                filename: String::new(),
                status: "done".to_string(),
                message: format!("打标已取消: 成功 {}, 失败 {}", success_count, fail_count),
            });
            return Ok(ProcessResult { success_count, fail_count, total, errors });
        }

        let filename = file_path.file_name().unwrap_or_default().to_string_lossy().to_string();

        let _ = app.emit("tagger-progress", ProgressEvent {
            current: i as u32 + 1, total,
            filename: filename.clone(),
            status: "processing".to_string(),
            message: format!("正在处理: {} ({}/{})", filename, i + 1, total),
        });

        match tag_single_image(&mut session, file_path, tag_defs, input_size, options, &enabled_cats, is_nchw) {
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
    is_nchw: bool,
) -> Result<usize, String> {
    let (shape, input_data) = if is_nchw {
        preprocess_image_nchw(img_path, input_size)?
    } else {
        preprocess_image_nhwc(img_path, input_size)?
    };

    let input_tensor = ort::value::Tensor::from_array((shape, input_data))
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
