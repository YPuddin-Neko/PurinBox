use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use super::ProgressEvent;
use crate::commands::llm_batch::{self, ItemOutcome, LlmBatchResult};
use crate::commands::llm_client::{self, ChatError, ChatMessage, ChatParams, RequestThrottle};
use crate::commands::{collect_image_files_with_recursive, file_name_lossy, ProblemArchive};

const EVENT: &str = "llm-tagger-progress";

pub(super) static LLM_CANCELLED: AtomicBool = AtomicBool::new(false);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmTaggerOptions {
    #[serde(default)]
    pub refine_existing: bool,
    #[serde(default)]
    pub gbc: Option<super::gbc::GbcSettings>,
    pub input_path: String,
    pub api_endpoint: String,
    pub api_key: String,
    pub model_name: String,
    pub system_prompt: String,
    pub user_prompt: String,
    pub temperature: f32,
    pub max_tokens: i32,
    /// 发送图片的最大边长（默认 1024，超出会等比缩放）
    #[serde(default = "default_image_size")]
    pub image_size: u32,
    /// Top P 采样参数（0~1，为 0 或负数时不发送）
    #[serde(default)]
    pub top_p: f64,
    #[serde(default = "llm_client::default_short_reply_threshold")]
    pub short_reply_threshold: u32,
    /// 是否跳过已有 .txt/.json 描述文件的图片
    #[serde(default)]
    pub skip_existing: bool,
    /// 输出格式: "txt" 或 "json"
    #[serde(default = "default_llm_output_format")]
    pub output_format: String,
    #[serde(default)]
    pub json_simplified: bool,
    /// 请求间隔 (毫秒), -1 表示无间隔
    #[serde(default = "default_interval")]
    pub request_interval_ms: i64,
    /// 并发线程数
    #[serde(default = "default_concurrency")]
    pub concurrency: u32,
    /// 是否递归扫描子文件夹
    #[serde(default)]
    pub recursive: bool,
    /// OpenAI Vision 的 image_url.detail：low / high / original / auto。
    /// 空则整个字段不发送——不认识它的端点会因未知字段报错
    #[serde(default)]
    pub image_detail: String,
}

impl LlmTaggerOptions {
    pub(super) fn event(&self) -> &'static str {
        if self.refine_existing { "gbc-refine-progress" } else { EVENT }
    }
}

fn existing_input(path: &Path, format: &str) -> Result<(String, Option<serde_json::Value>, String), String> {
    if !path.with_extension(format).is_file() && path.with_extension("gbc.json").is_file() {
        let original = std::fs::read_to_string(path.with_extension("gbc.json")).map_err(|e| e.to_string())?;
        let value: serde_json::Value = serde_json::from_str(&original).map_err(|e| format!("GBC 数据无效：{}", e))?;
        if !crate::commands::tag_manager::is_structured_caption(&value) { return Err("GBC 数据中没有图结构".into()); }
        return Ok((format!("Describe this image using its GBC scene graph: {}", original), None, original));
    }
    let original = std::fs::read_to_string(path.with_extension(format)).map_err(|e| format!("无法读取已有标签：{}", e))?;
    if original.trim().is_empty() { return Err("已有标签为空".into()); }
    if format == "txt" { return Ok((original.clone(), None, original)); }
    let value: serde_json::Value = serde_json::from_str(&original).map_err(|e| format!("JSON 标签无效：{}", e))?;
    if !value.is_object() || crate::commands::tag_manager::is_structured_caption(&value) {
        return Err("请选择包含自然语言或标签的 JSON 文件".into());
    }
    let body = if value.get("ai_output").is_some() {
        value.get("ai_output").filter(|v| v.is_object()).ok_or("ai_output 必须是 JSON 对象")?
    } else { &value };
    let caption = body.get("nl").and_then(|v| v.as_str()).filter(|v| !v.trim().is_empty());
    let reference = if let Some(caption) = caption { caption.to_owned() }
        else if body.get("tags").is_some_and(|v| match v { serde_json::Value::Array(a) => !a.is_empty(), serde_json::Value::Object(o) => !o.is_empty(), serde_json::Value::String(s) => !s.trim().is_empty(), _ => false }) {
            format!("Existing image tags: {}", body["tags"])
        } else { return Err("JSON 中没有可细化的自然语言或标签".into()); };
    Ok((reference, Some(value), original))
}

fn existing_format(image: &Path) -> Option<&'static str> {
    if image.with_extension("json").is_file() { Some("json") }
    else if image.with_extension("txt").is_file() || image.with_extension("gbc.json").is_file() { Some("txt") }
    else { None }
}

fn existing_files(options: &LlmTaggerOptions) -> Result<Vec<PathBuf>, String> {
    let input = PathBuf::from(&options.input_path);
    let files: Vec<_> = collect_image_files_with_recursive(&input, options.recursive)?.into_iter().filter(|p| existing_format(p).is_some()).collect();
    let mut targets = HashSet::new();
    for path in &files {
        if !targets.insert(path.with_extension(existing_format(path).unwrap())) { return Err("存在共用同一标签文件的重名图片，请先区分文件名".into()); }
    }
    if files.is_empty() { return Err("未找到带有同名 TXT、JSON 标签或 GBC 数据的图片".into()); }
    Ok(files)
}

