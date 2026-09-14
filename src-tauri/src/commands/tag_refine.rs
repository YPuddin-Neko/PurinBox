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
    /// OpenAI Vision 的 image_url.detail：low / high / original / auto。
    /// 空则整个字段不发送——Gemini 兼容层等不认识它的端点会因未知字段报错
    #[serde(default)]
    pub image_detail: String,
    /// 自然语言打标模式（仅 txt）：LLM 的回复整段就是标签文件的内容，
    /// 不做标签解析、不做增删对比。本地标签只作为 LLM 的校准参考，不写回文件。
    /// 与 JSON 的 nl 字段无关——JSON 路径完全不走这里
    #[serde(default)]
    pub caption_mode: bool,
    /// LoRA 触发词。txt 无论标签还是自然语言都强制置于最前；JSON 追加进 artist 字段。
    /// 由后端保证，不依赖 LLM 遵守提示词
    #[serde(default)]
    pub trigger_word: String,
    /// 保集合模式（仅 JSON）：LLM 只决定标签归属，不许增删。
    /// 标签只从原字段读取，LLM 新增的丢弃、漏掉的留原地——由后端保证，
    /// 不指望模型遵守"不要增删"的嘱咐
    #[serde(default)]
    pub preserve_tags: bool,
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
#[derive(Debug)]
enum FileResult {
    Success {
        filename: String,
        original_count: usize,
        refined_count: usize,
        changed: bool,
        warnings: Vec<String>,
        elapsed_ms: u128,
    },
    /// 自然语言打标：写入的是一整段描述，没有"标签数"和增删可言
    Captioned {
        filename: String,
        original_count: usize,
        word_count: usize,
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
    // 输入可以是单张图片，辅助打标又固定把输入路径当输出路径传进来。
    // 不取所在目录的话，下面的 create_dir_all 会试图创建一个与图片同名的目录
    let output_dir_path = crate::commands::dir_of(Path::new(&options.output_path));

    // 失败/警告图副本落在 Fail、Warn 里，收集侧已统一剪枝，不会被当成新图
    let files = collect_image_files_with_recursive_excluding(
        input_dir,
        options.recursive,
        Some(&output_dir_path),
    )?;
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
                FileResult::Captioned {
                    filename,
                    original_count,
                    word_count,
                    elapsed_ms,
                } => {
                    success_count.fetch_add(1, Ordering::SeqCst);
                    let elapsed_str = if elapsed_ms >= 1000 {
                        format!("{:.1}s", elapsed_ms as f64 / 1000.0)
                    } else {
                        format!("{}ms", elapsed_ms)
                    };
                    let _ = app.emit(
                        "tag-refine-progress",
                        ProgressEvent {
                            current: cur,
                            total,
                            filename: filename.clone(),
                            status: "success".to_string(),
                            message: format!(
                                "[完成] {} | 参考 {} 个标签 → 描述 {} 词 | {}",
                                filename, original_count, word_count, elapsed_str
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

    let err_files = error_files.lock().await.clone();
    let warn_files_list = warning_files.lock().await.clone();
    let mut copy_msg = String::new();

    // 单图输入时 input_dir_path 是文件，得跟它所在目录比，否则"就地更新"会被误判成
    // 输入输出不同目录，把整份警告图复制一遍
    let input_cmp_dir = crate::commands::dir_of(&input_dir_path);
    let same_io_dir = match (
        std::fs::canonicalize(&output_dir_path),
        std::fs::canonicalize(&input_cmp_dir),
    ) {
        (Ok(a), Ok(b)) => a == b,
        _ => {
            crate::commands::path_key_ci(&output_dir_path)
                == crate::commands::path_key_ci(&input_cmp_dir)
        }
    };

    // 出错图片一律复制出来备查——辅助打标固定就地更新（输出==输入），
    // 这里若跟着跳过，失败的是哪几张就再也找不回来了。
    // 副本落在产物目录内，收集侧已剪枝，下一轮不会被当成新图。
    if !err_files.is_empty() {
        match crate::commands::copy_files_into_artifact_dir(
            &input_dir_path,
            &output_dir_path,
            &err_files,
            crate::commands::FAIL_DIR_NAME,
            options.recursive,
        ) {
            Ok(copied) => copy_msg.push_str(&format!(
                "，{} 个错误文件已复制到 {}/",
                copied,
                crate::commands::FAIL_DIR_NAME
            )),
            Err(e) => copy_msg.push_str(&format!("，{}", e)),
        }
    }

    // 警告=LLM 增删了标签，正是调优该干的事，就地模式下几乎每张图都命中：
    // 真去复制等于把整个数据集又存了一遍，所以这一项保留就地跳过
    if !warn_files_list.is_empty() {
        if same_io_dir {
            copy_msg.push_str(&format!(
                "，输出与输入目录相同，已跳过 {}/ 复制",
                crate::commands::WARN_DIR_NAME
            ));
        } else {
            match crate::commands::copy_files_into_artifact_dir(
                &input_dir_path,
                &output_dir_path,
                &warn_files_list,
                crate::commands::WARN_DIR_NAME,
                options.recursive,
            ) {
                Ok(copied) => copy_msg.push_str(&format!(
                    "，{} 个警告文件已复制到 {}/",
                    copied,
                    crate::commands::WARN_DIR_NAME
                )),
                Err(e) => copy_msg.push_str(&format!("，{}", e)),
            }
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
/// string 字段是逗号分隔的标签串，array 字段是标签数组；新增标签落入 added_to；
/// nl_path 是自然语言描述字段（LLM 返回 NL: 段时写入）
struct JsonTagLayout {
    string_fields: &'static [&'static [&'static str]],
    array_fields: &'static [&'static [&'static str]],
    added_to: &'static [&'static str],
    nl_path: &'static [&'static str],
    /// LLM 可重排的四个字段路径，顺序对齐 `TagBuckets` 与 Anima caption 的字段顺序：
    /// count / appearance / tags / environment。
    /// 这几个是本地打标器靠关键词表猜出来的分类（"simple background" 之类经常落错格），
    /// 其余字段来自 tagger 的模型 category 或路径，属于事实信息，不交给 LLM 动
    bucket_paths: [&'static [&'static str]; 4],
    /// 触发词写入的位置：画师/风格字段
    artist_path: &'static [&'static str],
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
    nl_path: &["ai_output", "nl"],
    artist_path: &["fixed", "artist"],
    bucket_paths: [
        &["ai_output", "count"],
        &["ai_output", "appearance"],
        &["ai_output", "tags"],
        &["ai_output", "environment"],
    ],
};

const SIMPLIFIED_LAYOUT: JsonTagLayout = JsonTagLayout {
    string_fields: &[&["quality"], &["series"], &["artist"], &["character"], &["count"]],
    array_fields: &[&["appearance"], &["tags"], &["environment"]],
    added_to: &["tags"],
    nl_path: &["nl"],
    artist_path: &["artist"],
    bucket_paths: [&["count"], &["appearance"], &["tags"], &["environment"]],
};

fn is_full_json_layout(data: &serde_json::Value) -> bool {
    ["ai_output", "fixed", "from_path"]
        .iter()
        .any(|k| data.get(k).map(|v| v.is_object()).unwrap_or(false))
        || data.get("character").map(|v| v.is_object()).unwrap_or(false)
}

fn json_layout(data: &serde_json::Value) -> &'static JsonTagLayout {
    if is_full_json_layout(data) {
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

/// 沿路径写值，中间层级缺失或类型不对时逐级补建对象（根不是对象则放弃）
fn json_set_path(data: &mut serde_json::Value, path: &[&str], value: serde_json::Value) {
    let Some((last, parents)) = path.split_last() else {
        return;
    };
    let mut cur = data;
    for key in parents {
        if !cur.get(*key).map(|v| v.is_object()).unwrap_or(false) {
            let Some(obj) = cur.as_object_mut() else {
                return;
            };
            obj.insert(
                (*key).to_string(),
                serde_json::Value::Object(serde_json::Map::new()),
            );
        }
        cur = cur.get_mut(*key).expect("just inserted");
    }
    if let Some(obj) = cur.as_object_mut() {
        obj.insert((*last).to_string(), value);
    }
}

fn path_is_string_field(layout: &JsonTagLayout, path: &[&str]) -> bool {
    layout.string_fields.iter().any(|p| *p == path)
}

fn path_is_bucket(layout: &JsonTagLayout, path: &[&str]) -> bool {
    layout.bucket_paths.iter().any(|p| *p == path)
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

/// (语义字段名, 路径)，顺序对齐 Anima caption 的字段顺序；完整/简化格式各一份
const FULL_LABELED_FIELDS: &[(&str, &[&str])] = &[
    ("quality", &["fixed", "quality"]),
    ("count", &["ai_output", "count"]),
    ("character", &["character", "name"]),
    ("series", &["fixed", "series"]),
    ("artist", &["fixed", "artist"]),
    ("appearance", &["ai_output", "appearance"]),
    ("tags", &["ai_output", "tags"]),
    ("environment", &["ai_output", "environment"]),
];
const SIMPLIFIED_LABELED_FIELDS: &[(&str, &[&str])] = &[
    ("quality", &["quality"]),
    ("count", &["count"]),
    ("character", &["character"]),
    ("series", &["series"]),
    ("artist", &["artist"]),
    ("appearance", &["appearance"]),
    ("tags", &["tags"]),
    ("environment", &["environment"]),
];

fn json_labeled_fields(data: &serde_json::Value) -> &'static [(&'static str, &'static [&'static str])] {
    if is_full_json_layout(data) {
        FULL_LABELED_FIELDS
    } else {
        SIMPLIFIED_LABELED_FIELDS
    }
}

/// 把 JSON 标签按字段标签渲染成多行文本——txt 模式下没有 .txt 只有 .json 时，
/// 不摊平转换、直接读 JSON：字段结构带着语义喂给 VLM，
/// 比一串扁平标签更容易核对画面内容（字段含义随文本一并给出）
fn render_json_tags_labeled(data: &serde_json::Value) -> String {
    let mut out = String::from(
        "(字段含义 Field meanings: quality=质量标签, count=人数, character=角色名, \
         series=作品名, artist=画师(@ 前缀), appearance=外观(发型/发色/瞳色/服装/配饰), \
         tags=动作/表情/姿势/构图/物品, environment=背景/场景/光影/氛围)",
    );
    for (label, path) in json_labeled_fields(data) {
        let tags: Vec<&str> = match json_get_path(data, path) {
            Some(serde_json::Value::String(s)) => {
                s.split(',').map(|t| t.trim()).filter(|t| !t.is_empty()).collect()
            }
            Some(serde_json::Value::Array(arr)) => arr
                .iter()
                .filter_map(|v| v.as_str())
                .map(|t| t.trim())
                .filter(|t| !t.is_empty())
                .collect(),
            _ => continue,
        };
        if !tags.is_empty() {
            out.push_str(&format!("\n{}: {}", label, tags.join(", ")));
        }
    }
    out
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

/// 一次 LLM 调用的产出。两条路径互斥：
/// 标签路径解析出标签/nl/字段归属；自然语言打标路径只有一整段文本。
enum RefineOutput {
    Tags {
        tags: Vec<String>,
        nl: Option<String>,
        buckets: TagBuckets,
    },
    Caption(String),
}

/// LLM 按字段归类返回的结果，顺序对齐 `JsonTagLayout::bucket_paths`。
/// `None` = 响应里没有这一段，该字段维持本地打标器给的归属（只做删除清理）。
#[derive(Debug, Default, Clone, PartialEq)]
struct TagBuckets {
    count: Option<Vec<String>>,
    appearance: Option<Vec<String>>,
    environment: Option<Vec<String>>,
    tags: Option<Vec<String>>,
}

impl TagBuckets {
    fn slots(&self) -> [&Option<Vec<String>>; 4] {
        [
            &self.count,
            &self.appearance,
            &self.tags,
            &self.environment,
        ]
    }

    /// 是否真的给出了字段归属。只有 `TAGS:` 一段不算——那是旧协议，
    /// 走差量写回才能保住本地打标器的原有分类。
    /// 不看 count：`Count: 15 tags` 这类闲聊行会被误认成字段段，
    /// 而真正按格式回复的响应必定带 appearance 或 environment
    fn has_field_assignment(&self) -> bool {
        self.appearance.is_some() || self.environment.is_some()
    }

    /// 按 count → appearance → tags → environment 顺序展开全部标签（去重保序），
    /// 与 Anima caption 的字段顺序一致
    fn all_tags(&self) -> Vec<String> {
        let mut seen: HashSet<String> = HashSet::new();
        let mut out = Vec::new();
        for slot in self.slots().into_iter().flatten() {
            for t in slot {
                if seen.insert(t.clone()) {
                    out.push(t.clone());
                }
            }
        }
        out
    }
}

/// LLM 给出字段归属时的写回：非重排字段（quality/series/artist/character/from_path）
/// 仍按差量清理，count/appearance/environment/tags 四个字段则按 LLM 的归属重排。
/// 重排内容会剔除已留在其它字段里的标签，同一个标签不会出现两处。
fn apply_buckets_to_json(data: &mut serde_json::Value, buckets: &TagBuckets, refined: &[String]) {
    let layout = json_layout(data);
    let refined_set: HashSet<&str> = refined.iter().map(|s| s.as_str()).collect();
    let mut used: HashSet<String> = HashSet::new();

    // 1. 非重排字段（quality/series/artist/character/from_path）原样保留：
    //    它们来自 tagger 的模型 category 或路径，提示词也没要求 LLM 管这几类，
    //    按 refined 差量清理会把角色名、画师这些事实信息整片删掉。
    //    只登记内容，供第 2 步给重排字段去重
    for path in layout.string_fields.iter().chain(layout.array_fields) {
        if path_is_bucket(layout, path) {
            continue;
        }
        match json_get_path(data, path) {
            Some(serde_json::Value::String(s)) => {
                for t in s.split(',').map(|t| t.trim()).filter(|t| !t.is_empty()) {
                    used.insert(t.to_string());
                }
            }
            Some(serde_json::Value::Array(arr)) => {
                for t in arr.iter().filter_map(|v| v.as_str()) {
                    let t = t.trim();
                    if !t.is_empty() {
                        used.insert(t.to_string());
                    }
                }
            }
            _ => {}
        }
    }

    // 2. 四个重排字段：给了归属就覆盖，没给的段按差量清理后原样保留
    let slots = buckets.slots();
    for (i, path) in layout.bucket_paths.iter().enumerate() {
        let assigned: Vec<String> = match slots[i] {
            Some(list) => list
                .iter()
                .filter(|t| !t.is_empty() && used.insert((*t).clone()))
                .cloned()
                .collect(),
            None => {
                // 该段未出现：沿用原值，仅移除已被 LLM 删掉的标签
                let existing = json_get_path(data, path);
                let kept: Vec<String> = match existing {
                    Some(serde_json::Value::String(s)) => s
                        .split(',')
                        .map(|t| t.trim().to_string())
                        .filter(|t| !t.is_empty() && refined_set.contains(t.as_str()))
                        .collect(),
                    Some(serde_json::Value::Array(arr)) => arr
                        .iter()
                        .filter_map(|v| v.as_str())
                        .map(|t| t.trim().to_string())
                        .filter(|t| !t.is_empty() && refined_set.contains(t.as_str()))
                        .collect(),
                    _ => Vec::new(),
                };
                kept.into_iter().filter(|t| used.insert(t.clone())).collect()
            }
        };

        let value = if path_is_string_field(layout, path) {
            serde_json::Value::String(assigned.join(", "))
        } else {
            serde_json::Value::Array(assigned.into_iter().map(serde_json::Value::String).collect())
        };
        json_set_path(data, path, value);
    }

    // 3. LLM 新增但没落进任何段的标签，兜底追加到通用 tags
    let leftover: Vec<String> = refined
        .iter()
        .filter(|t| !used.contains(t.as_str()))
        .cloned()
        .collect();
    if leftover.is_empty() {
        return;
    }
    // added_to 就是通用 tags 路径，用它而不是 bucket_paths 的下标，顺序调整不会打偏
    let tags_path = layout.added_to;
    let mut merged: Vec<String> = json_get_path(data, tags_path)
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str())
                .map(|s| s.to_string())
                .collect()
        })
        .unwrap_or_default();
    merged.extend(leftover);
    json_set_path(
        data,
        tags_path,
        serde_json::Value::Array(merged.into_iter().map(serde_json::Value::String).collect()),
    );
}

/// 只按 LLM 的归属重排字段，标签集合保持不变（"归类字段 + 补描述"预设）。
///
/// 与 `apply_buckets_to_json` 的根本区别：标签**只从原有字段读取**，
/// LLM 的回复仅提供 `标签 → 目标字段` 的映射。因此
/// LLM 新增的标签会被丢弃、漏掉的标签留在原字段，集合恒定不变——
/// 已经打好的标签不会因为模型少写一个词就丢失。
/// 非重排字段（quality/series/artist/character/from_path）完全不碰。
fn apply_buckets_preserving(data: &mut serde_json::Value, buckets: &TagBuckets) {
    let layout = json_layout(data);

    // LLM 给出的归属：标签（小写）→ bucket 下标
    let mut assign: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for (i, slot) in buckets.slots().iter().enumerate() {
        if let Some(list) = slot {
            for t in list {
                assign.insert(t.trim().to_lowercase(), i);
            }
        }
    }

    // 读取原四个字段的现有标签
    let current: Vec<Vec<String>> = layout
        .bucket_paths
        .iter()
        .map(|path| match json_get_path(data, path) {
            Some(serde_json::Value::String(s)) => s
                .split(',')
                .map(|t| t.trim().to_string())
                .filter(|t| !t.is_empty())
                .collect(),
            Some(serde_json::Value::Array(arr)) => arr
                .iter()
                .filter_map(|v| v.as_str())
                .map(|t| t.trim().to_string())
                .filter(|t| !t.is_empty())
                .collect(),
            _ => Vec::new(),
        })
        .collect();

    // 按归属重新分配；LLM 没提到的标签留在原字段
    let mut next: Vec<Vec<String>> = vec![Vec::new(); layout.bucket_paths.len()];
    for (origin, tags) in current.iter().enumerate() {
        for t in tags {
            let target = assign
                .get(&t.to_lowercase())
                .copied()
                .filter(|i| *i < next.len())
                .unwrap_or(origin);
            if !next[target].iter().any(|x| x.eq_ignore_ascii_case(t)) {
                next[target].push(t.clone());
            }
        }
    }

    for (i, path) in layout.bucket_paths.iter().enumerate() {
        let value = if path_is_string_field(layout, path) {
            serde_json::Value::String(next[i].join(", "))
        } else {
            serde_json::Value::Array(
                next[i]
                    .iter()
                    .map(|t| serde_json::Value::String(t.clone()))
                    .collect(),
            )
        };
        json_set_path(data, path, value);
    }
}

/// txt 内容强制以触发词开头（标签列表和自然语言描述一视同仁）。
/// 幂等：LLM 自己已经按提示词带上了就不重复插入。
fn ensure_trigger_prefix(content: &str, trigger: &str) -> String {
    let t = trigger.trim();
    if t.is_empty() {
        return content.to_string();
    }
    let head = content.trim_start();
    if head.is_empty() {
        return t.to_string();
    }
    // 用 char 边界安全的前缀比较；触发词后面必须是逗号或空白，避免 "ypuddin" 命中 "ypuddinneko"
    if let Some(prefix) = head.get(..t.len()) {
        if prefix.eq_ignore_ascii_case(t) {
            let rest = &head[t.len()..];
            if rest.is_empty()
                || rest.starts_with(',')
                || rest.starts_with('，')
                || rest.starts_with(char::is_whitespace)
            {
                return head.to_string();
            }
        }
    }
    format!("{}, {}", t, head)
}

/// 触发词写入 JSON 的 artist 字段（完整格式 fixed.artist / 简化格式 artist），
/// 放在最前面并保留原有画师标签；已存在则不重复添加。
fn set_json_trigger(data: &mut serde_json::Value, trigger: &str) {
    let t = trigger.trim();
    if t.is_empty() {
        return;
    }
    let path = json_layout(data).artist_path;
    let existing: Vec<String> = json_get_path(data, path)
        .and_then(|v| v.as_str())
        .map(|s| {
            s.split(',')
                .map(|x| x.trim().to_string())
                .filter(|x| !x.is_empty())
                .collect()
        })
        .unwrap_or_default();
    if existing.iter().any(|p| p.eq_ignore_ascii_case(t)) {
        return;
    }
    let mut merged = vec![t.to_string()];
    merged.extend(existing);
    json_set_path(data, path, serde_json::Value::String(merged.join(", ")));
}

/// 将 LLM 返回的自然语言描述写入 JSON 的 nl 字段（完整格式 ai_output.nl / 简化格式 nl）。
/// 本地打标器不产生 nl，该字段由此补充；路径缺失时逐级补建。
fn set_json_nl(data: &mut serde_json::Value, nl: &str) {
    let path = json_layout(data).nl_path;
    json_set_path(data, path, serde_json::Value::String(nl.to_string()));
}

/// 大小写不敏感的行前缀剥离（仅 ASCII 前缀；非字符边界安全返回 None）
fn strip_ci_prefix<'a>(s: &'a str, prefix: &str) -> Option<&'a str> {
    let head = s.get(..prefix.len())?;
    if head.eq_ignore_ascii_case(prefix) {
        Some(&s[prefix.len()..])
    } else {
        None
    }
}

/// `split_marker_response` 的解析结果
#[derive(Debug, Default)]
struct MarkerResponse<'a> {
    /// 各字段段的原始行内容，顺序对齐 `TagBuckets`：count / appearance / environment / tags
    buckets: TagBuckets,
    nl: Option<String>,
    /// 未归入任何标记段的其余行
    rest: Vec<&'a str>,
}

