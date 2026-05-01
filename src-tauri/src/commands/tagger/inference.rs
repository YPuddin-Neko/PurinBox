use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use tauri::Emitter;

use super::{TagCategory, TagDefinition, TaggerOptions, ProcessResult, ProgressEvent, OnnxModelInfo};
use crate::commands::collect_image_files;

/// 全局打标取消标志
static TAGGING_CANCELLED: AtomicBool = AtomicBool::new(false);

/// 全局 Python 进程（切换硬件时会杀死重建）
static PYTHON_PROCESS: Mutex<Option<Child>> = Mutex::new(None);

/// 取消打标
pub fn cancel_tagging() {
    TAGGING_CANCELLED.store(true, Ordering::SeqCst);
    // 杀死正在运行的 Python 进程
    kill_python_process();
}

/// 重置取消标志（开始新任务前调用）
pub fn reset_tagging_cancel() {
    TAGGING_CANCELLED.store(false, Ordering::SeqCst);
}

/// 检查是否已取消
pub fn is_tagging_cancelled() -> bool {
    TAGGING_CANCELLED.load(Ordering::SeqCst)
}

/// 杀死正在运行的 Python 推理进程
pub fn kill_python_process() {
    if let Ok(mut guard) = PYTHON_PROCESS.lock() {
        if let Some(ref mut child) = *guard {
            let _ = child.kill();
            let _ = child.wait();
        }
        *guard = None;
    }
}

/// 在 Windows 上获取 nvidia-smi 的完整路径
fn get_nvidia_smi_path() -> String {
    #[cfg(target_os = "windows")]
    {
        let candidates = [
            r"C:\Windows\System32\nvidia-smi.exe",
            r"C:\Program Files\NVIDIA Corporation\NVSMI\nvidia-smi.exe",
        ];
        for path in &candidates {
            if std::path::Path::new(path).exists() {
                return path.to_string();
            }
        }
        if let Ok(output) = run_hidden_cmd("where", &["nvidia-smi"]) {
            if let Some(first_line) = output.lines().next() {
                let trimmed = first_line.trim();
                if !trimmed.is_empty() && std::path::Path::new(trimmed).exists() {
                    return trimmed.to_string();
                }
            }
        }
    }
    "nvidia-smi".to_string()
}

/// 运行命令并隐藏 Windows 控制台窗口，返回 stdout 或错误信息
fn run_hidden_cmd(program: &str, args: &[&str]) -> Result<String, String> {
    let mut cmd = Command::new(program);
    cmd.args(args);

    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
    }

    match cmd.output() {
        Ok(output) if output.status.success() => {
            Ok(String::from_utf8_lossy(&output.stdout).to_string())
        }
        Ok(output) => {
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();
            Err(format!("exit code: {:?}, stderr: {}", output.status.code(), stderr))
        }
        Err(e) => Err(format!("{}", e)),
    }
}

/// 公开接口：检测 NVIDIA 环境
pub fn detect_nvidia_env_pub(lines: &mut Vec<String>) -> bool {
    detect_nvidia_env(lines)
}

/// 公开接口：检测 CUDA Toolkit
pub fn detect_cuda_toolkit_pub(lines: &mut Vec<String>) {
    detect_cuda_toolkit(lines)
}

/// 检测 NVIDIA 驱动和 GPU 信息
fn detect_nvidia_env(lines: &mut Vec<String>) -> bool {
    let smi_path = get_nvidia_smi_path();
    lines.push(format!("nvidia-smi 路径: {}", smi_path));

    match run_hidden_cmd(&smi_path, &["--query-gpu=name,driver_version", "--format=csv,noheader,nounits"]) {
        Ok(stdout) => {
            if let Some(line) = stdout.lines().next() {
                let parts: Vec<&str> = line.split(',').map(|s| s.trim()).collect();
                let gpu_name = parts.first().unwrap_or(&"Unknown");
                let driver_ver = parts.get(1).unwrap_or(&"Unknown");
                lines.push(format!("NVIDIA GPU: {} (驱动 v{})", gpu_name, driver_ver));
            }

            if let Ok(full_output) = run_hidden_cmd(&smi_path, &[]) {
                if let Some(cuda_pos) = full_output.find("CUDA Version:") {
                    let rest = &full_output[cuda_pos + 14..];
                    let cuda_ver = rest.split_whitespace().next().unwrap_or("?");
                    lines.push(format!("CUDA 驱动版本: {}", cuda_ver));
                }
            }
            true
        }
        Err(err) => {
            lines.push(format!("NVIDIA GPU: 未检测到 ({})", err));
            false
        }
    }
}

