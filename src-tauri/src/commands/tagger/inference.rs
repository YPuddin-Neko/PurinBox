use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::Mutex;
use tauri::Emitter;

use super::{
    OnnxModelInfo, ProcessResult, ProgressEvent, TagCategory, TagDefinition, TaggerOptions,
};
use crate::commands::python_proc;
use crate::commands::{collect_image_files, collect_image_files_recursive};

/// 去除 ANSI 转义序列（颜色码等）
/// 同时处理 \x1b[...m 和 Windows 下残留的 [0;93m 格式
fn strip_ansi_codes(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            // 跳过 ESC[...m 序列
            if chars.peek() == Some(&'[') {
                chars.next();
                while let Some(&next) = chars.peek() {
                    chars.next();
                    if next.is_ascii_alphabetic() {
                        break;
                    }
                }
            }
        } else if c == '[' {
            // Windows 下可能 ESC 被吃掉，只剩 [0;93m 这样的
            // 检查是否是 ANSI 码模式: [数字;数字m 或 [m
            let mut buf = String::new();
            let mut is_ansi = false;
            while let Some(&next) = chars.peek() {
                if next.is_ascii_digit() || next == ';' {
                    buf.push(next);
                    chars.next();
                } else if next == 'm' && buf.len() <= 10 {
                    chars.next();
                    is_ansi = true;
                    break;
                } else {
                    break;
                }
            }
            if !is_ansi {
                result.push('[');
                result.push_str(&buf);
            }
        } else {
            result.push(c);
        }
    }
    result
}

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
    // 按进程树终止：Python 会派生工作进程，单杀直接子进程会留下孤儿进程
    crate::commands::kill_child_tree(&PYTHON_PROCESS);
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
    cmd.args(args).env("PYTHONIOENCODING", "utf-8");

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
            Err(format!(
                "exit code: {:?}, stderr: {}",
                output.status.code(),
                stderr
            ))
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

/// 公开接口：检测 Apple GPU (macOS)
pub fn detect_apple_gpu_pub(lines: &mut Vec<String>) {
    detect_apple_gpu(lines)
}

/// 检测 Apple Silicon GPU 信息 (macOS)
fn detect_apple_gpu(lines: &mut Vec<String>) {
    // 获取芯片型号
    if let Ok(output) = Command::new("sysctl")
        .args(["-n", "machdep.cpu.brand_string"])
        .output()
    {
        if output.status.success() {
            let chip = String::from_utf8_lossy(&output.stdout).trim().to_string();
            lines.push(format!("GPU: {} (Metal/MPS)", chip));
            return;
        }
    }
    lines.push("GPU: Apple Silicon (Metal/MPS)".into());
}

/// 检测 NVIDIA 驱动和 GPU 信息
fn detect_nvidia_env(lines: &mut Vec<String>) -> bool {
    let smi_path = get_nvidia_smi_path();

    match run_hidden_cmd(
        &smi_path,
        &[
            "--query-gpu=name,driver_version",
            "--format=csv,noheader,nounits",
        ],
    ) {
        Ok(stdout) => {
            for line in stdout.lines() {
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }
                let parts: Vec<&str> = line.split(',').map(|s| s.trim()).collect();
                let gpu_name = parts.first().unwrap_or(&"Unknown");
                let driver_ver = parts.get(1).unwrap_or(&"Unknown");
                lines.push(format!("GPU: {} (驱动 v{})", gpu_name, driver_ver));
            }
            true
        }
        Err(err) => {
            lines.push(format!("GPU: 未检测到 ({})", err));
            false
        }
    }
}