fn default_image_size() -> u32 {
    1024
}
fn default_llm_output_format() -> String {
    "txt".into()
}
fn default_interval() -> i64 {
    -1
}
fn default_concurrency() -> u32 {
    1
}

#[tauri::command]
pub fn cancel_llm_tagging() {
    LLM_CANCELLED.store(true, Ordering::SeqCst);
    super::gbc::cancel();
}

#[tauri::command]
pub async fn start_llm_tagging(
    app: tauri::AppHandle,
    options: LlmTaggerOptions,
) -> Result<LlmBatchResult, String> {
    static LLM_RUNNING: AtomicBool = AtomicBool::new(false);
    let _busy = crate::commands::BusyGuard::acquire(&LLM_RUNNING, "LLM 打标")?;
    LLM_CANCELLED.store(false, Ordering::SeqCst);
    crate::commands::python_env::clear_pending_cancel(options.event());
    if options.refine_existing && options.gbc.as_ref().is_none_or(|g| g.content_type != "caption") {
        return Err("GBC 细化需要自然语言分析配置".into());
    }
    crate::commands::begin_run(options.event());
    if options.gbc.as_ref().is_some_and(|g| g.content_type != "caption") {
        return tauri::async_runtime::spawn_blocking(move || super::gbc::run(app, options)).await.map_err(|e| e.to_string())?;
    }
    let client = llm_client::llm_http_client()?;
    run_llm_tagging(&app, options, client).await
}

async fn run_llm_tagging<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    options: LlmTaggerOptions,
    client: reqwest::Client,
) -> Result<LlmBatchResult, String> {
    let event = options.event();
    let input = PathBuf::from(&options.input_path);
    let input_dir = if input.is_file() { input.parent().unwrap().to_path_buf() } else { input.clone() };
    let files = if options.refine_existing { existing_files(&options)? } else { collect_image_files_with_recursive(&input, options.recursive)? };
    let total = files.len() as u32;
    ProgressEvent::new("info", format!("读取到 {} 张图片", total))
        .at(0, total)
        .emit(app, event);

    // 开始前一次定下要跳过的图片：本轮写出的标签不影响其他图片的判断
    let skipped: Arc<HashSet<PathBuf>> = Arc::new(if options.skip_existing && !options.refine_existing {
        files
            .iter()
            .filter(|path| super::has_labels(path))
            .cloned()
            .collect()
    } else {
        HashSet::new()
    });
    let analyzer = if options.gbc.is_some() {
        let (app, config) = (app.clone(), options.clone());
        Some(Arc::new(std::sync::Mutex::new(tauri::async_runtime::spawn_blocking(move || super::gbc::Analyzer::start_llm(&app, &config)).await.map_err(|e| e.to_string())??)))
    } else { None };
    let recursive = options.recursive;
    let concurrency = if analyzer.is_some() { 1 } else { options.concurrency.max(1) as usize };
    let throttle = Arc::new(RequestThrottle::new(options.request_interval_ms));
    let completion = if options.refine_existing { "GBC 细化完成" } else { "LLM 打标完成" };
    let options = Arc::new(options);
    let outcome = llm_batch::run_file_batch(
        app,
        event,
        &files,
        concurrency,
        &LLM_CANCELLED,
        move |path| {
            let analyzer = analyzer.clone();
            let (client, options, throttle, skipped) = (
                client.clone(),
                options.clone(),
                throttle.clone(),
                skipped.clone(),
            );
            async move {
                if skipped.contains(&path) {
                    return ItemOutcome::Done {
                        message: format!("[跳过] {} (已有描述)", file_name_lossy(&path)),
                        warning: false,
                    };
                }
                if options.refine_existing {
                    let mut current = (*options).clone();
                    current.output_format = existing_format(&path).unwrap_or("txt").into();
                    tag_file(&client, &path, &current, &throttle, analyzer).await
                } else {
                    tag_file(&client, &path, &options, &throttle, analyzer).await
                }
            }
        },
    )
    .await;
    Ok(outcome.finish(
        app,
        event,
        completion,
        &ProblemArchive::new(&input_dir, &input_dir, recursive),
    ))
}

