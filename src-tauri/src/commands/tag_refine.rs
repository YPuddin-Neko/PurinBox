use base64::Engine;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;
use tauri::Emitter;

use super::{wait_for_global_llm_slot, ProcessResult, ProgressEvent};
use crate::commands::{collect_image_files_with_recursive_excluding, output_path_for_input};

static TAG_REFINE_CANCELLED: AtomicBool = AtomicBool::new(false);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TagRefineOptions {
    pub input_path: String,
    pub output_path: String,
    pub api_endpoint: String,
    pub api_key: String,
    pub model_name: String,
    pub prompt: String,
    pub temperature: f32,
    pub max_tokens: i32,
    #[serde(default = "default_image_size")]
    pub image_size: u32,
    #[serde(default)]
    pub top_p: f64,
    #[serde(default = "default_interval")]
    pub request_interval_ms: i64,
    #[serde(default = "default_concurrency")]
    pub concurrency: u32,
    #[serde(default)]
    pub recursive: bool,
    /// 标签文件格式: "txt"（默认）| "json"（完整/简化格式自动识别，差量写回保留原分类）
    #[serde(default = "default_file_format")]
    pub file_format: String,
}

fn default_file_format() -> String {
    "txt".to_string()
}

fn default_image_size() -> u32 {
    1024
}
fn default_interval() -> i64 {
    -1
}
fn default_concurrency() -> u32 {
    1
}

#[derive(Serialize)]
struct ChatMessage {
    role: String,
    content: serde_json::Value,
}

#[derive(Serialize)]
struct ChatRequest {
    model: String,
    messages: Vec<ChatMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
    temperature: f32,
    #[serde(skip_serializing_if = "Option::is_none")]
    top_p: Option<f64>,
}

#[derive(Deserialize)]
struct ChatResponse {
    choices: Vec<ChatChoice>,
}

#[derive(Deserialize)]
struct ChatChoice {
    message: ChatChoiceMessage,
    #[serde(default)]
    finish_reason: Option<String>,
}

#[derive(Deserialize)]
struct ChatChoiceMessage {
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    reasoning_content: Option<String>,
}

/// 处理单个文件的结果
enum FileResult {
    Success {
        filename: String,
        original_count: usize,
        refined_count: usize,
        changed: bool,
        warnings: Vec<String>,
        elapsed_ms: u128,
    },
    Skipped {
        filename: String,
        reason: String,
    },
    Error {
        filename: String,
        message: String,
    },
}

#[tauri::command]
pub fn cancel_tag_refining() {
    TAG_REFINE_CANCELLED.store(true, Ordering::SeqCst);
}