/// 把 `xxx, yyy` 一行拆成标签列表（空段返回空 Vec，代表"该字段清空"而非"未给出"）
fn split_tag_line(line: &str) -> Vec<String> {
    line.split(',')
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
        .collect()
}

/// 解析 LLM 响应中的标记段（大小写不敏感，容忍 markdown 加粗/列表前缀）：
/// `COUNT:` / `APPEARANCE:` / `ENVIRONMENT:` / `TAGS:` 是标签段，`NL:` 是自然语言段。
/// 未使用标记格式的旧提示词：全部返回 None，由调用方走旧启发式。
fn split_marker_response(content: &str) -> MarkerResponse<'_> {
    let mut out = MarkerResponse::default();
    let mut nl_buf: Vec<String> = Vec::new();
    let mut in_nl = false;

    for line in content.lines() {
        let t = line
            .trim()
            .trim_start_matches(|c: char| matches!(c, '*' | '#' | '>' | '-' | '`'))
            .trim_start();

        let mut matched = false;
        for (prefix, slot) in [
            ("count:", &mut out.buckets.count),
            ("appearance:", &mut out.buckets.appearance),
            ("environment:", &mut out.buckets.environment),
            ("tags:", &mut out.buckets.tags),
        ] {
            if let Some(rest) = strip_ci_prefix(t, prefix) {
                *slot = Some(split_tag_line(rest.trim_start_matches('*').trim()));
                in_nl = false;
                matched = true;
                break;
            }
        }
        if matched {
            continue;
        }

        if let Some(rest) = strip_ci_prefix(t, "nl:") {
            nl_buf.clear();
            let first = rest.trim_start_matches('*').trim();
            if !first.is_empty() {
                nl_buf.push(first.to_string());
            }
            in_nl = true;
            continue;
        }
        if in_nl {
            // NL 段允许跨行，空行或代码围栏结束该段
            if t.is_empty() {
                in_nl = false;
            } else {
                nl_buf.push(t.to_string());
            }
        } else {
            out.rest.push(line);
        }
    }

    let joined = nl_buf.join(" ");
    let cleaned = joined.trim().trim_matches('"').trim().to_string();
    out.nl = if cleaned.is_empty() {
        None
    } else {
        Some(cleaned)
    };
    out
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
    let mut tag_path = parent.join(format!("{}.{}", stem, tag_ext));
    // txt 模式回退：没有 .txt 但有 .json 时直接读 JSON——字段结构带着语义
    // 喂给 VLM 比先摊平转换信息更全（输出是扁平 txt，写盘时反正要摊平）
    let mut json_fallback = false;
    if !tag_path.exists() && !is_json {
        let jp = parent.join(format!("{}.json", stem));
        if jp.exists() {
            tag_path = jp;
            json_fallback = true;
        }
    }
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

    // json 模式：解析并扁平化标签；txt 模式：逗号拆分。
    // json 回退（txt 模式读到了 .json）：同样解析展开，但不进 json_data——
    // 结果始终写回 .txt，JSON 原文件不动
    let mut json_data: Option<serde_json::Value> = None;
    let mut tags_display: Option<String> = None;
    let original_tags: Vec<String> = if is_json || json_fallback {
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
        if is_json {
            json_data = Some(parsed);
        } else {
            tags_display = Some(render_json_tags_labeled(&parsed));
        }
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

    // 调用 LLM 细化（tags_display：JSON 回退时带字段标签的展示文本）
    match refine_tags_with_llm(client, img_path, &original_tags, tags_display.as_deref(), options, last_req_time).await {
        // 自然语言打标：整段描述直接落盘，标签只是刚才喂给 LLM 的参考
        Ok(RefineOutput::Caption(caption)) => {
            let elapsed_ms = start.elapsed().as_millis();
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
            // 触发词由后端保证在最前，不依赖 LLM 遵守提示词
            let caption = ensure_trigger_prefix(&caption, &options.trigger_word);
            let word_count = caption.split_whitespace().count();
            match std::fs::write(&output_path, &caption) {
                Ok(_) => FileResult::Captioned {
                    filename,
                    original_count: original_tags.len(),
                    word_count,
                    elapsed_ms,
                },
                Err(e) => FileResult::Error {
                    filename,
                    message: format!("写入失败: {}", e),
                },
            }
        }
        Ok(RefineOutput::Tags {
            tags: refined_tags,
            nl,
            buckets,
        }) => {
            let elapsed_ms = start.elapsed().as_millis();
            let original_count = original_tags.len();
            // 保集合模式下写盘的标签就是原有那批，LLM 回复里的增删不会生效
            let preserving = is_json && options.preserve_tags;
            let refined_count = if preserving {
                original_count
            } else {
                refined_tags.len()
            };
            let nl_written = is_json && nl.is_some();
            // 字段归属可能变了而标签集合没变（例如 simple background 从 tags 挪到 environment），
            // 这种情况也算改动，否则日志会误报"未变化"
            let rebucketed = is_json && buckets.has_field_assignment();
            let changed = (!preserving && refined_tags != original_tags) || nl_written || rebucketed;
            let mut warnings: Vec<String> = Vec::new();

            // 增删对比。保集合模式跳过：标签实际没动，
            // 报"移除/新增"会误导，还会把整批图复制进 Warn/
            if !preserving {
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
                if options.preserve_tags {
                    // 只归类不增删：标签集合恒定，LLM 的回复只当作归属映射
                    apply_buckets_preserving(&mut data, &buckets);
                } else if buckets.has_field_assignment() {
                    // LLM 给了字段归属：重排 count/appearance/environment/tags
                    // （本地打标器靠关键词表分类，"simple background" 之类常落错格）
                    apply_buckets_to_json(&mut data, &buckets, &refined_tags);
                } else {
                    // 旧协议：差量写回，保留原字段归属，仅应用增删
                    apply_refined_tags_to_json(&mut data, &refined_tags);
                }
                // LLM 返回了 NL: 描述段时写入 nl 字段（本地打标器不产生 nl，由 LLM 补充）
                if let Some(nl_text) = nl.as_deref() {
                    set_json_nl(&mut data, nl_text);
                }
                // 触发词进 artist 字段（JSON 的触发词位置），txt 那边则是置于开头
                set_json_trigger(&mut data, &options.trigger_word);
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
                // 纯标签的 txt 同样要以触发词开头
                ensure_trigger_prefix(&refined_tags.join(", "), &options.trigger_word)
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

/// 调用多模态 LLM 进行标签细化（发送图片 + 已有标签）。
/// 返回 (细化后标签, NL 描述)：响应含 `NL:` 标记段时第二项为 Some，用于 JSON 模式补充 nl 字段。
async fn refine_tags_with_llm(
    client: &reqwest::Client,
    img_path: &Path,
    tags: &[String],
    tags_display: Option<&str>,
    options: &TagRefineOptions,
    last_req_time: &tokio::sync::Mutex<Option<std::time::Instant>>,
) -> Result<RefineOutput, String> {
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

    // JSON 回退时展示文本带字段标签和含义（count: 1girl / appearance: ...），
    // 否则就是扁平的逗号分隔列表
    let tag_list = tags_display
        .map(|s| s.to_string())
        .unwrap_or_else(|| tags.join(", "));

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
    let mut image_url = serde_json::json!({ "url": data_url });
    let detail = options.image_detail.trim();
    if !detail.is_empty() {
        image_url["detail"] = serde_json::Value::String(detail.to_string());
    }
    let messages = vec![ChatMessage {
        role: "user".to_string(),
        content: serde_json::json!([
            { "type": "text", "text": user_text },
            { "type": "image_url", "image_url": image_url }
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
    // 服务端内容安全审核直接拦下：这张图不该被当成"处理成功"
    if matches!(
        choice.finish_reason.as_deref(),
        Some("content_filter") | Some("safety")
    ) {
        return Err("LLM 内容安全审核拒绝了该图片，标签未改动".to_string());
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

    // 自然语言打标：回复整段就是标签文件内容，不进标签解析
    if options.caption_mode {
        let caption = final_content.trim();
        if caption.is_empty() {
            return Err("AI 返回空描述".to_string());
        }
        if crate::commands::looks_like_refusal(caption) {
            let excerpt: String = caption.chars().take(80).collect();
            return Err(format!("LLM 拒绝处理该图片（疑似内容安全审核）: {}", excerpt));
        }
        return Ok(RefineOutput::Caption(caption.to_string()));
    }

    let (tags, nl, buckets) = parse_refine_response(&final_content, tags)?;
    Ok(RefineOutput::Tags { tags, nl, buckets })
}

/// 把 LLM 的原始回复解析成 (最终标签, nl, 字段归属)。
/// 优先走标记格式（辅助打标 JSON 模式的默认提示词要求此格式）；
/// 无标记时退回旧启发式：多行取最长的含逗号行。
/// NL 段先于启发式剥离——自然语言长句常含逗号且比标签列表更长，会被启发式误选。
fn parse_refine_response(
    content: &str,
    original_tags: &[String],
) -> Result<(Vec<String>, Option<String>, TagBuckets), String> {
    let marker = split_marker_response(content);

    // 模型拒绝（NSFW 触发安全审核等）：拒绝语必须判失败，
    // 否则会被下面的"最长含逗号行"启发式当成标签写进标签文件
    let has_markers =
        marker.buckets.slots().iter().any(|s| s.is_some()) || marker.nl.is_some();
    if !has_markers && crate::commands::looks_like_refusal(content) {
        let excerpt: String = content.trim().chars().take(80).collect();
        return Err(format!("LLM 拒绝处理该图片（疑似内容安全审核）: {}", excerpt));
    }
    // 模型拒绝时也常常遵守输出格式（NL: I'm sorry, I cannot...）——
    // 上面那道闸被 has_markers 跳过，拒绝文本会被直接写进 nl 字段，
    // NL 段内容必须单独再过一遍（"仅补 nl 描述"预设必走这条路）
    if let Some(nl_text) = marker.nl.as_deref() {
        if crate::commands::looks_like_refusal(nl_text) {
            let excerpt: String = nl_text.trim().chars().take(80).collect();
            return Err(format!("LLM 拒绝处理该图片（疑似内容安全审核）: {}", excerpt));
        }
    }
    // TAGS:/字段段同理：TAGS: I'm sorry, I can't... 会被拆成"标签"写盘
    for seg in marker.buckets.slots().iter().filter_map(|s| s.as_ref()) {
        let joined = seg.join(", ");
        if crate::commands::looks_like_refusal(&joined) {
            let excerpt: String = joined.chars().take(80).collect();
            return Err(format!("LLM 拒绝处理该图片（疑似内容安全审核）: {}", excerpt));
        }
    }

    // 孤零零一个 count 段不足以判定是标记格式，让它退回启发式而不是劫持整个标签列表
    let refined_tags: Vec<String> = if marker.buckets.tags.is_some()
        || marker.buckets.has_field_assignment()
    {
        marker.buckets.all_tags()
    } else {
        let joined = marker.rest.join("\n");
        let cleaned = if joined.contains('\n') {
            joined
                .lines()
                .filter(|l| l.contains(','))
                .max_by_key(|l| l.len())
                .unwrap_or(&joined)
                .to_string()
        } else {
            joined
        };
        split_tag_line(&cleaned)
    };

    // 只回了 NL: 一段（"仅补 nl 描述"这类提示词）：标签原样保留，不是失败
    if refined_tags.is_empty() && marker.nl.is_some() {
        return Ok((original_tags.to_vec(), marker.nl, TagBuckets::default()));
    }

    if refined_tags.is_empty() {
        return Err("AI 返回的细化结果为空".to_string());
    }

    Ok((refined_tags, marker.nl, marker.buckets))
}

#[cfg(test)]
mod marker_tests {
    use super::*;

    #[test]
    fn parses_tags_and_nl_markers() {
        let m = split_marker_response(
            "TAGS: 1girl, long hair, smile\nNL: A girl smiles at the viewer.",
        );
        assert_eq!(
            m.buckets.tags,
            Some(vec![
                "1girl".to_string(),
                "long hair".to_string(),
                "smile".to_string()
            ])
        );
        assert_eq!(m.nl.as_deref(), Some("A girl smiles at the viewer."));
        // 只有 TAGS: 一段 = 旧协议，不能触发重排
        assert!(!m.buckets.has_field_assignment());
    }

    #[test]
    fn tolerates_markdown_and_case() {
        let m = split_marker_response(
            "**Tags:** 1girl, solo\n\n> nl: A solo girl, standing outdoors,\nunder a blue sky.",
        );
        assert_eq!(
            m.buckets.tags,
            Some(vec!["1girl".to_string(), "solo".to_string()])
        );
        // NL 段跨行拼接
        assert_eq!(
            m.nl.as_deref(),
            Some("A solo girl, standing outdoors, under a blue sky.")
        );
    }

    /// 无标记的旧格式响应：全部行留给启发式，nl 为 None
    #[test]
    fn legacy_response_passes_through() {
        let m = split_marker_response("some preamble\n1girl, solo, smile");
        assert!(m.buckets.slots().iter().all(|s| s.is_none()));
        assert!(m.nl.is_none());
        assert_eq!(m.rest, vec!["some preamble", "1girl, solo, smile"]);
    }

    /// NL 长句含逗号且比标签列表长：有标记时绝不能被当成标签列表
    #[test]
    fn nl_never_leaks_into_tags() {
        let m = split_marker_response(
            "TAGS: 1girl\nNL: An extremely long description, with many commas, that is much longer than the tag list itself.",
        );
        assert_eq!(m.buckets.tags, Some(vec!["1girl".to_string()]));
        assert!(m.nl.unwrap().starts_with("An extremely long"));
        assert!(m.rest.is_empty());
    }

    /// 四段字段归属格式：解析出各字段并按 count→appearance→environment→tags 展开
    #[test]
    fn parses_field_assignment_markers() {
        let m = split_marker_response(
            "COUNT: 1girl, solo\nAPPEARANCE: long hair, blue eyes\nENVIRONMENT: simple background, white background\nTAGS: smile\nNL: A girl.",
        );
        assert!(m.buckets.has_field_assignment());
        assert_eq!(
            m.buckets.environment,
            Some(vec![
                "simple background".to_string(),
                "white background".to_string()
            ])
        );
        // 展开顺序 = count → appearance → tags → environment
        assert_eq!(
            m.buckets.all_tags(),
            vec![
                "1girl",
                "solo",
                "long hair",
                "blue eyes",
                "smile",
                "simple background",
                "white background"
            ]
        );
        assert_eq!(m.nl.as_deref(), Some("A girl."));
    }

    /// 空字段段代表"该字段清空"，不是"未给出"
    #[test]
    fn empty_segment_means_cleared_not_absent() {
        let m = split_marker_response("COUNT: 1girl\nENVIRONMENT:\nTAGS: smile");
        assert_eq!(m.buckets.environment, Some(vec![]));
        assert!(m.buckets.appearance.is_none());
    }

    /// "仅补 nl 描述"的提示词：模型只回一段 NL，标签必须原样保留而不是判定失败
    #[test]
    fn nl_only_response_keeps_tags() {
        let original: Vec<String> = ["1girl", "long hair", "smile"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let (tags, nl, buckets) =
            parse_refine_response("NL: A girl with long hair smiles.", &original).unwrap();
        assert_eq!(tags, original);
        assert_eq!(nl.as_deref(), Some("A girl with long hair smiles."));
        assert_eq!(buckets, TagBuckets::default());
    }

    /// 既没有标签也没有 NL 才算失败
    #[test]
    fn truly_empty_response_is_error() {
        assert!(parse_refine_response("", &[]).is_err());
        assert!(parse_refine_response("\n\n", &[]).is_err());
        // 拒绝语同样是 Err（走的是拒绝识别那条路，不是"空响应"）
        assert!(parse_refine_response("Sorry, I cannot help.", &[]).is_err());
        // 普通单行仍按标签处理
        assert!(parse_refine_response("1girl, solo", &[]).is_ok());
    }

    /// 保集合归类：LLM 的增删一概不生效，只有归属被采纳
    #[test]
    fn preserving_rebucket_never_changes_the_tag_set() {
        let mut data = serde_json::json!({
            "fixed": {"quality": "masterpiece", "series": "", "artist": ""},
            "character": {"name": "hatsune miku", "variant": ""},
            "from_path": {"appearance": ["twintails"]},
            "ai_output": {
                "count": "1girl",
                "appearance": ["long hair", "smile"],
                "tags": ["simple background"],
                "environment": [],
                "nl": ""
            }
        });
        // LLM：把 simple background 挪到 environment、smile 挪到 tags（正确的归类），
        // 但同时私自删掉 long hair、新增 blush
        let buckets = TagBuckets {
            count: Some(vec!["1girl".into()]),
            appearance: Some(vec![]),
            tags: Some(vec!["smile".into(), "blush".into()]),
            environment: Some(vec!["simple background".into()]),
        };
        apply_buckets_preserving(&mut data, &buckets);

        // 归属被采纳
        assert_eq!(
            data["ai_output"]["environment"],
            serde_json::json!(["simple background"])
        );
        assert_eq!(data["ai_output"]["tags"], serde_json::json!(["smile"]));
        assert_eq!(data["ai_output"]["count"], "1girl");
        // LLM 私自新增的 blush 被丢弃
        assert!(!data["ai_output"]["tags"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v == "blush"));
        // LLM 漏掉的 long hair 留在原字段，没有丢失
        assert_eq!(
            data["ai_output"]["appearance"],
            serde_json::json!(["long hair"])
        );
        // 非重排字段一概不碰
        assert_eq!(data["character"]["name"], "hatsune miku");
        assert_eq!(data["fixed"]["quality"], "masterpiece");
        assert_eq!(
            data["from_path"]["appearance"],
            serde_json::json!(["twintails"])
        );
    }

    /// 触发词：txt 强制置于开头，且不能重复插入（caption 提示词也会要求 LLM 自己带上）
    #[test]
    fn trigger_prefix_is_forced_and_idempotent() {
        // 标签列表
        assert_eq!(
            ensure_trigger_prefix("1girl, solo", "ypuddinneko"),
            "ypuddinneko, 1girl, solo"
        );
        // LLM 已经带上了就不重复
        assert_eq!(
            ensure_trigger_prefix("ypuddinneko, 1girl, solo", "ypuddinneko"),
            "ypuddinneko, 1girl, solo"
        );
        // 大小写不敏感
        assert_eq!(
            ensure_trigger_prefix("YPuddinNeko, 1girl", "ypuddinneko"),
            "YPuddinNeko, 1girl"
        );
        // 前缀相同但不是同一个词，必须补
        assert_eq!(
            ensure_trigger_prefix("ypuddin, 1girl", "ypuddinneko"),
            "ypuddinneko, ypuddin, 1girl"
        );
        // 触发词为空时原样返回
        assert_eq!(ensure_trigger_prefix("1girl, solo", "  "), "1girl, solo");
        // 多字节内容不会在 char 边界上 panic
        assert_eq!(
            ensure_trigger_prefix("少女, 微笑", "触发词"),
            "触发词, 少女, 微笑"
        );
    }

    /// 触发词：JSON 进 artist 字段，放最前且保留原有画师标签
    #[test]
    fn trigger_goes_into_artist_field() {
        // 完整格式，artist 为空
        let mut full = serde_json::json!({"fixed": {"artist": ""}, "ai_output": {"tags": []}});
        set_json_trigger(&mut full, "ypuddinneko");
        assert_eq!(full["fixed"]["artist"], "ypuddinneko");
        // 已有画师标签：触发词插到最前，原值保留
        let mut kept = serde_json::json!({"fixed": {"artist": "@wlop"}, "ai_output": {}});
        set_json_trigger(&mut kept, "ypuddinneko");
        assert_eq!(kept["fixed"]["artist"], "ypuddinneko, @wlop");
        // 幂等
        set_json_trigger(&mut kept, "ypuddinneko");
        assert_eq!(kept["fixed"]["artist"], "ypuddinneko, @wlop");
        // 简化格式写顶层 artist
        let mut simp = serde_json::json!({"artist": "", "tags": [], "character": ""});
        set_json_trigger(&mut simp, "ypuddinneko");
        assert_eq!(simp["artist"], "ypuddinneko");
        // 空触发词不动
        let mut untouched = serde_json::json!({"fixed": {"artist": "@wlop"}, "ai_output": {}});
        set_json_trigger(&mut untouched, "");
        assert_eq!(untouched["fixed"]["artist"], "@wlop");
    }

    /// 安全审核拒绝：必须判失败，绝不能把拒绝语当成标签写进标签文件
    #[test]
    fn refusal_is_rejected_not_written_as_tags() {
        for refusal in [
            "I'm sorry, I can't help with that.",
            "I cannot assist with this request.",
            "抱歉，我无法处理这张图片。",
            "I am unable to describe this image due to content policy.",
        ] {
            assert!(
                parse_refine_response(refusal, &["1girl".to_string()]).is_err(),
                "应判为拒绝: {refusal}"
            );
        }
    }

    /// 正常标签列表里出现拒绝措辞的字样不能被误杀——逗号数量是那道闸
    #[test]
    fn normal_tag_lists_are_not_mistaken_for_refusal() {
        // 逗号多 = 标签列表，即便含 "i can't" 之类的字样
        let tags = "1girl, solo, i can't believe it's not butter, smile, outdoors";
        let (parsed, _, _) = parse_refine_response(tags, &[]).unwrap();
        assert!(parsed.contains(&"1girl".to_string()));
        // 按标记格式返回的短回复也不该被误判
        let marked = parse_refine_response("TAGS: sorry\nNL: A girl with a sorry expression.", &[])
            .unwrap();
        assert_eq!(marked.0, vec!["sorry".to_string()]);
    }

    /// 模型拒绝时也常常遵守输出格式——带 NL:/TAGS: 前缀的拒绝语照样要判失败，
    /// 否则"仅补 nl 描述"这类预设会把拒绝文本直接写进 nl 字段
    #[test]
    fn marked_refusal_is_rejected() {
        for refusal in [
            "NL: I'm sorry, I cannot describe this image.",
            "NL: I can't provide a description for this content.",
            "TAGS: I'm sorry, I can't help with that.",
            "NL: 抱歉，我无法处理这张图片。",
        ] {
            assert!(
                parse_refine_response(refusal, &["1girl".to_string()]).is_err(),
                "应判为拒绝: {refusal}"
            );
        }
    }

    /// 闲聊行 `Count: 15 tags` 不能被当成字段归属，否则整份标签会被它替换掉
    #[test]
    fn stray_count_line_does_not_trigger_rebucket() {
        let m = split_marker_response("Count: 15 tags\n1girl, solo, smile");
        assert!(!m.buckets.has_field_assignment());
        assert!(m.buckets.tags.is_none());
        // 真正的标签列表留在 rest 里交给启发式
        assert_eq!(m.rest, vec!["1girl, solo, smile"]);
    }

    /// txt 模式 JSON 回退：带字段标签渲染，空字段省略，nl 不出现
    #[test]
    fn labeled_render_skips_empty_and_nl() {
        let full = serde_json::json!({
            "fixed": {"quality": "newest, safe", "series": "", "artist": "@wlop"},
            "character": {"name": "hatsune miku", "variant": ""},
            "from_path": {"appearance": []},
            "ai_output": {"count": "1girl", "appearance": ["long hair", "blue eyes"],
                          "tags": ["smile"], "environment": [], "nl": "a secret description"}
        });
        let rendered = render_json_tags_labeled(&full);
        assert!(rendered.contains("quality: newest, safe"));
        assert!(rendered.contains("count: 1girl"));
        assert!(rendered.contains("character: hatsune miku"));
        assert!(rendered.contains("artist: @wlop"));
        assert!(rendered.contains("appearance: long hair, blue eyes"));
        assert!(rendered.contains("tags: smile"));
        // 空字段省略、nl 不出现
        assert!(!rendered.contains("series:"));
        assert!(!rendered.contains("environment:"));
        assert!(!rendered.contains("a secret description"));
        // 字段含义说明在最前
        assert!(rendered.starts_with("(字段含义"));

        // 简化格式同样渲染（逗号串 + 数组混合）
        let simp = serde_json::json!({"count": "2girls", "tags": "sitting, looking at viewer", "nl": "x"});
        let r2 = render_json_tags_labeled(&simp);
        assert!(r2.contains("count: 2girls"));
        assert!(r2.contains("tags: sitting, looking at viewer"));
        assert!(!r2.contains('x'));
    }

    #[test]
    fn set_nl_full_and_simplified() {
        // 完整格式：写入 ai_output.nl（路径缺失时补建）
        let mut full = serde_json::json!({"ai_output": {"tags": ["1girl"]}});
        set_json_nl(&mut full, "desc");
        assert_eq!(full["ai_output"]["nl"], "desc");

        let mut full_no_ai = serde_json::json!({"fixed": {"quality": ""}});
        set_json_nl(&mut full_no_ai, "desc2");
        assert_eq!(full_no_ai["ai_output"]["nl"], "desc2");

        // 简化格式：写入顶层 nl
        let mut simp = serde_json::json!({"tags": ["1girl"], "character": "miku"});
        set_json_nl(&mut simp, "desc3");
        assert_eq!(simp["nl"], "desc3");
    }

    /// 差量写回 + nl 补充的组合：空字符串字段（完整 schema 占位）不产生垃圾标签
    #[test]
    fn refine_roundtrip_with_skeleton_json() {
        let mut data = serde_json::json!({
            "fixed": {"quality": "", "series": "", "artist": ""},
            "character": {"name": "hatsune miku", "variant": ""},
            "from_path": {"appearance": []},
            "ai_output": {"count": "1girl", "appearance": ["long hair"], "tags": ["smile"], "environment": [], "nl": ""}
        });
        let flat = flatten_json_tags(&data);
        assert_eq!(flat, vec!["hatsune miku", "1girl", "long hair", "smile"]);

        // LLM 保留 miku/1girl/long hair，删除 smile，新增 standing
        let refined: Vec<String> = ["hatsune miku", "1girl", "long hair", "standing"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        apply_refined_tags_to_json(&mut data, &refined);
        set_json_nl(&mut data, "Miku stands with long hair.");

        assert_eq!(data["character"]["name"], "hatsune miku");
        assert_eq!(data["ai_output"]["count"], "1girl");
        assert_eq!(data["ai_output"]["appearance"], serde_json::json!(["long hair"]));
        assert_eq!(data["ai_output"]["tags"], serde_json::json!(["standing"]));
        assert_eq!(data["ai_output"]["nl"], "Miku stands with long hair.");
        // 空字符串字段保持为空，不产生 "@"/垃圾内容
        assert_eq!(data["fixed"]["quality"], "");
    }

    /// 字段重排（完整格式）：本地打标器把 simple background 分进了 tags，
    /// LLM 把它改判到 environment，写回后必须真的挪过去且不残留
    #[test]
    fn rebuckets_full_format_by_llm_assignment() {
        let mut data = serde_json::json!({
            "fixed": {"quality": "masterpiece", "series": "", "artist": ""},
            "character": {"name": "hatsune miku", "variant": ""},
            "from_path": {"appearance": ["twintails"]},
            "ai_output": {
                "count": "1girl",
                "appearance": ["long hair"],
                "tags": ["simple background", "smile", "outdoors"],
                "environment": [],
                "nl": ""
            }
        });
        let buckets = TagBuckets {
            count: Some(vec!["1girl".into()]),
            appearance: Some(vec!["long hair".into()]),
            // smile 是表情，按 Anima 规范属 tags；simple background 从 tags 改判到 environment
            tags: Some(vec!["smile".into()]),
            environment: Some(vec!["simple background".into(), "outdoors".into()]),
        };
        let refined = buckets.all_tags();
        apply_buckets_to_json(&mut data, &buckets, &refined);

        assert_eq!(
            data["ai_output"]["environment"],
            serde_json::json!(["simple background", "outdoors"])
        );
        assert_eq!(
            data["ai_output"]["appearance"],
            serde_json::json!(["long hair"])
        );
        assert_eq!(data["ai_output"]["tags"], serde_json::json!(["smile"]));
        assert_eq!(data["ai_output"]["count"], "1girl");
        // 非重排字段保持事实信息：LLM 没在段里列出它们也不该被删
        assert_eq!(data["character"]["name"], "hatsune miku");
        assert_eq!(data["fixed"]["quality"], "masterpiece");
        assert_eq!(
            data["from_path"]["appearance"],
            serde_json::json!(["twintails"])
        );
    }

    /// 重排时标签不能同时出现在两个字段：已留在 character/quality 的不再进重排字段
    #[test]
    fn rebucket_does_not_duplicate_across_fields() {
        let mut data = serde_json::json!({
            "quality": "masterpiece",
            "character": "hatsune miku",
            "count": "1girl",
            "appearance": [],
            "tags": ["smile"],
            "environment": [],
            "nl": ""
        });
        // LLM 把已经在 character/quality 里的标签也塞进了 appearance 段
        let buckets = TagBuckets {
            count: Some(vec!["1girl".into()]),
            appearance: Some(vec!["hatsune miku".into(), "masterpiece".into(), "smile".into()]),
            environment: Some(vec![]),
            tags: Some(vec![]),
        };
        let mut refined = buckets.all_tags();
        refined.push("hatsune miku".into());
        apply_buckets_to_json(&mut data, &buckets, &refined);

        assert_eq!(data["character"], "hatsune miku");
        assert_eq!(data["quality"], "masterpiece");
        // 重复的被剔除，只剩真正属于 appearance 的
        assert_eq!(data["appearance"], serde_json::json!(["smile"]));
        assert_eq!(data["tags"], serde_json::json!([]));
    }

    /// 只给了部分段时，未给出的段沿用原值（仅清理被删标签），新增标签兜底进 tags
    #[test]
    fn rebucket_partial_segments_keep_rest() {
        let mut data = serde_json::json!({
            "quality": "", "series": "", "artist": "", "character": "",
            "count": "1girl",
            "appearance": ["long hair", "red eyes"],
            "tags": ["smile"],
            "environment": [],
            "nl": ""
        });
        // 只给了 ENVIRONMENT 段；refined 里没有 red eyes（= 被删），多了 blush（未归段）
        let buckets = TagBuckets {
            count: None,
            appearance: None,
            environment: Some(vec!["simple background".into()]),
            tags: None,
        };
        let refined: Vec<String> = ["1girl", "long hair", "smile", "simple background", "blush"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        apply_buckets_to_json(&mut data, &buckets, &refined);

        assert_eq!(data["count"], "1girl");
        // red eyes 被删，long hair 留在原字段
        assert_eq!(data["appearance"], serde_json::json!(["long hair"]));
        assert_eq!(data["environment"], serde_json::json!(["simple background"]));
        // 未归入任何段的新增标签兜底追加到 tags
        assert_eq!(data["tags"], serde_json::json!(["smile", "blush"]));
    }
}

/// 端到端测试：本地 mock OpenAI 兼容服务器 + 真实图片/标签文件，
/// 跑 process_single_file 全链路（读标签 → 请求 → 解析 → 写盘）
#[cfg(test)]
mod e2e_tests {
    use super::*;
    use std::io::{Read, Write};
    use std::sync::mpsc;

    fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
        haystack.windows(needle.len()).position(|w| w == needle)
    }

    /// 起一个最小 mock：接受一个请求，body 发回 channel，返回固定 content 的 OpenAI 响应
    fn mock_openai_server(content: &str) -> (String, mpsc::Receiver<String>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let (tx, rx) = mpsc::channel();
        let body = serde_json::json!({
            "choices": [{
                "message": {"role": "assistant", "content": content},
                "finish_reason": "stop"
            }]
        })
        .to_string();
        std::thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                let mut buf = Vec::new();
                let mut tmp = [0u8; 8192];
                let mut header_end = None;
                let mut content_len = 0usize;
                loop {
                    let n = stream.read(&mut tmp).unwrap_or(0);
                    if n == 0 {
                        break;
                    }
                    buf.extend_from_slice(&tmp[..n]);
                    if header_end.is_none() {
                        if let Some(pos) = find_subslice(&buf, b"\r\n\r\n") {
                            header_end = Some(pos + 4);
                            for line in String::from_utf8_lossy(&buf[..pos]).lines() {
                                let l = line.to_lowercase();
                                if let Some(v) = l.strip_prefix("content-length:") {
                                    content_len = v.trim().parse().unwrap_or(0);
                                }
                            }
                        }
                    }
                    if let Some(he) = header_end {
                        if buf.len() >= he + content_len {
                            break;
                        }
                    }
                }
                if let Some(he) = header_end {
                    let _ = tx.send(String::from_utf8_lossy(&buf[he..]).to_string());
                }
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = stream.write_all(resp.as_bytes());
            }
        });
        (format!("http://127.0.0.1:{}/v1/chat/completions", port), rx)
    }

    fn make_options(endpoint: String) -> TagRefineOptions {
        TagRefineOptions {
            input_path: String::new(),
            output_path: String::new(),
            api_endpoint: endpoint,
            api_key: "test-key".into(),
            model_name: "mock-vlm".into(),
            prompt: "当前标签:\n{tags}\n请调优".into(),
            temperature: 0.3,
            max_tokens: -1,
            image_size: 512,
            top_p: 0.0,
            request_interval_ms: -1,
            concurrency: 1,
            recursive: false,
            file_format: "txt".into(),
            image_detail: String::new(),
            caption_mode: false,
            trigger_word: String::new(),
            preserve_tags: false,
        }
    }

    fn setup_dir(tag: &str) -> (PathBuf, PathBuf) {
        let root = std::env::temp_dir().join(format!("purinbox_refine_e2e_{}_{}", tag, std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let img = root.join("a.png");
        image::RgbImage::from_pixel(64, 64, image::Rgb([120, 80, 160]))
            .save(&img)
            .unwrap();
        (root, img)
    }

    const FIXTURE_JSON: &str = r#"{
        "fixed": {"quality": "newest, safe", "series": "", "artist": ""},
        "character": {"name": "hatsune miku", "variant": ""},
        "ai_output": {"count": "1girl", "appearance": ["long hair", "blue eyes"],
                      "tags": ["smile"], "environment": [], "nl": "keep me"}
    }"#;

    /// txt 模式 + 只有 .json：回退读取、带字段语义发给 LLM、结果写 .txt、JSON 不动
    #[tokio::test]
    async fn txt_mode_json_fallback_end_to_end() {
        let (endpoint, rx) =
            mock_openai_server("TAGS: 1girl, long hair, smile, outdoors\nNL: txt 模式忽略");
        let (root, img) = setup_dir("fallback");
        std::fs::write(root.join("a.json"), FIXTURE_JSON).unwrap();

        let options = make_options(endpoint);
        // 绕开代理环境变量，直连本地 mock
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let last_req = std::sync::Arc::new(tokio::sync::Mutex::new(None));
        let result = process_single_file(&client, &img, &root, &root, &options, &last_req).await;

        assert!(matches!(result, FileResult::Success { .. }), "应成功: {result:?}");
        let txt = std::fs::read_to_string(root.join("a.txt")).unwrap();
        assert_eq!(txt, "1girl, long hair, smile, outdoors");
        // JSON 原文件不动，nl 保留
        let json_raw: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(root.join("a.json")).unwrap()).unwrap();
        assert_eq!(json_raw["ai_output"]["nl"], "keep me");
        // 发给 LLM 的标签带字段标签（字段含义 + count: / appearance: 行）
        let sent = rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .expect("mock 服务器应收到请求");
        assert!(sent.contains("appearance: long hair, blue eyes"), "请求应有字段标签: {}", &sent[..sent.len().min(400)]);
        assert!(sent.contains("count: 1girl"));
        let _ = std::fs::remove_dir_all(&root);
    }

    /// 拒绝语带 NL: 前缀 → 判失败且不写任何文件
    #[tokio::test]
    async fn marked_refusal_fails_without_writing() {
        let (endpoint, _rx) =
            mock_openai_server("NL: I'm sorry, I cannot describe this image.");
        let (root, img) = setup_dir("refusal");
        std::fs::write(root.join("a.json"), FIXTURE_JSON).unwrap();

        let options = make_options(endpoint);
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let last_req = std::sync::Arc::new(tokio::sync::Mutex::new(None));
        let result = process_single_file(&client, &img, &root, &root, &options, &last_req).await;

        assert!(matches!(result, FileResult::Error { .. }), "拒绝应判失败: {result:?}");
        assert!(!root.join("a.txt").exists(), "拒绝时不应写出 txt");
        let _ = std::fs::remove_dir_all(&root);
    }
}