/// 一张图：请求 VLM、整理成标签文件写到图片旁
async fn tag_file(
    client: &reqwest::Client,
    path: &Path,
    options: &LlmTaggerOptions,
    throttle: &RequestThrottle,
    analyzer: Option<super::gbc::SharedAnalyzer>,
) -> ItemOutcome {
    let filename = file_name_lossy(path);
    let start = std::time::Instant::now();
    let written = async {
        let existing = if options.refine_existing { Some(existing_input(path, &options.output_format)?) } else { None };
        let text = if let Some((reference, _, _)) = &existing { reference.clone() } else { tag_with_llm(client, path, options, throttle).await? };
        let mut content = if let Some((_, _, original)) = &existing { original.clone() } else { format_output(&text, options)? };
        let mut warning_text = text.clone();
        if let Some(analyzer) = analyzer {
            let mut json = if options.output_format == "json" { Some(serde_json::from_str::<serde_json::Value>(&content).map_err(|e| e.to_string())?) } else { None };
            let simple = if options.refine_existing { json.as_ref().is_none_or(|v| v.get("ai_output").is_none()) } else { options.json_simplified };
            let caption = if options.refine_existing { text.as_str() } else { match &json {
                Some(value) => if options.json_simplified { value["nl"].as_str() } else { value["ai_output"]["nl"].as_str() }.unwrap_or(""),
                None => &content,
            } };
            let params = ChatParams { endpoint: &options.api_endpoint, api_key: &options.api_key, model: &options.model_name, temperature: options.temperature, max_tokens: options.max_tokens, top_p: options.top_p };
            let refined = super::gbc::refine_caption(analyzer, client, path, caption, &params, throttle, &LLM_CANCELLED, options.image_size, &options.image_detail).await?;
            warning_text = refined.clone();
            if let Some(value) = &mut json {
                if simple { value["nl"] = refined.into(); } else { value["ai_output"]["nl"] = refined.into(); }
                content = serde_json::to_string_pretty(value).map_err(|e| e.to_string())?;
            } else { content = refined; }
        }
        let warnings: Vec<String> =
            llm_client::short_reply_warning(llm_client::response_word_count(&warning_text), options.short_reply_threshold)
                .into_iter()
                .collect();
        // 回复到达前用户点了取消：不再写盘
        if LLM_CANCELLED.load(Ordering::SeqCst) {
            return Err(ChatError::Cancelled);
        }
        let output = path.with_extension(if options.output_format == "json" {
            "json"
        } else {
            "txt"
        });
        if let Some((_, _, original)) = existing {
            let source = if output.is_file() { output.clone() } else { path.with_extension("gbc.json") };
            if std::fs::read_to_string(&source).map_err(|e| e.to_string())? != original {
                return Err("标签文件在处理期间发生变化，未覆盖".into());
            }
        }
        crate::commands::config_paths::write_file_atomic(&output, content.as_bytes())
            .map(|()| warnings)
            .map_err(|e| ChatError::Other(format!("写入失败 {}", e)))
    }
    .await;
    let elapsed = llm_client::fmt_elapsed(start.elapsed().as_millis());
    match written {
        Ok(warnings) => ItemOutcome::completed(
            format!("[完成] {} ({})", filename, elapsed),
            &warnings,
            None,
        ),
        Err(error) => ItemOutcome::rejected(filename, error),
    }
}

/// VLM 回复 → 标签文件内容。txt 原样写；JSON 统一规范成恒定骨架（`tag_manager::normalize_tag_json`）：
/// 回复是 JSON 对象（可带 ``` 围栏）时补齐缺的段、画师逐位补 `@`、保留额外字段；
/// 不是 JSON 对象（自然语言、标签串、JSON 数组等）时原文整段放进 nl
fn format_output(text: &str, options: &LlmTaggerOptions) -> Result<String, String> {
    if options.output_format != "json" {
        return Ok(text.to_string());
    }
    let trimmed = text.trim();
    let cleaned = trimmed
        .strip_prefix("```json")
        .or_else(|| trimmed.strip_prefix("```"))
        .map(|s| s.trim_end().trim_end_matches("```"))
        .unwrap_or(trimmed)
        .trim();
    let value = match serde_json::from_str::<serde_json::Value>(cleaned) {
        Ok(value @ serde_json::Value::Object(_)) => value,
        _ => serde_json::Value::String(text.to_string()),
    };
    let normalized =
        crate::commands::tag_manager::normalize_tag_json(value, options.json_simplified);
    serde_json::to_string_pretty(&normalized).map_err(|e| format!("序列化标签失败: {}", e))
}

async fn tag_with_llm(
    client: &reqwest::Client,
    img_path: &Path,
    options: &LlmTaggerOptions,
    throttle: &RequestThrottle,
) -> Result<String, ChatError> {
    let data_url = llm_client::load_image_data_url(img_path, options.image_size).await?;
    let params = ChatParams {
        endpoint: &options.api_endpoint,
        api_key: &options.api_key,
        model: &options.model_name,
        temperature: options.temperature,
        max_tokens: options.max_tokens,
        top_p: options.top_p,
    };
    let messages = [
        ChatMessage::system(&options.system_prompt),
        ChatMessage::user(llm_client::vision_user_content(
            &options.user_prompt,
            &data_url,
            &options.image_detail,
        )),
    ];
    let reply =
        llm_client::chat_completion(client, &params, &messages, throttle, &LLM_CANCELLED).await;
    llm_client::accept_reply(&reply, "该图片").map(str::to_owned)
}