#[tauri::command]
pub async fn start_tag_refining(
    app: tauri::AppHandle,
    options: TagRefineOptions,
) -> Result<ProcessResult, String> {
    // 互斥：全局取消标志不允许并发运行（辅助打标与精修页并发会互吞取消）
    static REFINE_RUNNING: std::sync::atomic::AtomicBool =
        std::sync::atomic::AtomicBool::new(false);
    let _busy = crate::commands::BusyGuard::acquire(&REFINE_RUNNING, "标签精修")?;

    TAG_REFINE_CANCELLED.store(false, Ordering::SeqCst);

    let input_dir = Path::new(&options.input_path);
    let input_dir_path = input_dir.to_path_buf();
    let output_dir_path = PathBuf::from(&options.output_path);

    let mut files = collect_image_files_with_recursive_excluding(
        input_dir,
        options.recursive,
        Some(&output_dir_path),
    )?;
    // 历史版本会把警告/错误图复制进 _warnings/_errors，recursive 下不能把这些副本再当输入
    files.retain(|f| {
        !f.components().any(|c| {
            matches!(c.as_os_str().to_str(), Some("_warnings") | Some("_errors"))
        })
    });
    let total = files.len() as u32;

    if total == 0 {
        return Err("输入目录中没有找到图片文件".to_string());
    }

    std::fs::create_dir_all(&output_dir_path).map_err(|e| format!("创建输出目录失败: {}", e))?;

    let client = super::proxy_config::build_http_client_for_llm()
        .build()
        .map_err(|e| format!("创建 HTTP 客户端失败: {}", e))?;

    let concurrency = std::cmp::max(1, options.concurrency) as usize;

    let _ = app.emit(
        "tag-refine-progress",
        ProgressEvent {
            current: 0,
            total,
            filename: String::new(),
            status: "info".to_string(),
            message: format!("找到 {} 张图片，{} 线程开始标签细化...", total, concurrency),
            ..Default::default()
        },
    );

    let success_count = Arc::new(AtomicU32::new(0));
    let fail_count = Arc::new(AtomicU32::new(0));
    let processed = Arc::new(AtomicU32::new(0));
    let errors: Arc<tokio::sync::Mutex<Vec<String>>> =
        Arc::new(tokio::sync::Mutex::new(Vec::new()));
    let cancelled = Arc::new(AtomicBool::new(false));
    let error_files: Arc<tokio::sync::Mutex<Vec<PathBuf>>> =
        Arc::new(tokio::sync::Mutex::new(Vec::new()));
    let warning_files: Arc<tokio::sync::Mutex<Vec<PathBuf>>> =
        Arc::new(tokio::sync::Mutex::new(Vec::new()));

    let semaphore = Arc::new(tokio::sync::Semaphore::new(concurrency));
    let last_req_time = Arc::new(tokio::sync::Mutex::new(None));
    let mut handles = Vec::new();

    for file_path in files.iter() {
        if TAG_REFINE_CANCELLED.load(Ordering::SeqCst) {
            cancelled.store(true, Ordering::SeqCst);
            break;
        }

        let sem = semaphore.clone();
        let client = client.clone();
        let options = options.clone();
        let app = app.clone();
        let output_dir = output_dir_path.clone();
        let input_root = input_dir_path.clone();
        let file_path = file_path.clone();
        let success_count = success_count.clone();
        let fail_count = fail_count.clone();
        let processed = processed.clone();
        let errors = errors.clone();
        let cancelled = cancelled.clone();
        let error_files = error_files.clone();
        let warning_files = warning_files.clone();
        let last_req_time = last_req_time.clone();

        let handle = tokio::spawn(async move {
            if TAG_REFINE_CANCELLED.load(Ordering::SeqCst) {
                cancelled.store(true, Ordering::SeqCst);
                return;
            }

            let _permit = match sem.acquire().await {
                Ok(p) => p,
                Err(_) => return,
            };

            if TAG_REFINE_CANCELLED.load(Ordering::SeqCst) {
                cancelled.store(true, Ordering::SeqCst);
                return;
            }

            let result = tokio::select! {
                r = process_single_file(&client, &file_path, &input_root, &output_dir, &options, &last_req_time) => r,
                _ = async {
                    loop {
                        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                        if TAG_REFINE_CANCELLED.load(Ordering::SeqCst) { break; }
                    }
                } => {
                    cancelled.store(true, Ordering::SeqCst);
                    return;
                }
            };

            if TAG_REFINE_CANCELLED.load(Ordering::SeqCst) {
                cancelled.store(true, Ordering::SeqCst);
                return;
            }

            let cur = processed.fetch_add(1, Ordering::SeqCst) + 1;

            match result {
                FileResult::Success {
                    filename,
                    original_count,
                    refined_count,
                    changed,
                    warnings,
                    elapsed_ms,
                } => {
                    success_count.fetch_add(1, Ordering::SeqCst);
                    let has_warn = !warnings.is_empty();
                    if has_warn {
                        warning_files.lock().await.push(file_path.clone());
                    }
                    let elapsed_str = if elapsed_ms >= 1000 {
                        format!("{:.1}s", elapsed_ms as f64 / 1000.0)
                    } else {
                        format!("{}ms", elapsed_ms)
                    };
                    let warn_str = if has_warn {
                        format!(" ⚠ {}", warnings.join("; "))
                    } else {
                        String::new()
                    };
                    let _ = app.emit(
                        "tag-refine-progress",
                        ProgressEvent {
                            current: cur,
                            total,
                            filename: filename.clone(),
                            status: "success".to_string(),
                            message: format!(
                                "[完成] {} | 原TAG {} → 细化后 {} | {}{}{}",
                                filename,
                                original_count,
                                refined_count,
                                elapsed_str,
                                warn_str,
                                if !changed && !has_warn {
                                    " (未变化)"
                                } else {
                                    ""
                                }
                            ),
                            ..Default::default()
                        },
                    );
                }
                FileResult::Skipped { filename, reason } => {
                    success_count.fetch_add(1, Ordering::SeqCst);
                    let _ = app.emit(
                        "tag-refine-progress",
                        ProgressEvent {
                            current: cur,
                            total,
                            filename: filename.clone(),
                            status: "success".to_string(),
                            message: format!("[跳过] {} ({})", filename, reason),
                            ..Default::default()
                        },
                    );
                }
                FileResult::Error { filename, message } => {
                    fail_count.fetch_add(1, Ordering::SeqCst);
                    error_files.lock().await.push(file_path.clone());
                    errors
                        .lock()
                        .await
                        .push(format!("{}: {}", filename, message));
                    let _ = app.emit(
                        "tag-refine-progress",
                        ProgressEvent {
                            current: cur,
                            total,
                            filename: filename.clone(),
                            status: "error".to_string(),
                            message: format!("[错误] {}: {}", filename, message),
                            ..Default::default()
                        },
                    );
                }
            }
        });

        handles.push(handle);
    }

    for handle in handles {
        let _ = handle.await;
    }

    let sc = success_count.load(Ordering::SeqCst);
    let fc = fail_count.load(Ordering::SeqCst);
    let errs = errors.lock().await.clone();
    let was_cancelled =
        cancelled.load(Ordering::SeqCst) || TAG_REFINE_CANCELLED.load(Ordering::SeqCst);

    // 将出错文件复制到 _errors
    let err_files = error_files.lock().await.clone();
    let warn_files_list = warning_files.lock().await.clone();
    let mut copy_msg = String::new();

    // 输出目录==输入目录（辅助打标固定如此）时禁止复制：
    // 否则重复图片被灌进训练集本身，recursive 下次运行还会把副本当新图处理
    let same_io_dir = match (
        std::fs::canonicalize(&output_dir_path),
        std::fs::canonicalize(&input_dir_path),
    ) {
        (Ok(a), Ok(b)) => a == b,
        _ => crate::commands::path_key_ci(&output_dir_path) == crate::commands::path_key_ci(&input_dir_path),
    };
    if same_io_dir && (!err_files.is_empty() || !warn_files_list.is_empty()) {
        copy_msg.push_str("，输出与输入目录相同，已跳过 _errors/_warnings 复制");
    }

    if !err_files.is_empty() && !same_io_dir {
        let err_dir = output_dir_path.join("_errors");
        if std::fs::create_dir_all(&err_dir).is_ok() {
            let mut copied = 0u32;
            for src in &err_files {
                if let Some(name) = src.file_name() {
                    let name = name.to_string_lossy();
                    if let Ok(dest) = output_path_for_input(
                        &input_dir_path,
                        src,
                        &err_dir,
                        name.as_ref(),
                        options.recursive,
                    ) {
                        let _ = std::fs::copy(src, dest).map(|_| copied += 1);
                    }
                }
            }
            copy_msg.push_str(&format!("，{} 个错误文件已复制到 _errors/", copied));
        }
    }
    if !warn_files_list.is_empty() && !same_io_dir {
        let warn_dir = output_dir_path.join("_warnings");
        if std::fs::create_dir_all(&warn_dir).is_ok() {
            let mut copied = 0u32;
            for src in &warn_files_list {
                if let Some(name) = src.file_name() {
                    let name = name.to_string_lossy();
                    if let Ok(dest) = output_path_for_input(
                        &input_dir_path,
                        src,
                        &warn_dir,
                        name.as_ref(),
                        options.recursive,
                    ) {
                        let _ = std::fs::copy(src, dest).map(|_| copied += 1);
                    }
                }
            }
            copy_msg.push_str(&format!("，{} 个警告文件已复制到 _warnings/", copied));
        }
    }

    let _ = app.emit(
        "tag-refine-progress",
        ProgressEvent {
            current: total,
            total,
            filename: String::new(),
            status: "done".to_string(),
            message: if was_cancelled {
                format!(
                    "已取消: 成功 {}, 失败 {}, 共处理 {}/{}{}",
                    sc,
                    fc,
                    sc + fc,
                    total,
                    copy_msg
                )
            } else {
                format!(
                    "标签细化完成: 成功 {}, 失败 {}, 共 {}{}",
                    sc, fc, total, copy_msg
                )
            },
            ..Default::default()
        },
    );

    Ok(ProcessResult {
        success_count: sc,
        fail_count: fc,
        total,
        errors: errs,
    })
}


