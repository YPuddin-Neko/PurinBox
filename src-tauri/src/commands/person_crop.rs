use image::ImageFormat;
use serde::{Deserialize, Serialize};
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::time::Duration;
use tauri::Emitter;

use super::http_download::{
    download_client, download_files, huggingface_url, DownloadFile, DownloadFilesOptions,
    DownloadProgress,
};
use super::image_io::{probe_header, sniff_format};
use super::python_proc::{self, PythonCommand, PythonSession, SessionError, PYTHON_SILENCE_LIMIT};
use super::python_task::SessionControls;
use super::{
    collect_image_files_with_recursive_excluding, output_dir_for_input, ProcessResult,
    ProgressEvent,
};

const EVENT: &str = "person-crop-progress";
const DOWNLOAD_EVENT: &str = "person-crop-download";
/// 加载全部启用的检测模型（含 GPU 初始化）的总时限
const READY_TIMEOUT: Duration = Duration::from_secs(300);

// ===== DeepGHS Anime Detection Models =====

/// 4 个裁切类型对应的模型定义
struct CropModelDef {
    /// 模型在仓库里的子目录名，也是本地的目录名
    id: &'static str,
    name: &'static str,
    crop_type: &'static str, // "person" | "halfbody" | "head" | "eyes"
    repo: &'static str,
}

const CROP_MODELS: &[CropModelDef] = &[
    CropModelDef {
        id: "person_detect_v1.1_m",
        name: "全身检测 (person_detect_v1.1_m)",
        crop_type: "person",
        repo: "deepghs/anime_person_detection",
    },
    CropModelDef {
        id: "halfbody_detect_v1.0_s",
        name: "半身检测 (halfbody_detect_v1.0_s)",
        crop_type: "halfbody",
        repo: "deepghs/anime_halfbody_detection",
    },
    CropModelDef {
        id: "head_detect_v2.0_x",
        name: "头部检测 (head_detect_v2.0_x)",
        crop_type: "head",
        repo: "deepghs/anime_head_detection",
    },
    CropModelDef {
        id: "eye_detect_v1.0_s",
        name: "眼部检测 (eye_detect_v1.0_s)",
        crop_type: "eyes",
        repo: "deepghs/anime_eye_detection",
    },
];

fn get_models_dir() -> PathBuf {
    super::config_paths::models_dir("crop_models")
}