/// 检测 CUDA Toolkit (nvcc)
fn detect_cuda_toolkit(lines: &mut Vec<String>) {
    // 1. 尝试 PATH 中的 nvcc
    if let Some(ver) = try_nvcc_version("nvcc") {
        lines.push(format!("CUDA Toolkit: v{}", ver));
        return;
    }

    // 2. 尝试从 CUDA 环境变量（含注册表回退）中找 nvcc
    #[cfg(target_os = "windows")]
    {
        for (_key, val) in python_proc::get_cuda_env_vars() {
            let nvcc = format!(r"{}\bin\nvcc.exe", val);
            if std::path::Path::new(&nvcc).exists() {
                if let Some(ver) = try_nvcc_version(&nvcc) {
                    lines.push(format!("CUDA Toolkit: v{}", ver));
                    return;
                }
            }
            // 也记录一下找到了 CUDA 但 nvcc 不在
        }
    }

    #[cfg(not(target_os = "windows"))]
    {
        for (key, val) in std::env::vars() {
            if key == "CUDA_PATH" || key == "CUDA_HOME" {
                let nvcc = format!("{}/bin/nvcc", val);
                if std::path::Path::new(&nvcc).exists() {
                    if let Some(ver) = try_nvcc_version(&nvcc) {
                        lines.push(format!("CUDA Toolkit: v{}", ver));
                        return;
                    }
                }
            }
        }
    }

    lines.push("CUDA Toolkit: 未检测到".into());
}

/// 运行 nvcc --version 提取版本号
fn try_nvcc_version(nvcc_path: &str) -> Option<String> {
    let output = run_hidden_cmd(nvcc_path, &["--version"]).ok()?;
    let pos = output.find("release ")?;
    Some(output[pos + 8..].split(',').next()?.trim().to_string())
}

