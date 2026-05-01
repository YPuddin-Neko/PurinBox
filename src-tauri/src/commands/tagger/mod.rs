pub mod models;
pub mod download;
pub mod inference;
pub mod llm_tagger;
pub mod gpu_runtime;
pub mod python_env;

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use super::{ProcessResult, ProgressEvent};

/// 标签分类
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum TagCategory {
    General,
    Artist,
    Copyright,
    Character,
    Meta,
    Rating,
}

impl TagCategory {
    pub fn from_csv_id(id: i32) -> Option<Self> {
        match id {
            0 => Some(Self::General),
            1 => Some(Self::Artist),
            2 => Some(Self::Copyright),
            3 => Some(Self::Character),
            4 => Some(Self::Meta),
            9 => Some(Self::Rating),
            _ => None,
        }
    }

    pub fn key(&self) -> &str {
        match self {
            Self::General => "general",
            Self::Artist => "artist",
            Self::Copyright => "copyright",
            Self::Character => "character",
            Self::Meta => "meta",
            Self::Rating => "rating",
        }
    }
}

/// 打标选项
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaggerOptions {
    pub input_path: String,
    pub model_id: String,
    pub general_threshold: f32,
    pub character_threshold: f32,
    pub enabled_categories: Vec<String>,
    pub use_gpu: bool,
}

/// 模型信息（给前端用）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaggerModelInfo {
    pub id: String,
    pub name: String,
    pub description: String,
    pub input_size: u32,
    pub is_builtin: bool,
    pub is_downloaded: bool,
    pub repo_id: String,
    pub input_format: String,
}

/// 标签定义（从 CSV 解析）
#[derive(Debug, Clone)]
pub struct TagDefinition {
    pub name: String,
    pub category: TagCategory,
}

/// ONNX 模型自动检测结果
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OnnxModelInfo {
    pub input_size: u32,
    pub input_format: String,
    pub input_shape: Vec<i64>,
    pub channels: i64,
}

/// 获取模型存储根目录
pub fn get_models_dir() -> PathBuf {
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|p| p.to_path_buf()))
        .unwrap_or_else(|| PathBuf::from("."));

    // 开发模式下使用项目根目录
    let base = if cfg!(debug_assertions) {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or(exe_dir)
    } else {
        exe_dir
    };

    base.join("models").join("tagger_models")
}

/// 获取指定模型的目录
pub fn get_model_dir(model_id: &str) -> PathBuf {
    get_models_dir().join(model_id)
}

// ===== Tauri Commands =====

use tauri::Emitter;

/// 获取可用模型列表
#[tauri::command]
pub async fn get_tagger_models() -> Result<Vec<TaggerModelInfo>, String> {
    let builtin = models::get_builtin_models();
    let custom = models::load_custom_models().unwrap_or_default();
    let all = [builtin, custom].concat();

    let mut result = Vec::new();
    for m in &all {
        let model_dir = get_model_dir(&m.id);
        let tags_basename = std::path::Path::new(&m.tags_filename)
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        let is_downloaded = model_dir.join("model.onnx").exists()
            && model_dir.join(&tags_basename).exists();
        let fmt_str = match m.input_format {
            models::InputFormat::NHWC => "NHWC",
            models::InputFormat::NCHW => "NCHW",
        };
        result.push(TaggerModelInfo {
            id: m.id.clone(),
            name: m.name.clone(),
            description: m.description.clone(),
            input_size: m.input_size,
            is_builtin: m.is_builtin,
            is_downloaded,
            repo_id: m.repo_id.clone(),
            input_format: fmt_str.to_string(),
        });
    }
    Ok(result)
}

/// 自动检测 ONNX 模型的输入尺寸和通道格式
#[tauri::command]
pub async fn detect_onnx_model_info(model_path: String) -> Result<OnnxModelInfo, String> {
    tokio::task::spawn_blocking(move || {
        inference::detect_model_info(&model_path)
    })
    .await
    .map_err(|e| format!("检测失败: {}", e))?
}

/// 导入本地模型
#[tauri::command]
pub async fn import_local_tagger_model(
    name: String,
    model_path: String,
    tags_path: String,
    input_size: u32,
    input_format: String,
) -> Result<String, String> {
    let fmt = match input_format.as_str() {
        "NCHW" => models::InputFormat::NCHW,
        _ => models::InputFormat::NHWC,
    };
    models::add_local_model(name, model_path, tags_path, input_size, fmt)
}

