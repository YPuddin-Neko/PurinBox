pub mod models;
pub mod download;
pub mod inference;
pub mod llm_tagger;

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use ort::execution_providers::ExecutionProvider;

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

/// 检测 CUDA 是否可用，返回 (可用, 详情信息)
#[tauri::command]
pub async fn check_cuda_available() -> Result<(bool, String), String> {
    let mut lines: Vec<String> = Vec::new();

    // 1. 检测 ONNX Runtime 版本（快速，不会卡）
    let rt_info = std::panic::catch_unwind(|| {
        format!("ONNX Runtime: {}", ort::info())
    }).unwrap_or_else(|_| "ONNX Runtime: 信息获取失败".into());
    lines.push(rt_info);

    // 2. 检测 nvidia-smi 和 CUDA Toolkit（快速，通过系统命令）
    let nvidia_ok = tokio::task::spawn_blocking(|| {
        let mut env_lines: Vec<String> = Vec::new();
        let ok = inference::detect_nvidia_env_pub(&mut env_lines);
        inference::detect_cuda_toolkit_pub(&mut env_lines);
        (ok, env_lines)
    }).await.map_err(|e| format!("检测失败: {}", e))?;

    let (has_nvidia, env_lines) = nvidia_ok;
    lines.extend(env_lines);

    // 3. 检测 ort CUDA EP（可能卡住，单独超时 5 秒）
    let ep_task = tokio::task::spawn_blocking(|| {
        std::panic::catch_unwind(|| {
            ort::execution_providers::CUDAExecutionProvider::default().is_available()
        })
    });

    match tokio::time::timeout(std::time::Duration::from_secs(5), ep_task).await {
        Ok(Ok(Ok(Ok(true)))) => {
            lines.push("CUDA ExecutionProvider: ✓ 可用".into());
            lines.insert(0, "✓ CUDA 加速已启用".into());
            Ok((true, lines.join("\n")))
        }
        Ok(Ok(Ok(Ok(false)))) => {
            lines.push("CUDA ExecutionProvider: ✗ 不可用".into());
            lines.push("  → 当前 ONNX Runtime 为 CPU 版本".into());
            add_summary(&mut lines, has_nvidia);
            Ok((false, lines.join("\n")))
        }
        Ok(Ok(Ok(Err(e)))) => {
            lines.push(format!("CUDA ExecutionProvider: 异常 ({})", e));
            add_summary(&mut lines, has_nvidia);
            Ok((false, lines.join("\n")))
        }
        Ok(Ok(Err(_))) => {
            lines.push("CUDA ExecutionProvider: 内部 panic".into());
            add_summary(&mut lines, has_nvidia);
            Ok((false, lines.join("\n")))
        }
        Ok(Err(e)) => {
            lines.push(format!("CUDA ExecutionProvider: 线程异常 ({})", e));
            add_summary(&mut lines, has_nvidia);
            Ok((false, lines.join("\n")))
        }
        Err(_) => {
            lines.push("CUDA ExecutionProvider: ⚠ 检测超时 (5秒)".into());
            lines.push("  → ONNX Runtime 尝试加载 CUDA 库时卡住了".into());
            lines.push("  → 这通常说明系统有 CUDA 但 ort 库版本不兼容".into());
            add_summary(&mut lines, has_nvidia);
            Ok((false, lines.join("\n")))
        }
    }
}

fn add_summary(lines: &mut Vec<String>, has_nvidia: bool) {
    lines.push("".into());
    if has_nvidia {
        lines.push("结论: 系统已安装 NVIDIA 驱动和 CUDA".into());
        lines.push("但当前 ONNX Runtime 为 CPU 版本，不支持 GPU 加速".into());
        lines.push("CPU 推理仍可正常使用所有打标功能".into());
    } else {
        lines.push("结论: 未检测到 NVIDIA GPU，将使用 CPU 推理".into());
    }
}

/// 取消正在进行的下载
#[tauri::command]
pub fn cancel_tagger_download() {
    download::cancel_download();
}

/// 开始打标
#[tauri::command]
pub async fn start_tagging(
    app: tauri::AppHandle,
    options: TaggerOptions,
) -> Result<ProcessResult, String> {
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
    }

    // 3. 加载标签定义（支持 CSV 和 JSON 格式）
    let tag_defs = if tags_basename.ends_with(".json") {
        inference::load_tags_json(&tags_path)?
    } else {
        inference::load_tags(&tags_path)?
    };

    // 4. 执行推理
    let is_nchw = model_def.input_format == models::InputFormat::NCHW;
    let app_clone = app.clone();
    let opts = options.clone();
    tokio::task::spawn_blocking(move || {
        inference::run_tagging(&app_clone, &opts, &model_path, &tag_defs, model_def.input_size, is_nchw)
    })
    .await
    .map_err(|e| format!("任务执行失败: {}", e))?
}