/// JSON 标签字段布局（完整格式 vs 简化格式）
/// string 字段是逗号分隔的标签串，array 字段是标签数组；新增标签落入 added_to
struct JsonTagLayout {
    string_fields: &'static [&'static [&'static str]],
    array_fields: &'static [&'static [&'static str]],
    added_to: &'static [&'static str],
}

const FULL_LAYOUT: JsonTagLayout = JsonTagLayout {
    string_fields: &[
        &["fixed", "quality"],
        &["fixed", "series"],
        &["fixed", "artist"],
        &["character", "name"],
        &["character", "variant"],
        &["ai_output", "count"],
    ],
    array_fields: &[
        &["ai_output", "appearance"],
        &["ai_output", "tags"],
        &["ai_output", "environment"],
        &["from_path", "appearance"],
    ],
    added_to: &["ai_output", "tags"],
};

const SIMPLIFIED_LAYOUT: JsonTagLayout = JsonTagLayout {
    string_fields: &[&["quality"], &["series"], &["artist"], &["character"], &["count"]],
    array_fields: &[&["appearance"], &["tags"], &["environment"]],
    added_to: &["tags"],
};

fn json_layout(data: &serde_json::Value) -> &'static JsonTagLayout {
    let is_full = ["ai_output", "fixed", "from_path"]
        .iter()
        .any(|k| data.get(k).map(|v| v.is_object()).unwrap_or(false))
        || data.get("character").map(|v| v.is_object()).unwrap_or(false);
    if is_full {
        &FULL_LAYOUT
    } else {
        &SIMPLIFIED_LAYOUT
    }
}