/// 检测 CUDA Toolkit (nvcc)
fn detect_cuda_toolkit(lines: &mut Vec<String>) {
    if let Ok(output) = run_hidden_cmd("nvcc", &["--version"]) {
        if let Some(pos) = output.find("release ") {
            let ver = output[pos + 8..].split(',').next().unwrap_or("?").trim();
            lines.push(format!("CUDA Toolkit: {} (nvcc)", ver));
            return;
        }
    }

    #[cfg(target_os = "windows")]
    {
        let cuda_path = std::env::var("CUDA_PATH").ok();
        if let Some(ref cp) = cuda_path {
            if std::path::Path::new(cp).exists() {
                let ver = std::path::Path::new(cp)
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("unknown");
                lines.push(format!("CUDA Toolkit: {} (CUDA_PATH={})", ver, cp));
                return;
            }
        }

        if let Ok(entries) = std::fs::read_dir(r"C:\Program Files\NVIDIA GPU Computing Toolkit\CUDA") {
            let mut versions: Vec<String> = entries
                .filter_map(|e| e.ok())
                .filter(|e| e.path().is_dir())
                .filter_map(|e| e.file_name().into_string().ok())
                .collect();
            versions.sort();
            if let Some(latest) = versions.last() {
                lines.push(format!("CUDA Toolkit: {} (已安装)", latest));
                return;
            }
        }
    }

    lines.push("CUDA Toolkit: 未检测到".into());
}

/// 自动检测 ONNX 模型的输入信息（使用 Python 调用）
pub fn detect_model_info(model_path: &str) -> Result<OnnxModelInfo, String> {
    // 使用 Python 快速检测模型信息
    let python = find_python()?;
    let script = get_script_path()?;

    let mut child = Command::new(&python)
        .args([script.to_string_lossy().as_ref(), "--detect", model_path])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("启动 Python 失败: {}", e))?;

    let output = child.wait_with_output()
        .map_err(|e| format!("等待 Python 失败: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("模型检测失败: {}", stderr));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines() {
        if let Ok(val) = serde_json::from_str::<serde_json::Value>(line) {
            if val.get("type").and_then(|v| v.as_str()) == Some("model_info") {
                let input_size = val.get("input_size").and_then(|v| v.as_u64()).unwrap_or(448) as u32;
                let input_format = val.get("input_format").and_then(|v| v.as_str()).unwrap_or("NHWC").to_string();
                let shape: Vec<i64> = val.get("input_shape")
                    .and_then(|v| v.as_array())
                    .map(|a| a.iter().filter_map(|v| v.as_i64()).collect())
                    .unwrap_or_default();
                let channels = val.get("channels").and_then(|v| v.as_i64()).unwrap_or(3);

                return Ok(OnnxModelInfo {
                    input_size,
                    input_format,
                    input_shape: shape,
                    channels,
                });
            }
        }
    }

    Err("无法解析模型信息".into())
}

/// 从 CSV 文件加载标签定义
pub fn load_tags(csv_path: &Path) -> Result<Vec<TagDefinition>, String> {
    let mut reader = csv::Reader::from_path(csv_path)
        .map_err(|e| format!("无法读取标签文件: {}", e))?;

    let mut tags = Vec::new();
    for result in reader.records() {
        let record = result.map_err(|e| format!("CSV 解析错误: {}", e))?;
        if record.len() >= 3 {
            let name = record.get(1).unwrap_or("").to_string();
            let cat_id: i32 = record.get(2).unwrap_or("0").parse().unwrap_or(0);
            if let Some(category) = TagCategory::from_csv_id(cat_id) {
                tags.push(TagDefinition { name, category });
            }
        }
    }
    Ok(tags)
}