fn model_onnx_path(model: &CropModelDef) -> PathBuf {
    get_models_dir().join(model.id).join("model.onnx")
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CropModelInfo {
    pub crop_type: String,
    pub downloaded: bool,
}

#[tauri::command]
pub fn get_person_crop_models() -> Result<Vec<CropModelInfo>, String> {
    Ok(CROP_MODELS
        .iter()
        .map(|m| CropModelInfo {
            crop_type: m.crop_type.to_string(),
            downloaded: model_onnx_path(m).exists(),
        })
        .collect())
}

#[tauri::command]
pub async fn download_person_crop_model(app: tauri::AppHandle) -> Result<String, String> {
    let _busy = super::BusyGuard::acquire(&PERSON_CROP_RUNNING, "人物裁切")?;
    CONTROLS.reset();
    let emit = |progress: DownloadProgress| {
        let _ = app.emit(DOWNLOAD_EVENT, progress);
    };
    let missing: Vec<_> = CROP_MODELS
        .iter()
        .filter(|m| !model_onnx_path(m).is_file())
        .collect();
    if missing.is_empty() {
        emit(DownloadProgress::done("所有模型已就绪"));
        return Ok("all_ready".into());
    }
    let client = download_client()?;
    let files = missing
        .iter()
        .enumerate()
        .map(|(index, model)| {
            let url = huggingface_url(model.repo, &format!("{}/model.onnx", model.id));
            let label = format!("[{}/{}] {}", index + 1, missing.len(), model.name);
            DownloadFile::new(client.get(url), model_onnx_path(model), label)
        })
        .collect();
    let options = DownloadFilesOptions {
        skip_existing: true,
        done_each: true,
        ..Default::default()
    };
    match download_files(files, options, CONTROLS.cancel_flag(), emit).await {
        Ok(_) => Ok("done".into()),
        Err(error) => {
            emit(DownloadProgress::from_error(&error, CONTROLS.cancel_flag()));
            Err(error.into())
        }
    }
}

// ===== Person Crop Processing =====

static CONTROLS: SessionControls = SessionControls::new("person-crop");
static PERSON_CROP_RUNNING: AtomicBool = AtomicBool::new(false);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersonCropOptions {
    pub input_path: String,
    pub output_path: String,
    pub use_gpu: bool,
    // person (full body)
    pub person_enabled: bool,
    pub person_conf: f64,
    // upper body
    pub upper_enabled: bool,
    pub upper_conf: f64,
    pub upper_tag: String,
    // head
    pub head_enabled: bool,
    pub head_conf: f64,
    pub head_tag: String,
    pub head_scale: f64,
    // eyes
    pub eyes_enabled: bool,
    pub eyes_conf: f64,
    pub eyes_tag: String,
    pub eyes_scale: f64,
    // other
    pub keep_original_tags: bool,
    #[serde(default)]
    pub recursive: bool,
}

#[tauri::command]
pub async fn start_person_crop(
    app: tauri::AppHandle,
    options: PersonCropOptions,
) -> Result<ProcessResult, String> {
    let _busy = super::BusyGuard::acquire(&PERSON_CROP_RUNNING, "人物裁切")?;
    CONTROLS.reset();
    super::begin_run(EVENT);
    let scan = options.clone();
    let files = tokio::task::spawn_blocking(move || {
        collect_image_files_with_recursive_excluding(
            Path::new(&scan.input_path),
            scan.recursive,
            Some(Path::new(&scan.output_path)),
        )
    })
    .await
    .map_err(|e| format!("读取图片失败: {}", e))??;
    let total = files.len() as u32;
    let outcome = async {
        if files.is_empty() || CONTROLS.is_cancelled() {
            return Ok(ProcessResult {
                total,
                ..Default::default()
            });
        }
        let model_paths = build_model_paths(&options)?;
        let python = CONTROLS.prepare_python(&app).await?;
        let (files, opencv_warning) = prepare_opencv(
            &CONTROLS,
            files,
            async {
                super::python_env::probe_python(&python, "import cv2")
                    .await
                    .is_some()
            },
            || {
                super::python_env::pip_install_for(
                    &app,
                    &python,
                    &[super::upscale::OPENCV_PACKAGE],
                    "person-crop",
                )
            },
        )
        .await?;
        if let Some(warning) = opencv_warning {
            ProgressEvent::new("warning", warning).emit(&app, EVENT);
        }
        ProgressEvent::new("info", "开始裁切...").emit(&app, EVENT);
        let app_run = app.clone();
        tokio::task::spawn_blocking(move || {
            run_person_crop(
                &app_run,
                &CONTROLS,
                &python,
                &options,
                &files,
                model_paths,
                PYTHON_SILENCE_LIMIT,
            )
        })
        .await
        .map_err(|e| format!("任务执行失败: {}", e))?
    }
    .await;
    CONTROLS.finish_command(&app, EVENT, total, outcome, "处理完成")
}

#[tauri::command]
pub fn cancel_person_crop() {
    CONTROLS.cancel();
}

#[tauri::command]
pub fn force_cancel_person_crop() {
    CONTROLS.force_cancel();
}

/// 构建每种裁切类型对应的模型路径映射
fn build_model_paths(options: &PersonCropOptions) -> Result<serde_json::Value, String> {
    let mut paths = serde_json::Map::new();

    let check_model = |crop_type: &str| -> Result<String, String> {
        let model = CROP_MODELS
            .iter()
            .find(|m| m.crop_type == crop_type)
            .ok_or_else(|| format!("未找到 {} 类型模型定义", crop_type))?;
        let path = model_onnx_path(model);
        if !path.exists() {
            return Err(format!("{} 模型未下载，请先下载模型包", model.name));
        }
        Ok(path.to_string_lossy().to_string())
    };

    if options.person_enabled {
        paths.insert(
            "person".into(),
            serde_json::Value::String(check_model("person")?),
        );
    }
    if options.upper_enabled {
        paths.insert(
            "halfbody".into(),
            serde_json::Value::String(check_model("halfbody")?),
        );
    }
    if options.head_enabled {
        paths.insert(
            "head".into(),
            serde_json::Value::String(check_model("head")?),
        );
    }
    if options.eyes_enabled {
        paths.insert(
            "eyes".into(),
            serde_json::Value::String(check_model("eyes")?),
        );
    }

    if paths.is_empty() {
        return Err("请至少启用一种裁切类型".into());
    }

    Ok(serde_json::Value::Object(paths))
}

/// Pillow 会把这张图读成 8 位：16 位的彩色或带透明通道的 PNG/TIFF。
/// 裁切脚本要用 OpenCV 按原位深读写这类图（16 位的纯灰度图 Pillow 能原样读写）
fn pillow_drops_depth(path: &Path) -> bool {
    // 先只读魔数，其他格式不建解码器：JPEG 的解码器会把整个文件读进内存
    matches!(
        sniff_format(path),
        Some(ImageFormat::Png | ImageFormat::Tiff)
    ) && probe_header(path)
        .is_ok_and(|header| header.color.channel_count() > 1 && header.high_bit_depth())
}

/// 处理 Pillow 会读丢位深的图（见 `pillow_drops_depth`）要用 OpenCV。`has_opencv` 为 false 时才扫描输入，
/// 有这类图再调 `install` 安装。安装失败不中止任务，返回要发的警告，这几张图由裁切脚本逐张报错；
/// 取消时返回 Err
async fn prepare_opencv<F>(
    controls: &SessionControls,
    files: Vec<PathBuf>,
    has_opencv: impl Future<Output = bool>,
    install: impl FnOnce() -> F,
) -> Result<(Vec<PathBuf>, Option<String>), String>
where
    F: Future<Output = Result<(), String>>,
{
    if has_opencv.await {
        return Ok((files, None));
    }
    let (files, deep) = tokio::task::spawn_blocking(move || {
        let deep = files.iter().filter(|file| pillow_drops_depth(file)).count();
        (files, deep)
    })
    .await
    .map_err(|e| format!("读取图片失败: {}", e))?;
    if controls.is_cancelled() {
        return Err("已取消".into());
    }
    if deep == 0 {
        return Ok((files, None));
    }
    match install().await {
        Ok(()) => Ok((files, None)),
        Err(e) if controls.is_cancelled() => Err(e),
        Err(e) => {
            let warning = format!(
                "安装 OpenCV（opencv-python-headless）失败，{} 张 16 位的彩色或带透明通道的图片将无法处理: {}",
                deep, e
            );
            Ok((files, Some(warning)))
        }
    }
}

/// 等 `image_path` 的结果：result，或带这个 image_path 的 error。期间的 log 交给 `on_log`，
/// 其他图片的消息忽略；不带 image_path 的 error 是脚本级错误，返回 `SessionError::Script`
fn wait_image_result(
    session: &mut PythonSession<'_>,
    image_path: &str,
    silence: Duration,
    mut on_log: impl FnMut(&serde_json::Value),
) -> Result<serde_json::Value, SessionError> {
    loop {
        let msg = session.recv(silence)?;
        match msg["type"].as_str().unwrap_or("") {
            "log" => on_log(&msg),
            "result" | "error" if msg["image_path"].as_str() == Some(image_path) => return Ok(msg),
            "error" if msg.get("image_path").is_none() => {
                return Err(SessionError::Script(
                    msg["message"].as_str().unwrap_or("Python 处理失败").into(),
                ))
            }
            _ => {}
        }
    }
}

fn run_person_crop<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    controls: &SessionControls,
    python: &str,
    options: &PersonCropOptions,
    files: &[PathBuf],
    model_paths: serde_json::Value,
    silence: Duration,
) -> Result<ProcessResult, String> {
    let script = python_proc::find_script("person_crop.py")?;
    let input = Path::new(&options.input_path);
    let output_dir = Path::new(&options.output_path);
    let total = files.len() as u32;
    let mut result = ProcessResult {
        total,
        ..Default::default()
    };
    if controls.is_cancelled() {
        return Ok(result);
    }
    std::fs::create_dir_all(output_dir).map_err(|e| format!("无法创建输出目录: {}", e))?;
    ProgressEvent::new(
        "processing",
        format!("正在启动 Python 环境... (共 {} 张图片)", total),
    )
    .at(0, total)
    .emit(app, EVENT);
    let init = serde_json::json!({
        "cmd": "init", "model_paths": model_paths, "use_gpu": options.use_gpu,
        "options": {
            "person_conf": options.person_conf, "upper_conf": options.upper_conf,
            "upper_tag": options.upper_tag, "head_conf": options.head_conf,
            "head_tag": options.head_tag, "head_scale": options.head_scale,
            "eyes_conf": options.eyes_conf, "eyes_tag": options.eyes_tag,
            "eyes_scale": options.eyes_scale, "keep_original_tags": options.keep_original_tags,
        },
    });
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
    ProgressEvent::new("processing", "模型已加载，开始处理...")
        .at(0, total)
        .emit(app, EVENT);
    let mut stop = None;
    for (i, file_path) in files.iter().enumerate() {
        if controls.is_cancelled() {
            break;
        }
        let filename = super::file_name_lossy(file_path);
        let current = i as u32 + 1;
        ProgressEvent::new("processing", format!("正在处理: {}", filename))
            .at(current, total)
            .file(&filename)
            .emit(app, EVENT);
        let fail = |result: &mut ProcessResult, detail: &str| {
            result.fail_count += 1;
            result.errors.push(format!("{}: {}", filename, detail));
            ProgressEvent::new("error", format!("[失败] {} — {}", filename, detail))
                .at(current, total)
                .file(&filename)
                .emit(app, EVENT);
        };
        let target = match output_dir_for_input(input, file_path, output_dir, options.recursive) {
            Ok(target) => target,
            Err(error) => {
                fail(&mut result, &error);
                continue;
            }
        };
        let image_path = file_path.to_string_lossy();
        let command = serde_json::json!({
            "cmd": "process", "image_path": image_path, "output_dir": target.to_string_lossy(),
        });
        // 普通取消不打断这张图：等它写完，下一张之前再停
        let reply = session.send(&command).and_then(|()| {
            wait_image_result(&mut session, &image_path, silence, |msg| {
                ProgressEvent::python_log(msg, current, total)
                    .file(&filename)
                    .emit(app, EVENT);
            })
        });
        let msg = match reply {
            Ok(msg) => msg,
            Err(e) => {
                stop = Some(e);
                break;
            }
        };
        let detail = msg["message"].as_str().unwrap_or("");
        match (msg["type"].as_str(), msg["status"].as_str()) {
            (Some("result"), Some(status @ ("success" | "skip"))) => {
                result.success_count += 1;
                let label = if status == "skip" { "跳过" } else { "成功" };
                ProgressEvent::new("success", format!("[{}] {} — {}", label, filename, detail))
                    .at(current, total)
                    .file(&filename)
                    .emit(app, EVENT);
            }
            _ => fail(&mut result, detail),
        }
    }
    controls.finish_session(session, stop, app, EVENT, "人物裁切", &result)?;
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::test_support::TempDir;

    fn write_image(path: &Path, img: image::DynamicImage) {
        img.save(path).unwrap();
    }

    #[test]
    fn only_deep_color_or_alpha_images_need_opencv() {
        let root = TempDir::new("crop_depth");
        let cases = [
            ("rgb16.png", image::DynamicImage::new_rgb16(4, 4), true),
            ("rgba16.png", image::DynamicImage::new_rgba16(4, 4), true),
            ("la16.png", image::DynamicImage::new_luma_a16(4, 4), true),
            ("rgb16.tif", image::DynamicImage::new_rgb16(4, 4), true),
            ("gray16.png", image::DynamicImage::new_luma16(4, 4), false),
            ("rgb8.png", image::DynamicImage::new_rgb8(4, 4), false),
            ("rgb8.jpg", image::DynamicImage::new_rgb8(4, 4), false),
        ];
        for (name, img, expected) in cases {
            let path = root.join(name);
            write_image(&path, img);
            assert_eq!(pillow_drops_depth(&path), expected, "{name}");
        }
        std::fs::write(root.join("broken.png"), b"not an image").unwrap();
        assert!(!pillow_drops_depth(&root.join("broken.png")));
    }

    #[tokio::test]
    async fn opencv_is_installed_only_when_missing_and_needed_and_failure_only_warns() {
        let root = TempDir::new("crop_opencv");
        let (deep, plain) = (root.join("deep.png"), root.join("plain.png"));
        write_image(&deep, image::DynamicImage::new_rgb16(4, 4));
        write_image(&plain, image::DynamicImage::new_rgb8(4, 4));
        let both = vec![deep.clone(), plain.clone()];
        let controls = &SessionControls::new("person-crop-opencv-test");
        let never = || async { panic!("不该安装") };

        // 已有 OpenCV，或没有需要它的图：不装
        let (files, warning) = prepare_opencv(controls, both.clone(), async { true }, never)
            .await
            .unwrap();
        assert_eq!((files, warning), (both.clone(), None));
        let (_, warning) = prepare_opencv(controls, vec![plain.clone()], async { false }, never)
            .await
            .unwrap();
        assert_eq!(warning, None);

        let (_, warning) =
            prepare_opencv(controls, both.clone(), async { false }, || async { Ok(()) })
                .await
                .unwrap();
        assert_eq!(warning, None);

        // 安装失败只发警告，图片照常交给脚本
        let (files, warning) = prepare_opencv(controls, both.clone(), async { false }, || async {
            Err("网络错误".to_string())
        })
        .await
        .unwrap();
        assert_eq!(files, both);
        let warning = warning.expect("安装失败应给出警告");
        assert!(
            warning.contains("1 张") && warning.ends_with("网络错误"),
            "{warning}"
        );

        // 安装被取消：照旧按取消结束
        let cancelled = prepare_opencv(controls, both.clone(), async { false }, || async move {
            controls.cancel();
            Err("已取消".to_string())
        })
        .await;
        assert_eq!(cancelled.unwrap_err(), "已取消");
        controls.reset();
    }
}