/// 自动检测 ONNX 模型的输入信息（使用 Python 调用）
pub fn detect_model_info(model_path: &str) -> Result<OnnxModelInfo, String> {
    // 使用 Python 快速检测模型信息
    let python = find_python()?;
    let script = get_script_path()?;

    let child = Command::new(&python)
        .args([script.to_string_lossy().as_ref(), "--detect", model_path])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("启动 Python 失败: {}", e))?;

    let output = child
        .wait_with_output()
        .map_err(|e| format!("等待 Python 失败: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("模型检测失败: {}", stderr));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines() {
        if let Ok(val) = serde_json::from_str::<serde_json::Value>(line) {
            if val.get("type").and_then(|v| v.as_str()) == Some("model_info") {
                let input_size = val
                    .get("input_size")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(448) as u32;
                let input_format = val
                    .get("input_format")
                    .and_then(|v| v.as_str())
                    .unwrap_or("NHWC")
                    .to_string();
                let shape: Vec<i64> = val
                    .get("input_shape")
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
    let mut reader =
        csv::Reader::from_path(csv_path).map_err(|e| format!("无法读取标签文件: {}", e))?;

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
    let content =
        std::fs::read_to_string(json_path).map_err(|e| format!("无法读取标签文件: {}", e))?;

    let value: serde_json::Value =
        serde_json::from_str(&content).map_err(|e| format!("JSON 解析错误: {}", e))?;

    if let Some(idx_to_tag) = value.get("idx_to_tag") {
        return load_vocabulary_json_tags(idx_to_tag, &value);
    }

    load_legacy_json_tags(&value)
}

fn load_legacy_json_tags(value: &serde_json::Value) -> Result<Vec<TagDefinition>, String> {
    let map = value
        .as_object()
        .ok_or_else(|| "JSON 标签文件格式不支持".to_string())?;
    let mut tags: Vec<(usize, TagDefinition)> = Vec::new();
    for (idx_str, val) in map {
        let idx: usize = idx_str.parse().unwrap_or(0);
        let tag_name = val
            .get("tag")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let category = category_from_value(val.get("category"), None);
        tags.push((
            idx,
            TagDefinition {
                name: tag_name,
                category,
            },
        ));
    }
    tags.sort_by_key(|(idx, _)| *idx);
    Ok(tags.into_iter().map(|(_, tag)| tag).collect())
}

fn load_vocabulary_json_tags(
    idx_to_tag: &serde_json::Value,
    root: &serde_json::Value,
) -> Result<Vec<TagDefinition>, String> {
    let mut indexed_tags: Vec<(usize, String)> = Vec::new();

    if let Some(arr) = idx_to_tag.as_array() {
        for (idx, tag) in arr.iter().enumerate() {
            if let Some(name) = tag.as_str() {
                indexed_tags.push((idx, name.to_string()));
            }
        }
    } else if let Some(map) = idx_to_tag.as_object() {
        for (idx_str, tag) in map {
            if let Some(name) = tag.as_str() {
                let idx = idx_str.parse::<usize>().unwrap_or(indexed_tags.len());
                indexed_tags.push((idx, name.to_string()));
            }
        }
    } else {
        return Err("model_vocabulary.json 缺少 idx_to_tag".into());
    }

    indexed_tags.sort_by_key(|(idx, _)| *idx);
    let tag_to_category = root.get("tag_to_category").and_then(|v| v.as_object());
    let idx_to_category = root.get("idx_to_category").and_then(|v| v.as_object());
    let categories = root.get("categories");

    Ok(indexed_tags
        .into_iter()
        .map(|(idx, name)| {
            let category_value = tag_to_category
                .and_then(|map| map.get(&name))
                .or_else(|| idx_to_category.and_then(|map| map.get(&idx.to_string())));
            TagDefinition {
                name,
                category: category_from_value(category_value, categories),
            }
        })
        .collect())
}

fn category_from_value(
    value: Option<&serde_json::Value>,
    categories: Option<&serde_json::Value>,
) -> TagCategory {
    let raw = match value {
        Some(v) if v.is_string() => {
            let s = v.as_str().unwrap_or_default();
            if let Ok(idx) = s.parse::<usize>() {
                resolve_category_index(idx, categories).unwrap_or_else(|| s.to_string())
            } else {
                s.to_string()
            }
        }
        Some(v) if v.is_u64() => {
            resolve_category_index(v.as_u64().unwrap_or(0) as usize, categories)
                .unwrap_or_else(|| "General".to_string())
        }
        _ => "General".to_string(),
    };

    match raw.to_lowercase().replace('-', "_").as_str() {
        "artist" => TagCategory::Artist,
        "copyright" | "copyrights" => TagCategory::Copyright,
        "character" | "characters" => TagCategory::Character,
        "meta" => TagCategory::Meta,
        "rating" => TagCategory::Rating,
        "quality" => TagCategory::Quality,
        "model" => TagCategory::Model,
        _ => TagCategory::General,
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

/// 查找 Python 可执行文件
pub(crate) fn find_python() -> Result<String, String> {
    // 1. 优先使用 python_env 模块管理的环境
    if let Some(python) = super::python_env::get_python_exe() {
        return Ok(python);
    }

    // 2. 检查系统 Python（需要有 onnxruntime）
    for name in &["python3", "python"] {
        if let Ok(output) = run_hidden_cmd(name, &["--version"]) {
            if output.contains("Python 3")
                && run_hidden_cmd(name, &["-c", "import onnxruntime"]).is_ok()
            {
                return Ok(name.to_string());
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

    Err("未找到可用的 Python 环境".into())
}

/// 获取推理脚本路径
fn get_script_path() -> Result<std::path::PathBuf, String> {
    python_proc::find_script("tagger_inference.py")
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
        Err(_) => Err(format!(
            "onnxruntime 未安装。请运行:\n  {} -m pip install onnxruntime\n\
                 如需 GPU 加速:\n  {} -m pip install onnxruntime-gpu",
            python, python
        )),
    }
}

/// 执行批量打标（通过 Python 子进程）
#[allow(clippy::too_many_arguments)]
pub fn run_tagging(
    app: &tauri::AppHandle,
    options: &TaggerOptions,
    model_path: &Path,
    tags_path: &Path,
    _tag_defs: &[TagDefinition],
    _input_size: u32,
    _is_nchw: bool,
    preprocess_mode: &str,
) -> Result<ProcessResult, String> {
    // 杀死之前的进程（如果有）
    kill_python_process();

    // 查找 Python
    let python = find_python()?;
    let script = get_script_path()?;

    // 启动 Python 子进程
    let mut cmd = Command::new(&python);
    cmd.arg(script.to_string_lossy().as_ref())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env("NO_COLOR", "1")
        .env("PYTHONUNBUFFERED", "1")
        .env("PYTHONIOENCODING", "utf-8");

    // Windows: 无窗口 + GPU 模式注入 CUDA/cuDNN DLL 路径（共享实现见 python_proc）
    python_proc::configure_python_command(&mut cmd, options.use_gpu);

    let mut child = cmd
        .spawn()
        .map_err(|e| format!("启动 Python 进程失败: {}", e))?;

    // 取出管道句柄（take 出管道后 Child 仍可 kill/wait）
    let (mut stdin, stdout, stderr) =
        match (child.stdin.take(), child.stdout.take(), child.stderr.take()) {
            (Some(i), Some(o), Some(e)) => (i, o, e),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("无法获取 Python 进程管道".into());
            }
        };

    // 把 Child 句柄存入全局，这样 cancel_tagging() -> kill_python_process() 才能真正杀掉进程
    *PYTHON_PROCESS.lock().unwrap_or_else(|e| e.into_inner()) = Some(child);

    // 启动 stderr 读取线程（输出到日志，过滤 ANSI 颜色码）
    let app_err = app.clone();
    std::thread::spawn(move || {
        let mut reader = BufReader::new(stderr);
        let mut buf = Vec::new();
        use std::io::Read;
        let mut byte = [0u8; 1];
        loop {
            match reader.read(&mut byte) {
                Ok(0) => break,
                Ok(_) => {
                    if byte[0] == b'\n' {
                        let line = String::from_utf8(buf.clone()).unwrap_or_else(|_| {
                            let (s, _, _) = encoding_rs::GBK.decode(&buf);
                            s.to_string()
                        });
                        buf.clear();
                        let clean = strip_ansi_codes(&line);
                        let clean = clean.trim();
                        if clean.is_empty() {
                            continue;
                        }
                        let lower = clean.to_lowercase();
                        if lower.contains("context leak")
                            || lower.contains("msgtracer")
                            || lower.contains("number of partitions supported by coreml")
                            || lower.contains("cudnn")
                            || lower.contains("cuda_path")
                            || lower.contains("onnxruntime")
                            || lower.contains("could not load")
                            || lower.contains("loaded library")
                        {
                            continue;
                        }
                        let _ = app_err.emit(
                            "tagger-progress",
                            ProgressEvent {
                                current: 0,
                                total: 0,
                                filename: String::new(),
                                status: "warning".to_string(),
                                message: format!("[Python] {}", clean),
                                ..Default::default()
                            },
                        );
                    } else if byte[0] != b'\r' {
                        buf.push(byte[0]);
                    }
                }
                Err(_) => break,
            }
        }
    });

    let init_cmd = serde_json::json!({
        "cmd": "init",
        "model_path": model_path.to_string_lossy(),
        "tags_path": tags_path.to_string_lossy(),
        "use_gpu": options.use_gpu,
        "input_size": _input_size,
        "preprocess_mode": preprocess_mode,
    });

    if let Err(e) = writeln!(stdin, "{}", init_cmd) {
        kill_python_process();
        return Err(format!("发送 init 命令失败: {}", e));
    }

    // stdout 读取线程：阻塞的行读取放到独立线程，通过 channel 把行发给主逻辑，
    // 这样模型加载的超时检查不会被阻塞的读取卡住
    let (line_tx, line_rx) = mpsc::channel::<std::io::Result<String>>();
    std::thread::spawn(move || {
        let reader = BufReader::new(stdout);
        for line in reader.lines() {
            let is_err = line.is_err();
            if line_tx.send(line).is_err() || is_err {
                break;
            }
        }
        // 线程退出时 line_tx 被 drop，接收端收到 Disconnected 即等价于 EOF
    });

    // 等待 ready（120 秒加载超时，即使 Python 无任何输出挂死也能触发）
    let mut ready = false;
    let load_deadline = std::time::Instant::now() + std::time::Duration::from_secs(120);
    loop {
        if is_tagging_cancelled() {
            kill_python_process();
            return Ok(ProcessResult {
                success_count: 0,
                fail_count: 0,
                total: 0,
                errors: vec![],
            });
        }

        let now = std::time::Instant::now();
        if now >= load_deadline {
            kill_python_process();
            return Err("模型加载超时(120秒)".into());
        }

        let line = match line_rx.recv_timeout(load_deadline - now) {
            Ok(Ok(line)) => line,
            // 读取出错或进程退出（EOF）：跳出循环按未就绪处理
            Ok(Err(_)) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
            Err(mpsc::RecvTimeoutError::Timeout) => {
                kill_python_process();
                return Err("模型加载超时(120秒)".into());
            }
        };

        if let Ok(msg) = serde_json::from_str::<serde_json::Value>(&line) {
            let msg_type = msg.get("type").and_then(|v| v.as_str()).unwrap_or("");
            match msg_type {
                "log" => {
                    let text = msg
                        .get("message")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let i18n_key = msg
                        .get("i18n_key")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string());
                    let i18n_params = msg.get("i18n_params").cloned();
                    let _ = app.emit(
                        "tagger-progress",
                        ProgressEvent {
                            current: 0,
                            total: 0,
                            filename: String::new(),
                            status: "info".to_string(),
                            message: text,
                            i18n_key,
                            i18n_params,
                        },
                    );
                }
                "error" => {
                    let text = msg.get("message").and_then(|v| v.as_str()).unwrap_or("");
                    let _ = app.emit(
                        "tagger-progress",
                        ProgressEvent {
                            current: 0,
                            total: 0,
                            filename: String::new(),
                            status: "error".to_string(),
                            message: text.to_string(),
                            ..Default::default()
                        },
                    );
                    kill_python_process();
                    return Err(format!("Python 推理错误: {}", text));
                }
                "ready" => {
                    ready = true;
                    break;
                }
                _ => {}
            }
        }
    }

    if !ready {
        kill_python_process();
        if is_tagging_cancelled() {
            return Ok(ProcessResult {
                success_count: 0,
                fail_count: 0,
                total: 0,
                errors: vec![],
            });
        }
        return Err("Python 进程未能成功初始化".into());
    }

    // 收集图片文件（失败时杀掉已启动的 Python 进程，避免泄漏）
    let input_dir = Path::new(&options.input_path);
    let files_result = if options.recursive {
        collect_image_files_recursive(input_dir)
    } else {
        collect_image_files(input_dir)
    };
    let files = match files_result {
        Ok(f) => f,
        Err(e) => {
            kill_python_process();
            return Err(e);
        }
    };
    let total = files.len() as u32;
    let mut success_count = 0u32;
    let mut fail_count = 0u32;
    let mut errors = Vec::new();

    let enabled_cats: Vec<&str> = options
        .enabled_categories
        .iter()
        .map(|s| s.as_str())
        .collect();

    // 逐图片发送 tag 命令
    let _ = app.emit(
        "tagger-progress",
        ProgressEvent {
            current: 0,
            total,
            filename: String::new(),
            status: "info".to_string(),
            message: format!("读取到 {} 张图片", total),
            ..Default::default()
        },
    );

    let batch_size = options.batch_size.max(1) as usize;

    let mut i = 0usize;
    while i < files.len() {
        if is_tagging_cancelled() {
            let _ = app.emit(
                "tagger-progress",
                ProgressEvent {
                    current: i as u32,
                    total,
                    filename: String::new(),
                    status: "error".to_string(),
                    message: format!("打标已取消（已完成 {}/{}）", i, total),
                    ..Default::default()
                },
            );
            let _ = app.emit(
                "tagger-progress",
                ProgressEvent {
                    current: i as u32,
                    total,
                    filename: String::new(),
                    status: "done".to_string(),
                    message: format!("打标已取消: 成功 {}, 失败 {}", success_count, fail_count),
                    ..Default::default()
                },
            );
            break;
        }

        let end = (i + batch_size).min(files.len());
        let batch_files = &files[i..end];
        let batch_len = batch_files.len();

        let first_name = batch_files[0]
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        let _ = app.emit(
            "tagger-progress",
            ProgressEvent {
                current: i as u32 + 1,
                total,
                filename: first_name.clone(),
                status: "processing".to_string(),
                message: if batch_len > 1 {
                    format!(
                        "正在处理: {} 等 {} 张 ({}/{})",
                        first_name,
                        batch_len,
                        i + 1,
                        total
                    )
                } else {
                    format!("正在处理: {} ({}/{})", first_name, i + 1, total)
                },
                ..Default::default()
            },
        );

        if batch_len == 1 {
            // 单张模式
            let file_path = &batch_files[0];
            let filename = file_path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();
            let tag_cmd = serde_json::json!({
                "cmd": "tag",
                "image_path": file_path.to_string_lossy(),
                "general_threshold": options.general_threshold,
                "character_threshold": options.character_threshold,
                "enabled_categories": enabled_cats,
                "exclude_tags": options.exclude_tags,
                "append_tags": options.append_tags,
                "append_position": options.append_position,
                "replace_underscore": options.replace_underscore,
                "output_format": options.output_format,
                "json_simplified": options.json_simplified,
                "escape_parentheses": options.escape_parentheses,
                "sort_by": options.sort_by,
                "existing_tags_action": options.existing_tags_action,
            });
            if let Err(e) = writeln!(stdin, "{}", tag_cmd) {
                fail_count += 1;
                errors.push(format!("{}: 发送命令失败: {}", filename, e));
                break;
            }
            loop {
                match line_rx.recv() {
                    Ok(Ok(line)) => {
                        if let Ok(msg) = serde_json::from_str::<serde_json::Value>(&line) {
                            let msg_type = msg.get("type").and_then(|v| v.as_str()).unwrap_or("");
                            match msg_type {
                                "result" => {
                                    let tag_count =
                                        msg.get("tag_count").and_then(|v| v.as_u64()).unwrap_or(0);
                                    success_count += 1;
                                    let _ = app.emit(
                                        "tagger-progress",
                                        ProgressEvent {
                                            current: i as u32 + 1,
                                            total,
                                            filename: filename.clone(),
                                            status: "success".to_string(),
                                            message: format!(
                                                "[完成] {} → {} 个标签",
                                                filename, tag_count
                                            ),
                                            ..Default::default()
                                        },
                                    );
                                    break;
                                }
                                "error" => {
                                    let text = msg
                                        .get("message")
                                        .and_then(|v| v.as_str())
                                        .unwrap_or("unknown");
                                    fail_count += 1;
                                    errors.push(format!("{}: {}", filename, text));
                                    let _ = app.emit(
                                        "tagger-progress",
                                        ProgressEvent {
                                            current: i as u32 + 1,
                                            total,
                                            filename: filename.clone(),
                                            status: "error".to_string(),
                                            message: format!("[错误] {}: {}", filename, text),
                                            ..Default::default()
                                        },
                                    );
                                    break;
                                }
                                "log" => {
                                    let text = msg
                                        .get("message")
                                        .and_then(|v| v.as_str())
                                        .unwrap_or("")
                                        .to_string();
                                    let i18n_key = msg
                                        .get("i18n_key")
                                        .and_then(|v| v.as_str())
                                        .map(|s| s.to_string());
                                    let i18n_params = msg.get("i18n_params").cloned();
                                    let _ = app.emit(
                                        "tagger-progress",
                                        ProgressEvent {
                                            current: i as u32 + 1,
                                            total,
                                            filename: filename.clone(),
                                            status: "info".to_string(),
                                            message: text,
                                            i18n_key,
                                            i18n_params,
                                        },
                                    );
                                }
                                _ => {
                                    break;
                                }
                            }
                        }
                    }
                    Ok(Err(e)) => {
                        fail_count += 1;
                        errors.push(format!("{}: 读取失败: {}", filename, e));
                        break;
                    }
                    Err(_) => {
                        fail_count += 1;
                        errors.push(format!("{}: Python 进程退出", filename));
                        break;
                    }
                }
            }
        } else {
            // 批量模式
            let images: Vec<serde_json::Value> = batch_files
                .iter()
                .map(|fp| {
                    serde_json::json!({
                        "image_path": fp.to_string_lossy(),
                        "general_threshold": options.general_threshold,
                        "character_threshold": options.character_threshold,
                        "enabled_categories": enabled_cats,
                        "exclude_tags": options.exclude_tags,
                        "append_tags": options.append_tags,
                        "append_position": options.append_position,
                        "replace_underscore": options.replace_underscore,
                        "output_format": options.output_format,
                        "json_simplified": options.json_simplified,
                        "escape_parentheses": options.escape_parentheses,
                        "sort_by": options.sort_by,
                        "existing_tags_action": options.existing_tags_action,
                    })
                })
                .collect();
            let batch_cmd = serde_json::json!({ "cmd": "tag_batch", "images": images });
            if let Err(e) = writeln!(stdin, "{}", batch_cmd) {
                fail_count += batch_len as u32;
                errors.push(format!("批量发送失败: {}", e));
                break;
            }
            let mut results_read = 0usize;
            while results_read < batch_len {
                match line_rx.recv() {
                    Ok(Ok(line)) => {
                        if let Ok(msg) = serde_json::from_str::<serde_json::Value>(&line) {
                            let msg_type = msg.get("type").and_then(|v| v.as_str()).unwrap_or("");
                            match msg_type {
                                "result" => {
                                    let img_path = msg
                                        .get("image_path")
                                        .and_then(|v| v.as_str())
                                        .unwrap_or("");
                                    let fname = std::path::Path::new(img_path)
                                        .file_name()
                                        .unwrap_or_default()
                                        .to_string_lossy()
                                        .to_string();
                                    let tag_count =
                                        msg.get("tag_count").and_then(|v| v.as_u64()).unwrap_or(0);
                                    success_count += 1;
                                    results_read += 1;
                                    let _ = app.emit(
                                        "tagger-progress",
                                        ProgressEvent {
                                            current: (i + results_read) as u32,
                                            total,
                                            filename: fname.clone(),
                                            status: "success".to_string(),
                                            message: format!(
                                                "[完成] {} → {} 个标签",
                                                fname, tag_count
                                            ),
                                            ..Default::default()
                                        },
                                    );
                                }
                                "error" => {
                                    let img_path = msg
                                        .get("image_path")
                                        .and_then(|v| v.as_str())
                                        .unwrap_or("");
                                    let fname = std::path::Path::new(img_path)
                                        .file_name()
                                        .unwrap_or_default()
                                        .to_string_lossy()
                                        .to_string();
                                    let text = msg
                                        .get("message")
                                        .and_then(|v| v.as_str())
                                        .unwrap_or("unknown");
                                    fail_count += 1;
                                    results_read += 1;
                                    errors.push(format!("{}: {}", fname, text));
                                    let _ = app.emit(
                                        "tagger-progress",
                                        ProgressEvent {
                                            current: (i + results_read) as u32,
                                            total,
                                            filename: fname.clone(),
                                            status: "error".to_string(),
                                            message: format!("[错误] {}: {}", fname, text),
                                            ..Default::default()
                                        },
                                    );
                                }
                                "log" => {
                                    let text = msg
                                        .get("message")
                                        .and_then(|v| v.as_str())
                                        .unwrap_or("")
                                        .to_string();
                                    let i18n_key = msg
                                        .get("i18n_key")
                                        .and_then(|v| v.as_str())
                                        .map(|s| s.to_string());
                                    let i18n_params = msg.get("i18n_params").cloned();
                                    let _ = app.emit(
                                        "tagger-progress",
                                        ProgressEvent {
                                            current: (i + results_read) as u32 + 1,
                                            total,
                                            filename: String::new(),
                                            status: "info".to_string(),
                                            message: text,
                                            i18n_key,
                                            i18n_params,
                                        },
                                    );
                                }
                                _ => {
                                    results_read += 1;
                                }
                            }
                        }
                    }
                    Ok(Err(e)) => {
                        fail_count += (batch_len - results_read) as u32;
                        errors.push(format!("批量读取失败: {}", e));
                        break;
                    }
                    Err(_) => {
                        fail_count += (batch_len - results_read) as u32;
                        errors.push("Python 进程退出".to_string());
                        break;
                    }
                }
            }
        }
        i = end;
    }

    // 发送 quit 命令
    let _ = writeln!(stdin, r#"{{"cmd":"quit"}}"#);
    // 取出全局句柄并等待进程退出（若已被取消杀掉则为 None）
    if let Some(mut child) = PYTHON_PROCESS.lock().ok().and_then(|mut g| g.take()) {
        let _ = child.wait();
    }

    let _ = app.emit(
        "tagger-progress",
        ProgressEvent {
            current: total,
            total,
            filename: String::new(),
            status: "done".to_string(),
            message: format!(
                "打标完成: 成功 {}, 失败 {}, 共 {}",
                success_count, fail_count, total
            ),
            ..Default::default()
        },
    );

    Ok(ProcessResult {
        success_count,
        fail_count,
        total,
        errors,
    })
}