fn json_get_path<'a>(data: &'a serde_json::Value, path: &[&str]) -> Option<&'a serde_json::Value> {
    let mut cur = data;
    for key in path {
        cur = cur.get(key)?;
    }
    Some(cur)
}

fn json_get_path_mut<'a>(
    data: &'a mut serde_json::Value,
    path: &[&str],
) -> Option<&'a mut serde_json::Value> {
    let mut cur = data;
    for key in path {
        cur = cur.get_mut(key)?;
    }
    Some(cur)
}

/// 从 JSON 标签文件展开扁平标签列表（供 LLM 提示词使用）
fn flatten_json_tags(data: &serde_json::Value) -> Vec<String> {
    let layout = json_layout(data);
    let mut tags = Vec::new();
    for path in layout.string_fields {
        if let Some(v) = json_get_path(data, path).and_then(|v| v.as_str()) {
            tags.extend(v.split(',').map(|t| t.trim().to_string()).filter(|t| !t.is_empty()));
        }
    }
    for path in layout.array_fields {
        if let Some(arr) = json_get_path(data, path).and_then(|v| v.as_array()) {
            tags.extend(
                arr.iter()
                    .filter_map(|v| v.as_str())
                    .map(|t| t.trim().to_string())
                    .filter(|t| !t.is_empty()),
            );
        }
    }
    tags
}