/// 删除自定义模型
#[tauri::command]
pub async fn remove_custom_tagger_model(id: String) -> Result<(), String> {
    models::remove_custom_model(&id)
}

/// 检测 GPU 加速是否可用（Windows: CUDA, macOS: CoreML/Metal）
#[tauri::command]
pub async fn check_cuda_available(app: tauri::AppHandle) -> Result<(bool, String), String> {
    use tauri::Emitter;

    let emit_line = |msg: &str, status: &str| {
        let _ = app.emit("tagger-progress", ProgressEvent {
            current: 0, total: 0,
            filename: String::new(),
            status: status.to_string(),
            message: msg.to_string(),
        });
    };

    let mut summary_lines: Vec<String> = Vec::new();
    let is_macos = cfg!(target_os = "macos");

    // 1. 硬件检测
    if is_macos {
        emit_line("正在检测 GPU...", "info");
        let gpu_lines = tokio::task::spawn_blocking(|| {
            let mut lines: Vec<String> = Vec::new();
            inference::detect_apple_gpu_pub(&mut lines);
            lines
        }).await.unwrap_or_else(|_| vec!["检测线程异常".into()]);

        for line in &gpu_lines {
            emit_line(line, "success");
        }
        summary_lines.extend(gpu_lines);
    } else {
        // Windows/Linux: NVIDIA 检测
        emit_line("正在检测 NVIDIA 驱动...", "info");

        let nvidia_result = tokio::task::spawn_blocking(|| {
            let mut lines: Vec<String> = Vec::new();
            let ok = inference::detect_nvidia_env_pub(&mut lines);
            inference::detect_cuda_toolkit_pub(&mut lines);
            (ok, lines)
        }).await.unwrap_or_else(|_| (false, vec!["检测线程异常".into()]));

        let (_has_nvidia, env_lines) = nvidia_result;
        for line in &env_lines {
            let status = if line.contains("未检测到") { "error" } else { "success" };
            emit_line(line, status);
        }
        summary_lines.extend(env_lines);
    }

    // 2. Python onnxruntime 检测
    emit_line("正在检测 Python onnxruntime...", "info");

    let python_check = tokio::task::spawn_blocking(|| {
        inference::check_python_env()
    }).await.unwrap_or_else(|_| Err("检测线程异常".into()));

    let (mut gpu_ok, python_ok) = match python_check {
        Ok((ort_ver, providers)) => {
            let msg = format!("Python onnxruntime v{}", ort_ver);
            emit_line(&msg, "success");
            summary_lines.push(msg);

            let msg = format!("可用 providers: {}", providers);
            emit_line(&msg, "info");
            summary_lines.push(msg);

            // 检测 GPU provider
            let has_cuda = providers.contains("CUDAExecutionProvider");
            let has_coreml = providers.contains("CoreMLExecutionProvider");
            let has_gpu = has_cuda || has_coreml;

            if has_cuda {
                emit_line("✓ CUDA ExecutionProvider 可用", "success");
            } else if has_coreml {
                emit_line("✓ CoreML ExecutionProvider 可用", "success");
            } else {
                emit_line("GPU ExecutionProvider 不可用", "info");
            }
            (has_gpu, true)
        }
        Err(e) => {
            emit_line(&e, "error");
            summary_lines.push(e);
            (false, false)
        }
    };

    // 3. Windows: 如果有 NVIDIA 但没有 CUDA EP，自动安装 onnxruntime-gpu
    #[cfg(target_os = "windows")]
    if !gpu_ok && python_ok {
        // 检查是否有 NVIDIA GPU（重新检测一下）
        let has_nvidia = tokio::task::spawn_blocking(|| {
            let mut lines = Vec::new();
            inference::detect_nvidia_env_pub(&mut lines)
        }).await.unwrap_or(false);

        if has_nvidia {
            emit_line("正在自动安装 GPU 版 onnxruntime...", "info");
            let app_ref = app.clone();
            let install_result = tokio::task::spawn_blocking(move || {
                python_env::install_gpu_deps(&app_ref)
            }).await.unwrap_or_else(|_| Err("安装线程异常".into()));

            match install_result {
                Ok(()) => {
                    emit_line("✓ onnxruntime-gpu 已安装", "success");
                    let recheck = tokio::task::spawn_blocking(|| {
                        inference::check_python_env()
                    }).await.unwrap_or_else(|_| Err("检测线程异常".into()));
                    if let Ok((_, providers)) = recheck {
                        gpu_ok = providers.contains("CUDAExecutionProvider");
                        if gpu_ok {
                            emit_line("✓ CUDA 加速已启用", "success");
                        }
                    }
                }
                Err(e) => {
                    emit_line(&format!("GPU 版安装失败: {}", e), "error");
                    emit_line("将使用 CPU 推理", "info");
                }
            }
        }
    }

    // 4. 总结
    if gpu_ok {
        if is_macos {
            emit_line("✓ GPU 加速已就绪 (CoreML/Metal)", "success");
        } else {
            emit_line("✓ GPU 加速已就绪 (CUDA)", "success");
        }
    } else {
        emit_line("将使用 CPU 推理", "info");
    }

    Ok((gpu_ok, summary_lines.join("\n")))
}

