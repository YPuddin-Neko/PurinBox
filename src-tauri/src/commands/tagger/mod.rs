pub mod download;
pub mod hybrid;
pub mod inference;
pub mod llm_tagger;
pub mod models;

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tauri::Emitter;

use super::{ProcessResult, ProgressEvent};
use crate::commands::python_env;

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
    /// JSON 输出时追加标签的目标字段（quality/character/series/artist/count/
    /// appearance/tags/environment），仅 output_format=json 时有意义
    #[serde(default = "default_json_append_field")]
    pub json_append_field: String,
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
    /// txt 输出 + skip 时，同名 .json 也算"已有标签"（辅助打标：
    /// 调优阶段会直接读 JSON 的字段结构，这里不能让模型重打一份 txt 盖掉选择）
    #[serde(default)]
    pub also_skip_json: bool,
    #[serde(default = "default_batch_size")]
    pub batch_size: u32,
    /// 是否递归扫描子文件夹
    #[serde(default)]
    pub recursive: bool,
    #[serde(default)]
    pub hybrid_mode: bool,
}

fn default_batch_size() -> u32 {
    1
}

fn default_append_position() -> String {
    "append".into()
}
fn default_json_append_field() -> String {
    "tags".into()
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
    pub requires_token: bool,
    pub input_size: u32,
    pub is_builtin: bool,
    pub is_downloaded: bool,
    /// 该模型支持的标签分类列表
    pub supported_categories: Vec<String>,
    /// 官方推荐阈值（None = 沿用工具箱默认值）
    pub general_threshold: Option<f32>,
    pub character_threshold: Option<f32>,
}

/// ONNX 模型自动检测结果
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OnnxModelInfo {
    pub input_size: u32,
    pub input_format: String,
    pub input_shape: Vec<i64>,
}