/// 将 LLM 调优结果差量写回 JSON：
/// 保留的标签留在原字段（保序），被删除的从原字段移除，新增标签追加到通用 tags 数组。
/// 这样无需重新分类即可保持本地打标器给出的字段归属。
fn apply_refined_tags_to_json(data: &mut serde_json::Value, refined: &[String]) {
    let layout = json_layout(data);
    let refined_set: HashSet<&str> = refined.iter().map(|s| s.as_str()).collect();
    let mut seen: HashSet<String> = HashSet::new();

    for path in layout.string_fields {
        if let Some(slot) = json_get_path_mut(data, path) {
            if let Some(v) = slot.as_str() {
                let kept: Vec<String> = v
                    .split(',')
                    .map(|t| t.trim().to_string())
                    .filter(|t| !t.is_empty() && refined_set.contains(t.as_str()))
                    .collect();
                for t in &kept {
                    seen.insert(t.clone());
                }
                *slot = serde_json::Value::String(kept.join(", "));
            }
        }
    }
    for path in layout.array_fields {
        if let Some(slot) = json_get_path_mut(data, path) {
            if let Some(arr) = slot.as_array() {
                let kept: Vec<serde_json::Value> = arr
                    .iter()
                    .filter_map(|v| v.as_str())
                    .map(|t| t.trim().to_string())
                    .filter(|t| !t.is_empty() && refined_set.contains(t.as_str()))
                    .inspect(|t| {
                        seen.insert(t.clone());
                    })
                    .map(serde_json::Value::String)
                    .collect();
                *slot = serde_json::Value::Array(kept);
            }
        }
    }

    // 新增标签（按 LLM 返回顺序）追加到通用 tags 数组，路径缺失时逐级补建
    let added: Vec<&String> = refined.iter().filter(|t| !seen.contains(t.as_str())).collect();
    if added.is_empty() {
        return;
    }
    let mut cur = data;
    for (i, key) in layout.added_to.iter().enumerate() {
        if !cur.get(*key).map(|v| if i == layout.added_to.len() - 1 { v.is_array() } else { v.is_object() }).unwrap_or(false) {
            let empty = if i == layout.added_to.len() - 1 {
                serde_json::Value::Array(Vec::new())
            } else {
                serde_json::Value::Object(serde_json::Map::new())
            };
            if let Some(obj) = cur.as_object_mut() {
                obj.insert((*key).to_string(), empty);
            } else {
                return;
            }
        }
        cur = cur.get_mut(*key).unwrap();
    }
    if let Some(arr) = cur.as_array_mut() {
        for t in added {
            arr.push(serde_json::Value::String(t.clone()));
        }
    }
}

