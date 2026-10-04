use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::time::Duration;
use tauri::Emitter;

use super::http_download::{
    download_client, download_files, huggingface_url, DownloadFile, DownloadFilesOptions,
    DownloadProgress,
};
use super::python_proc::{self, PythonCommand, SessionError, PYTHON_SILENCE_LIMIT};
use super::python_task::SessionControls;
use super::{ProcessResult, ProgressEvent};

const EVENT: &str = "aesthetic-progress";
const DOWNLOAD_EVENT: &str = "aesthetic-download";
const MODEL_REPO: &str = "deepghs/anime_aesthetic";
const MODEL_NAME: &str = "swinv2pv3_v0_448_ls0.2_x";
const MODEL_FILES: [&str; 2] = ["model.onnx", "meta.json"];
/// 加载模型（含 GPU 初始化）的总时限
const READY_TIMEOUT: Duration = Duration::from_secs(180);

/// 美学评分选项
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AestheticOptions {
    pub input_path: String,
    #[serde(default)]
    pub output_path: String,
    #[serde(default)]
    pub use_gpu: bool,
    /// 复制而非移动（工作流用：输出是临时目录时移动会让原图随清理被删）
    #[serde(default)]
    pub copy_files: bool,
    #[serde(default = "default_batch_size")]
    pub batch_size: u32,
    #[serde(default)]
    pub recursive: bool,
}

fn default_batch_size() -> u32 {
    1
}

static RUNNING: AtomicBool = AtomicBool::new(false);
static CONTROLS: SessionControls = SessionControls::new("aesthetic");

fn get_aesthetic_model_dir() -> PathBuf {
    super::config_paths::models_dir("aesthetic_models").join(MODEL_NAME)
}

fn is_model_downloaded() -> bool {
    let dir = get_aesthetic_model_dir();
    MODEL_FILES.iter().all(|name| dir.join(name).is_file())
}

// ===== 美学评分 =====

async fn download_model(app: &tauri::AppHandle) -> Result<(), String> {
    ProgressEvent::new("info", "开始下载美学评分模型...").emit(app, EVENT);
    let client = download_client()?;
    let model_dir = get_aesthetic_model_dir();
    let files = MODEL_FILES
        .iter()
        .map(|name| {
            let url = huggingface_url(MODEL_REPO, &format!("{}/{}", MODEL_NAME, name));
            DownloadFile::new(client.get(url), model_dir.join(name), *name)
        })
        .collect();
    // 已完整下载的文件保留，重试时只补缺失的
    let options = DownloadFilesOptions {
        skip_existing: true,
        ..Default::default()
    };
    let emit = |progress: DownloadProgress| {
        let _ = app.emit(DOWNLOAD_EVENT, progress);
    };
    if let Err(error) = download_files(files, options, CONTROLS.cancel_flag(), emit).await {
        emit(DownloadProgress::from_error(&error, CONTROLS.cancel_flag()));
        return Err(error.into());
    }
    emit(DownloadProgress::done("美学评分模型下载完成"));
    ProgressEvent::new("success", "美学评分模型下载完成").emit(app, EVENT);
    Ok(())
}

