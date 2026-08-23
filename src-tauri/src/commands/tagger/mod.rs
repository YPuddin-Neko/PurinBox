pub mod download;
pub mod inference;
pub mod llm_tagger;
pub mod models;
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
    Quality,
    Model,
}

impl TagCategory {
    pub fn from_csv_id(id: i32) -> Option<Self> {
        match id {
            0 => Some(Self::General),
            1 => Some(Self::Artist),
            3 => Some(Self::Copyright),
            4 => Some(Self::Character),
            5 => Some(Self::Meta),
            6 => Some(Self::Quality),
            7 => Some(Self::Model),
            9 => Some(Self::Rating),
            _ => None,
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
    #[serde(default)]
    pub exclude_tags: String,
    #[serde(default)]
    pub append_tags: String,
    #[serde(default = "default_append_position")]
    pub append_position: String,
    #[serde(default = "default_true")]
    pub replace_underscore: bool,
    #[serde(default = "default_output_format")]
    pub output_format: String,
    #[serde(default)]
    pub json_simplified: bool,
    #[serde(default)]
    pub escape_parentheses: bool,
    #[serde(default = "default_sort_by")]
    pub sort_by: String,
    #[serde(default = "default_existing_tags_action")]
    pub existing_tags_action: String,
    #[serde(default = "default_batch_size")]
    pub batch_size: u32,
    /// 是否递归扫描子文件夹
    #[serde(default)]
    pub recursive: bool,
}

fn default_batch_size() -> u32 {
    1
}

fn default_append_position() -> String {
    "append".into()
}
fn default_true() -> bool {
    true
}
fn default_output_format() -> String {
    "txt".into()
}
fn default_sort_by() -> String {
    "confidence".into()
}
fn default_existing_tags_action() -> String {
    "overwrite".into()
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
    /// 该模型支持的标签分类列表
    pub supported_categories: Vec<String>,
}

/// 标签定义（从 CSV 解析）
#[derive(Debug, Clone)]
#[allow(dead_code)]
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

/// 从标签文件中扫描支持的分类
fn detect_supported_categories(tags_path: &std::path::Path) -> Vec<String> {
    use std::collections::BTreeSet;
    let mut cats = BTreeSet::new();

    if let Some(ext) = tags_path.extension().and_then(|e| e.to_str()) {
        if ext == "json" {
            // JSON 格式 (CL Tagger)
            if let Ok(content) = std::fs::read_to_string(tags_path) {
                if let Ok(value) = serde_json::from_str::<serde_json::Value>(&content) {
                    if let Some(tag_to_category) =
                        value.get("tag_to_category").and_then(|v| v.as_object())
                    {
                        let categories = value.get("categories");
                        for cat in tag_to_category.values() {
                            if let Some(name) = normalize_category_value(cat, categories) {
                                cats.insert(name);
                            }
                        }
                    } else if let Some(map) = value.as_object() {
                        for val in map.values() {
                            if let Some(cat) = val.get("category") {
                                if let Some(name) = normalize_category_value(cat, None) {
                                    cats.insert(name);
                                }
                            }
                        }
                    }
                } else if let Ok(map) = serde_json::from_str::<
                    std::collections::BTreeMap<String, serde_json::Value>,
                >(&content)
                {
                    for val in map.values() {
                        if let Some(cat) = val.get("category") {
                            if let Some(name) = normalize_category_value(cat, None) {
                                cats.insert(name);
                            }
                        }
                    }
                }
            }
        } else {
            // CSV 格式 (WD Tagger)
            let cat_map = [
                (0, "general"),
                (1, "artist"),
                (3, "copyright"),
                (4, "character"),
                (5, "meta"),
                (6, "quality"),
                (7, "model"),
                (9, "rating"),
            ];
            if let Ok(mut reader) = csv::Reader::from_path(tags_path) {
                for result in reader.records().flatten() {
                    if result.len() >= 3 {
                        if let Ok(cat_id) = result.get(2).unwrap_or("0").parse::<i32>() {
                            for (id, name) in &cat_map {
                                if cat_id == *id {
                                    cats.insert(name.to_string());
                                    break;
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    cats.into_iter().collect()
}

fn normalize_category_value(
    value: &serde_json::Value,
    categories: Option<&serde_json::Value>,
) -> Option<String> {
    let raw = if let Some(s) = value.as_str() {
        if let Ok(idx) = s.parse::<usize>() {
            resolve_category_index(idx, categories).unwrap_or_else(|| s.to_string())
        } else {
            s.to_string()
        }
    } else if let Some(idx) = value.as_u64() {
        resolve_category_index(idx as usize, categories)?
    } else {
        return None;
    };

    match raw.to_lowercase().replace('-', "_").as_str() {
        "general" => Some("general".into()),
        "artist" => Some("artist".into()),
        "copyright" | "copyrights" => Some("copyright".into()),
        "character" | "characters" => Some("character".into()),
        "meta" => Some("meta".into()),
        "rating" => Some("rating".into()),
        "quality" => Some("quality".into()),
        "model" => Some("model".into()),
        _ => None,
    }
}

fn resolve_category_index(index: usize, categories: Option<&serde_json::Value>) -> Option<String> {
    let categories = categories?;
    if let Some(arr) = categories.as_array() {
        return arr
            .get(index)
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
    }
    if let Some(obj) = categories.as_object() {
        return obj
            .get(&index.to_string())
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
    }
    None
}

/// 根据 tags_filename 推断默认支持分类（未下载时使用）
fn infer_categories_from_filename(tags_filename: &str) -> Vec<String> {
    if tags_filename.ends_with(".json") {
        // CL Tagger 类型: 全分类
        vec![
            "general",
            "character",
            "rating",
            "artist",
            "copyright",
            "meta",
            "quality",
            "model",
        ]
        .into_iter()
        .map(|s| s.to_string())
        .collect()
    } else {
        // WD Tagger 类型: 只有 general, character, rating
        vec!["general", "character", "rating"]
            .into_iter()
            .map(|s| s.to_string())
            .collect()
    }
}

// ===== Tauri Commands =====

/// 获取可用模型列表
#[tauri::command]
pub async fn get_tagger_models() -> Result<Vec<TaggerModelInfo>, String> {
    let builtin = models::get_builtin_models();
    let custom = models::load_custom_models().unwrap_or_default();
    let all = [builtin, custom].concat();

    let mut result = Vec::new();
    for m in &all {
        let model_dir = get_model_dir(&m.id);
        let tags_basename = m.tags_basename();
        let is_downloaded = m
            .required_local_files()
            .iter()
            .all(|filename| model_dir.join(filename).exists());
        let fmt_str = match m.input_format {
            models::InputFormat::NHWC => "NHWC",
            models::InputFormat::NCHW => "NCHW",
        };

        // 检测支持的分类
        let supported_categories = if is_downloaded {
            let tags_path = model_dir.join(&tags_basename);
            let detected = detect_supported_categories(&tags_path);
            if detected.is_empty() {
                infer_categories_from_filename(&m.tags_filename)
            } else {
                detected
            }
        } else {
            infer_categories_from_filename(&m.tags_filename)
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
            supported_categories,
        });
    }
    Ok(result)
}

/// 自动检测 ONNX 模型的输入尺寸和通道格式
#[tauri::command]
pub async fn detect_onnx_model_info(model_path: String) -> Result<OnnxModelInfo, String> {
    tokio::task::spawn_blocking(move || inference::detect_model_info(&model_path))
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

/// 取消正在进行的模型下载
#[tauri::command]
pub fn cancel_tagger_download() {
    download::cancel_download();
}

/// 取消打标（同时取消可能正在进行的下载/安装）
#[tauri::command]
pub fn cancel_tagging() {
    inference::cancel_tagging();
    download::cancel_download();
    // 只取消打标自己发起的环境部署，不连带中止其他功能的（归属隔离）
    python_env::cancel_setup_for("tagger");
}

/// 强制取消打标（终止整个子进程树）
#[tauri::command]
pub fn force_cancel_tagging() {
    inference::cancel_tagging();
    download::cancel_download();
    python_env::cancel_setup_for("tagger");
    inference::kill_python_process();
}

/// 开始打标
#[tauri::command]
pub async fn start_tagging(
    app: tauri::AppHandle,
    options: TaggerOptions,
) -> Result<ProcessResult, String> {
    use tauri::Emitter;

    // 互斥：打标页/辅助打标/工作流节点共用全局子进程句柄与取消标志，并发会互杀进程
    static TAGGING_RUNNING: std::sync::atomic::AtomicBool =
        std::sync::atomic::AtomicBool::new(false);
    let _busy = crate::commands::BusyGuard::acquire(&TAGGING_RUNNING, "打标")?;

    // 重置取消标志
    inference::reset_tagging_cancel();

    // 检查 Python 环境，没有则自动安装
    let python_check = tokio::task::spawn_blocking(inference::check_python_env)
        .await
        .map_err(|e| format!("检测线程异常: {}", e))?;

    let python = match &python_check {
        Ok((_ver, _providers)) => {
            // Python 环境已就绪
            python_env::get_python_exe().unwrap_or_default()
        }
        Err(_) => {
            // Python 环境不可用，自动安装
            let _ = app.emit(
                "tagger-progress",
                ProgressEvent {
                    current: 0,
                    total: 0,
                    filename: String::new(),
                    status: "info".to_string(),
                    message: "Python 环境未就绪，正在自动配置...".to_string(),
                    ..Default::default()
                },
            );
            python_env::setup_python_env(&app, "tagger").await?
        }
    };

    // 确保 onnxruntime GPU 运行时（统一入口，幂等：已可用则直接返回不重装）
    if !python.is_empty() {
        let _ = python_env::ensure_onnx_gpu_runtime(&app, &python, "tagger").await;
    }

    // 1. 查找模型
    let model_def = models::find_model(&options.model_id)
        .ok_or_else(|| format!("模型不存在: {}", options.model_id))?;

    let model_dir = get_model_dir(&model_def.id);
    let model_path = model_dir.join("model.onnx");
    let tags_basename = model_def.tags_basename();
    let tags_path = model_dir.join(&tags_basename);

    // 2. 如果未下载，先下载
    let is_model_ready = model_def
        .required_local_files()
        .iter()
        .all(|filename| model_dir.join(filename).exists());
    if !is_model_ready {
        let _ = app.emit(
            "tagger-progress",
            ProgressEvent {
                current: 0,
                total: 0,
                filename: String::new(),
                status: "info".to_string(),
                message: format!("模型 {} 未下载，开始下载...", model_def.name),
                ..Default::default()
            },
        );

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
    let preprocess_mode = model_def.preprocess_mode.clone();
    let app_clone = app.clone();
    let opts = options.clone();
    let tags_path_for_python = tags_path.clone();
    tokio::task::spawn_blocking(move || {
        inference::run_tagging(
            &app_clone,
            &opts,
            &model_path,
            &tags_path_for_python,
            &tag_defs,
            model_def.input_size,
            is_nchw,
            &preprocess_mode,
        )
    })
    .await
    .map_err(|e| format!("任务执行失败: {}", e))?
}

// ═══════════════ txt → JSON 标签格式转换 ═══════════════

/// txt → JSON 标签转换选项（辅助打标流水线的最后一步，也可独立使用）
#[derive(Debug, Clone, serde::Deserialize)]
pub struct ConvertTagsOptions {
    pub input_path: String,
    /// 提供标签分类的词表来源模型（须已下载）
    pub model_id: String,
    #[serde(default)]
    pub json_simplified: bool,
    /// 转换成功后删除源 .txt
    #[serde(default)]
    pub remove_txt: bool,
    #[serde(default)]
    pub recursive: bool,
}

/// 将图片旁的 .txt 标签按模型词表分类后转换为 JSON。
/// 复用 Python 端的分类与 JSON 构建逻辑（--convert 模式，不加载 ONNX，速度快）。
#[tauri::command]
pub async fn convert_tags_to_json(
    app: tauri::AppHandle,
    options: ConvertTagsOptions,
) -> Result<ProcessResult, String> {
    let model_def = models::find_model(&options.model_id)
        .ok_or_else(|| format!("模型不存在: {}", options.model_id))?;
    let tags_path = get_model_dir(&model_def.id).join(model_def.tags_basename());
    if !tags_path.exists() {
        return Err(format!("模型词表未下载: {}", tags_path.display()));
    }
    tokio::task::spawn_blocking(move || run_convert_tags(&app, &options, &tags_path))
        .await
        .map_err(|e| format!("转换任务执行失败: {}", e))?
}

fn run_convert_tags(
    app: &tauri::AppHandle,
    options: &ConvertTagsOptions,
    tags_path: &std::path::Path,
) -> Result<ProcessResult, String> {
    use std::io::BufRead;
    use tauri::Emitter;

    let python = inference::find_python()?;
    let script = crate::commands::python_proc::find_script("tagger_inference.py")?;

    let mut cmd = std::process::Command::new(&python);
    cmd.arg(script.to_string_lossy().as_ref())
        .arg("--convert")
        .arg("--input")
        .arg(&options.input_path)
        .arg("--tags-path")
        .arg(tags_path)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .env("PYTHONUNBUFFERED", "1")
        .env("PYTHONIOENCODING", "utf-8");
    if options.json_simplified {
        cmd.arg("--simplified");
    }
    if options.remove_txt {
        cmd.arg("--remove-txt");
    }
    if options.recursive {
        cmd.arg("--recursive");
    }
    crate::commands::python_proc::configure_python_command(&mut cmd, false);

    let mut child = cmd
        .spawn()
        .map_err(|e| format!("启动转换进程失败: {}", e))?;
    let stdout = child.stdout.take().ok_or("无法获取转换进程输出")?;

    let mut converted = 0u32;
    let mut failed = 0u32;
    let mut skipped = 0u32;
    let mut total = 0u32;
    for line in std::io::BufReader::new(stdout).lines().map_while(Result::ok) {
        let Ok(msg) = serde_json::from_str::<serde_json::Value>(&line) else {
            continue;
        };
        match msg.get("type").and_then(|t| t.as_str()) {
            Some("progress") => {
                let current = msg.get("current").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
                total = msg.get("total").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
                let filename = msg
                    .get("filename")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string();
                let _ = app.emit(
                    "tagger-progress",
                    ProgressEvent {
                        current,
                        total,
                        filename: filename.clone(),
                        status: "processing".to_string(),
                        message: format!("正在转换 JSON: {}", filename),
                        ..Default::default()
                    },
                );
            }
            Some("log") => {
                let message = msg
                    .get("message")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string();
                let _ = app.emit(
                    "tagger-progress",
                    ProgressEvent {
                        current: 0,
                        total,
                        filename: String::new(),
                        status: "warning".to_string(),
                        message,
                        ..Default::default()
                    },
                );
            }
            Some("done") => {
                converted = msg.get("converted").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
                failed = msg.get("failed").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
                skipped = msg.get("skipped").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
                total = msg.get("total").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
            }
            Some("error") => {
                let message = msg
                    .get("message")
                    .and_then(|v| v.as_str())
                    .unwrap_or("转换失败")
                    .to_string();
                let _ = child.wait();
                return Err(message);
            }
            _ => {}
        }
    }
    let status = child.wait().map_err(|e| format!("等待转换进程失败: {}", e))?;
    if !status.success() && converted == 0 {
        return Err("转换进程异常退出".to_string());
    }

    let _ = app.emit(
        "tagger-progress",
        ProgressEvent {
            current: total,
            total,
            filename: String::new(),
            status: "done".to_string(),
            message: format!(
                "JSON 转换完成：{} 个转换，{} 个无标签跳过，{} 个失败",
                converted, skipped, failed
            ),
            ..Default::default()
        },
    );

    Ok(ProcessResult {
        success_count: converted,
        fail_count: failed,
        total,
        errors: Vec::new(),
    })
}