/// 从 JSON 文件加载标签定义 (CL Tagger 格式)
pub fn load_tags_json(json_path: &Path) -> Result<Vec<TagDefinition>, String> {
    let content = std::fs::read_to_string(json_path)
        .map_err(|e| format!("无法读取标签文件: {}", e))?;

    let map: std::collections::BTreeMap<String, serde_json::Value> =
        serde_json::from_str(&content)
            .map_err(|e| format!("JSON 解析错误: {}", e))?;

    let mut tags: Vec<(usize, TagDefinition)> = Vec::new();
    for (idx_str, val) in &map {
        let idx: usize = idx_str.parse().unwrap_or(0);
        let tag_name = val.get("tag").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let cat_str = val.get("category").and_then(|v| v.as_str()).unwrap_or("General");

        let category = match cat_str {
            "General" => TagCategory::General,
            "Artist" => TagCategory::Artist,
            "Copyright" => TagCategory::Copyright,
            "Character" => TagCategory::Character,
            "Meta" => TagCategory::Meta,
            "Rating" => TagCategory::Rating,
            _ => TagCategory::General,
        };

        tags.push((idx, TagDefinition { name: tag_name, category }));
    }

    tags.sort_by_key(|(idx, _)| *idx);
    Ok(tags.into_iter().map(|(_, td)| td).collect())
}

/// 查找 Python 可执行文件
fn find_python() -> Result<String, String> {
    // 1. 优先使用 python_env 模块管理的环境
    if let Some(python) = super::python_env::get_python_exe() {
        return Ok(python);
    }

    // 2. 检查系统 Python（需要有 onnxruntime）
    for name in &["python3", "python"] {
        if let Ok(output) = run_hidden_cmd(name, &["--version"]) {
            if output.contains("Python 3") {
                if run_hidden_cmd(name, &["-c", "import onnxruntime"]).is_ok() {
                    return Ok(name.to_string());
                }
            }
        }
    }

    #[cfg(target_os = "windows")]
    {
        let candidates = [
            r"C:\Python312\python.exe",
            r"C:\Python311\python.exe",
            r"C:\Python310\python.exe",
        ];
        for path in &candidates {
            if std::path::Path::new(path).exists() {
                return Ok(path.to_string());
            }
        }
    }

    Err("未找到可用的 Python 3 环境".into())
}

/// 获取推理脚本路径
fn get_script_path() -> Result<std::path::PathBuf, String> {
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|p| p.to_path_buf()))
        .unwrap_or_else(|| std::path::PathBuf::from("."));

    // 搜索路径列表
    let candidates = vec![
        // 开发模式: CARGO_MANIFEST_DIR/scripts/
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("scripts/tagger_inference.py"),
        // 生产模式 Windows/Linux: exe 同级 scripts/
        exe_dir.join("scripts/tagger_inference.py"),
        // 生产模式 Windows NSIS: exe 同级
        exe_dir.join("tagger_inference.py"),
        // macOS .app bundle: Resources/scripts/
        exe_dir.join("../Resources/scripts/tagger_inference.py"),
    ];

    for path in &candidates {
        if path.exists() {
            return Ok(path.canonicalize().unwrap_or_else(|_| path.clone()));
        }
    }

    let paths_str = candidates.iter()
        .enumerate()
        .map(|(i, p)| format!("  {}. {}", i + 1, p.display()))
        .collect::<Vec<_>>()
        .join("\n");
    Err(format!("推理脚本未找到。\n搜索路径:\n{}", paths_str))
}

/// 检查 Python 环境是否满足要求
pub fn check_python_env() -> Result<(String, String), String> {
    let python = find_python()?;

    // 检查 onnxruntime
    let check_script = "import onnxruntime as ort; print(ort.__version__); print(','.join(ort.get_available_providers()))";
    let output = run_hidden_cmd(&python, &["-c", check_script]);

    match output {
        Ok(stdout) => {
            let lines: Vec<&str> = stdout.trim().lines().collect();
            let ort_version = lines.first().unwrap_or(&"unknown").to_string();
            let providers = lines.get(1).unwrap_or(&"CPUExecutionProvider").to_string();
            Ok((ort_version, providers))
        }
        Err(_) => {
            Err(format!(
                "onnxruntime 未安装。请运行:\n  {} -m pip install onnxruntime\n\
                 如需 GPU 加速:\n  {} -m pip install onnxruntime-gpu",
                python, python
            ))
        }
    }
}

