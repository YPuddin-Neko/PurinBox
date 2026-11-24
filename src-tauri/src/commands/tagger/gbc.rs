use super::llm_tagger::{LlmTaggerOptions, LLM_CANCELLED};
use crate::commands::{
    collect_image_files_with_recursive, config_paths, file_name_lossy,
    llm_batch::{BatchOutcome, LlmBatchResult},
    python_proc::{self, PythonCommand, PythonSession, SessionError, StderrNoise},
    ProblemArchive, ProgressEvent,
};
use serde::{Deserialize, Serialize};
use tauri::Emitter;
use crate::commands::http_download::DownloadProgress;
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Mutex,
    },
    time::{Duration, Instant},
};

const EVENT: &str = "llm-tagger-progress";
static BUSY: AtomicBool = AtomicBool::new(false);
static PROCESS: Mutex<Option<u32>> = Mutex::new(None);

fn wait_for_engine(
    session: &mut PythonSession<'_>, init: &serde_json::Value, limit: Duration,
    mut log: impl FnMut(&serde_json::Value),
) -> Result<(), String> {
    session.send(init).map_err(|e| e.to_string())?;
    let mut stage = "正在启动 GBC 分析引擎".to_owned();
    let mut deadline = Instant::now() + limit;
    let mut models_ready = false;
    loop {
        let msg = match session.recv(deadline.saturating_duration_since(Instant::now())) {
            Ok(msg) => msg,
            Err(SessionError::TimedOut(_)) => return Err(format!("{stage}超时（{} 秒）", limit.as_secs())),
            Err(error) => return Err(error.to_string()),
        };
        match msg["type"].as_str() {
            Some("log") => {
                if msg["model_stage"] == true {
                    stage = msg["message"].as_str().unwrap_or("正在加载 GBC 模型").to_owned();
                    deadline = Instant::now() + limit;
                }
                log(&msg);
            }
            Some("models_ready") => models_ready = true,
            Some("ready") => match msg["mode"].as_str() {
                Some("analysis") if models_ready => return Ok(()),
                Some("cached") if init["cache_only"] == true => return Ok(()),
                _ => return Err("GBC 分析引擎未确认就绪，请检查工作脚本版本".into()),
            },
            Some("error") => return Err(msg["message"].as_str().unwrap_or("GBC 启动失败").into()),
            _ => {}
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GbcSettings {
    pub python_path: String,
    pub detector_path: String,
    pub embedder_path: String,
    pub device: String,
    pub content_type: String,
    #[serde(default)]
    pub save_graph: bool,
    pub caption_prompt: String,
    pub max_nodes: u32,
    pub max_depth: u32,
    pub max_requests: u32,
}

fn validate_environment(settings: &GbcSettings) -> Result<PathBuf, String> {
    if !["gbc", "moss_layout", "caption"].contains(&settings.content_type.as_str())
        || !["cpu", "cuda", "mps"].contains(&settings.device.as_str())
        || !(1..=256).contains(&settings.max_requests)
        || !(1..=64).contains(&settings.max_nodes)
        || settings.max_depth > 5
    {
        return Err("GBC 参数无效".into());
    }
    let python = if settings.python_path.trim().is_empty() {
        config_paths::app_data_root().join(if cfg!(windows) {
            "env/gbc/Scripts/python.exe"
        } else {
            "env/gbc/bin/python"
        })
    } else {
        PathBuf::from(&settings.python_path)
    };
    if !python.is_file() {
        return Err("GBC 环境不可用".into());
    }
    Ok(python)
}

fn save_graph_sidecar(path: &std::path::Path, graph: &serde_json::Value, save_graph: bool) -> Result<(), String> {
    if !save_graph { return Ok(()); }
    config_paths::write_file_atomic(&path.with_extension("gbc.json"),
        serde_json::to_string_pretty(graph).map_err(|e| e.to_string())?.as_bytes())
        .map_err(|e| e.to_string())
}

pub fn cancel() {
    crate::commands::python_env::cancel_setup_for("llm-tagger-progress");
    crate::commands::python_env::cancel_setup_for("gbc-refine-progress");
    python_proc::kill_registered_pid(&PROCESS);
}

pub fn run<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    options: LlmTaggerOptions,
) -> Result<LlmBatchResult, String> {
    let _busy = crate::commands::BusyGuard::acquire_enforced(&BUSY, "GBC")?;
    let ready = prepare_environment(&app, options.gbc.as_ref().ok_or("缺少 GBC 配置")?, &LLM_CANCELLED, EVENT, &PROCESS)?;
    let settings = &ready;
    let python = validate_environment(settings)?;
    let input = PathBuf::from(&options.input_path);
    let files = collect_image_files_with_recursive(&input, options.recursive)?;
    let extension = if settings.content_type == "caption" && options.output_format != "json" {
        "txt"
    } else {
        "json"
    };
    let mut outputs = std::collections::HashSet::new();
    for path in &files {
        if !outputs.insert(path.with_extension(extension)) {
            return Err("存在同名但扩展名不同的图片，请分开目录后导出，避免标签相互覆盖".into());
        }
    }
    let existing: std::collections::HashSet<_> =
        outputs.iter().filter(|p| p.is_file()).cloned().collect();
    let mut outcome = BatchOutcome {
        total: files.len() as u32,
        ..Default::default()
    };
    if files.is_empty() {
        return Ok(outcome.finish(
            &app,
            EVENT,
            "精细描述完成",
            &ProblemArchive::new(&input, &input, options.recursive),
        ));
    }
    let script = python_proc::find_script("gbc_worker.py")?;
    let mut session = PythonSession::start(
        PythonCommand::new(python).arg(script),
        &PROCESS,
        &LLM_CANCELLED,
        python_proc::stderr_warnings(&app, EVENT, StderrNoise::Runtime, None),
    )
    .map_err(|e| e.to_string())?;
    let mut init = serde_json::to_value(&options).map_err(|e| e.to_string())?;
    let map = init.as_object_mut().unwrap();
    map.remove("gbc");
    map.extend(
        serde_json::to_value(settings)
            .unwrap()
            .as_object()
            .unwrap()
            .clone(),
    );
    let mut skip_outputs = Vec::new();
    let cache_only = files.iter().all(|path| {
        if path.with_extension("gbc.json").is_file() { return true; }
        let output = path.with_extension(extension);
        if options.skip_existing && existing.contains(&output) {
            skip_outputs.push(output);
            return true;
        }
        false
    });
    if cache_only {
        map.insert("cache_only".into(), true.into());
        map.insert("skip_outputs".into(), serde_json::json!(skip_outputs));
    }
    wait_for_engine(&mut session, &init, python_proc::PYTHON_SILENCE_LIMIT,
        |msg| ProgressEvent::python_log(msg, 0, outcome.total).emit(&app, EVENT))?;
    for (index, path) in files.iter().enumerate() {
        if LLM_CANCELLED.load(Ordering::SeqCst) {
            break;
        }
        let output = path.with_extension(
            if settings.content_type == "caption" && options.output_format != "json" {
                "txt"
            } else {
                "json"
            },
        );
        let mut warning = None;
        let result = (|| -> Result<bool, String> {
            session.send(&serde_json::json!({"cmd":"process", "path":path, "existing_path":if options.skip_existing && existing.contains(&output) {Some(&output)} else {None}})).map_err(|e| e.to_string())?;
            loop {
                let msg = session.recv(Duration::from_secs(900)).map_err(|e| e.to_string())?;
                match msg["type"].as_str() {
                    Some("skip") => return Ok(true),
                    Some("error") => {
                        return Err(msg["message"]
                            .as_str()
                            .unwrap_or("GBC 分析失败")
                            .to_string())
                    }
                    Some("graph") => {
                        if LLM_CANCELLED.load(Ordering::SeqCst) {
                            return Err("已取消".into());
                        }
                        save_graph_sidecar(&path, &msg["graph"], settings.save_graph)?;
                    }
                    Some("result") => {
                        if LLM_CANCELLED.load(Ordering::SeqCst) {
                            return Err("已取消".into());
                        }
                        let content = msg["content"].as_str().ok_or("GBC 导出结果为空")?;
                        if LLM_CANCELLED.load(Ordering::SeqCst) {
                            return Err("已取消".into());
                        }
                        config_paths::write_file_atomic(&output, content.as_bytes())
                            .map_err(|e| e.to_string())?;
                        if settings.content_type == "caption" {
                            let text = if options.output_format == "json" {
                                serde_json::from_str::<serde_json::Value>(content)
                                    .ok()
                                    .and_then(|v| v["nl"].as_str().map(str::to_owned))
                                    .unwrap_or_default()
                            } else {
                                content.to_owned()
                            };
                            warning = crate::commands::llm_client::short_reply_warning(
                                crate::commands::llm_client::response_word_count(&text),
                                options.short_reply_threshold,
                            );
                        }
                        return Ok(false);
                    }
                    _ => ProgressEvent::python_log(&msg, index as u32, outcome.total)
                        .emit(&app, EVENT),
                }
            }
        })();
        if LLM_CANCELLED.load(Ordering::SeqCst) {
            break;
        }
        match result {
            Ok(skipped) => {
                outcome.success += 1;
                if warning.is_some() {
                    outcome.warning_files.push(path.clone());
                }
                ProgressEvent::new(
                    if warning.is_some() {
                        "warning"
                    } else {
                        "success"
                    },
                    format!(
                        "[{}] {}{}",
                        if skipped { "跳过" } else { "完成" },
                        file_name_lossy(path),
                        warning.map(|w| format!(" | {}", w)).unwrap_or_default()
                    ),
                )
                .at(index as u32 + 1, outcome.total)
                .file(file_name_lossy(path))
                .emit(&app, EVENT);
            }
            Err(error) => {
                outcome.fail += 1;
                outcome
                    .errors
                    .push(format!("{}: {}", file_name_lossy(path), error));
                outcome.error_files.push(path.clone());
                ProgressEvent::new("error", format!("{}: {}", file_name_lossy(path), error))
                    .at(index as u32 + 1, outcome.total)
                    .file(file_name_lossy(path))
                    .emit(&app, EVENT);
            }
        }
    }
    outcome.cancelled = LLM_CANCELLED.load(Ordering::SeqCst);
    session.shutdown(Some(r#"{"cmd":"quit"}"#));
    Ok(outcome.finish(
        &app,
        EVENT,
        "精细描述完成",
        &ProblemArchive::new(&input, &input, options.recursive),
    ))
}

static REFINE_PROCESS: Mutex<Option<u32>> = Mutex::new(None);

pub(crate) fn cancel_refine() {
    crate::commands::python_env::cancel_setup_for("tag-refine-progress");
    python_proc::kill_registered_pid(&REFINE_PROCESS);
}

pub(crate) struct Analyzer {
    session: Option<PythonSession<'static>>,
    cancel: &'static AtomicBool,
    log: Box<dyn FnMut(&serde_json::Value) + Send>,
    save_graph: bool,
    _busy: crate::commands::BusyGuard,
}

fn image_oriented_size(path: &std::path::Path) -> Result<(u32, u32), String> {
    use image::ImageDecoder;
    let reader = image::ImageReader::open(path)
        .map_err(|e| format!("无法读取图片：{}", e))?
        .with_guessed_format()
        .map_err(|e| format!("无法识别图片格式：{}", e))?;
    let mut decoder = reader.into_decoder().map_err(|e| format!("无法解析图片尺寸：{}", e))?;
    let (w, h) = decoder.dimensions();
    let swapped = decoder.orientation().is_ok_and(|orientation| matches!(
        orientation,
        image::metadata::Orientation::Rotate90
            | image::metadata::Orientation::Rotate270
            | image::metadata::Orientation::Rotate90FlipH
            | image::metadata::Orientation::Rotate270FlipH
    ));
    Ok(if swapped { (h, w) } else { (w, h) })
}

fn load_cached_graph(path: &std::path::Path) -> Result<String, String> {
    let sidecar = path.with_extension("gbc.json");
    if !sidecar.is_file() {
        return Err("已有 GBC 数据已移除，请重新运行".into());
    }
    let raw = std::fs::read_to_string(&sidecar).map_err(|e| format!("无法读取 GBC 数据：{}", e))?;
    let value: serde_json::Value = serde_json::from_str(&raw).map_err(|e| format!("GBC JSON 无效：{}", e))?;
    crate::commands::gbc_document::validate(&value)?;
    let graph = value.get("graph").unwrap_or(&value);
    if let Some(arr) = graph.get("img_size").and_then(|v| v.as_array()) {
        if arr.len() == 2 {
            if let (Some(gw), Some(gh)) = (arr[0].as_u64(), arr[1].as_u64()) {
                let (iw, ih) = image_oriented_size(path)?;
                if gw != iw as u64 || gh != ih as u64 {
                    return Err("已有 GBC 数据的图片尺寸不匹配".into());
                }
            }
        }
    }
    serde_json::to_string(graph).map_err(|e| e.to_string())
}

impl Analyzer {
    pub(crate) fn start<R: tauri::Runtime>(
        app: &tauri::AppHandle<R>, options: &crate::commands::tag_refine::TagRefineOptions,
        cancel: &'static AtomicBool, cache_only: bool,
    ) -> Result<Self, String> {
        let mut init = serde_json::to_value(options).map_err(|e| e.to_string())?;
        init["cache_only"] = cache_only.into();
        Self::from_options(app, init, options.gbc.as_ref().ok_or("缺少 GBC 配置")?, cancel, "tag-refine-progress", &REFINE_PROCESS, 1)
    }

    pub(crate) fn start_llm<R: tauri::Runtime>(app: &tauri::AppHandle<R>, options: &LlmTaggerOptions, cache_only: bool) -> Result<Self, String> {
        let caption_requests = if options.refine_existing { 1 } else { 2 };
        let mut init = serde_json::to_value(options).map_err(|e| e.to_string())?;
        init["cache_only"] = cache_only.into();
        Self::from_options(app, init, options.gbc.as_ref().ok_or("缺少 GBC 配置")?, &LLM_CANCELLED, options.event(), &PROCESS, caption_requests)
    }

    fn from_options<R: tauri::Runtime>(app: &tauri::AppHandle<R>, mut init: serde_json::Value, settings: &GbcSettings, cancel: &'static AtomicBool, event: &'static str, process: &'static Mutex<Option<u32>>, caption_requests: u32) -> Result<Self, String> {
        let busy = crate::commands::BusyGuard::acquire_enforced(&BUSY, "GBC")?;
        if cancel.load(Ordering::SeqCst) {
            return Err("已取消".into());
        }
        if settings.max_requests <= caption_requests {
            return Err(format!("GBC 自然语言细化至少需要 {} 次单图请求额度", caption_requests + 1));
        }
        if init["cache_only"] == true {
            ProgressEvent::new("info", "复用已有 GBC 数据，跳过本地模型加载").emit(app, event);
            return Ok(Self {
                session: None,
                cancel,
                log: Box::new(|_| {}),
                save_graph: settings.save_graph,
                _busy: busy,
            });
        }
        let ready = prepare_environment(app, settings, cancel, event, process)?;
        let settings = &ready;
        let python = validate_environment(settings)?;
        let map = init.as_object_mut().unwrap();
        map.remove("gbc");
        map.insert("parallel_analysis".into(), serde_json::json!(true));
        map.extend(serde_json::to_value(settings).unwrap().as_object().unwrap().clone());
        map.insert("content_type".into(), serde_json::json!("gbc"));
        map.insert("output_format".into(), serde_json::json!("json"));
        map.entry("max_tokens").or_insert(serde_json::json!(-1));
        map.insert("max_requests".into(), serde_json::json!(settings.max_requests - caption_requests));
        let mut session = PythonSession::start(
            PythonCommand::new(python).arg(python_proc::find_script("gbc_worker.py")?),
            process, cancel,
            python_proc::stderr_warnings(app, event, StderrNoise::Runtime, None),
        ).map_err(|e| e.to_string())?;
        wait_for_engine(&mut session, &init, python_proc::PYTHON_SILENCE_LIMIT,
            |msg| ProgressEvent::python_log(msg, 0, 0).emit(app, event))?;
        let app = app.clone();
        Ok(Self { session: Some(session), cancel, log: Box::new(move |msg| ProgressEvent::python_log(msg, 0, 0).emit(&app, event)), save_graph: settings.save_graph, _busy: busy })
    }

    pub(crate) fn into_shared(self) -> SharedAnalyzer {
        let Some(session) = self.session else {
            return std::sync::Arc::new(AnalysisQueue {
                sender: None,
                worker: Mutex::new(None),
                cached_cancel: Some(self.cancel),
                _cached_busy: Some(self._busy),
            });
        };
        let (sender, receiver) = std::sync::mpsc::channel();
        let worker_state = Self {
            session: Some(session),
            cancel: self.cancel,
            log: self.log,
            save_graph: self.save_graph,
            _busy: self._busy,
        };
        let worker = std::thread::spawn(move || worker_state.serve(receiver));
        std::sync::Arc::new(AnalysisQueue {
            sender: Some(sender),
            worker: Mutex::new(Some(worker)),
            cached_cancel: None,
            _cached_busy: None,
        })
    }

    fn serve(mut self, receiver: std::sync::mpsc::Receiver<AnalysisJob>) {
        use std::collections::HashMap;
        let Some(mut session) = self.session.take() else { return; };
        let mut pending: HashMap<u64, AnalysisJob> = HashMap::new();
        let mut next_id = 0u64;
        let mut last_activity = std::time::Instant::now();
        let outcome = (|| -> Result<(), String> {
            loop {
                if self.cancel.load(Ordering::SeqCst) { return Err("已取消".into()); }
                let incoming = if pending.is_empty() {
                    match receiver.recv_timeout(Duration::from_millis(50)) {
                        Ok(job) => Some(job),
                        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
                        Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return Ok(()),
                    }
                } else { receiver.try_recv().ok() };
                if let Some(job) = incoming {
                    next_id += 1;
                    let command = serde_json::json!({"cmd":"process", "id":next_id.to_string(), "path":job.path});
                    pending.insert(next_id, job);
                    session.send(&command).map_err(|e| e.to_string())?;
                    last_activity = std::time::Instant::now();
                    continue;
                }
                let msg = match session.recv(Duration::from_millis(50)) {
                    Ok(msg) => { last_activity = std::time::Instant::now(); msg },
                    Err(SessionError::TimedOut(_)) if last_activity.elapsed() < Duration::from_secs(900) => continue,
                    Err(SessionError::TimedOut(_)) => return Err(SessionError::TimedOut(Duration::from_secs(900)).to_string()),
                    Err(error) => return Err(error.to_string()),
                };
                if self.cancel.load(Ordering::SeqCst) { return Err("已取消".into()); }
                match msg["type"].as_str() {
                    Some("graph" | "result" | "error") => {
                        let id = msg["id"].as_str().and_then(|id| id.parse::<u64>().ok())
                            .ok_or_else(|| msg["message"].as_str().unwrap_or("GBC 返回结果缺少任务编号").to_owned())?;
                        if msg["type"] == "graph" {
                            if let Some(job) = pending.get(&id) {
                                if let Err(error) = save_graph_sidecar(&job.path, &msg["graph"], self.save_graph) {
                                    if let Some(job) = pending.remove(&id) { let _ = job.reply.send(Err(error)); }
                                }
                            }
                        } else if let Some(job) = pending.remove(&id) {
                            let result = if msg["type"] == "result" {
                                msg["content"].as_str().map(str::to_owned).ok_or_else(|| "GBC 原始图为空".to_owned())
                            } else { Err(msg["message"].as_str().unwrap_or("GBC 分析失败").to_owned()) };
                            let _ = job.reply.send(result);
                        }
                    }
                    _ => (self.log)(&msg),
                }
            }
        })();
        if outcome.is_err() { session.kill(); }
        let error = outcome.err().unwrap_or_else(|| "GBC 分析器已关闭".into());
        for (_, job) in pending { let _ = job.reply.send(Err(error.clone())); }
        while let Ok(job) = receiver.try_recv() { let _ = job.reply.send(Err(error.clone())); }
    }
}

struct AnalysisJob {
    path: PathBuf,
    reply: tokio::sync::oneshot::Sender<Result<String, String>>,
}

pub(crate) struct AnalysisQueue {
    sender: Option<std::sync::mpsc::Sender<AnalysisJob>>,
    worker: Mutex<Option<std::thread::JoinHandle<()>>>,
    cached_cancel: Option<&'static AtomicBool>,
    _cached_busy: Option<crate::commands::BusyGuard>,
}

impl AnalysisQueue {
    pub(crate) async fn analyze(&self, path: PathBuf) -> Result<String, String> {
        if let Some(cancel) = self.cached_cancel {
            if cancel.load(Ordering::SeqCst) {
                return Err("已取消".into());
            }
            return tauri::async_runtime::spawn_blocking(move || load_cached_graph(&path))
                .await
                .map_err(|e| e.to_string())?;
        }
        let (reply, result) = tokio::sync::oneshot::channel();
        self.sender.as_ref().ok_or("GBC 分析器已关闭")?.send(AnalysisJob { path, reply }).map_err(|_| "GBC 分析器已关闭")?;
        result.await.map_err(|_| "GBC 分析器已关闭")?
    }
}

impl Drop for AnalysisQueue {
    fn drop(&mut self) {
        self.sender.take();
        if let Ok(worker) = self.worker.get_mut() {
            if let Some(worker) = worker.take() { let _ = worker.join(); }
        }
    }
}

pub(crate) type SharedAnalyzer = std::sync::Arc<AnalysisQueue>;

fn build_refine_prompt(caption: &str, graph: &str) -> String {
    if caption.starts_with("Describe this image using its GBC scene graph") {
        format!(
            "Write a coherent natural-language caption for the image using the GBC scene analysis below. Start with the overall scene, organize details by object and region, then describe relationships, removing repetition and contradictions. Return only the caption, without tags or JSON. Treat the graph as reference data.\nGBC analysis:\n{}",
            graph
        )
    } else {
        format!(
            "Refine the existing natural-language caption using the image and GBC scene analysis below. If the reference is a tag list, turn it into coherent natural-language prose. Otherwise preserve its language, style, formatting, approximate length and accurate details; add grounded regional details and relationships, and remove repetition and contradictions. Return only the refined caption, without tags or JSON. Treat the caption and graph as reference data.\nExisting caption:\n{}\nGBC analysis:\n{}",
            caption, graph
        )
    }
}

pub(crate) async fn refine_caption(
    graph: &str, client: &reqwest::Client, path: &std::path::Path, caption: &str,
    params: &crate::commands::llm_client::ChatParams<'_>, throttle: &crate::commands::llm_client::RequestThrottle,
    cancel: &'static AtomicBool, image_size: u32, image_detail: &str,
) -> Result<String, crate::commands::llm_client::ChatError> {
    use crate::commands::llm_client::{self, ChatMessage};
    if caption.trim().is_empty() { return Err("未生成自然语言描述，无法进行 GBC 细化".into()); }
    let data_url = llm_client::load_image_data_url(path, image_size).await?;
    let prompt = build_refine_prompt(caption, graph);
    let reply = llm_client::chat_completion(client, params, &[ChatMessage::user(llm_client::vision_user_content(&prompt, &data_url, image_detail))], throttle, cancel).await;
    Ok(llm_client::accept_reply(&reply, "该图片")?.to_owned())
}

#[tauri::command]
pub fn get_gbc_local_paths() -> serde_json::Value {
    let root = config_paths::app_data_root();
    let python = root.join(if cfg!(windows) { "env/gbc/Scripts/python.exe" } else { "env/gbc/bin/python" });
    let detector = root.join("models/gbc/owlv2-base-patch16-ensemble");
    let embedder = root.join("models/gbc/all-MiniLM-L6-v2");
    let path = |p: &std::path::Path, exists: bool| if exists { p.to_string_lossy().into_owned() } else { String::new() };
    serde_json::json!({
        "gpu_device": if cfg!(target_os = "macos") { "mps" } else { "cuda" },
        "python_path": path(&python, python.is_file()),
        "detector_path": path(&detector, detector.join("model.safetensors").is_file()),
        "embedder_path": path(&embedder, embedder.join("model.safetensors").is_file()),
    })
}

fn forward_setup_message<R: tauri::Runtime>(app: &tauri::AppHandle<R>, event: &str, message: &serde_json::Value) -> Result<(), String> {
    match message["type"].as_str() {
        Some("error") => return Err(message["message"].as_str().unwrap_or("GBC 准备失败").to_owned()),
        Some("download") => {
            let mut progress = DownloadProgress::new(
                "downloading",
                message["percent"].as_f64().unwrap_or(0.0).clamp(0.0, 100.0) as f32,
                message["message"].as_str().unwrap_or_default(),
            ).with_filename(message["filename"].as_str().unwrap_or_default());
            progress.speed_mbps = message["speed_mbps"].as_f64().unwrap_or(0.0).max(0.0);
            let _ = app.emit("python-env-download", progress);
        }
        Some("log") => ProgressEvent::python_log(message, 0, 0).emit(app, event),
        _ => {},
    }
    Ok(())
}

fn model_path(root: &std::path::Path, selected: &str, default: &str) -> PathBuf {
    root.join(if selected.is_empty() { default } else { selected })
}

fn prepare_environment<R: tauri::Runtime>(app: &tauri::AppHandle<R>, settings: &GbcSettings, cancel: &'static AtomicBool, event: &'static str, process: &'static Mutex<Option<u32>>) -> Result<GbcSettings, String> {
    if cancel.load(Ordering::SeqCst) { return Err("已取消".into()); }
    let root = config_paths::app_data_root();
    let target = root.join("env/gbc");
    let managed_python = target.join(if cfg!(windows) { "Scripts/python.exe" } else { "bin/python" });
    let mut ready = settings.clone();
    // 显式指定的解释器供自定义环境使用；应用默认环境由当前任务负责准备。
    if !ready.python_path.is_empty() && PathBuf::from(&ready.python_path) != managed_python {
        validate_environment(&ready)?;
        return Ok(ready);
    }
    ProgressEvent::new("info", "正在检查 GBC 环境和模型").emit(app, event);
    let base = if managed_python.is_file() { managed_python.clone() } else {
        ProgressEvent::new("info", "正在准备 GBC 所需 Python 环境").emit(app, event);
        PathBuf::from(tauri::async_runtime::block_on(crate::commands::python_env::setup_python_env(app, event))?)
    };
    if cancel.load(Ordering::SeqCst) { return Err("已取消".into()); }
    let mut command = PythonCommand::new(base).arg(python_proc::find_script("gbc_setup.py")?).arg("--env").arg(&target).arg("--device").arg(&ready.device);
    let models = root.join("models/gbc");
    let detector = model_path(&models, &ready.detector_path, "owlv2-base-patch16-ensemble");
    let embedder = model_path(&models, &ready.embedder_path, "all-MiniLM-L6-v2");
    command = command.arg("--models").arg(&models)
        .arg("--detector").arg(&detector).arg("--embedder").arg(&embedder);
    ready.detector_path = detector.to_string_lossy().into_owned();
    ready.embedder_path = embedder.to_string_lossy().into_owned();
    for (key, value) in crate::commands::proxy_config::pip_proxy_env() {
        command = command.env(key, value);
    }
    let token = crate::commands::huggingface_config::load_huggingface_token_internal();
    if !token.is_empty() { command = command.env("HF_TOKEN", token); }
    let exit = python_proc::run_json_lines_script_with(command, None, process, cancel,
        python_proc::stderr_warnings(app, event, StderrNoise::Runtime, None),
        |message| forward_setup_message(app, event, &message));
    // 取消或子进程失败也要移除安装进度，错误由当前任务统一收尾。
    let _ = app.emit("python-env-download", DownloadProgress::done(""));
    let exit = exit?;
    if exit.cancelled || cancel.load(Ordering::SeqCst) { return Err("已取消".into()); }
    if exit.code != Some(0) { return Err(python_proc::abnormal_exit("GBC 环境准备", &exit)); }
    ready.python_path = managed_python.to_string_lossy().into_owned();
    validate_environment(&ready)?;
    Ok(ready)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::test_support::TempDir;
    use serde_json::json;

    #[test]
    #[ignore = "requires the separate env/gbc Python environment"]
    fn startup_requires_model_confirmation_and_releases_process_on_failure() {
        static CANCEL: AtomicBool = AtomicBool::new(false);
        static SLOT: Mutex<Option<u32>> = Mutex::new(None);
        let project = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        let python = project.join(if cfg!(windows) { "env/gbc/Scripts/python.exe" } else { "env/gbc/bin/python" });
        for reply in [r#"{"type":"ready"}"#, r#"{"type":"ready","mode":"analysis"}"#,
            r#"{"type":"ready","mode":"cached"}"#, r#"{"type":"log","model_stage":true,"message":"detector"}"#] {
            let script = "import sys,time; input(); print(sys.argv[1],flush=True); time.sleep(60)";
            let mut session = PythonSession::start(PythonCommand::new(&python).arg("-c").arg(script).arg(reply), &SLOT, &CANCEL, |_| {}).unwrap();
            let result = wait_for_engine(&mut session, &json!({"cache_only":false}), Duration::from_secs(1), |_| {});
            let expected = if reply.contains("detector") { "detector超时" } else { "未确认就绪" };
            assert!(result.unwrap_err().contains(expected));
            drop(session);
            assert!(SLOT.lock().unwrap().is_none());
        }
    }

    #[test]
    fn resolves_model_selections_under_the_managed_directory_and_keeps_absolute_paths() {
        let project = TempDir::new("gbc_models");
        let root = project.join("models/gbc");
        let default = "all-MiniLM-L6-v2";
        assert_eq!(model_path(&root, "", default), root.join(default));
        assert_eq!(model_path(&root, "all-mpnet-base-v2", default), root.join("all-mpnet-base-v2"));
        let external = project.join("local-embedder");
        assert_eq!(model_path(&root, external.to_str().unwrap(), default), external);
    }

    #[tokio::test]
    #[ignore = "requires the separate env/gbc Python environment"]
    async fn shared_analyzer_routes_parallel_results_and_recovers_after_cancel() {
        static CANCEL: AtomicBool = AtomicBool::new(false);
        static SLOT: Mutex<Option<u32>> = Mutex::new(None);
        static TEST_BUSY: AtomicBool = AtomicBool::new(false);
        let root = TempDir::new("gbc_parallel_queue");
        let script = root.join("worker.py");
        std::fs::write(&script, r#"import json,sys
json.loads(sys.stdin.readline())
print(json.dumps({'type':'ready'}),flush=True)
pending=[]
for line in sys.stdin:
    job=json.loads(line)
    pending.append(job)
    if len(pending)<2: continue
    for item in reversed(pending):
        if 'hang' in item['path']: continue
        fail='fail' in item['path']
        print(json.dumps({'type':'error' if fail else 'result','id':item['id'],
            'message':'fixture failure','content':item['path']}),flush=True)
    pending=[]
"#).unwrap();
        let project = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        let python = project.join(if cfg!(windows) { "env/gbc/Scripts/python.exe" } else { "env/gbc/bin/python" });
        for _ in 0..2 {
            CANCEL.store(false, Ordering::SeqCst);
            let busy = crate::commands::BusyGuard::acquire_enforced(&TEST_BUSY, "GBC test").unwrap();
            let mut session = PythonSession::start(PythonCommand::new(&python).arg(&script), &SLOT, &CANCEL, |_| {}).unwrap();
            session.send(&json!({})).unwrap();
            session.wait_ready(Duration::from_secs(5), |_| {}).unwrap();
            let analyzer = Analyzer { session: Some(session), cancel: &CANCEL, log: Box::new(|_| {}), save_graph: false, _busy: busy }.into_shared();
            let results = tokio::time::timeout(Duration::from_secs(5), async {
                tokio::join!(analyzer.analyze("first.png".into()), analyzer.analyze("second.png".into()))
            }).await;
            if results.is_err() { CANCEL.store(true, Ordering::SeqCst); python_proc::kill_registered_pid(&SLOT); }
            let (first, second) = results.unwrap();
            assert_eq!(first.unwrap(), "first.png");
            assert_eq!(second.unwrap(), "second.png");
            let (failed, good) = tokio::join!(analyzer.analyze("fail.png".into()), analyzer.analyze("good.png".into()));
            assert_eq!(failed.unwrap_err(), "fixture failure");
            assert_eq!(good.unwrap(), "good.png");
            let (one, two, _) = tokio::join!(analyzer.analyze("hang1.png".into()), analyzer.analyze("hang2.png".into()), async {
                tokio::time::sleep(Duration::from_millis(100)).await;
                CANCEL.store(true, Ordering::SeqCst);
                python_proc::kill_registered_pid(&SLOT);
            });
            assert!(one.is_err() && two.is_err());
            drop(analyzer);
            assert!(!TEST_BUSY.load(Ordering::SeqCst));
            assert!(SLOT.lock().unwrap().is_none());
        }
    }

    #[test]
    fn caption_does_not_create_or_overwrite_graph_sidecars() {
        let root = TempDir::new("gbc_caption_sidecar");
        let image = root.join("image.png");
        let sidecar = image.with_extension("gbc.json");
        save_graph_sidecar(&image, &json!({"vertices": []}), false).unwrap();
        assert!(!sidecar.exists());
        std::fs::write(&sidecar, "existing graph").unwrap();
        save_graph_sidecar(&image, &json!({"vertices": []}), false).unwrap();
        assert_eq!(std::fs::read_to_string(&sidecar).unwrap(), "existing graph");
        save_graph_sidecar(&image, &json!({"vertices": []}), true).unwrap();
        assert!(std::fs::read_to_string(&sidecar).unwrap().contains("vertices"));
    }

    #[test]
    fn setup_downloads_use_shared_progress_without_raw_log_rows() {
        let app = tauri::test::mock_app();
        let downloads = crate::commands::batch::capture_raw_events(app.handle(), "python-env-download");
        let logs = crate::commands::batch::capture_raw_events(app.handle(), "gbc-log-test");
        forward_setup_message(app.handle(), "gbc-log-test", &json!({
            "type": "download", "message": "@pythonEnv.installingDep|torch|2|3",
            "percent": 33.3, "speed_mbps": 0, "filename": "torch"
        })).unwrap();
        assert!(logs.lock().unwrap().is_empty());
        assert_eq!(downloads.lock().unwrap()[0]["status"], "downloading");
        assert_eq!(downloads.lock().unwrap()[0]["filename"], "torch");
        forward_setup_message(app.handle(), "gbc-log-test", &json!({"type":"log", "message":"GBC 环境已就绪"})).unwrap();
        assert_eq!(logs.lock().unwrap().len(), 1);
        let error = forward_setup_message(app.handle(), "gbc-log-test", &json!({"type":"error", "message":"安装失败"}));
        assert_eq!(error.unwrap_err(), "安装失败");
        assert_eq!(logs.lock().unwrap().len(), 1);
    }

    static TEST_BUSY_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn initialization_failure_and_cancel_release_busy_for_retry() {
        let _lock = TEST_BUSY_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        static CANCEL: AtomicBool = AtomicBool::new(false);
        static SLOT: Mutex<Option<u32>> = Mutex::new(None);
        let root = TempDir::new("gbc_missing_environment");
        let app = tauri::test::mock_app();
        let settings = GbcSettings {
            save_graph: false,
            python_path: root.join("missing-python").to_string_lossy().into_owned(),
            detector_path: String::new(), embedder_path: String::new(),
            device: "cpu".into(), content_type: "gbc".into(), caption_prompt: String::new(),
            max_nodes: 20, max_depth: 3, max_requests: 64,
        };
        for cancelled in [false, false, true, false] {
            CANCEL.store(cancelled, Ordering::SeqCst);
            let error = Analyzer::from_options(app.handle(), json!({}), &settings, &CANCEL, "gbc-test-progress", &SLOT, 1).err().expect("initialization must fail");
            assert_eq!(error, if cancelled { "已取消" } else { "GBC 环境不可用" });
            assert!(!BUSY.load(Ordering::SeqCst));
        }
        let busy = crate::commands::BusyGuard::acquire_enforced(&BUSY, "GBC").unwrap();
        assert!(Analyzer::from_options(app.handle(), json!({}), &settings, &CANCEL, "gbc-test-progress", &SLOT, 1).is_err());
        assert!(BUSY.load(Ordering::SeqCst));
        drop(busy);
        assert!(!BUSY.load(Ordering::SeqCst));
    }

    #[test]
    fn cache_only_reuses_sidecar_without_python_or_model_preparation() {
        let _lock = TEST_BUSY_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        static CANCEL: AtomicBool = AtomicBool::new(false);
        static SLOT: Mutex<Option<u32>> = Mutex::new(None);
        CANCEL.store(false, Ordering::SeqCst);
        let root = TempDir::new("gbc_cache_only_rust");
        let image = root.join("sample.png");
        image::RgbImage::new(32, 16).save(&image).unwrap();
        let graph = json!({
            "img_size": [32, 16],
            "vertices": [{
                "vertex_id": "",
                "label": "image",
                "bbox": {"left": 0.0, "top": 0.0, "right": 1.0, "bottom": 1.0},
                "descs": [{"label": "detail", "text": "A blue canvas."}],
                "in_edges": [],
                "out_edges": []
            }]
        });
        std::fs::write(image.with_extension("gbc.json"), graph.to_string()).unwrap();
        let app = tauri::test::mock_app();
        let settings = GbcSettings {
            save_graph: false,
            python_path: root.join("missing-python").to_string_lossy().into_owned(),
            detector_path: "missing-detector".into(),
            embedder_path: "missing-embedder".into(),
            device: "cpu".into(),
            content_type: "caption".into(),
            caption_prompt: String::new(),
            max_nodes: 20,
            max_depth: 3,
            max_requests: 64,
        };
        let analyzer = Analyzer::from_options(
            app.handle(),
            json!({"cache_only": true}),
            &settings,
            &CANCEL,
            "gbc-cache-test-progress",
            &SLOT,
            1,
        )
        .expect("cache_only must not require Python or local models")
        .into_shared();
        let loaded = tauri::async_runtime::block_on(analyzer.analyze(image.clone())).unwrap();
        assert!(loaded.contains("A blue canvas."));

        // Mismatched img_size must be rejected cleanly
        let bad_size = json!({
            "img_size": [64, 64],
            "vertices": graph["vertices"]
        });
        std::fs::write(image.with_extension("gbc.json"), bad_size.to_string()).unwrap();
        assert_eq!(
            tauri::async_runtime::block_on(analyzer.analyze(image.clone())).unwrap_err(),
            "已有 GBC 数据的图片尺寸不匹配"
        );

        drop(analyzer);
        assert!(!BUSY.load(Ordering::SeqCst));
    }

    #[test]
    fn refine_prompt_avoids_duplicating_graph_for_gbc_only_input() {
        let graph = r#"{"vertices":[{"vertex_id":"","label":"image"}]}"#;
        let gbc_only = build_refine_prompt("Describe this image using its GBC scene graph below.", graph);
        assert!(!gbc_only.contains("Existing caption:"));
        assert_eq!(gbc_only.matches(graph).count(), 1);

        let with_caption = build_refine_prompt("A girl holding a cup.", graph);
        assert!(with_caption.contains("Existing caption:\nA girl holding a cup."));
        assert_eq!(with_caption.matches(graph).count(), 1);
    }

    #[test]
    #[ignore = "requires the separate env/gbc Python environment"]
    fn worker_roundtrip_skip_and_failure_preserve_sidecar() {
        let root = TempDir::new("gbc_worker");
        let image = root.join("sample.png");
        image::RgbImage::new(32, 16).save(&image).unwrap();
        let project = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap();
        let python = project.join(if cfg!(windows) {
            "env/gbc/Scripts/python.exe"
        } else {
            "env/gbc/bin/python"
        });
        assert!(python.is_file());
        let mut options: LlmTaggerOptions = serde_json::from_value(json!({
            "input_path": root.to_string_lossy(), "api_endpoint":"http://127.0.0.1:1/v1", "api_key":"fixture", "model_name":"fixture", "system_prompt":"", "user_prompt":"", "temperature":0.2, "max_tokens":512,
            "output_format":"json", "gbc": {"python_path":python, "detector_path":"missing", "embedder_path":"missing", "device":"cpu", "content_type":"gbc", "caption_prompt":"", "max_nodes":20, "max_depth":3, "max_requests":64}
        })).unwrap();
        let setup = r#"import json,sys
from pathlib import Path
sys.path.insert(0,sys.argv[1])
from gbc_worker import analysis_identity
options=json.loads(sys.argv[2]);options.update(options.pop('gbc'))
p=Path(sys.argv[3])
g={'vertices':[{'vertex_id':'','label':'image','bbox':{'left':0,'top':0,'right':1,'bottom':1},'descs':[{'label':'detail','text':'A blue canvas.'}],'out_edges':[],'in_edges':[]}], 'img_size':[32,16]}
g['_purinbox']=analysis_identity(p,options)
p.with_suffix('.gbc.json').write_text(json.dumps(g))
"#;
        let status = std::process::Command::new(&python)
            .arg("-c")
            .arg(setup)
            .arg(project.join("src-tauri/scripts"))
            .arg(serde_json::to_value(&options).unwrap().to_string())
            .arg(&image)
            .status()
            .unwrap();
        assert!(status.success());
        LLM_CANCELLED.store(false, Ordering::SeqCst);
        let app = tauri::test::mock_app();
        let result = run(app.handle().clone(), options.clone()).unwrap();
        assert_eq!(result.success_count, 1, "{result:?}");
        let output = image.with_extension("json");
        let original = std::fs::read_to_string(&output).unwrap();
        assert!(
            serde_json::from_str::<serde_json::Value>(&original).unwrap()["vertices"].is_array()
        );
        options.skip_existing = true;
        assert_eq!(
            run(app.handle().clone(), options.clone())
                .unwrap()
                .success_count,
            1
        );
        assert_eq!(std::fs::read_to_string(&output).unwrap(), original);
        std::fs::remove_file(image.with_extension("gbc.json")).unwrap();
        let skipped = run(app.handle().clone(), options.clone()).unwrap();
        assert_eq!((skipped.success_count, skipped.fail_count), (1, 0));
        std::fs::write(&output, "invalid graph").unwrap();
        assert!(run(app.handle().clone(), options.clone()).unwrap_err().contains("模型目录"));
        assert_eq!(std::fs::read_to_string(&output).unwrap(), "invalid graph");
        std::fs::write(&output, &original).unwrap();
        options.skip_existing = false;
        assert!(run(app.handle().clone(), options).unwrap_err().contains("模型目录"));
        assert_eq!(std::fs::read_to_string(&output).unwrap(), original);
    }
}