/// 获取模型存储根目录
pub fn get_models_dir() -> PathBuf {
    crate::commands::config_paths::models_dir("tagger_models")
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
            // JSON 词表（CL Tagger 系、PixAI v1 分组格式）
            if let Ok(content) = std::fs::read_to_string(tags_path) {
                if let Ok(value) = serde_json::from_str::<serde_json::Value>(&content) {
                    if let Some(groups) = value
                        .get("categories")
                        .and_then(|v| v.as_array())
                        .filter(|groups| groups.iter().any(|g| g.get("tags").is_some()))
                    {
                        for group in groups {
                            if let Some(name) = group
                                .get("name")
                                .and_then(|name| normalize_category_value(name, None))
                            {
                                cats.insert(name);
                            }
                        }
                    } else if let Some(tag_to_category) =
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
                }
            }
        } else {
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
            // CSV 格式 (WD Tagger)。按表头名取列：SmilingWolf 系是 tag_id,name,category,...，
            // PixAI(deepghs 导出)是 id,tag_id,name,category,...——列位置不同
            if let Ok(mut reader) = csv::Reader::from_path(tags_path) {
                let cat_idx = reader.headers().ok().and_then(|h| {
                    h.iter()
                        .position(|c| c.trim().eq_ignore_ascii_case("category"))
                });
                for result in reader.records().flatten() {
                    let cell = match cat_idx {
                        Some(i) => result.get(i),
                        // 无表头时使用旧格式的 tag_id,name,category 列顺序。
                        None => result.get(2),
                    };
                    if let Some(Ok(cat_id)) = cell.map(|c| c.trim().parse::<i32>()) {
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
    } else {
        let idx = value.as_u64()?;
        resolve_category_index(idx as usize, categories)?
    };

    match raw.to_lowercase().replace('-', "_").as_str() {
        "general" => Some("general".into()),
        "artist" => Some("artist".into()),
        "style" => Some("style".into()),
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
        let is_downloaded = m.is_downloaded();

        let supported_categories = if !m.category_thresholds.is_empty() {
            m.category_thresholds.keys().cloned().collect()
        } else if is_downloaded {
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
            requires_token: m.requires_token,
            input_size: m.input_size,
            is_builtin: m.is_builtin,
            is_downloaded,
            supported_categories,
            general_threshold: m.general_threshold,
            character_threshold: m.character_threshold,
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
) -> Result<String, String> {
    models::add_local_model(name, model_path, tags_path, input_size)
}

/// 删除自定义模型
#[tauri::command]
pub async fn remove_custom_tagger_model(id: String) -> Result<(), String> {
    models::remove_custom_model(&id)
}

/// 取消打标（同时取消可能正在进行的下载/安装）
#[tauri::command]
pub fn cancel_tagging() {
    inference::cancel_tagging();
    download::cancel_download();
    // 只取消打标自己发起的环境部署，不连带中止其他功能的（归属隔离）
    python_env::cancel_setup_for("tagger");
}

/// 强制取消打标：cancel_tagging 已按进程树终止子进程，两者行为相同
#[tauri::command]
pub fn force_cancel_tagging() {
    cancel_tagging();
}

// 打标和转换共用子进程槽及取消标志，必须共用同一把互斥锁。
static TAGGING_RUNNING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

#[cfg(test)]
pub(crate) static TAGGER_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// 开始打标
#[tauri::command]
pub async fn start_tagging(
    app: tauri::AppHandle,
    options: TaggerOptions,
) -> Result<ProcessResult, String> {
    let _busy = crate::commands::BusyGuard::acquire(&TAGGING_RUNNING, "打标")?;

    inference::reset_tagging_cancel();

    if options.hybrid_mode {
        let scan_options = options.clone();
        let skipped = tokio::task::spawn_blocking(move || inference::all_skipped(&scan_options))
            .await
            .map_err(|e| format!("读取图片失败: {}", e))??;
        if let Some(result) = skipped {
            inference::emit_summary(
                &app,
                &result,
                result.total,
                inference::is_tagging_cancelled(),
            );
            return Ok(result);
        }
    }

    let prepared = prepare_tagger(&app, &options.model_id, options.use_gpu).await;
    if inference::is_tagging_cancelled() {
        let result = ProcessResult::default();
        inference::emit_summary(&app, &result, 0, true);
        return Ok(result);
    }
    let (python, model_def, model_dir) = prepared?;

    tokio::task::spawn_blocking(move || {
        inference::run_tagging(&app, &options, &python, &model_def, &model_dir)
    })
    .await
    .map_err(|e| format!("任务执行失败: {}", e))?
}

async fn prepare_tagger(
    app: &tauri::AppHandle,
    model_id: &str,
    use_gpu: bool,
) -> Result<(String, models::ModelDefinition, PathBuf), String> {
    let python = python_env::setup_python_env(app, "tagger").await?;
    if inference::is_tagging_cancelled() {
        return Err("已取消".into());
    }
    if use_gpu {
        python_env::ensure_onnx_gpu_runtime(app, &python, "tagger").await?;
    }
    if inference::is_tagging_cancelled() {
        return Err("已取消".into());
    }
    let model = models::find_model(model_id).ok_or_else(|| format!("模型不存在: {}", model_id))?;
    let dir = ensure_model_files(app, &model).await?;
    Ok((python, model, dir))
}

/// 模型缺文件就整包下载，返回模型目录。打标和 txt → JSON 转换共用：
/// 只下词表会留下"词表有模型没有"的半吊子状态，本地打标照样跑不了
async fn ensure_model_files(
    app: &tauri::AppHandle,
    model: &models::ModelDefinition,
) -> Result<PathBuf, String> {
    if !model.is_downloaded() {
        let _ = app.emit(
            "tagger-progress",
            ProgressEvent::new("info", format!("模型 {} 未下载，开始下载...", model.name)),
        );
        download::download_model(app, model).await?;
        if inference::is_tagging_cancelled() {
            return Err("已取消".into());
        }
    }
    Ok(get_model_dir(&model.id))
}

// ═══════════════ txt → JSON 标签格式转换 ═══════════════

/// txt → JSON 标签转换选项（辅助打标流水线在 JSON 输出 + 优先使用已有标签时的第一步）
#[derive(Debug, Clone, serde::Deserialize)]
pub struct ConvertTagsOptions {
    pub input_path: String,
    /// 提供标签分类的词表来源模型（须已下载）
    pub model_id: String,
    #[serde(default)]
    pub json_simplified: bool,
    #[serde(default)]
    pub recursive: bool,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct PrepareHybridTagsOptions {
    pub input_path: String,
    pub model_id: String,
    pub file_format: String,
    #[serde(default)]
    pub json_simplified: bool,
    #[serde(default)]
    pub recursive: bool,
}

#[tauri::command]
pub async fn prepare_hybrid_tags(
    app: tauri::AppHandle,
    options: PrepareHybridTagsOptions,
) -> Result<ProcessResult, String> {
    let _busy = crate::commands::BusyGuard::acquire(&TAGGING_RUNNING, "打标")?;
    inference::reset_tagging_cancel();
    let scan = options.clone();
    let prepared = tokio::task::spawn_blocking(move || {
        hybrid::prepare_sources(
            std::path::Path::new(&scan.input_path),
            scan.recursive,
            &scan.file_format,
        )
    })
    .await
    .map_err(|e| format!("准备已有标签失败: {}", e))??;
    if prepared.needs_json_conversion {
        let mut result = convert_prepared_tags(
            app,
            ConvertTagsOptions {
                input_path: options.input_path,
                model_id: options.model_id,
                json_simplified: options.json_simplified,
                recursive: options.recursive,
            },
            true,
        )
        .await?;
        result.success_count += prepared.copied;
        return Ok(result);
    }
    ProgressEvent::new(
        "done",
        format!(
            "已有标签准备完成: {} 个复用，{} 个无标签",
            prepared.copied,
            prepared.total - prepared.copied
        ),
    )
    .at(prepared.total, prepared.total)
    .emit(&app, "tagger-progress");
    Ok(ProcessResult {
        total: prepared.total,
        success_count: prepared.copied,
        ..Default::default()
    })
}

/// 将图片旁的 .txt 标签按模型词表分类后转换为 JSON。
/// 复用 Python 端的分类与 JSON 构建逻辑（--convert 模式，不加载 ONNX，速度快）。
#[tauri::command]
pub async fn convert_tags_to_json(
    app: tauri::AppHandle,
    options: ConvertTagsOptions,
) -> Result<ProcessResult, String> {
    let _busy = crate::commands::BusyGuard::acquire(&TAGGING_RUNNING, "打标")?;
    // 与打标共用取消标志：拿到锁后才能复位
    inference::reset_tagging_cancel();

    convert_prepared_tags(app, options, false).await
}

async fn convert_prepared_tags(
    app: tauri::AppHandle,
    options: ConvertTagsOptions,
    intermediate: bool,
) -> Result<ProcessResult, String> {
    let prepared = prepare_tagger(&app, &options.model_id, false).await;
    if inference::is_tagging_cancelled() {
        let result = ProcessResult::default();
        inference::emit_summary(&app, &result, 0, true);
        return Ok(result);
    }
    let (python, model_def, model_dir) = prepared?;
    let tags_path = model_dir.join(model_def.tags_basename());
    if !tags_path.exists() {
        return Err(format!("模型词表下载后仍不存在: {}", tags_path.display()));
    }
    tokio::task::spawn_blocking(move || {
        run_convert_tags(&app, &options, &python, &tags_path, intermediate)
    })
    .await
    .map_err(|e| format!("转换任务执行失败: {}", e))?
}

fn run_convert_tags<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    options: &ConvertTagsOptions,
    python: &str,
    tags_path: &std::path::Path,
    intermediate: bool,
) -> Result<ProcessResult, String> {
    use crate::commands::python_proc::{ProtocolReader, Recv, PYTHON_SILENCE_LIMIT};

    let mut args: Vec<String> = vec![
        "--convert".into(),
        "--input".into(),
        options.input_path.clone(),
        "--tags-path".into(),
        tags_path.to_string_lossy().into_owned(),
    ];
    if options.json_simplified {
        args.push("--simplified".into());
    }
    if options.recursive {
        args.push("--recursive".into());
    }
    if intermediate {
        args.push("--intermediate".into());
    }

    let script = crate::commands::python_proc::find_script("tagger_inference.py")?;

    let mut cmd = std::process::Command::new(python);
    cmd.arg(script.to_string_lossy().as_ref())
        .args(&args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .env("PYTHONUNBUFFERED", "1")
        .env("PYTHONIOENCODING", "utf-8");
    crate::commands::python_proc::configure_python_command(&mut cmd, false);

    let mut child = cmd
        .spawn()
        .map_err(|e| format!("启动转换进程失败: {}", e))?;
    let stdout = child.stdout.take().ok_or("无法获取转换进程输出")?;
    // stderr 必须另起线程并发读走：piped 之后不读，缓冲区一满 Python 就阻塞在写日志上。
    // 只留尾部几行：Python 的关键异常信息在最后，整段 traceback 灌进错误提示没法看
    const STDERR_TAIL_LINES: usize = 4;
    let stderr_reader = child.stderr.take().map(|se| {
        std::thread::spawn(move || {
            let mut tail = std::collections::VecDeque::with_capacity(STDERR_TAIL_LINES);
            crate::commands::python_proc::for_each_stderr_line(se, |l| {
                if tail.len() == STDERR_TAIL_LINES {
                    tail.pop_front();
                }
                tail.push_back(l);
            });
            tail
        })
    });
    // 登记到全局句柄，取消时 kill_python_process 才杀得到它
    inference::register_python_process(child);
    let _guard = inference::ProcessGuard;

    let reader = ProtocolReader::spawn(stdout);
    let mut result = ProcessResult::default();
    let mut skipped = 0u32;
    let mut current = 0u32;
    let mut got_done = false;
    let mut failure = None;
    loop {
        let received = reader.recv(PYTHON_SILENCE_LIMIT);
        if inference::is_tagging_cancelled() {
            break;
        }
        let msg = match received {
            Recv::Msg(msg) => msg,
            Recv::Closed => break,
            Recv::TimedOut => {
                failure = Some(format!(
                    "Python 超过 {} 秒无响应，已终止转换进程",
                    PYTHON_SILENCE_LIMIT.as_secs()
                ));
                inference::kill_python_process();
                break;
            }
        };
        match msg["type"].as_str().unwrap_or("") {
            "progress" => {
                current = msg["current"].as_u64().unwrap_or(0) as u32;
                result.total = msg["total"].as_u64().unwrap_or(0) as u32;
                let filename = msg["filename"].as_str().unwrap_or("");
                ProgressEvent::new("processing", format!("正在转换JSON: {}", filename))
                    .at(current, result.total)
                    .file(filename)
                    .emit(app, "tagger-progress");
            }
            "log" => {
                let mut progress = ProgressEvent::python_log(&msg, 0, result.total);
                progress.status = "warning".into();
                progress.emit(app, "tagger-progress");
            }
            "done" => {
                result.success_count = msg["converted"].as_u64().unwrap_or(0) as u32;
                result.fail_count = msg["failed"].as_u64().unwrap_or(0) as u32;
                result.total = msg["total"].as_u64().unwrap_or(0) as u32;
                skipped = msg["skipped"].as_u64().unwrap_or(0) as u32;
                got_done = true;
            }
            "error" => {
                failure = Some(msg["message"].as_str().unwrap_or("转换失败").to_string());
                inference::kill_python_process();
                break;
            }
            _ => {}
        }
    }
    let status = inference::take_python_process()
        .map(|mut c| c.wait().map_err(|e| format!("等待转换进程失败: {}", e)))
        .transpose()?;
    let stderr_tail = stderr_reader
        .and_then(|h| h.join().ok())
        .map(|tail| Vec::from(tail).join(" | "))
        .unwrap_or_default();
    if inference::is_tagging_cancelled() {
        ProgressEvent::new(
            "done",
            format!("已取消: 已完成 {}/{}", current, result.total),
        )
        .at(current, result.total)
        .emit(app, "tagger-progress");
        return Ok(result);
    }
    if let Some(message) = failure {
        return Err(message);
    }
    if !got_done || status.is_none_or(|s| !s.success()) {
        return Err(if stderr_tail.is_empty() {
            "转换进程异常退出".into()
        } else {
            format!("转换进程异常退出: {}", stderr_tail)
        });
    }
    ProgressEvent::new(
        "done",
        format!(
            "JSON转换完成：{} 个转换，{} 个跳过，{} 个失败",
            result.success_count, skipped, result.fail_count
        ),
    )
    .at(result.total, result.total)
    .emit(app, "tagger-progress");
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::batch::capture_events;
    use crate::commands::http_download::test_support::TempDir;
    use tauri::Listener;

    #[test]
    fn hybrid_conversion_protocol_preserves_txt_and_writes_private_json() {
        let _lock = TAGGER_TEST_LOCK.lock().unwrap();
        let temp = TempDir::new("hybrid_convert_protocol");
        let tags = temp.join("tags.csv");
        std::fs::write(&tags, "id,name,category\n0,solo,0\n").unwrap();
        let python =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../env/python/venv/bin/python3");
        let python = if python.exists() {
            python.to_string_lossy().into_owned()
        } else {
            "python3".into()
        };
        for simplified in [false, true] {
            let input = temp.join(simplified.to_string());
            std::fs::create_dir(&input).unwrap();
            let image = input.join("a.png");
            std::fs::write(&image, "image").unwrap();
            std::fs::write(image.with_extension("txt"), "solo").unwrap();
            std::fs::write(hybrid::draft_path(&image, "json"), "stale").unwrap();
            inference::reset_tagging_cancel();
            let prepared = hybrid::prepare_sources(&input, false, "json").unwrap();
            assert!(prepared.needs_json_conversion);
            assert!(!hybrid::draft_path(&image, "json").exists());
            let app = tauri::test::mock_app();
            let opts = ConvertTagsOptions {
                input_path: input.to_string_lossy().into_owned(),
                model_id: "mock".into(),
                json_simplified: simplified,
                recursive: false,
            };
            let result = run_convert_tags(app.handle(), &opts, &python, &tags, true).unwrap();
            assert_eq!((result.success_count, result.fail_count), (1, 0));
            let data: serde_json::Value = serde_json::from_str(
                &std::fs::read_to_string(hybrid::draft_path(&image, "json")).unwrap(),
            )
            .unwrap();
            assert_eq!(data.get("ai_output").is_some(), !simplified);
            assert!(!image.with_extension("json").exists());
            assert_eq!(
                std::fs::read_to_string(image.with_extension("txt")).unwrap(),
                "solo"
            );
        }
    }

    #[test]
    fn conversion_reads_protocol_and_cancels_with_one_done_event() {
        let _lock = TAGGER_TEST_LOCK.lock().unwrap();
        let temp = TempDir::new("tagger_convert");
        let tags = temp.join("tags.csv");
        std::fs::write(&tags, "id,name,category\n0,solo,0\n").unwrap();
        let python =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../env/python/venv/bin/python3");
        let python = if python.exists() {
            python.to_string_lossy().into_owned()
        } else {
            "python3".into()
        };
        for cancel in [false, true] {
            let input = temp.join(if cancel { "cancel" } else { "complete" });
            std::fs::create_dir_all(&input).unwrap();
            for i in 0..10 {
                std::fs::write(input.join(format!("{i}.png")), "fixture").unwrap();
                std::fs::write(input.join(format!("{i}.txt")), "solo").unwrap();
            }
            let app = tauri::test::mock_app();
            let events = capture_events(app.handle(), "tagger-progress");
            if cancel {
                app.listen_any("tagger-progress", |event| {
                    let value: serde_json::Value = serde_json::from_str(event.payload()).unwrap();
                    if value["status"] == "processing" {
                        inference::cancel_tagging();
                    }
                });
            }
            inference::reset_tagging_cancel();
            let opts = ConvertTagsOptions {
                input_path: input.to_string_lossy().into_owned(),
                model_id: "mock".into(),
                json_simplified: false,
                recursive: false,
            };
            let result = run_convert_tags(app.handle(), &opts, &python, &tags, false).unwrap();
            let events = events.lock().unwrap();
            let done: Vec<_> = events.iter().filter(|e| e["status"] == "done").collect();
            assert_eq!(done.len(), 1);
            if cancel {
                assert!(done[0]["message"].as_str().unwrap().starts_with("已取消"));
                assert!(!events.iter().any(|e| e["status"] == "error"));
            } else {
                assert_eq!(
                    (result.success_count, result.fail_count, result.total),
                    (10, 0, 10)
                );
                let output: serde_json::Value =
                    serde_json::from_slice(&std::fs::read(input.join("0.json")).unwrap()).unwrap();
                assert_eq!(output["ai_output"]["count"], "solo");
                assert_eq!(output["ai_output"]["tags"], serde_json::json!([]));
            }
            assert!(inference::take_python_process().is_none());
        }
        inference::reset_tagging_cancel();
    }
}