#[tauri::command]
pub async fn fetch_llm_models(
    api_endpoint: String,
    api_key: String,
) -> Result<Vec<String>, String> {
    llm_client::list_models(&llm_client::llm_http_client()?, &api_endpoint, &api_key).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::batch::capture_events;
    use crate::commands::llm_client::test_support::{client, serve_chat_reply};
    use crate::commands::test_support::TempDir;
    use serde_json::json;

    fn options() -> LlmTaggerOptions {
        serde_json::from_value(json!({
            "input_path": "", "api_endpoint": "", "api_key": "", "model_name": "mock",
            "system_prompt": "describe", "user_prompt": "image", "temperature": 0.5,
            "max_tokens": 20, "output_format": "json",
        }))
        .unwrap()
    }

    fn parse(text: &str, opts: &LlmTaggerOptions) -> serde_json::Value {
        serde_json::from_str(&format_output(text, opts).unwrap()).unwrap()
    }

    #[tokio::test]
    #[ignore = "requires env/gbc; uses cached graphs and a local mock VLM"]
    async fn gbc_caption_refinement_preserves_json_tags_and_failed_outputs() {
        let project = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        let python = project.join(if cfg!(windows) { "env/gbc/Scripts/python.exe" } else { "env/gbc/bin/python" });
        for (format, simple) in [("txt", false), ("json", false), ("json", true)] {
            for fail in [false, true] {
                let root = TempDir::new("gbc_caption_refine");
                let image = root.join("image.png");
                image::RgbImage::new(32, 16).save(&image).unwrap();
                std::fs::write(image.with_extension(format), "original sidecar").unwrap();
                let first = if format == "txt" { "A girl." } else { r#"{"tags":["solo"],"nl":"A girl."}"# };
                let mut calls = 0;
                let server = crate::commands::test_support::serve_n(2, move |stream, _, _| {
                    let content = if calls == 0 { first } else if fail { "" } else { "A girl next to a blue cup." };
                    calls += 1;
                    let body = json!({"choices":[{"message":{"content":content},"finish_reason":"stop"}]}).to_string();
                    crate::commands::test_support::write_response(stream, "200 OK", "application/json", body.as_bytes());
                });
                let mut opts = options();
                opts.input_path = root.to_string_lossy().into_owned();
                opts.api_endpoint = server.url.clone();
                opts.output_format = format.into(); opts.json_simplified = simple;
                opts.gbc = Some(serde_json::from_value(json!({"python_path":python,"detector_path":"missing","embedder_path":"missing","device":"cpu","content_type":"caption","caption_prompt":"","max_nodes":20,"max_depth":3,"max_requests":64})).unwrap());
                let setup = r#"import json,sys
from pathlib import Path
sys.path.insert(0,sys.argv[1])
from gbc_worker import analysis_identity
options=json.loads(sys.argv[2]);options.update(options.pop('gbc'))
p=Path(sys.argv[3])
g={'vertices':[{'vertex_id':'','label':'image','bbox':{'left':0,'top':0,'right':1,'bottom':1},'descs':[{'label':'detail','text':'GBC fixture: a blue cup.'}],'out_edges':[],'in_edges':[]}], 'img_size':[32,16]}
g['_purinbox']=analysis_identity(p,options)
p.with_suffix('.gbc.json').write_text(json.dumps(g))
"#;
                assert!(std::process::Command::new(&python).arg("-c").arg(setup).arg(project.join("src-tauri/scripts")).arg(serde_json::to_value(&opts).unwrap().to_string()).arg(&image).status().unwrap().success());
                LLM_CANCELLED.store(false, Ordering::SeqCst);
                let app = tauri::test::mock_app();
                let result = run_llm_tagging(app.handle(), opts.clone(), client()).await.unwrap();
                let written = std::fs::read_to_string(image.with_extension(format)).unwrap();
                if fail { assert_eq!(result.success_count, 0); assert_eq!(written, "original sidecar"); }
                else if format == "txt" { assert_eq!(written, "A girl next to a blue cup."); }
                else {
                    let mut expected = parse(first, &opts);
                    if simple { expected["nl"] = json!("A girl next to a blue cup."); } else { expected["ai_output"]["nl"] = json!("A girl next to a blue cup."); }
                    assert_eq!(serde_json::from_str::<serde_json::Value>(&written).unwrap(), expected);
                }
                let _first = server.requests.recv_timeout(std::time::Duration::from_secs(5)).unwrap();
                let second = server.requests.recv_timeout(std::time::Duration::from_secs(5)).unwrap().json();
                let text = second["messages"][0]["content"][0]["text"].as_str().unwrap();
                assert!(text.contains("A girl.") && text.contains("GBC fixture: a blue cup."));
            }
        }
    }

    #[tokio::test]
    #[ignore = "requires env/gbc; uses cached graphs and a local mock VLM"]
    async fn gbc_existing_refines_without_initial_generation() {
        let project = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        let python = project.join(if cfg!(windows) { "env/gbc/Scripts/python.exe" } else { "env/gbc/bin/python" });
        for (format, simple) in [("txt", false), ("json", false), ("json", true)] {
            for fail in [false, true] {
                let root = TempDir::new("gbc_caption_refine");
                let image = root.join("image.png");
                image::RgbImage::new(32, 16).save(&image).unwrap();
                let original = if format == "txt" { "A girl.".to_owned() } else if simple { r#"{"tags":["solo"],"nl":"A girl.","custom":{"keep":true}}"#.to_owned() } else { r#"{"ai_output":{"tags":["solo"],"nl":"A girl."},"custom":{"keep":true}}"#.to_owned() };
                std::fs::write(image.with_extension(format), &original).unwrap();
                let server = crate::commands::test_support::serve_n(1, move |stream, _, _| {
                    let content = if fail { "" } else { "A girl next to a blue cup." };
                    let body = json!({"choices":[{"message":{"content":content},"finish_reason":"stop"}]}).to_string();
                    crate::commands::test_support::write_response(stream, "200 OK", "application/json", body.as_bytes());
                });
                let mut opts = options();
                opts.refine_existing = true;
                opts.input_path = image.to_string_lossy().into_owned();
                opts.api_endpoint = server.url.clone();
                opts.output_format = format.into(); opts.json_simplified = simple;
                opts.gbc = Some(serde_json::from_value(json!({"python_path":python,"detector_path":"missing","embedder_path":"missing","device":"cpu","content_type":"caption","caption_prompt":"","max_nodes":20,"max_depth":3,"max_requests":64})).unwrap());
                let setup = r#"import json,sys
from pathlib import Path
sys.path.insert(0,sys.argv[1])
from gbc_worker import analysis_identity
options=json.loads(sys.argv[2]);options.update(options.pop('gbc'))
p=Path(sys.argv[3])
g={'vertices':[{'vertex_id':'','label':'image','bbox':{'left':0,'top':0,'right':1,'bottom':1},'descs':[{'label':'detail','text':'GBC fixture: a blue cup.'}],'out_edges':[],'in_edges':[]}], 'img_size':[32,16]}
g['_purinbox']=analysis_identity(p,options)
p.with_suffix('.gbc.json').write_text(json.dumps(g))
"#;
                assert!(std::process::Command::new(&python).arg("-c").arg(setup).arg(project.join("src-tauri/scripts")).arg(serde_json::to_value(&opts).unwrap().to_string()).arg(&image).status().unwrap().success());
                LLM_CANCELLED.store(false, Ordering::SeqCst);
                let app = tauri::test::mock_app();
                let result = run_llm_tagging(app.handle(), opts.clone(), client()).await.unwrap();
                let written = std::fs::read_to_string(image.with_extension(format)).unwrap();
                if fail { assert_eq!(result.success_count, 0); assert_eq!(written, original); }
                else if format == "txt" { assert_eq!(written, "A girl next to a blue cup."); }
                else {
                    let mut expected: serde_json::Value = serde_json::from_str(&original).unwrap();
                    if simple { expected["nl"] = json!("A girl next to a blue cup."); } else { expected["ai_output"]["nl"] = json!("A girl next to a blue cup."); }
                    assert_eq!(serde_json::from_str::<serde_json::Value>(&written).unwrap(), expected);
                }
                let second = server.requests.recv_timeout(std::time::Duration::from_secs(5)).unwrap().json();
                let text = second["messages"][0]["content"][0]["text"].as_str().unwrap();
                assert!(text.contains("A girl.") && text.contains("GBC fixture: a blue cup."));
                assert!(server.requests.try_recv().is_err());
            }
        }
    }

    #[test]
    fn existing_gbc_without_caption_can_continue_to_txt() {
        let root = crate::commands::test_support::TempDir::new("gbc_only_input");
        let image = root.join("sample.png");
        std::fs::write(image.with_extension("gbc.json"), r#"{"vertices":[]}"#).unwrap();
        assert_eq!(existing_format(&image), Some("txt"));
        let (reference, json, _) = existing_input(&image, "txt").unwrap();
        assert!(reference.contains("GBC scene graph"));
        assert!(json.is_none());
        assert!(!image.with_extension("txt").exists());
    }

    #[test]
    fn existing_labels_validate_and_preserve_source_structure() {
        let root = TempDir::new("gbc_existing_sources");
        let image = root.join("sample.png");
        image::RgbImage::new(8, 8).save(&image).unwrap();
        let source = image.with_extension("json");
        for value in [json!({"tags":["girl","red dress"],"custom":42}), json!({"ai_output":{"tags":["girl"]},"custom":42})] {
            let raw = value.to_string();
            std::fs::write(&source, &raw).unwrap();
            let (reference, parsed, original) = existing_input(&image, "json").unwrap();
            assert!(reference.contains("girl"));
            assert_eq!(parsed, Some(value)); assert_eq!(original, raw);
        }
        for raw in ["{", "{}", r#"{"vertices":[]}"#, r#"{"ai_output":null}"#, r#"{"nl":"","tags":[]}"#] {
            std::fs::write(&source, raw).unwrap();
            assert!(existing_input(&image, "json").is_err());
        }
        let mut opts = options(); opts.refine_existing = true;
        opts.input_path = image.to_string_lossy().into_owned();
        assert_eq!(existing_files(&mut opts).unwrap(), vec![image.clone()]);
        assert_eq!(opts.event(), "gbc-refine-progress");
        std::fs::write(image.with_extension("txt"), "also present").unwrap();
        assert_eq!(existing_format(&image), Some("json"));
        let txt_image = root.join("txt_only.png");
        image::RgbImage::new(8, 8).save(&txt_image).unwrap();
        std::fs::write(txt_image.with_extension("txt"), "A tree.").unwrap();
        assert_eq!(existing_format(&txt_image), Some("txt"));
        opts.input_path = root.to_string_lossy().into_owned();
        let mixed = existing_files(&opts).unwrap();
        assert!(mixed.contains(&image) && mixed.contains(&txt_image));
        std::fs::remove_file(&txt_image).unwrap();
        opts.input_path = image.to_string_lossy().into_owned();
        image::RgbImage::new(8, 8).save(image.with_extension("jpg")).unwrap();
        assert_eq!(existing_files(&opts).unwrap(), vec![image.clone()]);
        opts.input_path = root.to_string_lossy().into_owned();
        assert!(existing_files(&mut opts).is_err());
        std::fs::remove_file(&image).unwrap();
        std::fs::remove_file(image.with_extension("jpg")).unwrap();
        opts.input_path = image.to_string_lossy().into_owned();
        assert!(existing_files(&mut opts).is_err());
    }

    #[test]
    fn non_json_responses_keep_the_full_schema() {
        let mut opts = options();
        let full = parse("description", &opts);
        assert_eq!(
            full,
            json!({
                "fixed": {"quality": "", "series": "", "artist": ""},
                "character": {"name": "", "variant": ""},
                "from_path": {"appearance": []},
                "ai_output": {"count": "", "appearance": [], "tags": [], "environment": [], "nl": "description"},
            })
        );
        opts.json_simplified = true;
        let simple = parse("description", &opts);
        assert_eq!(simple.as_object().unwrap().len(), 9);
        assert_eq!(simple["nl"], "description");
        assert_eq!(simple["tags"], json!([]));
        assert_eq!(simple["artist"], "");
    }

    #[test]
    fn opening_only_fences_do_not_discard_json() {
        for text in [
            "```json\n{\"tags\": [\"solo\"]}\n```",
            "```json\n{\"tags\": [\"solo\"]}",
            "```\n{\"tags\": [\"solo\"]}",
        ] {
            let parsed = parse(text, &options());
            assert_eq!(parsed["ai_output"]["tags"], json!(["solo"]), "{text}");
            assert_eq!(parsed["ai_output"]["nl"], "", "{text}");
            assert_eq!(
                parsed.as_object().unwrap().keys().collect::<Vec<_>>(),
                ["fixed", "character", "from_path", "ai_output"]
            );
        }
    }

    /// JSON 回复也补齐骨架：默认完整格式提示词只要求 ai_output 一段；画师逐位补 @，额外字段保留
    #[test]
    fn json_replies_are_normalized_to_the_complete_schema() {
        let mut opts = options();
        let reply = r#"{"ai_output": {"count": "1girl", "tags": "smile, standing", "nl": "A girl."},
                        "fixed": {"artist": "foo, @bar"}, "rating": "safe"}"#;
        let full = parse(reply, &opts);
        assert_eq!(
            full["fixed"],
            json!({"quality": "", "series": "", "artist": "@foo, @bar"})
        );
        assert_eq!(full["character"], json!({"name": "", "variant": ""}));
        assert_eq!(full["from_path"], json!({"appearance": []}));
        assert_eq!(full["ai_output"]["tags"], json!(["smile", "standing"]));
        assert_eq!(full["ai_output"]["nl"], "A girl.");
        assert_eq!(full["rating"], "safe");

        opts.json_simplified = true;
        let simple = parse(
            r#"{"artist": ["foo", "bar"], "tags": ["solo"], "mood": "calm"}"#,
            &opts,
        );
        assert_eq!(simple["artist"], "@foo, @bar");
        assert_eq!(simple["tags"], json!(["solo"]));
        assert_eq!(simple["mood"], "calm");
        assert_eq!(simple["character"], "");
    }

    /// 合法 JSON 但不是对象：原文整段放进 nl，不丢内容
    #[test]
    fn json_that_is_not_an_object_goes_into_nl_verbatim() {
        for text in ["[\"solo\", \"smile\"]", "42", "\"just a string\"", "null"] {
            let parsed = parse(text, &options());
            assert_eq!(parsed["ai_output"]["nl"], text, "{text}");
            assert_eq!(parsed["ai_output"]["tags"], json!([]), "{text}");
        }
    }

    #[test]
    fn txt_output_is_written_verbatim() {
        let mut opts = options();
        opts.output_format = "txt".into();
        assert_eq!(
            format_output("{\"tags\": []}", &opts).unwrap(),
            "{\"tags\": []}"
        );
    }

    /// 「已有标签」按文件存在且非空判断，txt 与 json 各自判断，不读内容
    #[test]
    fn existing_labels_are_judged_by_size_per_format() {
        use crate::commands::tagger::has_labels;
        let root = TempDir::new("llm_tagger_existing");
        let image = root.join("a.png");
        assert!(!has_labels(&image));
        std::fs::write(
            image.with_extension("txt"),
            b"\xd2\xbb\xb8\xf6\xc5\xae\xba\xa2",
        )
        .unwrap();
        assert!(has_labels(&image), "GBK 编码的旧 txt 也算已有标签");
        std::fs::write(image.with_extension("txt"), "").unwrap();
        std::fs::write(image.with_extension("json"), " \n").unwrap();
        assert!(has_labels(&image));
        std::fs::write(image.with_extension("json"), "").unwrap();
        assert!(!has_labels(&image));
        std::fs::write(image.with_extension("txt"), "solo").unwrap();
        assert!(has_labels(&image), "空 json 不挡住对 txt 的判断");
    }

    #[tokio::test]
    async fn short_replies_warn_and_are_saved_in_all_output_formats() {
        for (format, simplified) in [("txt", false), ("json", false), ("json", true)] {
            for (count, threshold) in [(99, 100), (100, 100), (1, 1), (1, 2), (499, 500), (500, 500)] {
                let root = TempDir::new("llm_tagger_short_reply");
                let path = root.join("a.png");
                image::RgbImage::new(2, 2).save(&path).unwrap();
                let han_count = count.min(49);
                let text = format!("{}{}", "字".repeat(han_count), "word ".repeat(count - han_count));
                let server = serve_chat_reply(Some(&text), "stop");
                let mut opts = options();
                assert_eq!(opts.short_reply_threshold, 100);
                opts.short_reply_threshold = threshold as u32;
                opts.api_endpoint = server.url.clone();
                opts.output_format = format.into();
                opts.json_simplified = simplified;
                let result = tag_file(&client(), &path, &opts, &RequestThrottle::new(-1), None).await;
                match result {
                    ItemOutcome::Done { message, warning } => {
                        assert_eq!(warning, count < threshold, "{message}");
                        assert_eq!(message.contains("疑似截断"), count < threshold, "{message}");
                        if count < threshold {
                            assert!(message.ends_with(" | 疑似截断"), "{message}");
                            assert!(!message.contains("回复仅"), "{message}");
                            assert!(!message.contains('⚠'), "{message}");
                        }
                    }
                    ItemOutcome::Failed { message, .. } | ItemOutcome::Warned { message, .. } => panic!("{message}"),
                }
                assert_eq!(
                    std::fs::read_to_string(path.with_extension(format)).unwrap(),
                    format_output(text.trim(), &opts).unwrap()
                );
            }
        }
    }

    #[tokio::test]
    async fn rejects_truncated_filtered_and_refused_responses() {
        let temp = TempDir::new("llm_tagger_responses");
        let path = temp.join("image.wrong_extension");
        image::RgbImage::new(2, 2)
            .save_with_format(&path, image::ImageFormat::Png)
            .unwrap();
        for (text, finish, expected) in [
            ("partial response", "length", "max_tokens 上限（20）被截断"),
            ("", "content_filter", "LLM 内容安全审核拒绝了该图片"),
            (
                "I cannot assist with this request",
                "stop",
                "拒绝处理该图片",
            ),
        ] {
            let server = serve_chat_reply(Some(text), finish);
            let mut opts = options();
            opts.api_endpoint = server.url.clone();
            let error = tag_with_llm(&client(), &path, &opts, &RequestThrottle::new(-1))
                .await
                .unwrap_err();
            assert!(error.is_warning());
            assert!(error.to_string().contains(expected), "{error}");
            assert!(!path.with_extension("json").exists());
        }
        let server = serve_chat_reply(Some("a complete description"), "stop");
        let mut opts = options();
        opts.api_endpoint = server.url.clone();
        assert_eq!(
            tag_with_llm(&client(), &path, &opts, &RequestThrottle::new(-1))
                .await
                .unwrap(),
            "a complete description"
        );
    }

    #[tokio::test]
    async fn reply_warnings_and_execution_errors_archive_separately_in_place() {
        use crate::commands::llm_client::test_support::{serve_json, serve_status};
        let mut evidence = Vec::new();
        for (format, simplified) in [("txt", false), ("json", false), ("json", true)] {
            for input in ["directory", "file", "recursive"] {
                for case in ["empty", "empty_choices", "length", "empty_length", "safety", "refusal", "refusal_field", "safety_http", "short", "http", "auth", "protocol", "read", "write"] {
                    let root = TempDir::new("llm_warning_archive");
                    let relative = if input == "recursive" { "nested/image.png" } else { "image.png" };
                    let path = root.join(relative);
                    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
                    image::RgbImage::new(2, 2).save(&path).unwrap();
                    if case == "read" { std::fs::write(&path, "broken image").unwrap(); }
                    let label = path.with_extension(format);
                    let original = if format == "txt" { "original tags" } else { r#"{"tags":["original"],"nl":"original"}"# };
                    if case == "write" {
                        std::fs::create_dir(&label).unwrap();
                    } else {
                        std::fs::write(&label, original).unwrap();
                    }
                    // Old archive copies must never become another input on recursive reruns.
                    for folder in ["Warn", "Fail"] {
                        std::fs::create_dir_all(root.join(folder)).unwrap();
                        std::fs::write(root.join(folder).join("previous.png"), "old").unwrap();
                    }
                    let server = match case {
                        "empty" => serve_chat_reply(None, "stop"),
                        "empty_choices" => serve_json(json!({"choices": []})),
                        "length" => serve_chat_reply(Some("A partial description"), "length"),
                        "empty_length" => serve_chat_reply(None, "length"),
                        "safety" => serve_chat_reply(None, "safety"),
                        "refusal" => serve_chat_reply(Some("I cannot assist with this request"), "stop"),
                        "refusal_field" => serve_json(json!({"choices":[{"message":{"content":null,"refusal":"blocked"},"finish_reason":"stop"}]})),
                        "safety_http" => serve_status("400 Bad Request", "application/json", br#"{"error":{"code":"content_filter"}}"#.to_vec()),
                        "http" => serve_status("503 Service Unavailable", "text/plain", b"unavailable".to_vec()),
                        "auth" => serve_status("403 Forbidden", "application/json", br#"{"error":{"code":"permission_denied"}}"#.to_vec()),
                        "protocol" => serve_status("200 OK", "application/json", b"invalid json".to_vec()),
                        _ => serve_chat_reply(Some("A girl smiling."), "stop"),
                    };
                    let warning = !matches!(case, "http" | "auth" | "protocol" | "read" | "write");
                    let mut opts = options();
                    opts.input_path = if input == "file" { &path } else { &*root }.to_string_lossy().into_owned();
                    opts.recursive = input == "recursive";
                    opts.api_endpoint = server.url.clone();
                    opts.output_format = format.into();
                    opts.json_simplified = simplified;
                    let app = tauri::test::mock_app();
                    let events = capture_events(app.handle(), EVENT);
                    let result = run_llm_tagging(app.handle(), opts, client()).await.unwrap();
                    assert_eq!(result.total, 1, "{case}/{input}/{format}");
                    assert_eq!(result.success_count, u32::from(case == "short"));
                    assert_eq!(result.warning_count, u32::from(warning));
                    assert_eq!(result.fail_count, u32::from(!warning));
                    let folder = if warning { "Warn" } else { "Fail" };
                    assert_eq!(std::fs::read(root.join(folder).join(relative)).unwrap(), std::fs::read(&path).unwrap());
                    let other = if warning { "Fail" } else { "Warn" };
                    assert!(!root.join(other).join(relative).exists());
                    if case == "short" {
                        assert!(std::fs::read_to_string(&label).unwrap().contains("A girl smiling."));
                    } else if case == "write" {
                        assert!(label.is_dir());
                    } else {
                        assert_eq!(std::fs::read_to_string(&label).unwrap(), original);
                    }
                    let archived_label = root.join(folder).join(relative).with_extension(format);
                    if case == "write" {
                        assert!(!archived_label.exists());
                    } else {
                        assert_eq!(std::fs::read(archived_label).unwrap(), std::fs::read(&label).unwrap());
                    }
                    let events = events.lock().unwrap();
                    let items: Vec<_> = events.iter().filter(|e| e["filename"] == "image.png").collect();
                    assert_eq!(items.len(), 1);
                    assert_eq!(items[0]["status"], if warning { "warning" } else { "error" });
                    assert_eq!(items[0]["current"], 1);
                    assert_eq!(items[0]["unwritten"] == true, warning && case != "short");
                    assert!(!items[0]["message"].as_str().unwrap().contains('⚠'));
                    assert_eq!(events.iter().filter(|e| e["status"] == "done").count(), 1);
                    assert_eq!(events.iter().filter(|e| e["message"].as_str().unwrap().contains("复制到")).count(), 1);
                    if format == "json" && simplified && input == "directory" {
                        evidence.push(json!({"case": case, "result": result, "events": *events, "archive": folder}));
                    }
                }
            }
        }
        if let Ok(path) = std::env::var("PURINBOX_TEST_LOG_EVIDENCE") {
            std::fs::write(path, serde_json::to_vec_pretty(&evidence).unwrap()).unwrap();
        }
    }

    /// 一轮：跳过已有标签的、成功的写出规范化 JSON、失败的进 Fail/，终态 done 只有一条
    #[tokio::test]
    async fn run_skips_labelled_images_writes_json_and_archives_failures() {
        LLM_CANCELLED.store(false, Ordering::SeqCst);
        let root = TempDir::new("llm_tagger_run");
        let good = root.join("a_good.png");
        image::RgbImage::new(2, 2).save(&good).unwrap();
        let skipped = root.join("b_skipped.png");
        image::RgbImage::new(2, 2).save(&skipped).unwrap();
        std::fs::write(skipped.with_extension("txt"), b"\xc4\xe3\xba\xc3").unwrap();
        let broken = root.join("c_broken.png");
        std::fs::write(&broken, "not an image").unwrap();
        let empty_label = root.join("d_empty.png");
        std::fs::write(&empty_label, "not an image").unwrap();
        std::fs::write(empty_label.with_extension("txt"), "").unwrap();

        let server = serve_chat_reply(Some(r#"{"ai_output": {"tags": ["solo"]}}"#), "stop");
        let mut opts = options();
        opts.input_path = root.to_string_lossy().into_owned();
        opts.api_endpoint = server.url.clone();
        opts.skip_existing = true;
        let app = tauri::test::mock_app();
        let events = capture_events(app.handle(), EVENT);
        let result = run_llm_tagging(app.handle(), opts, client()).await.unwrap();

        assert_eq!(
            (result.success_count, result.fail_count, result.total),
            (2, 2, 4)
        );
        let written: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(good.with_extension("json")).unwrap())
                .unwrap();
        assert_eq!(written["ai_output"]["tags"], json!(["solo"]));
        assert_eq!(written["fixed"]["artist"], "");
        assert!(!skipped.with_extension("json").exists());
        assert!(root.join("Fail/c_broken.png").exists());
        assert!(
            root.join("Fail/d_empty.png").exists(),
            "空 txt 不算已有标签"
        );
        assert!(!root.join("Fail/a_good.png").exists());
        assert!(root.join("Warn/a_good.png").exists());
        let events = events.lock().unwrap();
        let warnings: Vec<_> = events.iter().filter(|e| e["status"] == "warning").collect();
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0]["message"].as_str().unwrap().contains("疑似截断"));
        let done: Vec<_> = events.iter().filter(|e| e["status"] == "done").collect();
        assert_eq!(done.len(), 1);
        assert_eq!(done[0]["message"], "LLM 打标完成: 成功 2, 失败 2, 共 4");
        assert!(events
            .iter()
            .any(|e| e["message"] == "[跳过] b_skipped.png (已有描述)"));
        assert!(events
            .iter()
            .filter(|e| matches!(e["status"].as_str(), Some("success" | "warning" | "error")))
            .all(|e| e["total"] == 4));
    }
}