/// 处理单个文件：读取图片 + 对应标签 → LLM 细化
async fn process_single_file(
    client: &reqwest::Client,
    img_path: &Path,
    input_root: &Path,
    output_dir: &Path,
    options: &TagRefineOptions,
    last_req_time: &tokio::sync::Mutex<Option<std::time::Instant>>,
) -> FileResult {
    let start = std::time::Instant::now();
    let filename = img_path
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();
    let stem = img_path
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();
    let parent = img_path.parent().unwrap_or(Path::new("."));

    // 查找对应的标签文件（txt 或 json）
    let is_json = options.file_format == "json";
    let tag_ext = if is_json { "json" } else { "txt" };
    let tag_path = parent.join(format!("{}.{}", stem, tag_ext));
    if !tag_path.exists() {
        return FileResult::Skipped {
            filename,
            reason: format!("无对应 .{} 标签文件", tag_ext),
        };
    }

    let tag_content = match std::fs::read_to_string(&tag_path) {
        Ok(c) => c.trim().to_string(),
        Err(e) => {
            return FileResult::Error {
                filename,
                message: format!("读取标签文件失败: {}", e),
            }
        }
    };

    if tag_content.is_empty() {
        return FileResult::Skipped {
            filename,
            reason: "标签文件为空".to_string(),
        };
    }

    // json 模式：解析并扁平化标签；txt 模式：逗号拆分
    let mut json_data: Option<serde_json::Value> = None;
    let original_tags: Vec<String> = if is_json {
        let parsed: serde_json::Value = match serde_json::from_str(&tag_content) {
            Ok(v) => v,
            Err(e) => {
                return FileResult::Error {
                    filename,
                    message: format!("解析 JSON 标签失败: {}", e),
                }
            }
        };
        let tags = flatten_json_tags(&parsed);
        json_data = Some(parsed);
        tags
    } else {
        tag_content
            .split(',')
            .map(|t| t.trim().to_string())
            .filter(|t| !t.is_empty())
            .collect()
    };

    if original_tags.is_empty() {
        return FileResult::Skipped {
            filename,
            reason: "无有效标签".to_string(),
        };
    }

    // 调用 LLM 细化
    match refine_tags_with_llm(client, img_path, &original_tags, options, last_req_time).await {
        Ok(refined_tags) => {
            let elapsed_ms = start.elapsed().as_millis();
            let original_count = original_tags.len();
            let refined_count = refined_tags.len();
            let changed = refined_tags != original_tags;
            let mut warnings: Vec<String> = Vec::new();

            // 对比分析
            let orig_set: HashSet<&str> = original_tags.iter().map(|s| s.as_str()).collect();
            let refine_set: HashSet<&str> = refined_tags.iter().map(|s| s.as_str()).collect();

            let removed: Vec<&str> = orig_set.difference(&refine_set).copied().collect();
            let added: Vec<&str> = refine_set.difference(&orig_set).copied().collect();

            if !removed.is_empty() {
                let display: Vec<&str> = removed.iter().take(5).copied().collect();
                let suffix = if removed.len() > 5 {
                    format!("等{}个", removed.len())
                } else {
                    String::new()
                };
                warnings.push(format!("移除: {}{}", display.join(", "), suffix));
            }
            if !added.is_empty() {
                let display: Vec<&str> = added.iter().take(5).copied().collect();
                let suffix = if added.len() > 5 {
                    format!("等{}个", added.len())
                } else {
                    String::new()
                };
                warnings.push(format!("新增: {}{}", display.join(", "), suffix));
            }

            let output_name = format!("{}.{}", stem, tag_ext);
            let output_path = match output_path_for_input(
                input_root,
                img_path,
                output_dir,
                &output_name,
                options.recursive,
            ) {
                Ok(path) => path,
                Err(e) => {
                    return FileResult::Error {
                        filename,
                        message: e,
                    }
                }
            };
            let output_content = if let Some(mut data) = json_data {
                // 差量写回：保留原字段归属，仅应用增删
                apply_refined_tags_to_json(&mut data, &refined_tags);
                match serde_json::to_string_pretty(&data) {
                    Ok(s) => s,
                    Err(e) => {
                        return FileResult::Error {
                            filename,
                            message: format!("序列化 JSON 失败: {}", e),
                        }
                    }
                }
            } else {
                refined_tags.join(", ")
            };
            match std::fs::write(&output_path, &output_content) {
                Ok(_) => FileResult::Success {
                    filename,
                    original_count,
                    refined_count,
                    changed,
                    warnings,
                    elapsed_ms,
                },
                Err(e) => FileResult::Error {
                    filename,
                    message: format!("写入失败: {}", e),
                },
            }
        }
        Err(e) => FileResult::Error {
            filename,
            message: e,
        },
    }
}

