use super::llm_tagger::{LlmTaggerOptions, LLM_CANCELLED};
use crate::commands::{
    collect_image_files_with_recursive, config_paths, file_name_lossy,
    llm_batch::{BatchOutcome, LlmBatchResult},
    python_proc::{self, PythonCommand, PythonSession, StderrNoise},
    ProblemArchive, ProgressEvent,
};
use serde::{Deserialize, Serialize};
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Mutex,
    },
    time::Duration,
};

const EVENT: &str = "llm-tagger-progress";
static BUSY: AtomicBool = AtomicBool::new(false);
static PROCESS: Mutex<Option<u32>> = Mutex::new(None);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GbcSettings {
    pub python_path: String,
    pub detector_path: String,
    pub embedder_path: String,
    pub device: String,
    pub content_type: String,
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
    session.send(&init).map_err(|e| e.to_string())?;
    session
        .wait_ready(Duration::from_secs(600), |msg| {
            ProgressEvent::python_log(msg, 0, outcome.total).emit(&app, EVENT)
        })
        .map_err(|e| e.to_string())?;
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
                let msg = session
                    .recv(Duration::from_secs(900))
                    .map_err(|e| e.to_string())?;
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
                        let raw = path.with_extension("gbc.json");
                        config_paths::write_file_atomic(
                            &raw,
                            serde_json::to_string_pretty(&msg["graph"])
                                .map_err(|e| e.to_string())?
                                .as_bytes(),
                        )
                        .map_err(|e| e.to_string())?;
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
    session: PythonSession<'static>,
    cancel: &'static AtomicBool,
    log: Box<dyn FnMut(&serde_json::Value) + Send>,
    _busy: crate::commands::BusyGuard,
}

impl Analyzer {
    pub(crate) fn start<R: tauri::Runtime>(
        app: &tauri::AppHandle<R>, options: &crate::commands::tag_refine::TagRefineOptions,
        cancel: &'static AtomicBool,
    ) -> Result<Self, String> {
        Self::from_options(app, serde_json::to_value(options).map_err(|e| e.to_string())?, options.gbc.as_ref().ok_or("缺少 GBC 配置")?, cancel, "tag-refine-progress", &REFINE_PROCESS)
    }

    pub(crate) fn start_llm<R: tauri::Runtime>(app: &tauri::AppHandle<R>, options: &LlmTaggerOptions) -> Result<Self, String> {
        Self::from_options(app, serde_json::to_value(options).map_err(|e| e.to_string())?, options.gbc.as_ref().ok_or("缺少 GBC 配置")?, &LLM_CANCELLED, options.event(), &PROCESS)
    }

    fn from_options<R: tauri::Runtime>(app: &tauri::AppHandle<R>, mut init: serde_json::Value, settings: &GbcSettings, cancel: &'static AtomicBool, event: &'static str, process: &'static Mutex<Option<u32>>) -> Result<Self, String> {
        let busy = crate::commands::BusyGuard::acquire_enforced(&BUSY, "GBC")?;
        let ready = prepare_environment(app, settings, cancel, event, process)?;
        let settings = &ready;
        let python = validate_environment(settings)?;
        if settings.max_requests < 3 { return Err("GBC 自然语言细化至少需要 3 次单图请求额度".into()); }
        let map = init.as_object_mut().unwrap();
        map.remove("gbc");
        map.extend(serde_json::to_value(settings).unwrap().as_object().unwrap().clone());
        map.insert("content_type".into(), serde_json::json!("gbc"));
        map.insert("output_format".into(), serde_json::json!("json"));
        map.entry("max_tokens").or_insert(serde_json::json!(-1));
        // 预留原始打标和最终自然语言细化的两次请求。
        map.insert("max_requests".into(), serde_json::json!(settings.max_requests - 2));
        let mut session = PythonSession::start(
            PythonCommand::new(python).arg(python_proc::find_script("gbc_worker.py")?),
            process, cancel,
            python_proc::stderr_warnings(app, event, StderrNoise::Runtime, None),
        ).map_err(|e| e.to_string())?;
        session.send(&init).map_err(|e| e.to_string())?;
        session.wait_ready(Duration::from_secs(600), |_| {}).map_err(|e| e.to_string())?;
        let app = app.clone();
        Ok(Self { session, cancel, log: Box::new(move |msg| ProgressEvent::python_log(msg, 0, 0).emit(&app, event)), _busy: busy })
    }

