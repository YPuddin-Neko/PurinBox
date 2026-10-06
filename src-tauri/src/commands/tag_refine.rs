use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use super::llm_batch::{self, ItemOutcome, LlmBatchResult};
use super::llm_client::{
    self, fmt_elapsed, pick_tag_line, reject_refusal, summarize_tags, ChatMessage, ChatParams,
    RequestThrottle, ChatError,
};
use super::tag_manager::is_full_json;
use super::tag_text::{field_tags, split_tags};
use super::tagger::hybrid;
use super::{ProblemArchive, ProgressEvent};
use crate::commands::{collect_image_files_with_recursive_excluding, output_path_for_input};

const EVENT: &str = "tag-refine-progress";

/// 辅助打标里本地标签为空的图片的错误
const EMPTY_LOCAL_LABELS: &str = "本地标签为空，已跳过 VLM 打标";

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
    #[serde(default = "default_image_size")]
    pub image_size: u32,
    #[serde(default)]
    pub top_p: f64,
    #[serde(default = "llm_client::default_short_reply_threshold")]
    pub short_reply_threshold: u32,
    #[serde(default = "default_interval")]
    pub request_interval_ms: i64,
    #[serde(default = "default_concurrency")]
    pub concurrency: u32,
    #[serde(default)]
    pub recursive: bool,
    /// 标签文件格式: "txt"（默认）| "json"（完整/简化格式自动识别；按 preserve_tags 与回复格式
    /// 分三种写回：保集合归类、按字段归属重排、差量写回）
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
    /// 仅 JSON：只补写自然语言字段，不应用回复中的标签更改。
    #[serde(default)]
    pub nl_only: bool,
    /// 辅助打标的调优阶段：标签从本地打标的草稿读（见 `hybrid::source_path`），
    /// 草稿读过之后无论成败都删除
    #[serde(default)]
    pub hybrid_mode: bool,
    /// 仅辅助打标：不覆盖已有的 txt/json 标签文件，调优期间才出现的也不覆盖
    #[serde(default)]
    pub skip_existing_labels: bool,
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