/// 调用多模态 LLM 进行标签细化（发送图片 + 已有标签）
async fn refine_tags_with_llm(
    client: &reqwest::Client,
    img_path: &Path,
    tags: &[String],
    options: &TagRefineOptions,
    last_req_time: &tokio::sync::Mutex<Option<std::time::Instant>>,
) -> Result<Vec<String>, String> {
    // 读取并缩放图片
    let max_side = if options.image_size > 0 {
        options.image_size
    } else {
        1024
    };
    let img = image::ImageReader::open(img_path)
        .map_err(|e| format!("读取图片失败: {}", e))?
        .with_guessed_format()
        .map_err(|e| format!("无法识别图片格式: {}", e))?
        .decode()
        .map_err(|e| format!("无法解码图片: {}", e))?;

    let img = if img.width() > max_side || img.height() > max_side {
        img.resize(max_side, max_side, image::imageops::FilterType::Lanczos3)
    } else {
        img
    };

    // 编码为 JPEG base64（JPEG 编码器不接受 RGBA，透明图需先按白底拍平）
    let img = super::flatten_to_rgb_white(img);
    let mut buf = std::io::Cursor::new(Vec::new());
    img.write_to(&mut buf, image::ImageFormat::Jpeg)
        .map_err(|e| format!("编码图片失败: {}", e))?;
    let b64 = base64::engine::general_purpose::STANDARD.encode(buf.get_ref());
    let data_url = format!("data:image/jpeg;base64,{}", b64);

    let tag_list = tags.join(", ");

    // 构造 prompt
    let user_text = if options.prompt.contains("{tags}") {
        options.prompt.replace("{tags}", &tag_list)
    } else {
        format!(
            "{}\n\nExisting tags: {}\n\nRefined tags:",
            options.prompt, tag_list
        )
    };

    // 构造多模态请求（图片 + 文字）
    let messages = vec![ChatMessage {
        role: "user".to_string(),
        content: serde_json::json!([
            { "type": "text", "text": user_text },
            { "type": "image_url", "image_url": { "url": data_url } }
        ]),
    }];

    let request_body = ChatRequest {
        model: options.model_name.clone(),
        messages,
        max_tokens: if options.max_tokens > 0 {
            Some(options.max_tokens as u32)
        } else {
            None
        },
        temperature: options.temperature,
        top_p: if options.top_p > 0.0 && options.top_p <= 1.0 {
            Some(options.top_p)
        } else {
            None
        },
    };

    let endpoint = if options.api_endpoint.ends_with('/') {
        format!("{}chat/completions", options.api_endpoint)
    } else {
        format!("{}/chat/completions", options.api_endpoint)
    };

    let mut req = client
        .post(&endpoint)
        .header("Content-Type", "application/json")
        .json(&request_body);

    if !options.api_key.is_empty() {
        req = req.header("Authorization", format!("Bearer {}", options.api_key));
    }

    if !wait_for_global_llm_slot(
        last_req_time,
        options.request_interval_ms,
        &TAG_REFINE_CANCELLED,
    )
    .await
    {
        return Err("已取消".to_string());
    }

    let response = req
        .send()
        .await
        .map_err(|e| format!("API 请求失败: {}", e))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(format!("API 错误 ({}): {}", status, body));
    }

    let chat_resp: ChatResponse = response
        .json()
        .await
        .map_err(|e| format!("解析响应失败: {}", e))?;

    let choice = chat_resp
        .choices
        .first()
        .ok_or_else(|| "API 未返回任何结果".to_string())?;

    // 截断的响应是残缺的标签列表，写盘会把未包含的原标签全部删掉
    if choice.finish_reason.as_deref() == Some("length") {
        return Err("响应因 max_tokens 被截断，已丢弃（请调大 max_tokens）".to_string());
    }

    let content = choice
        .message
        .content
        .as_deref()
        .unwrap_or("")
        .trim()
        .to_string();
    let reasoning = choice
        .message
        .reasoning_content
        .as_deref()
        .unwrap_or("")
        .trim()
        .to_string();

    let final_content = if !content.is_empty() {
        content
    } else if !reasoning.is_empty() {
        reasoning
    } else {
        return Err("API 返回空内容".to_string());
    };

    // 解析返回的标签
    let cleaned = if final_content.contains('\n') {
        final_content
            .lines()
            .filter(|l| l.contains(','))
            .max_by_key(|l| l.len())
            .unwrap_or(&final_content)
            .to_string()
    } else {
        final_content
    };

    let refined_tags: Vec<String> = cleaned
        .split(',')
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
        .collect();

    if refined_tags.is_empty() {
        return Err("AI 返回的细化结果为空".to_string());
    }

    Ok(refined_tags)
}