fn run_aesthetic_scoring<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    controls: &SessionControls,
    python: &str,
    options: &AestheticOptions,
    model_path: &Path,
    files: &[PathBuf],
    silence: Duration,
) -> Result<ProcessResult, String> {
    let total = files.len() as u32;
    let mut result = ProcessResult {
        total,
        ..Default::default()
    };
    if controls.is_cancelled() {
        return Ok(result);
    }
    let script = python_proc::find_script("aesthetic_inference.py")?;
    let init = serde_json::json!({"cmd": "init", "model_path": model_path.to_string_lossy(), "use_gpu": options.use_gpu});
    let Some(mut session) = controls.open_session(
        app,
        EVENT,
        PythonCommand::new(python)
            .arg(&script)
            .use_gpu(options.use_gpu),
        &init,
        READY_TIMEOUT,
        total,
    )?
    else {
        return Ok(result);
    };
    ProgressEvent::new("info", format!("读取到 {} 张图片", total))
        .at(0, total)
        .emit(app, EVENT);
    let input_dir = Path::new(&options.input_path);
    let batch_size = options.batch_size.max(1) as usize;
    let mut stop = None;
    'batches: for (batch_index, batch) in files.chunks(batch_size).enumerate() {
        if controls.is_cancelled() {
            break;
        }
        let offset = batch_index * batch_size;
        let first_name = super::file_name_lossy(&batch[0]);
        let message = if batch.len() > 1 {
            format!(
                "正在评分: {} 等 {} 张 ({}/{})",
                first_name,
                batch.len(),
                offset + 1,
                total
            )
        } else {
            format!("正在评分: {} ({}/{})", first_name, offset + 1, total)
        };
        ProgressEvent::new("processing", message)
            .at(offset as u32 + 1, total)
            .file(&first_name)
            .emit(app, EVENT);
        let images: Vec<_> = batch
            .iter()
            .map(|path| {
                let relative_dir = if options.output_path.is_empty() {
                    String::new()
                } else {
                    super::relative_dir_for_input(input_dir, path, options.recursive)
                        .map(|p| p.to_string_lossy().replace('\\', "/"))
                        .unwrap_or_default()
                };
                serde_json::json!({
                    "image_path": path.to_string_lossy(), "copy_files": options.copy_files,
                    "output_path": options.output_path, "relative_dir": relative_dir,
                })
            })
            .collect();
        let mut pending: HashSet<String> = batch
            .iter()
            .map(|p| p.to_string_lossy().into_owned())
            .collect();
        if let Err(e) = session.send(&serde_json::json!({"cmd": "score_batch", "images": images})) {
            stop = Some(e);
            break;
        }
        // 普通取消不打断这一批：等它们写完，下一批之前再停
        while !pending.is_empty() {
            let msg = match session.recv(silence) {
                Ok(msg) => msg,
                Err(e) => {
                    stop = Some(e);
                    break 'batches;
                }
            };
            let kind = msg["type"].as_str().unwrap_or("");
            if kind == "log" {
                ProgressEvent::python_log(
                    &msg,
                    result.success_count + result.fail_count + 1,
                    total,
                )
                .emit(app, EVENT);
                continue;
            }
            if !matches!(kind, "result" | "error") {
                continue;
            }
            let Some(path) = msg["image_path"].as_str() else {
                if kind == "error" {
                    // 不带 image_path 的 error 是脚本级错误，这一批不会再有结果
                    let message = msg["message"].as_str().unwrap_or("Python 处理失败");
                    stop = Some(SessionError::Script(message.into()));
                    break 'batches;
                }
                continue;
            };
            if !pending.remove(path) {
                continue;
            }
            let filename = super::file_name_lossy(Path::new(path));
            let (status, message) = if kind == "result" {
                result.success_count += 1;
                (
                    "success",
                    format!(
                        "[完成] {} → {} (分数: {:.2}, 置信度: {:.1}%)",
                        filename,
                        msg["label"].as_str().unwrap_or("?"),
                        msg["score"].as_f64().unwrap_or(0.0),
                        msg["confidence"].as_f64().unwrap_or(0.0) * 100.0
                    ),
                )
            } else {
                result.fail_count += 1;
                let text = msg["message"].as_str().unwrap_or("unknown");
                result.errors.push(format!("{}: {}", filename, text));
                ("error", format!("[错误] {}: {}", filename, text))
            };
            ProgressEvent::new(status, message)
                .at(result.success_count + result.fail_count, total)
                .file(filename)
                .emit(app, EVENT);
        }
    }
    controls.finish_session(session, stop, app, EVENT, "美学评分", &result)?;
    Ok(result)
}