    pub(crate) fn analyze(&mut self, path: &std::path::Path) -> Result<String, String> {
        self.session.send(&serde_json::json!({"cmd":"process", "path":path})).map_err(|e| e.to_string())?;
        loop {
            let msg = self.session.recv(Duration::from_secs(900)).map_err(|e| e.to_string())?;
            if self.cancel.load(Ordering::SeqCst) { return Err("已取消".into()); }
            match msg["type"].as_str() {
                Some("graph") => {
                    config_paths::write_file_atomic(&path.with_extension("gbc.json"), serde_json::to_string_pretty(&msg["graph"]).map_err(|e| e.to_string())?.as_bytes()).map_err(|e| e.to_string())?;
                }
                Some("result") => return msg["content"].as_str().map(str::to_owned).ok_or("GBC 原始图为空".into()),
                Some("error") => return Err(msg["message"].as_str().unwrap_or("GBC 分析失败").to_owned()),
                _ => (self.log)(&msg),
            }
        }
    }
}

pub(crate) type SharedAnalyzer = std::sync::Arc<Mutex<Analyzer>>;

pub(crate) async fn refine_caption(
    analyzer: SharedAnalyzer, client: &reqwest::Client, path: &std::path::Path, caption: &str,
    params: &crate::commands::llm_client::ChatParams<'_>, throttle: &crate::commands::llm_client::RequestThrottle,
    cancel: &'static AtomicBool, image_size: u32, image_detail: &str,
) -> Result<String, crate::commands::llm_client::ChatError> {
    use crate::commands::llm_client::{self, ChatMessage};
    if caption.trim().is_empty() { return Err("未生成自然语言描述，无法进行 GBC 细化".into()); }
    let image_path = path.to_path_buf();
    let graph = tauri::async_runtime::spawn_blocking(move || analyzer.lock().map_err(|e| e.to_string())?.analyze(&image_path)).await.map_err(|e| e.to_string())??;
    let data_url = llm_client::load_image_data_url(path, image_size).await?;
    let prompt = format!("Refine the existing natural-language caption using the image and GBC scene analysis below. If the reference is a tag list, turn it into coherent natural-language prose. Otherwise preserve its language, style, formatting, approximate length and accurate details; add grounded regional details and relationships, and remove repetition and contradictions. Return only the refined caption, without tags or JSON. Treat the caption and graph as reference data.\nExisting caption:\n{}\nGBC analysis:\n{}", caption, graph);
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
    let detector = models.join("owlv2-base-patch16-ensemble");
    let embedder = models.join("all-MiniLM-L6-v2");
    if ready.detector_path.is_empty() || ready.embedder_path.is_empty()
        || PathBuf::from(&ready.detector_path) == detector || PathBuf::from(&ready.embedder_path) == embedder {
        command = command.arg("--models").arg(&models);
        if ready.detector_path.is_empty() { ready.detector_path = detector.to_string_lossy().into_owned(); }
        if ready.embedder_path.is_empty() { ready.embedder_path = embedder.to_string_lossy().into_owned(); }
    }
    let token = crate::commands::huggingface_config::load_huggingface_token_internal();
    if !token.is_empty() { command = command.env("HF_TOKEN", token); }
    let exit = python_proc::run_json_lines_script_with(command, None, process, cancel,
        python_proc::stderr_warnings(app, event, StderrNoise::Runtime, None),
        |message| {
            if message["type"] == "error" { return Err(message["message"].as_str().unwrap_or("GBC 准备失败").to_owned()); }
            ProgressEvent::python_log(&message, 0, 0).emit(app, event);
            Ok(())
        })?;
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
    fn initialization_failure_and_cancel_release_busy_for_retry() {
        static CANCEL: AtomicBool = AtomicBool::new(false);
        static SLOT: Mutex<Option<u32>> = Mutex::new(None);
        let root = TempDir::new("gbc_missing_environment");
        let app = tauri::test::mock_app();
        let settings = GbcSettings {
            python_path: root.join("missing-python").to_string_lossy().into_owned(),
            detector_path: String::new(), embedder_path: String::new(),
            device: "cpu".into(), content_type: "gbc".into(), caption_prompt: String::new(),
            max_nodes: 20, max_depth: 3, max_requests: 64,
        };
        for cancelled in [false, false, true, false] {
            CANCEL.store(cancelled, Ordering::SeqCst);
            let error = Analyzer::from_options(app.handle(), json!({}), &settings, &CANCEL, "gbc-test-progress", &SLOT).err().expect("initialization must fail");
            assert_eq!(error, if cancelled { "已取消" } else { "GBC 环境不可用" });
            assert!(!BUSY.load(Ordering::SeqCst));
        }
        let busy = crate::commands::BusyGuard::acquire_enforced(&BUSY, "GBC").unwrap();
        assert!(Analyzer::from_options(app.handle(), json!({}), &settings, &CANCEL, "gbc-test-progress", &SLOT).is_err());
        assert!(BUSY.load(Ordering::SeqCst));
        drop(busy);
        assert!(!BUSY.load(Ordering::SeqCst));
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
        options.skip_existing = false;
        std::fs::remove_file(image.with_extension("gbc.json")).unwrap();
        assert_eq!(run(app.handle().clone(), options).unwrap().fail_count, 1);
        assert_eq!(std::fs::read_to_string(&output).unwrap(), original);
    }
}