#[cfg(all(test, unix))]
mod session_tests {
    use super::*;
    use crate::commands::python_task::session_test::{quit_marker, SessionRun};
    use std::time::Instant;

    const READY: &str = "read -r line\ncase \"$line\" in *'\"cmd\":\"init\"'*) ;; *) exit 3;; esac\nprintf '%s\\n' '{\"type\":\"ready\"}'\n";
    /// 读一条处理命令，不是 process 就异常退出
    const READ_PROCESS: &str =
        "read -r line\ncase \"$line\" in *'\"cmd\":\"process\"'*) ;; *) exit 4;; esac\n";

    fn reply(kind: &str, image: &Path, status: &str, message: &str) -> String {
        format!(
            "printf '%s\\n' '{}'\n",
            serde_json::json!({"type": kind, "image_path": image, "status": status, "message": message})
        )
    }

    fn options(root: &Path, recursive: bool) -> PersonCropOptions {
        PersonCropOptions {
            input_path: root.join("in").to_string_lossy().into_owned(),
            output_path: root.join("out").to_string_lossy().into_owned(),
            use_gpu: false,
            person_enabled: true,
            person_conf: 0.3,
            upper_enabled: false,
            upper_conf: 0.5,
            upper_tag: String::new(),
            head_enabled: false,
            head_conf: 0.5,
            head_tag: String::new(),
            head_scale: 1.5,
            eyes_enabled: false,
            eyes_conf: 0.5,
            eyes_tag: String::new(),
            eyes_scale: 2.4,
            keep_original_tags: false,
            recursive,
        }
    }