#[tauri::command]
pub async fn start_aesthetic_scoring(
    app: tauri::AppHandle,
    options: AestheticOptions,
) -> Result<ProcessResult, String> {
    let _busy = super::BusyGuard::acquire(&RUNNING, "美学评分")?;
    CONTROLS.reset();
    super::begin_run(EVENT);
    let scan = options.clone();
    let files = tokio::task::spawn_blocking(move || {
        let output = (!scan.output_path.is_empty()).then(|| Path::new(&scan.output_path));
        super::collect_image_files_with_recursive_excluding(
            Path::new(&scan.input_path),
            scan.recursive,
            output,
        )
    })
    .await
    .map_err(|e| format!("读取图片失败: {}", e))??;
    let total = files.len() as u32;
    let outcome = async {
        if total == 0 || CONTROLS.is_cancelled() {
            return Ok(ProcessResult {
                total,
                ..Default::default()
            });
        }
        let python = CONTROLS.prepare_python(&app).await?;
        if !is_model_downloaded() {
            download_model(&app).await?;
        }
        let model_path = get_aesthetic_model_dir().join(MODEL_FILES[0]);
        let app_run = app.clone();
        tokio::task::spawn_blocking(move || {
            run_aesthetic_scoring(
                &app_run,
                &CONTROLS,
                &python,
                &options,
                &model_path,
                &files,
                PYTHON_SILENCE_LIMIT,
            )
        })
        .await
        .map_err(|e| format!("任务执行失败: {}", e))?
    }
    .await;
    CONTROLS.finish_command(&app, EVENT, total, outcome, "美学评分完成")
}

#[tauri::command]
pub fn cancel_aesthetic_scoring() {
    CONTROLS.cancel();
}