#[derive(Debug)]
enum FileResult {
    Success {
        filename: String,
        original_count: usize,
        refined_count: usize,
        nl_written: bool,
        changed: bool,
        warnings: Vec<String>,
        elapsed_ms: u128,
    },
    DescriptionAdded {
        filename: String,
        tag_count: usize,
        word_count: usize,
        warnings: Vec<String>,
        elapsed_ms: u128,
    },
    /// 自然语言打标：写入的是一整段描述，没有"标签数"和增删可言
    Captioned {
        filename: String,
        original_count: usize,
        word_count: usize,
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
    Warning {
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
) -> Result<LlmBatchResult, String> {
    // 互斥：全局取消标志不允许并发运行（辅助打标与精修页并发会互吞取消）
    static REFINE_RUNNING: AtomicBool = AtomicBool::new(false);
    let _busy = crate::commands::BusyGuard::acquire(&REFINE_RUNNING, "标签精修")?;

    TAG_REFINE_CANCELLED.store(false, Ordering::SeqCst);
    let client = llm_client::llm_http_client()?;
    refine_dataset(&app, options, client).await
}

/// 细化输入里的全部图片：开始新一轮运行，逐张请求 VLM 写回标签，
/// 收尾时归集问题文件并发唯一的终态 done
async fn refine_dataset<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    options: TagRefineOptions,
    client: reqwest::Client,
) -> Result<LlmBatchResult, String> {
    super::begin_run(EVENT);
    let input_dir = PathBuf::from(&options.input_path);
    // 输入可以是单张图片，辅助打标又固定把输入路径当输出路径传进来。
    // 不取所在目录的话，下面的 create_dir_all 会试图创建一个与图片同名的目录
    let output_dir = crate::commands::dir_of(Path::new(&options.output_path));

    // 失败/警告图副本落在 Fail、Warn 里，收集侧已统一剪枝，不会被当成新图
    let files = collect_image_files_with_recursive_excluding(
        &input_dir,
        options.recursive,
        Some(&output_dir),
    )?;
    let total = files.len() as u32;

    if total == 0 {
        return Err("输入目录中没有找到图片文件".to_string());
    }

    std::fs::create_dir_all(&output_dir).map_err(|e| format!("创建输出目录失败: {}", e))?;

    let concurrency = std::cmp::max(1, options.concurrency) as usize;

    let action = if options.nl_only {
        "补充自然语言描述"
    } else if options.hybrid_mode {
        "辅助打标（VLM 调优）"
    } else {
        "标签细化"
    };
    let summary = if options.nl_only {
        "自然语言描述补充完成"
    } else if options.hybrid_mode {
        "VLM 调优完成"
    } else {
        "标签细化完成"
    };
    ProgressEvent::new(
        "info",
        format!(
            "找到 {} 张图片，{} 线程开始{}...",
            total, concurrency, action
        ),
    )
    .at(0, total)
    .emit(app, EVENT);

    let (recursive, hybrid_mode) = (options.recursive, options.hybrid_mode);
    let throttle = Arc::new(RequestThrottle::new(options.request_interval_ms));
    let input_root = input_dir.clone();
    let work_output_dir = output_dir.clone();
    let outcome = llm_batch::run_file_batch(
        app,
        EVENT,
        &files,
        concurrency,
        &TAG_REFINE_CANCELLED,
        move |file_path| {
            let (client, options, input_root, output_dir, throttle) = (
                client.clone(),
                options.clone(),
                input_root.clone(),
                work_output_dir.clone(),
                throttle.clone(),
            );
            async move {
                process_single_file(
                    &client,
                    &file_path,
                    &input_root,
                    &output_dir,
                    &options,
                    &throttle,
                )
                .await
                .into_outcome(options.hybrid_mode)
            }
        },
    )
    .await;

    if hybrid_mode {
        clear_batch_drafts(app, &files);
    }

    let archive = ProblemArchive::new(&input_dir, &output_dir, recursive);
    Ok(outcome.finish(app, EVENT, summary, &archive))
}

/// 辅助打标收尾：本批每张图的草稿都删除，取消后没轮到的图也删（下次运行会重新准备草稿）；
/// 删不掉的汇总成一条警告
fn clear_batch_drafts<R: tauri::Runtime>(app: &tauri::AppHandle<R>, images: &[PathBuf]) {
    let errors: Vec<String> = images
        .iter()
        .filter_map(|image| hybrid::clear_drafts(image).err())
        .collect();
    if let Some(first) = errors.first() {
        ProgressEvent::new(
            "warning",
            format!("{}（共 {} 张图片）", first, errors.len()),
        )
        .emit(app, EVENT);
    }
}

impl FileResult {
    fn into_outcome(self, hybrid_mode: bool) -> ItemOutcome {
        match self {
            Self::Success {
                filename,
                original_count,
                refined_count,
                nl_written,
                changed,
                warnings,
                elapsed_ms,
            } => {
                let detail = if hybrid_mode {
                    format!(
                        "TAG {} → {}{}",
                        original_count,
                        refined_count,
                        if nl_written {
                            " | 自然语言描述已补充"
                        } else {
                            ""
                        },
                    )
                } else {
                    format!("原TAG {} → 细化后 {}", original_count, refined_count)
                };
                ItemOutcome::completed(
                    format!(
                        "[完成] {} | {} | {}",
                        filename, detail, fmt_elapsed(elapsed_ms)
                    ),
                    &warnings,
                    (!changed).then_some(" (未变化)"),
                )
            }
            Self::DescriptionAdded {
                filename,
                tag_count,
                word_count,
                warnings,
                elapsed_ms,
            } => ItemOutcome::completed(
                format!(
                    "[完成] {} | 自然语言描述已补充 {} 字/词 | TAG {} | {}",
                    filename, word_count, tag_count, fmt_elapsed(elapsed_ms),
                ),
                &warnings,
                None,
            ),
            Self::Captioned {
                filename,
                original_count,
                word_count,
                warnings,
                elapsed_ms,
            } => ItemOutcome::completed(
                format!(
                    "[完成] {} | 参考 {} 个标签 → 描述 {} 字/词 | {}",
                    filename,
                    original_count,
                    word_count,
                    fmt_elapsed(elapsed_ms)
                ),
                &warnings,
                None,
            ),
            Self::Skipped { filename, reason } => ItemOutcome::Done {
                message: format!("[跳过] {} ({})", filename, reason),
                warning: false,
            },
            Self::Error { filename, message } => ItemOutcome::Failed { filename, message },
            Self::Warning { filename, message } => ItemOutcome::Warned { filename, message },
        }
    }
}

/// JSON 标签字段布局（完整格式 vs 简化格式）
/// string_fields 指定重排时的字符串字段，其余写数组；新增标签落入 added_to；
/// nl_path 是自然语言描述字段（LLM 返回 NL: 段时写入）
struct JsonTagLayout {
    labeled: &'static [(&'static str, &'static [&'static str])],
    string_fields: &'static [&'static [&'static str]],
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
    labeled: FULL_LABELED_FIELDS,
    string_fields: &[
        &["fixed", "quality"],
        &["fixed", "series"],
        &["fixed", "artist"],
        &["character", "name"],
        &["character", "variant"],
        &["ai_output", "count"],
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
    labeled: SIMPLIFIED_LABELED_FIELDS,
    string_fields: &[
        &["quality"],
        &["series"],
        &["artist"],
        &["character"],
        &["count"],
    ],
    added_to: &["tags"],
    nl_path: &["nl"],
    artist_path: &["artist"],
    bucket_paths: [&["count"], &["appearance"], &["tags"], &["environment"]],
};

fn json_layout(data: &serde_json::Value) -> &'static JsonTagLayout {
    if is_full_json(data) {
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
    layout.string_fields.contains(&path)
}

/// 沿路径读一个字段的标签（见 `tag_text::field_tags`），字段不存在时为空
fn path_tags(data: &serde_json::Value, path: &[&str]) -> Vec<String> {
    json_get_path(data, path)
        .map(field_tags)
        .unwrap_or_default()
}

fn tags_value(as_string: bool, tags: Vec<String>) -> serde_json::Value {
    if as_string {
        serde_json::Value::String(tags.join(", "))
    } else {
        serde_json::Value::Array(tags.into_iter().map(serde_json::Value::String).collect())
    }
}

fn append_tags(data: &mut serde_json::Value, path: &[&str], extra: Vec<String>) {
    if extra.is_empty() {
        return;
    }
    let as_string = json_get_path(data, path).is_some_and(serde_json::Value::is_string);
    let mut tags = path_tags(data, path);
    tags.extend(extra);
    json_set_path(data, path, tags_value(as_string, tags));
}

/// 文件里的全部标签（不含 nl），单值字段在前、其余按字段表顺序。
/// 发给 LLM 的原标签列表和写回后的增删统计都用它
fn flatten_json_tags(data: &serde_json::Value) -> Vec<String> {
    let layout = json_layout(data);
    layout
        .string_fields
        .iter()
        .copied()
        .chain(
            layout
                .labeled
                .iter()
                .map(|(_, path)| *path)
                .filter(|path| !path_is_string_field(layout, path)),
        )
        .flat_map(|path| path_tags(data, path))
        .collect()
}

/// (语义字段名, 路径)，顺序对齐 Anima caption 的字段顺序；完整/简化格式各一份
const FULL_LABELED_FIELDS: &[(&str, &[&str])] = &[
    ("quality", &["fixed", "quality"]),
    ("count", &["ai_output", "count"]),
    ("character", &["character", "name"]),
    ("character.variant", &["character", "variant"]),
    ("series", &["fixed", "series"]),
    ("artist", &["fixed", "artist"]),
    ("appearance", &["ai_output", "appearance"]),
    ("tags", &["ai_output", "tags"]),
    ("environment", &["ai_output", "environment"]),
    ("from_path.appearance", &["from_path", "appearance"]),
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

/// 把 JSON 标签按字段标签渲染成多行文本——txt 模式下没有 .txt 只有 .json 时，
/// 不摊平转换、直接读 JSON：字段结构带着语义喂给 VLM，
/// 比一串扁平标签更容易核对画面内容（字段含义随文本一并给出）
fn render_json_tags_labeled(data: &serde_json::Value) -> String {
    let mut out = String::from(
        "(字段含义 Field meanings: quality=质量标签, count=人数, character=角色名, \
         character.variant=角色版本, series=作品名, artist=画师(@ 前缀), \
         from_path.appearance=路径中的外观标签, appearance=外观(发型/发色/瞳色/服装/配饰), \
         tags=动作/表情/姿势/构图/物品, environment=背景/场景/光影/氛围)",
    );
    for (label, path) in json_layout(data).labeled {
        let tags = path_tags(data, path);
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

    for (_, path) in layout.labeled {
        if let Some(slot) = json_get_path_mut(data, path) {
            if !slot.is_string() && !slot.is_array() {
                continue;
            }
            let kept: Vec<String> = field_tags(slot)
                .into_iter()
                .filter(|tag| refined_set.contains(tag.as_str()))
                .collect();
            seen.extend(kept.iter().cloned());
            *slot = tags_value(slot.is_string(), kept);
        }
    }
    let added = refined
        .iter()
        .filter(|tag| !seen.contains(tag.as_str()))
        .cloned()
        .collect();
    append_tags(data, layout.added_to, added);
}

/// 一次 LLM 调用的产出：标签调优、TXT 描述或仅补 JSON 描述。
enum RefineOutput {
    Tags {
        tags: Vec<String>,
        nl: Option<String>,
        buckets: TagBuckets,
    },
    Caption(String),
    Description(String),
}

/// LLM 按字段归类返回的结果。
/// `None` = 响应里没有这一段，该字段原样保留，已有标签维持本地打标器给的归属。
#[derive(Debug, Default, Clone, PartialEq)]
struct TagBuckets {
    count: Option<Vec<String>>,
    appearance: Option<Vec<String>>,
    environment: Option<Vec<String>>,
    tags: Option<Vec<String>>,
}

impl TagBuckets {
    /// 四个字段按 `JsonTagLayout::bucket_paths` 的顺序排列：count / appearance / tags / environment
    fn slots(&self) -> [&Option<Vec<String>>; 4] {
        [&self.count, &self.appearance, &self.tags, &self.environment]
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

/// 非重排字段和回复中缺失的字段原样保留；显式给出的字段可增删、重排。
/// 原样保留的字段优先占用标签，防止回复把它们复制到其他字段。
fn apply_buckets_to_json(data: &mut serde_json::Value, buckets: &TagBuckets) {
    let layout = json_layout(data);
    let slots = buckets.slots();
    let mut used: HashSet<String> = HashSet::new();
    for (_, path) in layout.labeled {
        let assigned = layout
            .bucket_paths
            .iter()
            .position(|bucket| bucket == path)
            .is_some_and(|index| slots[index].is_some());
        if !assigned {
            used.extend(path_tags(data, path));
        }
    }
    for (index, path) in layout.bucket_paths.iter().enumerate() {
        let Some(tags) = slots[index] else { continue };
        let assigned = tags
            .iter()
            .filter(|tag| !tag.is_empty() && used.insert((*tag).clone()))
            .cloned()
            .collect();
        json_set_path(
            data,
            path,
            tags_value(path_is_string_field(layout, path), assigned),
        );
    }
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
        .map(|path| path_tags(data, path))
        .collect();

    // 按归属重新分配；LLM 没提到的标签留在原字段
    let mut next: Vec<Vec<String>> = vec![Vec::new(); layout.bucket_paths.len()];
    for (origin, tags) in current.iter().enumerate() {
        for t in tags {
            let target = assign.get(&t.to_lowercase()).copied().unwrap_or(origin);
            if !next[target].iter().any(|x| x.eq_ignore_ascii_case(t)) {
                next[target].push(t.clone());
            }
        }
    }

    for (i, path) in layout.bucket_paths.iter().enumerate() {
        let value = tags_value(
            path_is_string_field(layout, path),
            std::mem::take(&mut next[i]),
        );
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
    let existing = path_tags(data, path);
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
    /// 各字段段拆出的标签；响应里没有的段为 None
    buckets: TagBuckets,
    nl: Option<String>,
    /// 未归入任何标记段的其余行
    rest: Vec<&'a str>,
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
            .trim_start_matches(['*', '#', '>', '-', '`'])
            .trim_start();

        let mut matched = false;
        for (prefix, slot) in [
            ("count:", &mut out.buckets.count),
            ("appearance:", &mut out.buckets.appearance),
            ("environment:", &mut out.buckets.environment),
            ("tags:", &mut out.buckets.tags),
        ] {
            if let Some(rest) = strip_ci_prefix(t, prefix) {
                // 空段得到空列表，代表"该字段清空"而非"未给出"
                *slot = Some(split_tags(rest.trim_start_matches('*')));
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

/// 处理单个文件：读取图片 + 对应标签 → LLM 细化。
/// 辅助打标时这张图的草稿读过之后无论成败都删除；删不掉的由整批收尾时再清理并报告
async fn process_single_file(
    client: &reqwest::Client,
    img_path: &Path,
    input_root: &Path,
    output_dir: &Path,
    options: &TagRefineOptions,
    throttle: &RequestThrottle,
) -> FileResult {
    let result =
        refine_single_file(client, img_path, input_root, output_dir, options, throttle).await;
    if options.hybrid_mode {
        let _ = hybrid::clear_drafts(img_path);
    }
    result
}

async fn refine_single_file(
    client: &reqwest::Client,
    img_path: &Path,
    input_root: &Path,
    output_dir: &Path,
    options: &TagRefineOptions,
    throttle: &RequestThrottle,
) -> FileResult {
    let start = std::time::Instant::now();
    let filename = super::file_name_lossy(img_path);
    let stem = img_path
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();
    let parent = img_path.parent().unwrap_or(Path::new("."));

    let skip_existing = options.hybrid_mode && options.skip_existing_labels;
    if skip_existing && super::tagger::has_labels(img_path) {
        return FileResult::Skipped {
            filename,
            reason: "已有标签文件".into(),
        };
    }

    // 查找对应的标签文件（txt 或 json）
    let is_json = options.file_format == "json";
    let tag_ext = if is_json { "json" } else { "txt" };
    let mut tag_path = if options.hybrid_mode {
        hybrid::source_path(img_path, tag_ext)
    } else {
        parent.join(format!("{}.{}", stem, tag_ext))
    };
    // txt 模式回退：没有 .txt 但有 .json 时直接读 JSON——字段结构带着语义
    // 喂给 VLM 比先摊平转换信息更全（输出是扁平 txt，写盘时反正要摊平）
    let mut json_fallback = !is_json
        && (tag_path.extension().is_some_and(|ext| ext == "json")
            || hybrid::draft_label_format(&tag_path) == Some("json"));
    if !options.hybrid_mode && !tag_path.exists() && !is_json {
        let jp = parent.join(format!("{}.json", stem));
        if jp.exists() {
            tag_path = jp;
            json_fallback = true;
        }
    }
    if !tag_path.exists() {
        return FileResult::Skipped {
            filename,
            reason: if options.hybrid_mode {
                "无可用的本地标签".into()
            } else {
                format!("无对应 .{} 标签文件", tag_ext)
            },
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

    // json 模式：解析并扁平化标签；txt 模式：逗号拆分。
    // json 回退（txt 模式读到了 .json）：同样解析展开，但不进 json_data——
    // 结果始终写回 .txt，JSON 原文件不动
    let reads_json = is_json || json_fallback;
    let parsed = if reads_json && !tag_content.is_empty() {
        match serde_json::from_str::<serde_json::Value>(&tag_content) {
            Ok(v) => Some(v),
            Err(e) => {
                return FileResult::Error {
                    filename,
                    message: format!("解析 JSON 标签失败: {}", e),
                }
            }
        }
    } else {
        None
    };
    let original_tags: Vec<String> = match &parsed {
        Some(data) => flatten_json_tags(data),
        None if reads_json => Vec::new(),
        None => split_tags(&tag_content),
    };
    let output_path = || {
        output_path_for_input(
            input_root,
            img_path,
            output_dir,
            &format!("{}.{}", stem, tag_ext),
            options.recursive,
        )
    };

    if original_tags.is_empty() {
        if !options.hybrid_mode {
            return FileResult::Skipped {
                filename,
                reason: if tag_content.is_empty() {
                    "标签文件为空"
                } else {
                    "无有效标签"
                }
                .to_string(),
            };
        }
        // 正常的本地打标不会一个标签都没有：按出错处理，不请求 VLM、不写标签文件
        return FileResult::Error {
            filename,
            message: EMPTY_LOCAL_LABELS.into(),
        };
    }

    let tags_display = parsed
        .as_ref()
        .filter(|_| json_fallback)
        .map(render_json_tags_labeled);
    let json_data = parsed.filter(|_| is_json);

    // 调用 LLM 细化（tags_display：JSON 回退时带字段标签的展示文本）
    let (output, response_count) = match refine_tags_with_llm(
        client,
        img_path,
        &original_tags,
        tags_display.as_deref(),
        options,
        throttle,
    )
    .await
    {
        Ok(output) => output,
        Err(e) if e.is_warning() => {
            return FileResult::Warning { filename, message: e.to_string() };
        }
        Err(e) => {
            return FileResult::Error {
                filename,
                message: e.to_string(),
            }
        }
    };
    let short_warnings: Vec<String> = if options.hybrid_mode {
        llm_client::short_reply_warning(response_count, options.short_reply_threshold)
            .into_iter()
            .collect()
    } else {
        Vec::new()
    };
    let elapsed_ms = start.elapsed().as_millis();
    let original_count = original_tags.len();
    let output_path = match output_path() {
        Ok(path) => path,
        Err(e) => {
            return FileResult::Error {
                filename,
                message: e,
            }
        }
    };

    let (output_content, done) = match output {
        RefineOutput::Description(description) => {
            let Some(mut data) = json_data else {
                return FileResult::Error {
                    filename,
                    message: "仅补自然语言描述需要 JSON 标签".into(),
                };
            };
            set_json_nl(&mut data, &description);
            set_json_trigger(&mut data, &options.trigger_word);
            let content = match serde_json::to_string_pretty(&data) {
                Ok(content) => content,
                Err(error) => {
                    return FileResult::Error {
                        filename,
                        message: format!("序列化 JSON 失败: {}", error),
                    }
                }
            };
            let done = FileResult::DescriptionAdded {
                filename: filename.clone(),
                tag_count: flatten_json_tags(&data).len(),
                word_count: llm_client::response_word_count(&description),
                warnings: short_warnings,
                elapsed_ms,
            };
            (content, done)
        }
        // 自然语言打标：整段描述直接落盘，标签只是刚才喂给 LLM 的参考
        RefineOutput::Caption(caption) => {
            let word_count = llm_client::response_word_count(&caption);
            // 触发词由后端保证在最前，不依赖 LLM 遵守提示词
            let caption = ensure_trigger_prefix(&caption, &options.trigger_word);
            let done = FileResult::Captioned {
                filename: filename.clone(),
                original_count,
                word_count,
                warnings: short_warnings,
                elapsed_ms,
            };
            (caption, done)
        }
        RefineOutput::Tags {
            tags: refined_tags,
            nl,
            buckets,
        } => {
            let preserving = is_json && options.preserve_tags;
            let mut written_tags = refined_tags.clone();
            let mut json_changed = false;
            let content = if let Some(mut data) = json_data {
                let original_data = data.clone();
                if options.preserve_tags {
                    // 只归类不增删：标签集合恒定，LLM 的回复只当作归属映射
                    if buckets.has_field_assignment() {
                        apply_buckets_preserving(&mut data, &buckets);
                    }
                } else if buckets.has_field_assignment() {
                    // LLM 给了字段归属：重排 count/appearance/environment/tags
                    // （本地打标器靠关键词表分类，"simple background" 之类常落错格）
                    apply_buckets_to_json(&mut data, &buckets);
                } else {
                    // 旧协议：差量写回，保留原字段归属，仅应用增删
                    apply_refined_tags_to_json(&mut data, &refined_tags);
                }
                written_tags = flatten_json_tags(&data);
                // LLM 返回了 NL: 描述段时写入 nl 字段（本地打标器不产生 nl，由 LLM 补充）
                if let Some(nl_text) = nl.as_deref() {
                    set_json_nl(&mut data, nl_text);
                }
                // 触发词进 artist 字段（JSON 的触发词位置），txt 那边则是置于开头
                set_json_trigger(&mut data, &options.trigger_word);
                json_changed = data != original_data;
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
            let refined_count = written_tags.len();
            let changed = if is_json {
                json_changed
            } else {
                written_tags != original_tags
            };
            let mut warnings = short_warnings;
            // 比较实际写回的集合，缺段保留、非重排字段不会被误报为移除。
            if !preserving {
                let original: HashSet<&str> = original_tags.iter().map(String::as_str).collect();
                let written: HashSet<&str> = written_tags.iter().map(String::as_str).collect();
                let removed: Vec<&str> = original.difference(&written).copied().collect();
                let added: Vec<&str> = written.difference(&original).copied().collect();
                warnings.extend(summarize_tags("移除", &removed));
                warnings.extend(summarize_tags("新增", &added));
            }
            let done = FileResult::Success {
                filename: filename.clone(),
                original_count,
                refined_count,
                nl_written: is_json && nl.is_some(),
                changed,
                warnings,
                elapsed_ms,
            };
            (content, done)
        }
    };

    match write_label(img_path, &output_path, &output_content, skip_existing) {
        Ok(true) => done,
        Ok(false) => FileResult::Skipped {
            filename,
            reason: "已有标签文件".into(),
        },
        Err(message) => FileResult::Error { filename, message },
    }
}

/// 写出正式标签。`keep_existing` 时（辅助打标跳过已有标签）不覆盖调优期间出现的标签文件，
/// 返回 Ok(false)；否则先写临时文件再替换，就地写回时写入中途失败也不会截断原标签
fn write_label(
    image: &Path,
    output: &Path,
    content: &str,
    keep_existing: bool,
) -> Result<bool, String> {
    if keep_existing {
        return hybrid::write_final(image, output, content);
    }
    super::config_paths::write_file_atomic(output, content.as_bytes())
        .map(|()| true)
        .map_err(|e| format!("写入失败: {}", e))
}

/// 调用多模态 LLM 进行标签细化（发送图片 + 已有标签）。
/// 自然语言打标模式返回整段描述；否则返回解析出的标签、`NL:` 段描述（JSON 模式用来补 nl 字段）
/// 和字段归属。
async fn refine_tags_with_llm(
    client: &reqwest::Client,
    img_path: &Path,
    tags: &[String],
    tags_display: Option<&str>,
    options: &TagRefineOptions,
    throttle: &RequestThrottle,
) -> Result<(RefineOutput, usize), ChatError> {
    if options.nl_only && (options.file_format != "json" || options.caption_mode) {
        return Err("仅补自然语言描述需要 JSON 输出，不能同时启用 TXT 自然语言打标".into());
    }
    let data_url = llm_client::load_image_data_url(img_path, options.image_size).await?;

    // JSON 回退时展示文本带字段标签和含义（count: 1girl / appearance: ...），
    // 否则就是扁平的逗号分隔列表
    let tag_list = tags_display
        .map(|s| s.to_string())
        .unwrap_or_else(|| tags.join(", "));

    let user_text = if options.prompt.contains("{tags}") {
        options.prompt.replace("{tags}", &tag_list)
    } else {
        format!(
            "{}\n\nExisting tags: {}\n\nRefined tags:",
            options.prompt, tag_list
        )
    };

    let params = ChatParams {
        endpoint: &options.api_endpoint,
        api_key: &options.api_key,
        model: &options.model_name,
        temperature: options.temperature,
        max_tokens: -1,
        top_p: options.top_p,
    };
    let reply = llm_client::chat_completion(
        client,
        &params,
        &[ChatMessage::user(llm_client::vision_user_content(
            &user_text,
            &data_url,
            &options.image_detail,
        ))],
        throttle,
        &TAG_REFINE_CANCELLED,
    )
    .await;

    // 自然语言打标：回复整段就是标签文件内容，不进标签解析
    if options.caption_mode {
        let caption = llm_client::accept_reply(&reply, "该图片")?;
        return Ok((
            RefineOutput::Caption(caption.to_string()),
            llm_client::response_word_count(caption),
        ));
    }

    // 拒绝语由 parse_refine_response 按段判定：标记格式的 NL 段里，正常句子也可能含拒绝措辞
    let content = llm_client::accept_reply_unscreened(&reply, "该图片")?;
    if options.nl_only {
        let description = match split_marker_response(content).nl {
            Some(description) => description,
            None => {
                reject_refusal(content, "该图片")?;
                return Err(ChatError::ReplyWarning("未返回有效的自然语言描述（NL），原标签文件未修改".into()));
            }
        };
        reject_refusal(&description, "该图片")?;
        let count = llm_client::response_word_count(&description);
        return Ok((RefineOutput::Description(description), count));
    }
    let (tags, nl, buckets) = parse_refine_response(content, tags)?;
    Ok((
        RefineOutput::Tags { tags, nl, buckets },
        llm_client::response_word_count(content),
    ))
}

/// 把 LLM 的原始回复解析成 (最终标签, nl, 字段归属)。
/// 优先走标记格式（辅助打标 JSON 模式的默认提示词要求此格式）；
/// 无标记时退回旧启发式：多行取最长的含逗号行。
/// NL 段先于启发式剥离——自然语言长句常含逗号且比标签列表更长，会被启发式误选。
fn parse_refine_response(
    content: &str,
    original_tags: &[String],
) -> Result<(Vec<String>, Option<String>, TagBuckets), ChatError> {
    let marker = split_marker_response(content);

    // 拒绝语不能被标签列表启发式写入标签文件。
    let has_markers = marker.buckets.slots().iter().any(|s| s.is_some()) || marker.nl.is_some();
    if !has_markers {
        reject_refusal(content, "该图片")?;
    }
    // 模型拒绝时也常常遵守输出格式（NL: I'm sorry, I cannot...）——
    // 上面那道闸被 has_markers 跳过，拒绝文本会被直接写进 nl 字段，
    // NL 段内容必须单独再过一遍（"仅补 nl 描述"预设必走这条路）
    if let Some(nl_text) = marker.nl.as_deref() {
        reject_refusal(nl_text, "该图片")?;
    }
    // TAGS:/字段段同理：TAGS: I'm sorry, I can't... 会被拆成"标签"写盘
    for seg in marker.buckets.slots().into_iter().flatten() {
        reject_refusal(&seg.join(", "), "该图片")?;
    }

    // 仅有 count 段不足以判定为标记格式，继续使用普通标签解析。
    let refined_tags: Vec<String> =
        if marker.buckets.tags.is_some() || marker.buckets.has_field_assignment() {
            marker.buckets.all_tags()
        } else {
            split_tags(pick_tag_line(&marker.rest.join("\n")))
        };

    // 只回了 NL: 一段（"仅补 nl 描述"这类提示词）：标签原样保留，不是失败
    if refined_tags.is_empty() && marker.nl.is_some() {
        return Ok((original_tags.to_vec(), marker.nl, TagBuckets::default()));
    }

    if refined_tags.is_empty() {
        return Err(ChatError::ReplyWarning("AI 返回的标签结果为空".into()));
    }

    Ok((refined_tags, marker.nl, marker.buckets))
}

#[cfg(test)]
mod marker_tests {
    use super::*;

    #[test]
    fn successful_warnings_and_caption_keep_completion_text() {
        let result = FileResult::Success {
            filename: "a b.png".into(),
            original_count: 2,
            refined_count: 3,
            nl_written: false,
            changed: true,
            warnings: vec!["新增: smile".into()],
            elapsed_ms: 1500,
        }
        .into_outcome(false);
        match result {
            ItemOutcome::Done { message, warning } => {
                assert!(warning);
                assert_eq!(
                    message,
                    "[完成] a b.png | 原TAG 2 → 细化后 3 | 1.5s | 新增: smile"
                );
            }
            _ => panic!("successful warning expected"),
        }
        let result = FileResult::Captioned {
            filename: "a b.png".into(),
            original_count: 2,
            word_count: 4,
            warnings: Vec::new(),
            elapsed_ms: 500,
        }
        .into_outcome(false);
        match result {
            ItemOutcome::Done { message, warning } => {
                assert!(!warning);
                assert_eq!(
                    message,
                    "[完成] a b.png | 参考 2 个标签 → 描述 4 字/词 | 500ms"
                );
            }
            _ => panic!("caption expected"),
        }
    }

    #[test]
    fn omitted_count_is_preserved_exactly_in_both_json_layouts() {
        for mut data in [
            serde_json::json!({"count": " 1girl, solo ", "tags": ["smile"], "environment": []}),
            serde_json::json!({"ai_output": {"count": ["1girl", "solo"], "tags": ["smile"], "environment": []}}),
        ] {
            let count_path = json_layout(&data).bucket_paths[0];
            let original_count = json_get_path(&data, count_path).cloned();
            let (_, _, buckets) = parse_refine_response(
                "APPEARANCE: long hair\nTAGS: smile\nENVIRONMENT: outdoors",
                &[],
            )
            .unwrap();
            assert!(buckets.count.is_none());
            apply_buckets_to_json(&mut data, &buckets);
            assert_eq!(json_get_path(&data, count_path).cloned(), original_count);
        }
    }

    #[test]
    fn omitted_fields_keep_ownership_even_when_other_segments_repeat_their_tags() {
        let mut data = serde_json::json!({"appearance": ["long hair"], "tags": ["smile"]});
        let (_, _, buckets) =
            parse_refine_response("ENVIRONMENT: long hair, outdoors", &[]).unwrap();
        apply_buckets_to_json(&mut data, &buckets);
        assert_eq!(data["appearance"], serde_json::json!(["long hair"]));
        assert_eq!(data["tags"], serde_json::json!(["smile"]));
        assert_eq!(data["environment"], serde_json::json!(["outdoors"]));
        assert!(data.get("count").is_none());
    }

    #[test]
    fn mixed_field_types_share_reading_and_keep_storage_type_on_delta_write() {
        let mut data = serde_json::json!({
            "fixed": {"quality": ["best quality"], "artist": ["@artist"]},
            "character": {"name": ["miku"], "variant": "winter outfit"},
            "from_path": {"appearance": "scarf, boots"},
            "ai_output": {"count": ["1girl"], "appearance": "long hair, blue eyes", "tags": "smile"},
            "extra": {"untouched": true}
        });
        let flat = flatten_json_tags(&data);
        let rendered = render_json_tags_labeled(&data);
        for tag in &flat {
            assert!(rendered.contains(tag), "{tag}");
        }
        assert!(flat.contains(&"boots".to_string()));
        assert!(flat.contains(&"winter outfit".to_string()));
        let mut refined = flat;
        refined.retain(|tag| tag != "blue eyes");
        refined.push("standing".into());
        apply_refined_tags_to_json(&mut data, &refined);
        assert_eq!(data["ai_output"]["appearance"], "long hair");
        assert_eq!(data["ai_output"]["tags"], "smile, standing");
        assert_eq!(
            data["fixed"]["quality"],
            serde_json::json!(["best quality"])
        );
        assert_eq!(data["from_path"]["appearance"], "scarf, boots");
        assert_eq!(data["extra"], serde_json::json!({"untouched": true}));
        set_json_trigger(&mut data, "trigger");
        assert_eq!(data["fixed"]["artist"], "trigger, @artist");
    }

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

    /// 有标记时优先按标记解析，避免把 NL 长句当成标签列表。
    #[test]
    fn nl_never_leaks_into_tags() {
        let m = split_marker_response(
            "TAGS: 1girl\nNL: An extremely long description, with many commas, that is much longer than the tag list itself.",
        );
        assert_eq!(m.buckets.tags, Some(vec!["1girl".to_string()]));
        assert!(m.nl.unwrap().starts_with("An extremely long"));
        assert!(m.rest.is_empty());
    }

    /// 四段字段归属格式：解析出各字段并按 count→appearance→tags→environment 展开
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

    /// 安全审核拒绝必须失败，拒绝语不能写入标签文件。
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

    /// 正常标签列表可包含拒绝措辞，逗号数量用于区分两种内容。
    #[test]
    fn normal_tag_lists_are_not_mistaken_for_refusal() {
        // 逗号多 = 标签列表，即便含 "i can't" 之类的字样
        let tags = "1girl, solo, i can't believe it's not butter, smile, outdoors";
        let (parsed, _, _) = parse_refine_response(tags, &[]).unwrap();
        assert!(parsed.contains(&"1girl".to_string()));
        // 按标记格式返回的短回复也不该被误判
        let marked =
            parse_refine_response("TAGS: sorry\nNL: A girl with a sorry expression.", &[]).unwrap();
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
        let simp =
            serde_json::json!({"count": "2girls", "tags": "sitting, looking at viewer", "nl": "x"});
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
        assert_eq!(
            data["ai_output"]["appearance"],
            serde_json::json!(["long hair"])
        );
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
        apply_buckets_to_json(&mut data, &buckets);

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

    /// 重排时不让标签同时出现在 character/quality 和其他字段。
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
            appearance: Some(vec![
                "hatsune miku".into(),
                "masterpiece".into(),
                "smile".into(),
            ]),
            environment: Some(vec![]),
            tags: Some(vec![]),
        };
        apply_buckets_to_json(&mut data, &buckets);

        assert_eq!(data["character"], "hatsune miku");
        assert_eq!(data["quality"], "masterpiece");
        // 重复的被剔除，只剩真正属于 appearance 的
        assert_eq!(data["appearance"], serde_json::json!(["smile"]));
        assert_eq!(data["tags"], serde_json::json!([]));
    }

    /// 生产解析路径只返回部分字段时，其他字段原样保留。
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
        let (_, _, buckets) = parse_refine_response("ENVIRONMENT: simple background", &[]).unwrap();
        apply_buckets_to_json(&mut data, &buckets);

        assert_eq!(data["count"], "1girl");
        assert_eq!(
            data["appearance"],
            serde_json::json!(["long hair", "red eyes"])
        );
        assert_eq!(
            data["environment"],
            serde_json::json!(["simple background"])
        );
        assert_eq!(data["tags"], serde_json::json!(["smile"]));
    }
}

#[cfg(test)]
mod e2e_tests {
    use super::*;
    use crate::commands::llm_client::test_support::{client, serve_chat_reply, serve_json};
    use crate::commands::test_support::TempDir;

    fn make_options(endpoint: String) -> TagRefineOptions {
        TagRefineOptions {
            input_path: String::new(),
            output_path: String::new(),
            api_endpoint: endpoint,
            api_key: "test-key".into(),
            model_name: "mock-vlm".into(),
            prompt: "当前标签:\n{tags}\n请调优".into(),
            temperature: 0.3,
            image_size: 512,
            top_p: 0.0,
            short_reply_threshold: 100,
            request_interval_ms: -1,
            concurrency: 1,
            recursive: false,
            file_format: "txt".into(),
            image_detail: String::new(),
            caption_mode: false,
            trigger_word: String::new(),
            preserve_tags: false,
            nl_only: false,
            hybrid_mode: false,
            skip_existing_labels: false,
        }
    }

    fn setup_dir(tag: &str) -> (TempDir, PathBuf) {
        let root = TempDir::new(tag);
        let img = root.join("a.png");
        image::RgbImage::from_pixel(64, 64, image::Rgb([120, 80, 160]))
            .save(&img)
            .unwrap();
        (root, img)
    }

    const FIXTURE_JSON: &str = r#"{
        "fixed": {"quality": "newest, safe", "series": "", "artist": ""},
        "character": {"name": "hatsune miku", "variant": "winter outfit"},
        "from_path": {"appearance": "scarf, boots"},
        "ai_output": {"count": "1girl", "appearance": "long hair, blue eyes",
                      "tags": ["smile"], "environment": [], "nl": "keep me"}
    }"#;

    fn nl_only_options(endpoint: String) -> TagRefineOptions {
        let mut value = serde_json::to_value(make_options(endpoint)).unwrap();
        value["file_format"] = serde_json::json!("json");
        value["hybrid_mode"] = serde_json::json!(true);
        value["nl_only"] = serde_json::json!(true);
        value["prompt"] = serde_json::json!(
            "Write a natural language description. Do NOT modify tags.\nExisting tags: {tags}\nReply: NL: <description>"
        );
        serde_json::from_value(value).unwrap()
    }

    #[tokio::test]
    async fn hybrid_nl_only_writes_description_without_changing_either_json_layout() {
        use crate::commands::batch::capture_events;
        let full: serde_json::Value = serde_json::from_str(FIXTURE_JSON).unwrap();
        let mut full_without_nl = full.clone();
        full_without_nl["ai_output"].as_object_mut().unwrap().remove("nl");
        for source in [
            full,
            full_without_nl,
            serde_json::json!({"count": "1girl", "appearance": ["long hair"], "tags": ["smile"]}),
            serde_json::json!({
                "quality": "newest, safe", "artist": "original artist", "character": "example",
                "count": "1girl", "appearance": ["long hair", "blue eyes"],
                "tags": ["smile"], "environment": ["outdoors"], "nl": "old description",
                "custom": {"keep": true}
            }),
        ] {
            for reply in [
                "NL: A girl smiles outdoors.",
                "COUNT: 2boys\nAPPEARANCE: short hair\nTAGS: running\nENVIRONMENT: indoors\nNL: A girl smiles outdoors.",
            ] {
                let (root, img) = setup_dir("hybrid_nl_only");
                std::fs::write(img.with_extension("json"), source.to_string()).unwrap();
                let prepared = hybrid::prepare_sources(&root, false, "json").unwrap();
                assert_eq!((prepared.copied, prepared.unlabeled), (1, 0));
                let server = serve_chat_reply(Some(reply), "stop");
                let mut options = nl_only_options(server.url.clone());
                options.input_path = root.to_string_lossy().into_owned();
                options.output_path = options.input_path.clone();
                let app = tauri::test::mock_app();
                let events = capture_events(app.handle(), EVENT);
                let result = refine_dataset(app.handle(), options, client())
                    .await
                    .unwrap();
                assert_eq!((result.success_count, result.fail_count), (1, 0));
                let written: serde_json::Value =
                    serde_json::from_slice(&std::fs::read(img.with_extension("json")).unwrap())
                        .unwrap();
                let mut expected = source.clone();
                set_json_nl(&mut expected, "A girl smiles outdoors.");
                assert_eq!(written, expected, "only nl may change");
                assert!(!hybrid::draft_path(&img, "json").exists());
                let request = server
                    .requests
                    .recv_timeout(std::time::Duration::from_secs(5))
                    .unwrap();
                let body = request.json();
                let text = body["messages"][0]["content"][0]["text"]
                    .as_str()
                    .unwrap();
                assert!(text.contains("Do NOT modify tags."));
                assert!(text.contains("long hair"));
                assert!(!text.contains("{tags}"));
                let events = events.lock().unwrap();
                let warnings: Vec<_> = events
                    .iter()
                    .filter(|event| event["status"] == "warning")
                    .collect();
                assert_eq!(warnings.len(), 1, "{events:?}");
                assert!(warnings[0]["message"].as_str().unwrap().contains("疑似截断"));
                assert!(events.iter().all(|event| event["status"] != "error"));
                let messages: Vec<&str> = events
                    .iter()
                    .map(|event| event["message"].as_str().unwrap())
                    .collect();
                assert!(
                    messages.iter().all(|message| !message.contains("细化")),
                    "{messages:?}"
                );
                assert!(
                    messages.iter().any(|message| message.contains("自然语言描述已补充 4 字/词")),
                    "{messages:?}"
                );
                assert_eq!(
                    messages.first(),
                    Some(&"找到 1 张图片，1 线程开始补充自然语言描述...")
                );
                assert_eq!(
                    messages.last(),
                    Some(&"自然语言描述补充完成: 成功 1, 失败 0, 共 1")
                );
            }
        }
    }

    #[tokio::test]
    async fn hybrid_short_replies_warn_but_keep_results_for_txt_and_both_json_layouts() {
        for (format, source, caption_mode, nl_only) in [
            ("json", FIXTURE_JSON, false, true),
            ("json", r#"{"count":"1girl","tags":["smile"]}"#, false, true),
            ("txt", "1girl, smile", true, false),
            ("txt", "1girl, smile", false, false),
            ("json", FIXTURE_JSON, false, false),
            ("json", r#"{"count":"1girl","tags":["smile"]}"#, false, false),
        ] {
            for (count, threshold) in [(99, 100), (100, 100), (1, 1), (1, 2), (499, 500), (500, 500)] {
                let (root, img) = setup_dir("hybrid_short_reply");
                std::fs::write(img.with_extension(format), source).unwrap();
                hybrid::prepare_sources(&root, false, format).unwrap();
                let han_count = count.min(49);
                let text = format!("{}{}", "字".repeat(han_count), "word ".repeat(count - han_count));
                let reply = if nl_only {
                    format!("NL: {text}")
                } else {
                    text.clone()
                };
                let server = serve_chat_reply(Some(&reply), "stop");
                let mut options = make_options(server.url.clone());
                options.hybrid_mode = true;
                options.file_format = format.into();
                options.caption_mode = caption_mode;
                options.nl_only = nl_only;
                options.short_reply_threshold = threshold as u32;
                options.trigger_word = "trigger ".repeat(110);
                let result = process_single_file(
                    &client(),
                    &img,
                    &root,
                    &root,
                    &options,
                    &RequestThrottle::new(-1),
                )
                .await;
                match result.into_outcome(true) {
                    ItemOutcome::Done { message, warning } => {
                        assert_eq!(message.contains("疑似截断"), count < threshold, "{message}");
                        if count < threshold {
                            assert!(warning);
                            assert!(message.contains(" | 疑似截断"), "{message}");
                            assert!(!message.contains("回复仅"), "{message}");
                            assert!(!message.contains('⚠'), "{message}");
                        }
                    }
                    ItemOutcome::Failed { message, .. } | ItemOutcome::Warned { message, .. } => panic!("{message}"),
                }
                let written = std::fs::read_to_string(img.with_extension(format)).unwrap();
                if nl_only {
                    let actual: serde_json::Value = serde_json::from_str(&written).unwrap();
                    let mut expected: serde_json::Value = serde_json::from_str(source).unwrap();
                    set_json_nl(&mut expected, text.trim());
                    set_json_trigger(&mut expected, &options.trigger_word);
                    assert_eq!(actual, expected);
                } else if caption_mode {
                    assert_eq!(
                        written,
                        ensure_trigger_prefix(text.trim(), &options.trigger_word)
                    );
                } else {
                    assert!(written.contains('字'));
                }
                assert!(!hybrid::draft_path(&img, format).exists());
            }
        }
    }

    #[tokio::test]
    async fn hybrid_warning_archives_cover_formats_and_input_shapes() {
        use crate::commands::batch::capture_events;
        for (format, source, caption_mode, nl_only) in [
            ("json", FIXTURE_JSON, false, true),
            ("json", r#"{"count":"1girl","tags":["smile"]}"#, false, true),
            ("txt", "1girl, smile", true, false),
            ("txt", "1girl, smile", false, false),
            ("json", FIXTURE_JSON, false, false),
            ("json", r#"{"count":"1girl","tags":["smile"]}"#, false, false),
        ] {
            for (single, recursive) in [(false, false), (true, false), (false, true)] {
                for short_reply in [true, false] {
                    let root = TempDir::new("hybrid_warning_archive");
                    let relative = if recursive { "nested/a.png" } else { "a.png" };
                    let img = root.join(relative);
                    std::fs::create_dir_all(img.parent().unwrap()).unwrap();
                    image::RgbImage::new(2, 2).save(&img).unwrap();
                    let original_image = std::fs::read(&img).unwrap();
                    let count = if short_reply { 29 } else { 30 };
                    let text = "字".repeat(count);
                    // 标签调优保持标签不变，避免增删标签的警告干扰长度阈值断言。
                    let source = if nl_only || caption_mode {
                        source.to_string()
                    } else if format == "txt" {
                        text.clone()
                    } else if source == FIXTURE_JSON {
                        serde_json::json!({"ai_output": {"tags": [&text]}}).to_string()
                    } else {
                        serde_json::json!({"tags": [&text]}).to_string()
                    };
                    std::fs::write(img.with_extension(format), source).unwrap();
                    for artifact in ["Warn", "Fail"] {
                        let dir = root.join(artifact);
                        std::fs::create_dir_all(&dir).unwrap();
                        image::RgbImage::new(2, 2).save(dir.join("previous.png")).unwrap();
                    }
                    let input = if single { img.clone() } else { root.to_path_buf() };
                    let prepared = hybrid::prepare_sources(&input, recursive, format).unwrap();
                    assert_eq!((prepared.copied, prepared.unlabeled), (1, 0));
                    let reply = if nl_only { format!("NL: {text}") } else { text.clone() };
                    let server = serve_chat_reply(Some(&reply), "stop");
                    let mut options = make_options(server.url.clone());
                    options.input_path = input.to_string_lossy().into_owned();
                    options.output_path = options.input_path.clone();
                    options.file_format = format.into();
                    options.hybrid_mode = true;
                    options.caption_mode = caption_mode;
                    options.nl_only = nl_only;
                    options.recursive = recursive;
                    options.short_reply_threshold = 30;
                    let app = tauri::test::mock_app();
                    let events = capture_events(app.handle(), EVENT);
                    let result = refine_dataset(app.handle(), options, client()).await.unwrap();
                    assert_eq!((result.success_count, result.fail_count, result.total), (1, 0, 1));
                    assert_eq!(std::fs::read(&img).unwrap(), original_image);
                    assert!(std::fs::read_to_string(img.with_extension(format)).unwrap().contains(&text));
                    assert!(!hybrid::draft_path(&img, format).exists());
                    let archived = root.join("Warn").join(relative);
                    assert_eq!(archived.exists(), short_reply, "{format}, {single}, {recursive}");
                    if short_reply {
                        assert_eq!(std::fs::read(&archived).unwrap(), original_image);
                        assert_eq!(std::fs::read(archived.with_extension(format)).unwrap(), std::fs::read(img.with_extension(format)).unwrap());
                    }
                    assert!(!root.join("Fail").join(relative).exists());
                    let files = collect_image_files_with_recursive_excluding(&input, recursive, Some(&root)).unwrap();
                    assert_eq!(files, [img], "archive copies must not be scanned again");
                    let events = events.lock().unwrap();
                    let warnings: Vec<_> = events.iter().filter(|e| e["status"] == "warning").collect();
                    assert_eq!(warnings.len(), usize::from(short_reply));
                    if short_reply {
                        let message = warnings[0]["message"].as_str().unwrap();
                        assert!(message.contains(" | 疑似截断"), "{message}");
                        assert!(!message.contains('⚠') && !message.contains("回复仅"), "{message}");
                        if nl_only || caption_mode {
                            assert_eq!(message.matches("29 字/词").count(), 1, "{message}");
                        }
                    }
                    assert_eq!(events.iter().filter(|e| e["message"] == "已将 1 个警告文件复制到 Warn/ 文件夹").count(), usize::from(short_reply));
                    assert!(events.iter().all(|e| !e["message"].as_str().unwrap().contains("已跳过 Warn/")));
                    assert_eq!(events.iter().filter(|e| e["status"] == "done").count(), 1);
                }
            }
        }
    }

    #[tokio::test]
    async fn hybrid_nl_only_missing_description_is_not_success_and_preserves_original() {
        for source in [FIXTURE_JSON, r#"{"count":"1girl","tags":["smile"],"nl":"old"}"#] {
            for (reply, reason) in [
                (Some("TAGS: 1girl, smile"), "stop"),
                (Some("NL:"), "stop"),
                (Some("NL: I'm sorry, I cannot describe this image."), "stop"),
                (Some("NL: A girl"), "length"),
                (None, "stop"),
            ] {
                let (root, img) = setup_dir("hybrid_nl_missing");
                std::fs::write(img.with_extension("json"), source).unwrap();
                hybrid::prepare_sources(&root, false, "json").unwrap();
                let server = serve_chat_reply(reply, reason);
                let result = process_single_file(
                    &client(),
                    &img,
                    &root,
                    &root,
                    &nl_only_options(server.url.clone()),
                    &RequestThrottle::new(-1),
                )
                .await;
                assert!(
                    matches!(result, FileResult::Warning { .. }),
                    "{reply:?}: {result:?}"
                );
                assert_eq!(
                    std::fs::read_to_string(img.with_extension("json")).unwrap(),
                    source
                );
                assert!(!hybrid::draft_path(&img, "json").exists());
            }
        }
    }

    #[tokio::test]
    async fn hybrid_unusable_replies_go_to_warn_and_api_errors_go_to_fail() {
        use crate::commands::batch::capture_events;
        use crate::commands::llm_client::test_support::serve_status;
        let mut evidence = Vec::new();
        for (format, source, caption, nl_only) in [
            ("txt", "1girl, smile", false, false),
            ("txt", "1girl, smile", true, false),
            ("json", FIXTURE_JSON, false, false),
            ("json", r#"{"tags":["smile"],"nl":"old"}"#, false, false),
            ("json", FIXTURE_JSON, false, true),
            ("json", r#"{"tags":["smile"],"nl":"old"}"#, false, true),
        ] {
            for (case, text, reason) in [
                ("回空", None, "stop"),
                ("截断", Some("NL: A partial description"), "length"),
                ("截断回空", None, "length"),
                ("安全拦截", None, "content_filter"),
                ("拒绝回复", Some("I'm sorry, I cannot describe this image."), "stop"),
                ("请求失败", None, "http"),
                ("疑似截断", Some("NL: A girl smiling."), "stop"),
            ] {
                let (root, img) = setup_dir("hybrid_reply_classification");
                std::fs::write(img.with_extension(format), source).unwrap();
                hybrid::prepare_sources(&root, false, format).unwrap();
                let server = if reason == "http" {
                    serve_status("503 Service Unavailable", "text/plain", b"unavailable".to_vec())
                } else { serve_chat_reply(text, reason) };
                let mut options = make_options(server.url.clone());
                options.input_path = img.to_string_lossy().into_owned();
                options.output_path = options.input_path.clone();
                options.hybrid_mode = true;
                options.file_format = format.into();
                options.caption_mode = caption;
                options.nl_only = nl_only;
                let app = tauri::test::mock_app();
                let events = capture_events(app.handle(), EVENT);
                let result = refine_dataset(app.handle(), options, client()).await.unwrap();
                let warning = reason != "http";
                let saved = case == "疑似截断";
                assert_eq!((result.success_count, result.fail_count, result.warning_count, result.total),
                    (u32::from(saved), u32::from(!warning), u32::from(warning), 1), "{case}/{format}/{nl_only}/{caption}");
                assert!(!hybrid::draft_path(&img, format).exists());
                let written = std::fs::read_to_string(img.with_extension(format)).unwrap();
                if !saved { assert_eq!(written, source); }
                else if nl_only || caption { assert!(written.contains("A girl smiling.")); }
                let folder = if warning { "Warn" } else { "Fail" };
                assert_eq!(std::fs::read(root.join(folder).join("a.png")).unwrap(), std::fs::read(&img).unwrap());
                assert_eq!(std::fs::read(root.join(folder).join(format!("a.{format}"))).unwrap(), written.as_bytes());
                assert!(!root.join(if warning { "Fail" } else { "Warn" }).exists());
                let events = events.lock().unwrap();
                let items: Vec<_> = events.iter().filter(|e| e["filename"] == "a.png").collect();
                assert_eq!(items.len(), 1);
                assert_eq!(items[0]["status"], if warning { "warning" } else { "error" });
                assert_eq!(items[0]["unwritten"] == true, warning && !saved);
                assert_eq!(events.iter().filter(|e| e["status"] == "done").count(), 1);
                assert_eq!(events.iter().filter(|e| e["message"].as_str().unwrap().contains("复制到")).count(), 1);
                if nl_only && source != FIXTURE_JSON {
                    evidence.push(serde_json::json!({"case": case, "events": *events, "result": result, "archive": folder}));
                }
            }
        }
        if let Ok(path) = std::env::var("PURINBOX_TEST_HYBRID_LOG_EVIDENCE") {
            std::fs::write(path, serde_json::to_vec_pretty(&evidence).unwrap()).unwrap();
        }
    }

    #[test]
    fn old_options_keep_tag_refining_as_default() {
        let mut value = serde_json::to_value(make_options("unused".into())).unwrap();
        value.as_object_mut().unwrap().remove("nl_only");
        value.as_object_mut().unwrap().remove("short_reply_threshold");
        let options: TagRefineOptions = serde_json::from_value(value).unwrap();
        assert!(!options.nl_only);
        assert_eq!(options.short_reply_threshold, 100);
    }

    #[tokio::test]
    async fn hybrid_nl_refusals_keep_detection_logs_and_warning_archive() {
        use crate::commands::batch::capture_events;
        for source in [FIXTURE_JSON, r#"{"count":"1girl","tags":["smile"],"nl":"old"}"#] {
            for (reply, reason, expected) in [
                (
                    Some("I'm sorry, I cannot describe this image."),
                    "stop",
                    "疑似内容安全审核",
                ),
                (
                    Some("NL: I'm sorry, I cannot describe this image."),
                    "stop",
                    "疑似内容安全审核",
                ),
                (
                    Some("抱歉，我无法处理这张图片。"),
                    "stop",
                    "疑似内容安全审核",
                ),
                (
                    Some("NL: 抱歉，我无法处理这张图片。"),
                    "stop",
                    "疑似内容安全审核",
                ),
                (None, "content_filter", "LLM 内容安全审核拒绝了该图片"),
                (None, "safety", "LLM 内容安全审核拒绝了该图片"),
            ] {
                let (root, img) = setup_dir("hybrid_nl_refusal");
                std::fs::write(img.with_extension("json"), source).unwrap();
                hybrid::prepare_sources(&root, false, "json").unwrap();
                let server = serve_chat_reply(reply, reason);
                let mut options = nl_only_options(server.url.clone());
                options.input_path = root.to_string_lossy().into_owned();
                options.output_path = options.input_path.clone();
                let app = tauri::test::mock_app();
                let events = capture_events(app.handle(), EVENT);
                let result = refine_dataset(app.handle(), options, client())
                    .await
                    .unwrap();
                assert_eq!(
                    (result.success_count, result.fail_count, result.total),
                    (0, 0, 1)
                );
                assert_eq!(result.warning_count, 1);
                assert!(result.errors.is_empty());
                assert_eq!(
                    std::fs::read_to_string(img.with_extension("json")).unwrap(),
                    source
                );
                assert_eq!(
                    std::fs::read(root.join("Warn/a.png")).unwrap(),
                    std::fs::read(&img).unwrap()
                );
                assert!(!hybrid::draft_path(&img, "json").exists());
                let events = events.lock().unwrap();
                let errors: Vec<&str> = events
                    .iter()
                    .filter(|event| event["status"] == "warning")
                    .map(|event| event["message"].as_str().unwrap())
                    .collect();
                assert_eq!(errors.len(), 1, "{events:?}");
                assert!(errors[0].contains(expected), "{errors:?}");
                assert!(!errors[0].contains("未返回有效的自然语言描述"));
                if let Some(text) = reply {
                    assert!(errors[0].contains(text.strip_prefix("NL: ").unwrap_or(text)));
                }
                assert!(events.iter().all(|event| event["status"] != "success"));
                assert!(events.iter().all(|event| event["status"] != "error"));
                assert!(!root.join("Fail").exists());
                assert!(events.iter().any(|e| e["unwritten"] == true && e["current"] == 1));
                assert_eq!(
                    events.last().unwrap()["message"],
                    "自然语言描述补充完成: 成功 0, 未写入 1（警告）, 失败 0, 共 1"
                );
            }
        }
    }

    #[tokio::test]
    async fn nl_only_keeps_explicit_trigger_and_existing_label_skip() {
        let (root, img) = setup_dir("hybrid_nl_trigger");
        std::fs::write(img.with_extension("json"), FIXTURE_JSON).unwrap();
        hybrid::prepare_sources(&root, false, "json").unwrap();
        let server = serve_chat_reply(Some("NL: A girl smiles."), "stop");
        let mut options = nl_only_options(server.url.clone());
        options.trigger_word = "test_trigger".into();
        let result = process_single_file(
            &client(),
            &img,
            &root,
            &root,
            &options,
            &RequestThrottle::new(-1),
        )
        .await;
        assert!(matches!(result, FileResult::DescriptionAdded { .. }));
        let written = std::fs::read_to_string(img.with_extension("json")).unwrap();
        let mut expected: serde_json::Value = serde_json::from_str(FIXTURE_JSON).unwrap();
        set_json_nl(&mut expected, "A girl smiles.");
        set_json_trigger(&mut expected, "test_trigger");
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&written).unwrap(),
            expected
        );

        options.skip_existing_labels = true;
        options.api_endpoint = "http://127.0.0.1:1".into();
        let result = process_single_file(
            &client(),
            &img,
            &root,
            &root,
            &options,
            &RequestThrottle::new(-1),
        )
        .await;
        assert!(matches!(result, FileResult::Skipped { .. }));
        assert_eq!(
            std::fs::read_to_string(img.with_extension("json")).unwrap(),
            written
        );
    }

    #[tokio::test]
    async fn hybrid_intermediate_labels_reach_vlm_and_only_success_is_published() {
        use crate::commands::tagger::{has_labels, hybrid::draft_path};
        for (format, local) in [
            ("txt", "1girl, smile"),
            ("json", FIXTURE_JSON),
            (
                "json",
                r#"{"count":"1girl","appearance":[],"tags":["smile"],"environment":[],"nl":""}"#,
            ),
        ] {
            let (root, img) = setup_dir("hybrid_refine_success");
            let source = draft_path(&img, format);
            std::fs::write(&source, local).unwrap();
            assert!(!has_labels(&img));
            let server = serve_chat_reply(Some("COUNT: 1girl\nAPPEARANCE: long hair\nTAGS: smile\nENVIRONMENT:\nNL: A smiling girl."), "stop");
            let mut options = make_options(server.url.clone());
            options.input_path = root.to_string_lossy().into_owned();
            options.file_format = format.into();
            options.hybrid_mode = true;
            options.skip_existing_labels = true;
            let result = process_single_file(
                &client(),
                &img,
                &root,
                &root,
                &options,
                &RequestThrottle::new(-1),
            )
            .await;
            assert!(matches!(result, FileResult::Success { .. }), "{result:?}");
            let written = std::fs::read_to_string(img.with_extension(format)).unwrap();
            assert!(written.contains("smile"));
            assert!(!source.exists());
            assert!(has_labels(&img));
            let request = server
                .requests
                .recv_timeout(std::time::Duration::from_secs(5))
                .unwrap();
            assert!(request.json()["messages"][0]["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains("smile"));
            if format == "json" {
                let value: serde_json::Value = serde_json::from_str(&written).unwrap();
                assert_eq!(
                    value.get("ai_output").is_some(),
                    local.contains("ai_output")
                );
            }
            options.api_endpoint = "http://127.0.0.1:1".into();
            let retry = process_single_file(
                &client(),
                &img,
                &root,
                &root,
                &options,
                &RequestThrottle::new(-1),
            )
            .await;
            assert!(matches!(retry, FileResult::Skipped { .. }), "{retry:?}");
            assert_eq!(
                std::fs::read_to_string(img.with_extension(format)).unwrap(),
                written
            );
        }
    }

    /// 调优失败：不写正式标签，读过的草稿照样删除；下次运行重新准备草稿后可以再调优
    #[tokio::test]
    async fn hybrid_failed_refinement_removes_draft_and_writes_no_label() {
        use crate::commands::tagger::{has_labels, hybrid::draft_path};
        for (format, local) in [
            ("txt", "1girl, smile"),
            ("json", FIXTURE_JSON),
            ("json", r#"{"count":"1girl","tags":["smile"]}"#),
        ] {
            let (root, img) = setup_dir("hybrid_refine_failed");
            let source = draft_path(&img, format);
            std::fs::write(&source, local).unwrap();
            let server = serve_chat_reply(None, "stop");
            let mut options = make_options(server.url.clone());
            options.input_path = root.to_string_lossy().into_owned();
            options.file_format = format.into();
            options.hybrid_mode = true;
            options.skip_existing_labels = true;
            let result = process_single_file(
                &client(),
                &img,
                &root,
                &root,
                &options,
                &RequestThrottle::new(-1),
            )
            .await;
            assert!(matches!(result, FileResult::Warning { .. }), "{result:?}");
            assert!(!has_labels(&img));
            assert!(!source.exists(), "失败时草稿也要删除");

            std::fs::write(&source, local).unwrap();
            let retry_server = serve_chat_reply(Some("COUNT: 1girl\nTAGS: smile"), "stop");
            options.api_endpoint = retry_server.url.clone();
            let retry = process_single_file(
                &client(),
                &img,
                &root,
                &root,
                &options,
                &RequestThrottle::new(-1),
            )
            .await;
            assert!(matches!(retry, FileResult::Success { .. }), "{retry:?}");
            assert!(has_labels(&img));
            assert!(!source.exists());
        }
    }

    #[tokio::test]
    async fn hybrid_does_not_call_vlm_for_late_labels_or_missing_local_output() {
        use crate::commands::tagger::hybrid::draft_path;
        for format in ["txt", "json"] {
            for existing in ["txt", "json"] {
                let (root, img) = setup_dir("hybrid_late_labels");
                let mut options = make_options("http://127.0.0.1:1".into());
                options.input_path = root.to_string_lossy().into_owned();
                options.hybrid_mode = true;
                options.skip_existing_labels = true;
                options.file_format = format.into();
                let missing = process_single_file(
                    &client(),
                    &img,
                    &root,
                    &root,
                    &options,
                    &RequestThrottle::new(-1),
                )
                .await;
                assert!(matches!(missing, FileResult::Skipped { .. }), "{missing:?}");
                std::fs::write(draft_path(&img, format), "local tags").unwrap();
                std::fs::write(img.with_extension(existing), "external").unwrap();
                let result = process_single_file(
                    &client(),
                    &img,
                    &root,
                    &root,
                    &options,
                    &RequestThrottle::new(-1),
                )
                .await;
                assert!(matches!(result, FileResult::Skipped { .. }), "{result:?}");
                assert_eq!(
                    std::fs::read_to_string(img.with_extension(existing)).unwrap(),
                    "external"
                );
                assert!(!draft_path(&img, format).exists());
            }
        }
    }

    #[tokio::test]
    async fn hybrid_reuse_and_regeneration_follow_local_option() {
        use crate::commands::tagger::hybrid::draft_path;
        for (format, old, draft) in [
            ("txt", "1girl, old tag", "1girl, fresh tag"),
            (
                "json",
                r#"{"ai_output":{"count":"1girl","tags":["old tag"]}}"#,
                r#"{"ai_output":{"count":"1girl","tags":["fresh tag"]}}"#,
            ),
            (
                "json",
                r#"{"count":"1girl","tags":["old tag"]}"#,
                r#"{"count":"1girl","tags":["fresh tag"]}"#,
            ),
        ] {
            // 前端只在复用已有标签时才可能要求跳过（skip_existing_labels = 复用 && 跳过），
            // 这里覆盖不跳过时的复用与重新生成
            for prefer_existing in [true, false] {
                let (root, img) = setup_dir("hybrid_compat");
                std::fs::write(img.with_extension(format), old).unwrap();
                let source = draft_path(&img, format);
                if prefer_existing {
                    let _lock = crate::commands::tagger::TAGGER_TEST_LOCK.lock().unwrap();
                    crate::commands::tagger::inference::reset_tagging_cancel();
                    let prepared =
                        crate::commands::tagger::hybrid::prepare_sources(&root, false, format)
                            .unwrap();
                    assert_eq!(prepared.copied, 1);
                    assert_eq!(
                        std::fs::read_to_string(img.with_extension(format)).unwrap(),
                        old
                    );
                } else {
                    std::fs::write(&source, draft).unwrap();
                }
                let server = serve_chat_reply(Some("COUNT: 1girl\nTAGS: refined tag"), "stop");
                let mut options = make_options(server.url.clone());
                options.hybrid_mode = true;
                options.file_format = format.into();
                let result = process_single_file(
                    &client(),
                    &img,
                    &root,
                    &root,
                    &options,
                    &RequestThrottle::new(-1),
                )
                .await;
                assert!(matches!(result, FileResult::Success { .. }), "{result:?}");
                let request = server
                    .requests
                    .recv_timeout(std::time::Duration::from_secs(5))
                    .unwrap()
                    .json();
                let text = request["messages"][0]["content"][0]["text"]
                    .as_str()
                    .unwrap();
                assert!(
                    text.contains(if prefer_existing {
                        "old tag"
                    } else {
                        "fresh tag"
                    }),
                    "{text}"
                );
                assert!(
                    !text.contains(if prefer_existing {
                        "fresh tag"
                    } else {
                        "old tag"
                    }),
                    "{text}"
                );
                assert!(std::fs::read_to_string(img.with_extension(format))
                    .unwrap()
                    .contains("refined tag"));
                assert!(!source.exists());
            }
        }
    }

    #[tokio::test]
    async fn hybrid_json_source_can_be_reused_for_txt_output() {
        use crate::commands::tagger::hybrid::{draft_path, prepare_sources};
        for original in [FIXTURE_JSON, r#"{"count":"1girl","tags":["smile"]}"#] {
            let (root, img) = setup_dir("hybrid_json_to_txt");
            std::fs::write(img.with_extension("json"), original).unwrap();
            {
                let _lock = crate::commands::tagger::TAGGER_TEST_LOCK.lock().unwrap();
                crate::commands::tagger::inference::reset_tagging_cancel();
                assert_eq!(prepare_sources(&root, false, "txt").unwrap().copied, 1);
            }
            let server = serve_chat_reply(Some("COUNT: 1girl\nTAGS: smile"), "stop");
            let mut options = make_options(server.url.clone());
            options.hybrid_mode = true;
            let result = process_single_file(
                &client(),
                &img,
                &root,
                &root,
                &options,
                &RequestThrottle::new(-1),
            )
            .await;
            assert!(matches!(result, FileResult::Success { .. }), "{result:?}");
            assert!(std::fs::read_to_string(img.with_extension("txt"))
                .unwrap()
                .contains("smile"));
            assert_eq!(
                std::fs::read_to_string(img.with_extension("json")).unwrap(),
                original
            );
            assert!(!draft_path(&img, "json").exists());
            // JSON 草稿按字段结构读取，带字段名发给 VLM
            let request = server
                .requests
                .recv_timeout(std::time::Duration::from_secs(5))
                .unwrap()
                .json();
            let text = request["messages"][0]["content"][0]["text"]
                .as_str()
                .unwrap();
            assert!(text.contains("count: 1girl"), "{text}");
        }
    }

    const EMPTY_FULL: &str = r#"{"fixed":{"quality":"","series":"","artist":""},"character":{"name":"","variant":""},"from_path":{"appearance":[]},"ai_output":{"count":"","appearance":[],"tags":[],"environment":[],"nl":""}}"#;
    const EMPTY_SIMPLE: &str = r#"{"quality":"","series":"","artist":"","character":"","count":"","appearance":[],"tags":[],"environment":[],"nl":""}"#;

    /// 辅助打标里本地标签为空按出错处理：不请求 VLM，不写标签文件，已有的标签原样保留，草稿删除
    #[tokio::test]
    async fn hybrid_empty_local_labels_fail_without_requesting_vlm() {
        use crate::commands::tagger::hybrid::draft_path;
        // (输出格式, 草稿格式, 草稿内容, 自然语言模式, 触发词, 已有标签)
        let cases = [
            ("txt", "txt", "", false, "", None),
            ("txt", "txt", " , ，\n", false, "trigger", None),
            (
                "txt",
                "json",
                r#"{"ai_output":{"nl":"x"}}"#,
                false,
                "",
                None,
            ),
            ("txt", "txt", "", true, "trigger", None),
            ("txt", "txt", "", true, "", Some("a hand-written caption")),
            ("txt", "txt", "", false, "", Some("1girl, solo")),
            ("json", "json", "", false, "", None),
            ("json", "json", EMPTY_FULL, false, "", None),
            ("json", "json", EMPTY_SIMPLE, false, "trigger", None),
            (
                "json",
                "json",
                EMPTY_FULL,
                false,
                "",
                Some(r#"{"ai_output":{"nl":"keep"}}"#),
            ),
        ];
        for (format, draft_format, draft, caption, trigger, existing) in cases {
            let (root, img) = setup_dir("hybrid_empty_local");
            let draft_file = draft_path(&img, draft_format);
            std::fs::write(&draft_file, draft).unwrap();
            let label = img.with_extension(format);
            if let Some(existing) = existing {
                std::fs::write(&label, existing).unwrap();
            }
            // 端口 1 没人应答：一旦请求 VLM，报的就是连接错误
            let mut options = make_options("http://127.0.0.1:1".into());
            options.hybrid_mode = true;
            options.file_format = format.into();
            options.caption_mode = caption;
            options.trigger_word = trigger.into();
            let result = process_single_file(
                &client(),
                &img,
                &root,
                &root,
                &options,
                &RequestThrottle::new(-1),
            )
            .await;
            let case = format!("{format} {draft:?} caption={caption} existing={existing:?}");
            match result {
                FileResult::Error { message, .. } => {
                    assert_eq!(message, EMPTY_LOCAL_LABELS, "{case}")
                }
                other => panic!("{case}: {other:?}"),
            }
            match existing {
                Some(existing) => {
                    assert_eq!(std::fs::read_to_string(&label).unwrap(), existing, "{case}")
                }
                None => assert!(!label.exists(), "{case}"),
            }
            assert!(!draft_file.exists(), "{case}");
        }
    }

    /// 只补缺（跳过已有标签）时，调优期间出现的标签文件让这张图照常跳过，不报空标签错误、不写文件
    #[tokio::test]
    async fn hybrid_skip_existing_wins_over_empty_local_labels() {
        use crate::commands::tagger::hybrid::draft_path;
        let (root, img) = setup_dir("hybrid_empty_late");
        std::fs::write(draft_path(&img, "txt"), "").unwrap();
        std::fs::write(img.with_extension("json"), "external").unwrap();
        let mut options = make_options("http://127.0.0.1:1".into());
        options.hybrid_mode = true;
        options.skip_existing_labels = true;
        let result = process_single_file(
            &client(),
            &img,
            &root,
            &root,
            &options,
            &RequestThrottle::new(-1),
        )
        .await;
        assert!(matches!(result, FileResult::Skipped { .. }), "{result:?}");
        assert!(!img.with_extension("txt").exists());
        assert!(!draft_path(&img, "txt").exists());
    }

    /// 标签细化页（非辅助打标）遇到空标签文件仍是跳过，不写任何东西
    #[tokio::test]
    async fn empty_label_files_are_skipped_outside_hybrid_mode() {
        for (content, reason) in [("", "标签文件为空"), (" , ", "无有效标签")] {
            let (root, img) = setup_dir("refine_empty_txt");
            std::fs::write(root.join("a.txt"), content).unwrap();
            let options = make_options("http://127.0.0.1:1".into());
            let result = process_single_file(
                &client(),
                &img,
                &root,
                &root,
                &options,
                &RequestThrottle::new(-1),
            )
            .await;
            match result {
                FileResult::Skipped { reason: got, .. } => assert_eq!(got, reason),
                other => panic!("{other:?}"),
            }
            assert_eq!(
                std::fs::read_to_string(root.join("a.txt")).unwrap(),
                content
            );
        }
    }

    /// 整批就地调优：同一轮运行 ID、逐条事件、收尾说明和 done；本批的草稿全部清掉
    #[tokio::test]
    async fn hybrid_batch_reports_one_run_and_clears_drafts() {
        use crate::commands::batch::capture_raw_events;
        use crate::commands::tagger::hybrid::draft_path;
        let root = TempDir::new("hybrid_refine_batch");
        for name in ["a.png", "b.png"] {
            image::RgbImage::from_pixel(8, 8, image::Rgb([10, 20, 30]))
                .save(root.join(name))
                .unwrap();
        }
        std::fs::write(draft_path(&root.join("a.png"), "txt"), "").unwrap();
        let app = tauri::test::mock_app();
        let events = capture_raw_events(app.handle(), EVENT);
        let mut options = make_options("http://127.0.0.1:1".into());
        options.input_path = root.to_string_lossy().into_owned();
        options.output_path = options.input_path.clone();
        options.hybrid_mode = true;
        let before = crate::commands::begin_run("tag-refine-test-marker");
        let result = refine_dataset(app.handle(), options, client())
            .await
            .unwrap();

        assert_eq!(
            (result.success_count, result.fail_count, result.total),
            (1, 1, 2)
        );
        assert!(!root.join("a.txt").exists());
        assert!(root.join("Fail/a.png").is_file());
        assert!(!root.join("b.txt").exists());
        assert!(!draft_path(&root.join("a.png"), "txt").exists());
        let events = events.lock().unwrap();
        let run_id = events[0]["run_id"].as_u64().unwrap();
        assert!(run_id > before);
        assert!(events.iter().all(|e| e["run_id"] == run_id), "{events:?}");
        let summary: Vec<(&str, &str)> = events
            .iter()
            .map(|e| {
                (
                    e["status"].as_str().unwrap(),
                    e["message"].as_str().unwrap(),
                )
            })
            .collect();
        assert_eq!(
            summary,
            [
                ("info", "找到 2 张图片，1 线程开始辅助打标（VLM 调优）..."),
                ("error", "[错误] a.png: 本地标签为空，已跳过 VLM 打标"),
                ("success", "[跳过] b.png (无可用的本地标签)"),
                ("info", "已将 1 个失败文件复制到 Fail/ 文件夹"),
                ("done", "VLM 调优完成: 成功 1, 失败 1, 共 2"),
            ]
        );
    }

    /// 收尾清理：删掉本批所有草稿，删不掉的汇总成一条警告
    #[test]
    fn batch_draft_cleanup_reports_leftovers() {
        use crate::commands::batch::capture_events;
        use crate::commands::tagger::hybrid::draft_path;
        let root = TempDir::new("hybrid_draft_sweep");
        let (a, b) = (root.join("a.png"), root.join("b.png"));
        std::fs::write(draft_path(&a, "txt"), "x").unwrap();
        std::fs::write(draft_path(&a, "json"), "{}").unwrap();
        // 草稿位置是个目录：删除必然失败
        std::fs::create_dir(draft_path(&b, "txt")).unwrap();
        let app = tauri::test::mock_app();
        let events = capture_events(app.handle(), EVENT);
        clear_batch_drafts(app.handle(), &[a.clone(), b.clone(), root.join("c.png")]);
        assert!(!draft_path(&a, "txt").exists() && !draft_path(&a, "json").exists());
        let events = events.lock().unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0]["status"], "warning");
        let message = events[0]["message"].as_str().unwrap();
        assert!(
            message.starts_with("清理中间标签失败") && message.ends_with("（共 1 张图片）"),
            "{message}"
        );
    }

    #[tokio::test]
    async fn txt_mode_json_fallback_end_to_end() {
        let source: serde_json::Value = serde_json::from_str(FIXTURE_JSON).unwrap();
        let expected = flatten_json_tags(&source).join(", ");
        let server = serve_chat_reply(
            Some(&format!("TAGS: {}\nNL: txt 模式忽略", expected)),
            "stop",
        );
        let (root, img) = setup_dir("refine_fallback");
        std::fs::write(root.join("a.json"), FIXTURE_JSON).unwrap();
        let options = make_options(server.url.clone());
        let result = process_single_file(
            &client(),
            &img,
            &root,
            &root,
            &options,
            &RequestThrottle::new(-1),
        )
        .await;
        match result {
            FileResult::Success {
                warnings,
                original_count,
                refined_count,
                ..
            } => {
                assert!(warnings.is_empty(), "{warnings:?}");
                assert_eq!(original_count, refined_count);
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(
            std::fs::read_to_string(root.join("a.txt")).unwrap(),
            expected
        );
        assert_eq!(
            std::fs::read_to_string(root.join("a.json")).unwrap(),
            FIXTURE_JSON
        );
        let request = server
            .requests
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap();
        let body = request.json();
        let text = body["messages"][0]["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("appearance: long hair, blue eyes"));
        assert!(text.contains("count: 1girl"));
        assert!(text.contains("character.variant: winter outfit"));
        assert!(text.contains("from_path.appearance: scarf, boots"));
        assert!(body.get("max_tokens").is_none());
        assert!(body["messages"][0]["content"][1]["image_url"]
            .get("detail")
            .is_none());
        assert_eq!(request.path, "/v1/chat/completions");
    }

    #[tokio::test]
    async fn partial_json_response_preserves_count_and_does_not_warn_about_retained_tags() {
        let server = serve_chat_reply(
            Some("APPEARANCE: long hair, blue eyes\nTAGS: smile\nENVIRONMENT:\nNL: A girl."),
            "stop",
        );
        let (root, img) = setup_dir("refine_missing_count");
        std::fs::write(root.join("a.json"), FIXTURE_JSON).unwrap();
        let mut options = make_options(server.url.clone());
        options.file_format = "json".into();
        let result = process_single_file(
            &client(),
            &img,
            &root,
            &root,
            &options,
            &RequestThrottle::new(-1),
        )
        .await;
        match result {
            FileResult::Success {
                warnings,
                original_count,
                refined_count,
                changed,
                ..
            } => {
                assert!(warnings.is_empty(), "{warnings:?}");
                assert_eq!(original_count, refined_count);
                assert!(changed);
            }
            other => panic!("{other:?}"),
        }
        let written: serde_json::Value =
            serde_json::from_slice(&std::fs::read(root.join("a.json")).unwrap()).unwrap();
        assert_eq!(written["ai_output"]["count"], "1girl");
        assert_eq!(written["character"]["variant"], "winter outfit");
        assert_eq!(written["from_path"]["appearance"], "scarf, boots");
        assert_eq!(written["ai_output"]["nl"], "A girl.");
    }

    #[tokio::test]
    async fn preserve_mode_tags_only_does_not_reassign_fields() {
        let server = serve_chat_reply(Some("TAGS: 1girl, long hair, smile, new tag"), "stop");
        let (root, img) = setup_dir("refine_preserve_legacy");
        std::fs::write(root.join("a.json"), FIXTURE_JSON).unwrap();
        let mut options = make_options(server.url.clone());
        options.file_format = "json".into();
        options.preserve_tags = true;
        let result = process_single_file(
            &client(),
            &img,
            &root,
            &root,
            &options,
            &RequestThrottle::new(-1),
        )
        .await;
        match result {
            FileResult::Success {
                warnings, changed, ..
            } => {
                assert!(!changed);
                assert!(warnings.is_empty());
            }
            other => panic!("{other:?}"),
        }
        let written: serde_json::Value =
            serde_json::from_slice(&std::fs::read(root.join("a.json")).unwrap()).unwrap();
        assert_eq!(
            written,
            serde_json::from_str::<serde_json::Value>(FIXTURE_JSON).unwrap()
        );
    }

    #[tokio::test]
    async fn rejected_truncated_and_empty_replies_never_overwrite_tags() {
        for (name, content, reason, expected) in [
            (
                "refusal",
                Some("NL: I'm sorry, I cannot describe this image."),
                "stop",
                "拒绝",
            ),
            ("truncated", Some("TAGS: smile"), "length", "输出长度上限"),
            ("safety", None, "content_filter", "内容安全审核"),
            ("empty", None, "stop", "空内容"),
        ] {
            let server = serve_chat_reply(content, reason);
            let (root, img) = setup_dir(name);
            std::fs::write(root.join("a.txt"), "1girl, smile").unwrap();
            let options = make_options(server.url.clone());
            let result = process_single_file(
                &client(),
                &img,
                &root,
                &root,
                &options,
                &RequestThrottle::new(-1),
            )
            .await;
            match result {
                FileResult::Warning { message, .. } => {
                    assert!(message.contains(expected), "{message}");
                    assert!(!message.contains("max_tokens"));
                }
                other => panic!("{other:?}"),
            }
            assert_eq!(
                std::fs::read_to_string(root.join("a.txt")).unwrap(),
                "1girl, smile"
            );
        }
    }

    #[tokio::test]
    async fn caption_reasoning_fallback_keeps_trigger_and_vision_options() {
        let server = serve_json(serde_json::json!({"choices": [{
            "message": {"content": "", "reasoning_content": "A girl smiles."},
            "finish_reason": "stop"
        }]}));
        let (root, img) = setup_dir("refine_caption");
        std::fs::write(root.join("a.txt"), "1girl, smile").unwrap();
        let mut options = make_options(server.url.clone());
        options.caption_mode = true;
        options.trigger_word = "trigger".into();
        options.image_detail = " high ".into();
        options.top_p = 0.8;
        let result = process_single_file(
            &client(),
            &img,
            &root,
            &root,
            &options,
            &RequestThrottle::new(-1),
        )
        .await;
        assert!(matches!(result, FileResult::Captioned { .. }));
        assert_eq!(
            std::fs::read_to_string(root.join("a.txt")).unwrap(),
            "trigger, A girl smiles."
        );
        let request = server
            .requests
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap();
        assert_eq!(request.json()["top_p"], 0.8);
        assert_eq!(
            request.json()["messages"][0]["content"][1]["image_url"]["detail"],
            "high"
        );
    }

    #[test]
    fn old_max_tokens_is_ignored_and_not_serialized() {
        let mut old = serde_json::to_value(make_options("local".into())).unwrap();
        old["max_tokens"] = serde_json::json!(-1);
        let parsed: TagRefineOptions = serde_json::from_value(old).unwrap();
        assert!(serde_json::to_value(parsed)
            .unwrap()
            .get("max_tokens")
            .is_none());
    }
}