    fn new_run(tag: &str) -> SessionRun {
        let run = SessionRun::new(tag, EVENT, "person-crop-test");
        std::fs::create_dir_all(run.root.join("in")).unwrap();
        run
    }

    fn input(run: &SessionRun, name: &str) -> PathBuf {
        run.root.join("in").join(name)
    }

    fn crop(
        run: &SessionRun,
        body: &str,
        files: &[PathBuf],
        recursive: bool,
    ) -> Result<ProcessResult, String> {
        run_person_crop(
            run.app.handle(),
            &run.controls,
            &run.fake_python(body),
            &options(&run.root, recursive),
            files,
            serde_json::json!({"person": "fake.onnx"}),
            PYTHON_SILENCE_LIMIT,
        )
    }

    #[test]
    fn commands_use_cmd_lines_and_session_quits_gracefully() {
        let run = new_run("crop_protocol");
        let (a, b) = (input(&run, "a.png"), input(&run, "b.png"));
        let marker = run.root.join("quit-received");
        let body = format!(
            "{READY}{READ_PROCESS}printf '\\377noise\\n'\nprintf '%s\\n' '{{\"type\":\"log\",\"message\":\"fallback\"}}' '{{\"type\":\"result\",\"image_path\":\"other.png\"}}'\n{}{READ_PROCESS}{}{}",
            reply("result", &a, "success", "裁切: 全身(0.90)"),
            reply("error", &b, "", "broken"),
            quit_marker(&marker)
        );
        let result = crop(&run, &body, &[a, b], false).unwrap();
        assert_eq!(
            (result.success_count, result.fail_count, result.total),
            (1, 1, 2)
        );
        assert!(marker.exists(), "进程应收到退出命令后自行退出");
        assert!(!run.controls.has_registered_process());
        assert_eq!(run.messages("success"), ["[成功] a.png — 裁切: 全身(0.90)"]);
        assert_eq!(run.messages("error"), ["[失败] b.png — broken"]);
        assert!(run.messages("info").iter().any(|m| m == "fallback"));
    }

