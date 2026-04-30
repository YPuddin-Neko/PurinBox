pub mod models;
pub mod download;
pub mod inference;
pub mod llm_tagger;

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
}

/// 标签定义（从 CSV 解析）
#[derive(Debug, Clone)]
pub struct TagDefinition {
    pub name: String,
    pub category: TagCategory,
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
        let is_downloaded = model_dir.join("model.onnx").exists()
            && model_dir.join("selected_tags.csv").exists();
        result.push(TaggerModelInfo {
            id: m.id.clone(),
            name: m.name.clone(),
            description: m.description.clone(),
            input_size: m.input_size,
            is_builtin: m.is_builtin,
            is_downloaded,
            repo_id: m.repo_id.clone(),
        });
    }
    Ok(result)
}

/// 添加自定义模型
#[tauri::command]
pub async fn add_custom_tagger_model(
    id: String,
    name: String,
    repo_id: String,
    input_size: u32,
) -> Result<(), String> {
    models::add_custom_model(id, name, repo_id, input_size)
}

/// 检测 CUDA 是否可用
#[tauri::command]
pub async fn check_cuda_available() -> Result<bool, String> {
    Ok(tokio::task::spawn_blocking(inference::check_cuda)
        .await
        .unwrap_or(false))
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
    let tags_path = model_dir.join("selected_tags.csv");

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

    // 3. 加载标签定义
    let tag_defs = inference::load_tags(&tags_path)?;

    // 4. 执行推理
    let app_clone = app.clone();
    let opts = options.clone();
    tokio::task::spawn_blocking(move || {
        inference::run_tagging(&app_clone, &opts, &model_path, &tag_defs, model_def.input_size)
    })
    .await
    .map_err(|e| format!("任务执行失败: {}", e))?
}