#[tauri::command]
pub fn force_cancel_aesthetic_scoring() {
    CONTROLS.force_cancel();
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::commands::python_task::session_test::{quit_marker, SessionRun};
    use std::time::Instant;

    /// 读 init、回 ready，再读一条 score_batch，之后执行 `after_batch`
    fn session_script(after_batch: &str) -> String {
        format!(
            "read -r line\ncase \"$line\" in *'\"cmd\":\"init\"'*) ;; *) exit 3;; esac\nprintf '%s\\n' '{{\"type\":\"ready\"}}'\nread -r line\ncase \"$line\" in *score_batch*) ;; *) exit 4;; esac\n{}",
            after_batch
        )
    }

    fn result_line(image: &Path) -> String {
        serde_json::json!({"type": "result", "image_path": image, "label": "good", "score": 3.0, "confidence": 1.0}).to_string()
    }

    fn new_run(tag: &str) -> SessionRun {
        SessionRun::new(tag, EVENT, "aesthetic-test")
    }

    fn score(
        run: &SessionRun,
        body: &str,
        files: &[PathBuf],
        silence: Duration,
    ) -> Result<ProcessResult, String> {
        let options = AestheticOptions {
            input_path: run.root.to_string_lossy().into_owned(),
            output_path: String::new(),
            use_gpu: false,
            copy_files: true,
            batch_size: 1,
            recursive: false,
        };
        run_aesthetic_scoring(
            run.app.handle(),
            &run.controls,
            &run.fake_python(body),
            &options,
            &run.root.join("model.onnx"),
            files,
            silence,
        )
    }

    #[test]
    fn cancel_lets_the_batch_in_hand_finish_and_quits_gracefully() {
        let run = new_run("aesthetic_graceful");
        let (a, b) = (run.root.join("a.png"), run.root.join("b.png"));
        let marker = run.root.join("quit-received");
        let body = session_script(&format!(
            "printf '\\377noise\\n'\nprintf '%s\\n' '{{\"type\":\"result\",\"image_path\":\"other.png\"}}' '{{\"type\":\"log\",\"message\":\"fallback\"}}' '{}'\n{}",
            result_line(&a),
            quit_marker(&marker)
        ));
        run.on_event("[完成]", SessionControls::cancel);
        let result = score(&run, &body, &[a, b], PYTHON_SILENCE_LIMIT).unwrap();
        assert_eq!(
            (result.success_count, result.fail_count, result.total),
            (1, 0, 2)
        );
        assert!(marker.exists(), "进程应收到退出命令后自行退出");
        assert!(!run.controls.has_registered_process());
        assert!(run.messages("info").iter().any(|m| m == "fallback"));
        assert!(run.messages("error").is_empty());
    }

    #[test]
    fn force_cancel_ends_the_process_at_once() {
        let run = new_run("aesthetic_force");
        run.on_event("正在评分", SessionControls::force_cancel);
        let started = Instant::now();
        let result = score(
            &run,
            &session_script("sleep 30\n"),
            &[run.root.join("a.png")],
            PYTHON_SILENCE_LIMIT,
        )
        .unwrap();
        assert!(started.elapsed() < Duration::from_secs(10));
        assert_eq!((result.success_count, result.fail_count), (0, 0));
    }

    #[test]
    fn cancel_while_loading_the_model_ends_the_process_at_once() {
        let run = new_run("aesthetic_cancel_loading");
        run.on_event("loading", SessionControls::cancel);
        let body =
            "read -r line\nprintf '%s\\n' '{\"type\":\"log\",\"message\":\"loading\"}'\nsleep 30\n";
        let started = Instant::now();
        let result = score(&run, body, &[run.root.join("a.png")], PYTHON_SILENCE_LIMIT).unwrap();
        assert!(started.elapsed() < Duration::from_secs(10));
        assert_eq!((result.success_count, result.fail_count), (0, 0));
    }

    #[test]
    fn crash_mid_run_fails_the_rest_and_reports_the_reason() {
        let run = new_run("aesthetic_crash");
        let files: Vec<_> = ["a.png", "b.png", "c.png"]
            .iter()
            .map(|n| run.root.join(n))
            .collect();
        let body = session_script(&format!(
            "printf '%s\\n' '{}'\necho 'Traceback: boom' >&2\nexit 1\n",
            result_line(&files[0])
        ));
        let error = score(&run, &body, &files, PYTHON_SILENCE_LIMIT).unwrap_err();
        assert!(
            error.starts_with("美学评分中断: Python 进程已退出"),
            "{error}"
        );
        assert!(error.contains("boom"), "{error}");
        let errors = run.messages("error");
        assert_eq!(errors.len(), 1);
        assert!(errors[0].starts_with(&error));
        assert!(errors[0].contains("未处理的 2 张记为失败"), "{}", errors[0]);
        let events = run.events.lock().unwrap();
        let last = events.last().unwrap();
        assert_eq!(
            (last["current"].as_u64(), last["total"].as_u64()),
            (Some(3), Some(3))
        );
    }

    #[test]
    fn script_level_error_stops_the_run_and_quits_gracefully() {
        let run = new_run("aesthetic_script_error");
        let marker = run.root.join("quit-received");
        let body = session_script(&format!(
            "printf '%s\\n' '{{\"type\":\"error\",\"message\":\"模型未初始化\"}}'\n{}",
            quit_marker(&marker)
        ));
        let files = [run.root.join("a.png"), run.root.join("b.png")];
        let error = score(&run, &body, &files, PYTHON_SILENCE_LIMIT).unwrap_err();
        assert_eq!(error, "美学评分中断: 模型未初始化");
        assert!(marker.exists());
        assert!(run.messages("error")[0].contains("未处理的 2 张记为失败"));
    }

    #[test]
    fn silent_process_times_out_and_fails_the_rest() {
        let run = new_run("aesthetic_silent");
        let started = Instant::now();
        let error = score(
            &run,
            &session_script("sleep 30\n"),
            &[run.root.join("a.png")],
            Duration::from_millis(300),
        )
        .unwrap_err();
        assert!(started.elapsed() < Duration::from_secs(10));
        assert!(error.contains("无响应"), "{error}");
        assert!(run.messages("error")[0].contains("未处理的 1 张记为失败"));
    }

    #[test]
    fn init_error_is_reported_as_is() {
        let run = new_run("aesthetic_init_error");
        let body = "read -r line\nprintf '%s\\n' '{\"type\":\"error\",\"message\":\"初始化失败: no model\"}'\nread -r line\n";
        let error = score(&run, body, &[run.root.join("a.png")], PYTHON_SILENCE_LIMIT).unwrap_err();
        assert_eq!(error, "初始化失败: no model");
    }
}