    #[test]
    fn cancel_lets_the_image_in_hand_finish() {
        let run = new_run("crop_graceful");
        let (a, b) = (input(&run, "a.png"), input(&run, "b.png"));
        let marker = run.root.join("quit-received");
        let body = format!(
            "{READY}{READ_PROCESS}printf '%s\\n' '{{\"type\":\"log\",\"message\":\"cancel-now\"}}'\n{}{}",
            reply("result", &a, "skip", "未检测到目标"),
            quit_marker(&marker)
        );
        run.on_event("cancel-now", SessionControls::cancel);
        let result = crop(&run, &body, &[a, b], false).unwrap();
        assert_eq!((result.success_count, result.fail_count), (1, 0));
        assert!(marker.exists());
        assert_eq!(run.messages("success"), ["[跳过] a.png — 未检测到目标"]);
    }

    #[test]
    fn force_cancel_ends_the_process_at_once() {
        let run = new_run("crop_force");
        let body = format!("{READY}{READ_PROCESS}printf '%s\\n' '{{\"type\":\"log\",\"message\":\"cancel-now\"}}'\nsleep 30\n");
        run.on_event("cancel-now", SessionControls::force_cancel);
        let started = Instant::now();
        let result = crop(&run, &body, &[input(&run, "a.png")], false).unwrap();
        assert!(started.elapsed() < Duration::from_secs(10));
        assert_eq!((result.success_count, result.fail_count), (0, 0));
        assert!(run.messages("error").is_empty());
    }