/// 执行批量打标（通过 Python 子进程）
pub fn run_tagging(
    app: &tauri::AppHandle,
    options: &TaggerOptions,
    model_path: &Path,
    tag_defs: &[TagDefinition],
    _input_size: u32,
    _is_nchw: bool,
) -> Result<ProcessResult, String> {
    // 杀死之前的进程（如果有）
    kill_python_process();

    // 查找 Python
    let python = find_python()?;
    let script = get_script_path()?;

    let _ = app.emit("tagger-progress", ProgressEvent {
        current: 0, total: 0, filename: String::new(),
        status: "info".to_string(),
        message: format!("启动 Python 推理进程 ({})", python),
    });

    // 启动 Python 子进程
    let mut cmd = Command::new(&python);
    cmd.arg(script.to_string_lossy().as_ref())
       .stdin(Stdio::piped())
       .stdout(Stdio::piped())
       .stderr(Stdio::piped());

    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
    }

    let mut child = cmd.spawn()
        .map_err(|e| format!("启动 Python 进程失败: {}", e))?;

    let mut stdin = child.stdin.take().ok_or("无法获取 Python stdin")?;
    let stdout = child.stdout.take().ok_or("无法获取 Python stdout")?;
    let stderr = child.stderr.take().ok_or("无法获取 Python stderr")?;

    // 启动 stderr 读取线程（输出到日志）
    let app_err = app.clone();
    std::thread::spawn(move || {
        let reader = BufReader::new(stderr);
        for line in reader.lines() {
            if let Ok(line) = line {
                let _ = app_err.emit("tagger-progress", ProgressEvent {
                    current: 0, total: 0, filename: String::new(),
                    status: "error".to_string(),
                    message: format!("[Python stderr] {}", line),
                });
            }
        }
    });

    // 发送 init 命令
    let tags_path = model_path.parent()
        .unwrap_or(Path::new("."))
        .join(if tag_defs.is_empty() { "selected_tags.csv" } else {
            // 根据模型目录查找标签文件
            let dir = model_path.parent().unwrap_or(Path::new("."));
            let json = dir.join("tag_mapping.json");
            let csv = dir.join("selected_tags.csv");
            if json.exists() {
                "tag_mapping.json"
            } else if csv.exists() {
                "selected_tags.csv"
            } else {
                "selected_tags.csv"
            }
        });

    let init_cmd = serde_json::json!({
        "cmd": "init",
        "model_path": model_path.to_string_lossy(),
        "tags_path": tags_path.to_string_lossy(),
        "use_gpu": options.use_gpu,
        "input_size": _input_size,
    });

    writeln!(stdin, "{}", init_cmd)
        .map_err(|e| format!("发送 init 命令失败: {}", e))?;

    // 读取 stdout 直到 ready
    let reader = BufReader::new(stdout);
    let mut lines_iter = reader.lines();

    let mut ready = false;
    let timeout = std::time::Instant::now();
    while let Some(Ok(line)) = lines_iter.next() {
        if is_tagging_cancelled() {
            let _ = child.kill();
            return Ok(ProcessResult { success_count: 0, fail_count: 0, total: 0, errors: vec![] });
        }

        if let Ok(msg) = serde_json::from_str::<serde_json::Value>(&line) {
            let msg_type = msg.get("type").and_then(|v| v.as_str()).unwrap_or("");
            match msg_type {
                "log" => {
                    let text = msg.get("message").and_then(|v| v.as_str()).unwrap_or("");
                    let _ = app.emit("tagger-progress", ProgressEvent {
                        current: 0, total: 0, filename: String::new(),
                        status: "info".to_string(),
                        message: text.to_string(),
                    });
                }
                "error" => {
                    let text = msg.get("message").and_then(|v| v.as_str()).unwrap_or("");
                    let _ = app.emit("tagger-progress", ProgressEvent {
                        current: 0, total: 0, filename: String::new(),
                        status: "error".to_string(),
                        message: text.to_string(),
                    });
                    let _ = child.kill();
                    return Err(format!("Python 推理错误: {}", text));
                }
                "ready" => {
                    let info = msg.get("info").and_then(|v| v.as_str()).unwrap_or("ready");
                    let _ = app.emit("tagger-progress", ProgressEvent {
                        current: 0, total: 0, filename: String::new(),
                        status: "success".to_string(),
                        message: format!("✓ 模型加载完成 ({})", info),
                    });
                    ready = true;
                    break;
                }
                _ => {}
            }
        }

        if timeout.elapsed() > std::time::Duration::from_secs(120) {
            let _ = child.kill();
            return Err("模型加载超时(120秒)".into());
        }
    }

    if !ready {
        let _ = child.kill();
        return Err("Python 进程未能成功初始化".into());
    }

    // 收集图片文件
    let input_dir = Path::new(&options.input_path);
    let files = collect_image_files(input_dir)?;
    let total = files.len() as u32;
    let mut success_count = 0u32;
    let mut fail_count = 0u32;
    let mut errors = Vec::new();

    let enabled_cats: Vec<&str> = options.enabled_categories.iter().map(|s| s.as_str()).collect();

    // 逐图片发送 tag 命令
    for (i, file_path) in files.iter().enumerate() {
        if is_tagging_cancelled() {
            let _ = app.emit("tagger-progress", ProgressEvent {
                current: i as u32, total,
                filename: String::new(),
                status: "error".to_string(),
                message: format!("打标已取消（已完成 {}/{}）", i, total),
            });
            let _ = app.emit("tagger-progress", ProgressEvent {
                current: i as u32, total,
                filename: String::new(),
                status: "done".to_string(),
                message: format!("打标已取消: 成功 {}, 失败 {}", success_count, fail_count),
            });
            break;
        }

        let filename = file_path.file_name().unwrap_or_default().to_string_lossy().to_string();
        let _ = app.emit("tagger-progress", ProgressEvent {
            current: i as u32 + 1, total,
            filename: filename.clone(),
            status: "processing".to_string(),
            message: format!("正在处理: {} ({}/{})", filename, i + 1, total),
        });

        let tag_cmd = serde_json::json!({
            "cmd": "tag",
            "image_path": file_path.to_string_lossy(),
            "general_threshold": options.general_threshold,
            "character_threshold": options.character_threshold,
            "enabled_categories": enabled_cats,
        });

        if let Err(e) = writeln!(stdin, "{}", tag_cmd) {
            fail_count += 1;
            let err_msg = format!("{}: 发送命令失败: {}", filename, e);
            errors.push(err_msg.clone());
            break;
        }

        // 读取结果
        match lines_iter.next() {
            Some(Ok(line)) => {
                if let Ok(msg) = serde_json::from_str::<serde_json::Value>(&line) {
                    let msg_type = msg.get("type").and_then(|v| v.as_str()).unwrap_or("");
                    match msg_type {
                        "result" => {
                            let tag_count = msg.get("tag_count").and_then(|v| v.as_u64()).unwrap_or(0);
                            success_count += 1;
                            let _ = app.emit("tagger-progress", ProgressEvent {
                                current: i as u32 + 1, total,
                                filename: filename.clone(),
                                status: "success".to_string(),
                                message: format!("[完成] {} → {} 个标签", filename, tag_count),
                            });
                        }
                        "error" => {
                            let text = msg.get("message").and_then(|v| v.as_str()).unwrap_or("unknown");
                            fail_count += 1;
                            let err_msg = format!("{}: {}", filename, text);
                            errors.push(err_msg.clone());
                            let _ = app.emit("tagger-progress", ProgressEvent {
                                current: i as u32 + 1, total,
                                filename: filename.clone(),
                                status: "error".to_string(),
                                message: format!("[错误] {}", err_msg),
                            });
                        }
                        "log" => {
                            // 日志消息，可能需要继续读取 result
                            let text = msg.get("message").and_then(|v| v.as_str()).unwrap_or("");
                            let _ = app.emit("tagger-progress", ProgressEvent {
                                current: i as u32 + 1, total,
                                filename: filename.clone(),
                                status: "info".to_string(),
                                message: text.to_string(),
                            });
                        }
                        _ => {}
                    }
                }
            }
            Some(Err(e)) => {
                fail_count += 1;
                errors.push(format!("{}: 读取结果失败: {}", filename, e));
                break;
            }
            None => {
                fail_count += 1;
                errors.push(format!("{}: Python 进程意外退出", filename));
                break;
            }
        }
    }

    // 发送 quit 命令
    let _ = writeln!(stdin, r#"{{"cmd":"quit"}}"#);
    let _ = child.wait();

    let _ = app.emit("tagger-progress", ProgressEvent {
        current: total, total, filename: String::new(),
        status: "done".to_string(),
        message: format!("打标完成: 成功 {}, 失败 {}, 共 {}", success_count, fail_count, total),
    });

    Ok(ProcessResult { success_count, fail_count, total, errors })
}
