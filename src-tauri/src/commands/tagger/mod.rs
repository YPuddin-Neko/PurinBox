pub mod download;
pub mod hybrid;
pub mod inference;
pub mod llm_tagger;
pub mod models;

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::Duration;

use super::python_proc::{self, PythonCommand, PYTHON_SILENCE_LIMIT};
use super::{ProcessResult, ProgressEvent};
use crate::commands::python_env;
use inference::{Tally, EVENT};

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
    pub input_shape: Vec<i64>,
}

/// 文件名不是有效 UTF-8 的图片发不进 Python 的 JSON 协议，直接记为失败
pub(crate) const NON_UTF8_NAME: &str = "文件名不是有效的 UTF-8";

/// 图片旁有没有 `format`（"txt" / "json"）格式的标签：同名标签文件存在且不是空文件。
/// 「已有标签」只按这一条判断——打标跳过已有标签、辅助打标复用已有标签、VLM 打标跳过已有标签都用它。
pub(crate) fn has_label(image: &Path, format: &str) -> bool {
    std::fs::metadata(image.with_extension(format))
        .is_ok_and(|meta| meta.is_file() && meta.len() > 0)
}

/// 图片旁有没有任一格式（txt / json）的标签
pub(crate) fn has_labels(image: &Path) -> bool {
    has_label(image, "txt") || has_label(image, "json")
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
            // PixAI(deepghs 导出)是 id,tag_id,name,category,...——列位置不同。
            // 第一行没有 category 表头时按旧格式 tag_id,name,category 取列，这一行本身也是标签
            if let Ok(mut reader) = csv::ReaderBuilder::new()
                .has_headers(false)
                .from_path(tags_path)
            {
                let mut records = reader.records().flatten();
                let first = records.next();
                let header_idx = first.as_ref().and_then(|row| {
                    row.iter()
                        .position(|c| c.trim().eq_ignore_ascii_case("category"))
                });
                let cat_idx = header_idx.unwrap_or(2);
                let first_row = first.filter(|_| header_idx.is_none());
                for record in first_row.into_iter().chain(records) {
                    if let Some(Ok(cat_id)) = record.get(cat_idx).map(|c| c.trim().parse::<i32>()) {
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

/// 自动检测 ONNX 模型的输入尺寸
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

// 打标和准备阶段共用子进程槽及取消标志，必须共用同一把互斥锁。
static TAGGING_RUNNING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

#[cfg(test)]
pub(crate) static TAGGER_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// 拿到打标互斥锁之后的共同开头：复位取消、丢掉本功能排队中的环境部署取消、开始新一轮事件
fn begin_tagging_run() {
    inference::reset_tagging_cancel();
    python_env::clear_pending_cancel("tagger");
    super::begin_run(EVENT);
}

/// 开始打标
#[tauri::command]
pub async fn start_tagging(
    app: tauri::AppHandle,
    options: TaggerOptions,
) -> Result<ProcessResult, String> {
    let _busy = crate::commands::BusyGuard::acquire(&TAGGING_RUNNING, "打标")?;
    begin_tagging_run();

    if options.hybrid_mode {
        let scan_options = options.clone();
        let skipped = tokio::task::spawn_blocking(move || inference::all_skipped(&scan_options))
            .await
            .map_err(|e| format!("读取图片失败: {}", e))??;
        if let Some(result) = skipped {
            inference::finish_tagging(
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
        inference::finish_tagging(&app, &result, 0, true);
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
        ProgressEvent::new("info", format!("模型 {} 未下载，开始下载...", model.name))
            .emit(app, EVENT);
        download::download_model(app, model).await?;
        if inference::is_tagging_cancelled() {
            return Err("已取消".into());
        }
    }
    Ok(get_model_dir(&model.id))
}

// ═══════════════ 辅助打标：准备已有标签 ═══════════════

/// 辅助打标「优先使用已有标签」时的第一步，见 `prepare_hybrid_tags`
#[derive(Debug, Clone, serde::Deserialize)]
pub struct PrepareHybridTagsOptions {
    pub input_path: String,
    /// 提供标签分类词表的模型：JSON 输出时把已有 txt 标签按它的词表分类（缺文件时先下载）
    pub model_id: String,
    /// 输出格式："txt" | "json"
    pub file_format: String,
    #[serde(default)]
    pub json_simplified: bool,
    #[serde(default)]
    pub recursive: bool,
}

/// 准备已有标签：清掉残留草稿，已有标签复制成草稿；JSON 输出且只有 txt 标签的图片按模型词表
/// 转换成 JSON 草稿（`tagger_inference.py --convert`，只加载词表，不加载 ONNX）。
///
/// 转换失败的图片计入失败并复制进 Fail/，其余照常完成；转换进程中途异常退出时，
/// 没转换的图片也计为失败，并返回 Err。
#[tauri::command]
pub async fn prepare_hybrid_tags(
    app: tauri::AppHandle,
    options: PrepareHybridTagsOptions,
) -> Result<ProcessResult, String> {
    let _busy = crate::commands::BusyGuard::acquire(&TAGGING_RUNNING, "打标")?;
    begin_tagging_run();

    let scan = options.clone();
    let prepared = tokio::task::spawn_blocking(move || {
        hybrid::prepare_sources(
            Path::new(&scan.input_path),
            scan.recursive,
            &scan.file_format,
        )
    })
    .await
    .map_err(|e| format!("准备已有标签失败: {}", e))??;

    let mut converter = None;
    if !prepared.to_convert.is_empty() && !inference::is_tagging_cancelled() {
        let tools = prepare_tagger(&app, &options.model_id, false).await;
        if !inference::is_tagging_cancelled() {
            let (python, model, dir) = tools?;
            let tags_path = dir.join(model.tags_basename());
            if !tags_path.exists() {
                return Err(format!("模型词表下载后仍不存在: {}", tags_path.display()));
            }
            converter = Some(Converter {
                python,
                script: python_proc::find_script("tagger_inference.py")?,
                tags_path,
                silence: PYTHON_SILENCE_LIMIT,
            });
        }
    }
    tokio::task::spawn_blocking(move || {
        complete_preparation(&app, &options, prepared, converter.as_ref())
    })
    .await
    .map_err(|e| format!("准备已有标签失败: {}", e))?
}

/// txt → JSON 草稿转换用的解释器、脚本和词表
struct Converter {
    python: String,
    script: PathBuf,
    tags_path: PathBuf,
    /// 超过这么久收不到转换进程的消息就判定它卡死
    silence: Duration,
}

/// 扫描之后的部分：转换、失败文件复制进 Fail/、终态 done。
/// `converter` 只在有图片要转换且没有取消时给出
fn complete_preparation<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    options: &PrepareHybridTagsOptions,
    prepared: hybrid::Preparation,
    converter: Option<&Converter>,
) -> Result<ProcessResult, String> {
    let input = Path::new(&options.input_path);
    let mut tally = Tally::new(app, prepared.total);
    tally.counts.success = prepared.copied;
    tally.counts.skipped = prepared.unlabeled;
    for (image, reason) in &prepared.failed {
        tally.fail(image, reason);
    }
    let mut converted = 0;
    if let Some(converter) = converter.filter(|_| !inference::is_tagging_cancelled()) {
        match run_convert_tags(
            &mut tally,
            converter,
            options.json_simplified,
            &prepared.to_convert,
        ) {
            Ok(count) => converted = count,
            Err(abort) if abort.started => {
                return Err(tally.abort(
                    input,
                    options.recursive,
                    &abort.reason,
                    &abort.unprocessed,
                ))
            }
            Err(abort) => return Err(tally.give_up(input, options.recursive, abort.reason)),
        }
    }
    let cancelled = inference::is_tagging_cancelled();
    let copied = prepared.copied;
    Ok(tally.finish(input, options.recursive, cancelled, |counts| {
        let mut parts = vec![format!("{} 个复用", copied)];
        if converted > 0 {
            parts.push(format!("{} 个转换", converted));
        }
        parts.push(format!("{} 个无标签", counts.skipped));
        if counts.failed > 0 {
            parts.push(format!("{} 个失败", counts.failed));
        }
        format!("已有标签准备完成: {}", parts.join("，"))
    }))
}

/// 转换没能正常结束（脚本报错、进程异常退出、无响应、协议出错）
struct ConvertAbort {
    reason: String,
    /// 脚本已读好词表和清单、开始逐张转换
    started: bool,
    /// 没有转换结果的图片
    unprocessed: Vec<PathBuf>,
}

/// 把 `images` 旁的 .txt 标签转换成 JSON 草稿（`hybrid::draft_path(image, "json")`），返回成功数。
/// 每张转换完发一条 processing，转换失败的计入 `tally` 的失败；被取消时返回已转换的数量。
fn run_convert_tags<R: tauri::Runtime>(
    tally: &mut Tally<'_, R>,
    converter: &Converter,
    simplified: bool,
    images: &[PathBuf],
) -> Result<u32, ConvertAbort> {
    let mut items = Vec::new();
    let mut pending = Vec::new();
    for image in images {
        let source = image.with_extension("txt");
        let output = hybrid::draft_path(image, "json");
        match (image.to_str(), source.to_str(), output.to_str()) {
            (Some(image_path), Some(source_path), Some(output_path)) => {
                items.push(serde_json::json!({
                    "image_path": image_path,
                    "source_path": source_path,
                    "output_path": output_path,
                }));
                pending.push(image);
            }
            _ => tally.fail(image, NON_UTF8_NAME),
        }
    }
    if pending.is_empty() {
        return Ok(0);
    }
    let manifest =
        python_proc::ManifestFile::write("purinbox-tagger-convert", &items).map_err(|reason| {
            ConvertAbort {
                reason,
                started: false,
                unprocessed: Vec::new(),
            }
        })?;

    let mut command = PythonCommand::new(&converter.python)
        .arg(&converter.script)
        .arg("--convert")
        .arg("--manifest")
        .arg(manifest.path())
        .arg("--tags-path")
        .arg(&converter.tags_path);
    if simplified {
        command = command.arg("--simplified");
    }
    let mut started = false;
    let mut finished = false;
    let mut converted = 0;
    let exit = python_proc::run_json_lines_script_with(
        command,
        Some(converter.silence),
        &inference::PYTHON_PROCESS,
        &inference::TAGGING_CANCELLED,
        |_| {},
        |msg| {
            match msg["type"].as_str().unwrap_or("") {
                "ready" => started = true,
                "log" => tally.log(&msg),
                kind @ ("result" | "error") => {
                    let Some(path) = msg["image_path"].as_str() else {
                        return Err(msg["message"]
                            .as_str()
                            .unwrap_or("转换脚本出错")
                            .to_string());
                    };
                    let Some(index) = pending.iter().position(|p| p.to_str() == Some(path)) else {
                        return Err(format!("Python 返回了无法对应的结果: {}", path));
                    };
                    let image = pending.remove(index);
                    if kind == "result" {
                        converted += 1;
                        let name = crate::commands::file_name_lossy(image);
                        tally.succeed(image, "processing", format!("正在转换JSON: {}", name));
                    } else {
                        tally.fail(image, msg["message"].as_str().unwrap_or("转换失败"));
                    }
                }
                "done" => finished = true,
                _ => {}
            }
            Ok(())
        },
    );
    if inference::is_tagging_cancelled() {
        return Ok(converted);
    }
    let reason = match exit {
        Err(reason) => reason,
        Ok(exit) if exit.cancelled => return Ok(converted),
        Ok(exit) if exit.timed_out => format!(
            "Python 超过 {} 秒无响应，已终止转换进程",
            converter.silence.as_secs()
        ),
        Ok(exit) if !finished || exit.code != Some(0) => {
            if exit.stderr_tail.is_empty() {
                "转换进程异常退出".to_string()
            } else {
                format!("转换进程异常退出: {}", exit.stderr_tail)
            }
        }
        Ok(_) => {
            for image in pending {
                tally.fail(image, "转换进程没有返回这张图的结果");
            }
            return Ok(converted);
        }
    };
    Err(ConvertAbort {
        reason,
        started,
        unprocessed: pending.into_iter().cloned().collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::batch::capture_events;
    use crate::commands::test_support::{test_python, TempDir};
    use serde_json::{json, Value};
    use tauri::Listener;

    fn converter(root: &Path, script: Option<&str>) -> Converter {
        let tags_path = root.join("tags.csv");
        std::fs::write(&tags_path, "id,name,category\n0,solo,0\n1,blue_hair,0\n").unwrap();
        let script = match script {
            Some(source) => {
                let path = root.join("fake_convert.py");
                std::fs::write(&path, source).unwrap();
                path
            }
            None => python_proc::find_script("tagger_inference.py").unwrap(),
        };
        Converter {
            python: test_python(),
            script,
            tags_path,
            silence: Duration::from_secs(30),
        }
    }

    fn options(input: &Path, file_format: &str, simplified: bool) -> PrepareHybridTagsOptions {
        PrepareHybridTagsOptions {
            input_path: input.to_string_lossy().into_owned(),
            model_id: "mock".into(),
            file_format: file_format.into(),
            json_simplified: simplified,
            recursive: true,
        }
    }

    fn prepare(
        options: &PrepareHybridTagsOptions,
        converter: Option<&Converter>,
    ) -> (Result<ProcessResult, String>, Vec<Value>) {
        let app = tauri::test::mock_app();
        let events = capture_events(app.handle(), EVENT);
        let prepared = hybrid::prepare_sources(
            Path::new(&options.input_path),
            options.recursive,
            &options.file_format,
        )
        .unwrap();
        let result = complete_preparation(app.handle(), options, prepared, converter);
        let events = events.lock().unwrap().clone();
        (result, events)
    }

    fn terminal(events: &[Value]) -> Vec<&Value> {
        events.iter().filter(|e| e["status"] == "done").collect()
    }

    #[test]
    fn has_label_needs_a_non_empty_file_of_that_format() {
        let root = TempDir::new("tagger_has_label");
        let image = root.join("a.png");
        assert!(!has_label(&image, "txt"));
        std::fs::write(image.with_extension("txt"), "").unwrap();
        assert!(!has_label(&image, "txt"));
        std::fs::write(image.with_extension("txt"), b"\xc4\xe3\xba\xc3").unwrap();
        assert!(has_label(&image, "txt"));
        assert!(!has_label(&image, "json"));
        std::fs::create_dir(image.with_extension("json")).unwrap();
        assert!(!has_label(&image, "json"));
    }

    #[test]
    fn headerless_csv_vocabulary_keeps_its_first_row() {
        let root = TempDir::new("tagger_csv_categories");
        let path = root.join("tags.csv");
        for (content, expected) in [
            ("0,a,4\n1,b,0\n", vec!["character", "general"]),
            ("tag_id,name,category\n0,a,4\n", vec!["character"]),
            ("id,tag_id,name,category,count\n0,0,a,9,1\n", vec!["rating"]),
        ] {
            std::fs::write(&path, content).unwrap();
            assert_eq!(detect_supported_categories(&path), expected, "{content}");
        }
    }

    /// 复用、转换、无标签、转换失败各算一类；失败的图进 Fail/，原 txt 不动，JSON 草稿两种布局都对
    #[test]
    fn preparation_converts_txt_into_json_drafts_and_reports_failures() {
        let _lock = TAGGER_TEST_LOCK.lock().unwrap();
        for simplified in [false, true] {
            inference::reset_tagging_cancel();
            let temp = TempDir::new("hybrid_prepare_convert");
            let input = temp.join("dataset");
            std::fs::create_dir_all(input.join("nested")).unwrap();
            std::fs::create_dir_all(input.join("Fail")).unwrap();
            for name in [
                "a.png",
                "reuse.png",
                "none.png",
                "bad.png",
                "nested/deep.png",
                "Fail/x.png",
            ] {
                std::fs::write(input.join(name), "image").unwrap();
            }
            for name in ["a.txt", "nested/deep.txt", "Fail/x.txt"] {
                std::fs::write(input.join(name), "solo, blue hair").unwrap();
            }
            std::fs::write(input.join("reuse.json"), r#"{"ai_output":{"nl":"keep"}}"#).unwrap();
            std::fs::write(input.join("bad.txt"), b"\xff\xfe\xff").unwrap();
            std::fs::write(hybrid::draft_path(&input.join("a.png"), "json"), "stale").unwrap();
            std::fs::write(input.join("old.png.purin-local-json"), "orphan").unwrap();

            let options = options(&input, "json", simplified);
            let converter = converter(&temp, None);
            let (result, events) = prepare(&options, Some(&converter));
            let result = result.unwrap();
            assert_eq!(
                (result.success_count, result.fail_count, result.total),
                (3, 1, 5),
                "{events:?}"
            );
            assert!(
                result.errors[0].starts_with("bad.png: 转换失败"),
                "{:?}",
                result.errors
            );
            assert!(input.join("Fail/bad.png").exists());
            assert!(!input.join("Fail/a.png").exists());
            assert!(!input.join("old.png.purin-local-json").exists());
            for image in ["a.png", "nested/deep.png"] {
                let image = input.join(image);
                let draft: Value = serde_json::from_str(
                    &std::fs::read_to_string(hybrid::draft_path(&image, "json")).unwrap(),
                )
                .unwrap();
                assert_eq!(draft.get("ai_output").is_some(), !simplified);
                assert!(draft.to_string().contains("blue hair"));
                assert!(!image.with_extension("json").exists());
                assert_eq!(
                    std::fs::read_to_string(image.with_extension("txt")).unwrap(),
                    "solo, blue hair"
                );
            }
            assert!(!hybrid::draft_path(&input.join("bad.png"), "json").exists());
            assert!(!hybrid::draft_path(&input.join("Fail/x.png"), "json").exists());
            assert_eq!(
                std::fs::read_to_string(hybrid::draft_path(&input.join("reuse.png"), "json"))
                    .unwrap(),
                r#"{"ai_output":{"nl":"keep"}}"#
            );
            let errors: Vec<_> = events.iter().filter(|e| e["status"] == "error").collect();
            assert_eq!(errors.len(), 1);
            assert_eq!(errors[0]["filename"], "bad.png");
            let done = terminal(&events);
            assert_eq!(done.len(), 1);
            assert_eq!(
                done[0]["message"],
                "已有标签准备完成: 1 个复用，2 个转换，1 个无标签，1 个失败"
            );
            assert_eq!(
                (done[0]["current"].as_u64(), done[0]["total"].as_u64()),
                (Some(5), Some(5))
            );
            assert!(inference::PYTHON_PROCESS.lock().unwrap().is_none());
        }
    }

    #[test]
    fn preparation_without_conversion_needs_no_python() {
        let _lock = TAGGER_TEST_LOCK.lock().unwrap();
        inference::reset_tagging_cancel();
        let input = TempDir::new("hybrid_prepare_txt");
        for name in ["a.png", "b.png"] {
            std::fs::write(input.join(name), "image").unwrap();
        }
        std::fs::write(input.join("a.txt"), "solo").unwrap();
        let (result, events) = prepare(&options(&input, "txt", false), None);
        let result = result.unwrap();
        assert_eq!(
            (result.success_count, result.fail_count, result.total),
            (1, 0, 2)
        );
        assert_eq!(
            terminal(&events)[0]["message"],
            "已有标签准备完成: 1 个复用，1 个无标签"
        );
    }

    /// 准备阶段被取消：恰好一条带 cancelled 的终态 done，命令照常返回
    #[test]
    fn cancelled_preparation_reports_one_cancelled_done() {
        let _lock = TAGGER_TEST_LOCK.lock().unwrap();
        let input = TempDir::new("hybrid_prepare_cancelled");
        for name in ["a.png", "b.png"] {
            std::fs::write(input.join(name), "image").unwrap();
            std::fs::write(input.join(name).with_extension("txt"), "solo").unwrap();
        }
        inference::cancel_tagging();
        let (result, events) = prepare(&options(&input, "json", false), None);
        inference::reset_tagging_cancel();
        assert_eq!(result.unwrap().total, 2);
        assert_eq!(
            events,
            [
                json!({"current": 0, "total": 2, "filename": "", "status": "done",
                    "message": "已取消: 已处理 0/2, 成功 0, 失败 0", "cancelled": true})
            ]
        );
    }

    #[test]
    fn conversion_cancelled_midway_keeps_finished_drafts() {
        let _lock = TAGGER_TEST_LOCK.lock().unwrap();
        inference::reset_tagging_cancel();
        let temp = TempDir::new("hybrid_convert_cancel");
        let input = temp.join("dataset");
        std::fs::create_dir(&input).unwrap();
        for i in 0..3 {
            std::fs::write(input.join(format!("{i}.png")), "image").unwrap();
            std::fs::write(input.join(format!("{i}.txt")), "solo").unwrap();
        }
        let script = r#"
import json, sys, time
items = json.load(open(sys.argv[sys.argv.index('--manifest') + 1], encoding='utf-8'))
print(json.dumps({'type': 'ready'}), flush=True)
open(items[0]['output_path'], 'w').write('{}')
print(json.dumps({'type': 'result', 'image_path': items[0]['image_path'], 'tag_count': 1}), flush=True)
time.sleep(30)
"#;
        let converter = converter(&temp, Some(script));
        let app = tauri::test::mock_app();
        let events = capture_events(app.handle(), EVENT);
        app.listen_any(EVENT, |event| {
            let value: Value = serde_json::from_str(event.payload()).unwrap();
            if value["status"] == "processing" {
                inference::cancel_tagging();
            }
        });
        let options = options(&input, "json", false);
        let prepared = hybrid::prepare_sources(&input, true, "json").unwrap();
        let started = std::time::Instant::now();
        let result = complete_preparation(app.handle(), &options, prepared, Some(&converter));
        inference::reset_tagging_cancel();
        assert!(started.elapsed() < Duration::from_secs(15));
        let result = result.unwrap();
        assert_eq!((result.success_count, result.fail_count), (1, 0));
        assert!(!input.join("Fail").exists());
        let events = events.lock().unwrap();
        let done = terminal(&events);
        assert_eq!(done.len(), 1);
        assert_eq!(done[0]["cancelled"], true);
        assert_eq!(done[0]["message"], "已取消: 已处理 1/3, 成功 1, 失败 0");
    }

    /// 转换进程中途异常（退出、无响应、结果对不上）：没转换的图计为失败、进 Fail/，返回 Err，不发 done；
    /// 还没读好词表就失败时直接报原因，不归集
    #[test]
    fn interrupted_conversion_fails_the_rest() {
        let _lock = TAGGER_TEST_LOCK.lock().unwrap();
        let head = r#"
import json, sys, time
items = json.load(open(sys.argv[sys.argv.index('--manifest') + 1], encoding='utf-8'))
def emit(**v): print(json.dumps(v), flush=True)
"#;
        let first = "emit(type='ready'); emit(type='result', image_path=items[0]['image_path'], tag_count=1)\n";
        for (case, body, expected) in [
            ("exit", format!("{first}sys.stderr.write('MemoryError\\n'); sys.exit(1)"), "转换进程异常退出: MemoryError，2 张未处理"),
            ("silent", format!("{first}time.sleep(30)"), "Python 超过 3 秒无响应"),
            ("mismatch", format!("{first}emit(type='result', image_path='elsewhere.png', tag_count=1); time.sleep(30)"), "Python 返回了无法对应的结果: elsewhere.png，2 张未处理"),
            ("vocabulary", "emit(type='error', message='读取词表或转换清单失败: broken')".to_string(), "读取词表或转换清单失败: broken"),
        ] {
            inference::reset_tagging_cancel();
            let temp = TempDir::new("hybrid_convert_abort");
            let input = temp.join("dataset");
            std::fs::create_dir(&input).unwrap();
            for i in 0..3 {
                std::fs::write(input.join(format!("{i}.png")), "image").unwrap();
                std::fs::write(input.join(format!("{i}.txt")), "solo").unwrap();
            }
            let mut converter = converter(&temp, Some(&format!("{head}{body}")));
            converter.silence = Duration::from_secs(3);
            let started = std::time::Instant::now();
            let (result, events) = prepare(&options(&input, "json", false), Some(&converter));
            assert!(started.elapsed() < Duration::from_secs(15), "{case}");
            let error = result.unwrap_err();
            assert!(error.contains(expected), "{case}: {error}");
            assert!(terminal(&events).is_empty(), "{case}: {events:?}");
            if case == "vocabulary" {
                assert!(!input.join("Fail").exists(), "{case}");
            } else {
                assert!(error.ends_with("成功 1, 失败 2, 共 3"), "{case}: {error}");
                assert!(!input.join("Fail/0.png").exists(), "{case}");
                assert!(input.join("Fail/1.png").exists(), "{case}");
                assert!(input.join("Fail/2.png").exists(), "{case}");
            }
            assert!(inference::PYTHON_PROCESS.lock().unwrap().is_none());
        }
    }

    /// 非 UTF-8 文件名放不进清单：直接记失败，其余照常转换。
    /// 路径不必真的存在（APFS 等文件系统不接受这种文件名），它不会被读
    #[cfg(unix)]
    #[test]
    fn non_utf8_names_fail_without_reaching_the_converter() {
        use std::os::unix::ffi::OsStrExt;
        let _lock = TAGGER_TEST_LOCK.lock().unwrap();
        inference::reset_tagging_cancel();
        let temp = TempDir::new("hybrid_convert_non_utf8");
        let good = temp.join("good.png");
        std::fs::write(&good, "image").unwrap();
        std::fs::write(good.with_extension("txt"), "solo").unwrap();
        let odd = temp.join(std::ffi::OsStr::from_bytes(b"bad\xff.png"));
        let app = tauri::test::mock_app();
        let events = capture_events(app.handle(), EVENT);
        let mut tally = Tally::new(app.handle(), 2);
        let converted = run_convert_tags(
            &mut tally,
            &converter(&temp, None),
            false,
            &[odd.clone(), good.clone()],
        )
        .unwrap_or_else(|abort| panic!("{}", abort.reason));
        assert_eq!(converted, 1);
        let result = tally.finish(&temp, false, false, |c| c.summary("完成"));
        assert_eq!((result.success_count, result.fail_count), (1, 1));
        assert_eq!(
            result.errors,
            [format!(
                "{}: {}",
                crate::commands::file_name_lossy(&odd),
                NON_UTF8_NAME
            )]
        );
        assert!(hybrid::draft_path(&good, "json").exists());
        assert_eq!(terminal(&events.lock().unwrap()).len(), 1);
    }
}