    #[test]
    fn script_level_error_fails_the_rest_and_quits_gracefully() {
        let run = new_run("crop_script_error");
        let marker = run.root.join("quit-received");
        let body = format!(
            "{READY}{READ_PROCESS}printf '%s\\n' '{{\"type\":\"error\",\"message\":\"未知命令: None\"}}'\n{}",
            quit_marker(&marker)
        );
        let files = [input(&run, "a.png"), input(&run, "b.png")];
        let error = crop(&run, &body, &files, false).unwrap_err();
        assert_eq!(error, "人物裁切中断: 未知命令: None");
        assert!(marker.exists());
        let errors = run.messages("error");
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains("未处理的 2 张记为失败"), "{}", errors[0]);
    }

    #[test]
    fn crash_mid_run_fails_the_rest_and_reports_the_reason() {
        let run = new_run("crop_crash");
        let (a, b) = (input(&run, "a.png"), input(&run, "b.png"));
        let body = format!(
            "{READY}{READ_PROCESS}{}echo 'Segmentation fault' >&2\nexit 139\n",
            reply("result", &a, "success", "裁切: 全身(0.90)")
        );
        let error = crop(&run, &body, &[a, b], false).unwrap_err();
        assert!(
            error.starts_with("人物裁切中断: Python 进程已退出"),
            "{error}"
        );
        assert!(error.contains("Segmentation fault"), "{error}");
        assert!(run.messages("error")[0].contains("未处理的 1 张记为失败"));
    }

    #[test]
    fn unusable_output_dir_fails_only_that_image() {
        let run = new_run("crop_output_dir");
        std::fs::create_dir_all(run.root.join("in/sub")).unwrap();
        std::fs::create_dir_all(run.root.join("out")).unwrap();
        // 输出目录里同名的文件挡住了要建的子目录
        std::fs::write(run.root.join("out/sub"), b"").unwrap();
        let (nested, top) = (input(&run, "sub/a.png"), input(&run, "b.png"));
        let marker = run.root.join("quit-received");
        let body = format!(
            "{READY}{READ_PROCESS}{}{}",
            reply("result", &top, "success", "裁切: 全身(0.90)"),
            quit_marker(&marker)
        );
        let result = crop(&run, &body, &[nested, top], true).unwrap();
        assert_eq!((result.success_count, result.fail_count), (1, 1));
        assert!(run.messages("error")[0].starts_with("[失败] a.png — 无法创建输出目录"));
        assert!(marker.exists());
    }
}