/// 取消正在进行的模型下载
#[tauri::command]
pub fn cancel_tagger_download() {
    download::cancel_download();
}

/// 获取 ONNX Runtime 状态（Python 环境检测）
#[tauri::command]
pub fn get_gpu_runtime_status() -> gpu_runtime::OrtRuntimeStatus {
    gpu_runtime::check_ort_runtime()
}

/// 下载 ONNX Runtime（保留兼容性，但实际推荐 pip install）
#[tauri::command]
pub async fn download_gpu_runtime(app: tauri::AppHandle) -> Result<(), String> {
    gpu_runtime::download_ort_runtime(&app).await
}

/// 取消 ONNX Runtime 下载
#[tauri::command]
pub fn cancel_gpu_runtime_download() {
    gpu_runtime::cancel_ort_download();
}

/// 取消打标（同时取消可能正在进行的下载/安装）
#[tauri::command]
pub fn cancel_tagging() {
    inference::cancel_tagging();
    download::cancel_download();
    gpu_runtime::cancel_ort_download();
    python_env::cancel_setup();
}

/// 开始打标
#[tauri::command]
pub async fn start_tagging(
    app: tauri::AppHandle,
    options: TaggerOptions,
) -> Result<ProcessResult, String> {
    use tauri::Emitter;

    // 重置取消标志
    inference::reset_tagging_cancel();

    // 检查 Python 环境，没有则自动安装
    let python_check = tokio::task::spawn_blocking(|| {
        inference::check_python_env()
    }).await.map_err(|e| format!("检测线程异常: {}", e))?;

    match &python_check {
        Ok((ver, providers)) => {
            let _ = app.emit("tagger-progress", ProgressEvent {
                current: 0, total: 0, filename: String::new(),
                status: "success".to_string(),
                message: format!("Python onnxruntime v{} ({})", ver, providers),
            });
        }
        Err(_) => {
            // Python 环境不可用，自动安装
            let _ = app.emit("tagger-progress", ProgressEvent {
                current: 0, total: 0, filename: String::new(),
                status: "info".to_string(),
                message: "未检测到 Python 推理环境，正在自动安装...".to_string(),
            });
            python_env::setup_python_env(&app).await?;
        }
    }

    // 1. 查找模型
    let model_def = models::find_model(&options.model_id)
        .ok_or_else(|| format!("模型不存在: {}", options.model_id))?;

    let model_dir = get_model_dir(&model_def.id);
    let model_path = model_dir.join("model.onnx");
    let tags_basename = std::path::Path::new(&model_def.tags_filename)
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();
    let tags_path = model_dir.join(&tags_basename);

    // 2. 如果未下载，先下载
    if !model_path.exists() || !tags_path.exists() {
        let _ = app.emit("tagger-progress", ProgressEvent {
            current: 0, total: 0,
            filename: String::new(),
            status: "info".to_string(),
            message: format!("模型 {} 未下载，开始下载...", model_def.name),
        });

        download::download_model(&app, &model_def).await?;

        if inference::is_tagging_cancelled() {
            return Err("已取消".into());
        }
    }

    // 3. 加载标签定义（仅用于计数，Python 端会重新加载）
    let tag_defs = if tags_basename.ends_with(".json") {
        inference::load_tags_json(&tags_path)?
    } else {
        inference::load_tags(&tags_path)?
    };

    // 4. 通过 Python 子进程执行推理
    let is_nchw = model_def.input_format == models::InputFormat::NCHW;
    let app_clone = app.clone();
    let opts = options.clone();
    tokio::task::spawn_blocking(move || {
        inference::run_tagging(&app_clone, &opts, &model_path, &tag_defs, model_def.input_size, is_nchw)
    })
    .await
    .map_err(|e| format!("任务执行失败: {}", e))?
}

