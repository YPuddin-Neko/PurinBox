use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use tauri::Emitter;

use super::{TagCategory, TagDefinition, TaggerOptions, ProcessResult, ProgressEvent, OnnxModelInfo};
use crate::commands::collect_image_files;

/// 从 Windows 注册表读取系统环境变量（GUI 进程可能没有最新环境变量）
#[cfg(target_os = "windows")]
fn read_env_from_registry(name: &str) -> Option<String> {
    use std::process::Command;
    use std::os::windows::process::CommandExt;
    // 使用 reg query 读取系统环境变量
    let output = Command::new("reg")
        .args(["query", r"HKLM\SYSTEM\CurrentControlSet\Control\Session Manager\Environment", "/v", name])
        .creation_flags(0x08000000)
        .output()
        .ok()?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    // 格式: "    CUDA_PATH    REG_SZ    J:\NVIDIA\CUDA"
    for line in stdout.lines() {
        let line = line.trim();
        if line.starts_with(name) {
            // 按空白分割，取最后一个值
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 3 {
                return Some(parts[2..].join(" "));
            }
        }
    }
    // 用户环境变量
    let output = Command::new("reg")
        .args(["query", r"HKCU\Environment", "/v", name])
        .creation_flags(0x08000000)
        .output()
        .ok()?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines() {
        let line = line.trim();
        if line.starts_with(name) {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 3 {
                return Some(parts[2..].join(" "));
            }
        }
    }
    None
}

/// 获取 CUDA 相关环境变量（进程环境 + 注册表补充）
#[cfg(target_os = "windows")]
pub fn get_cuda_env_vars() -> Vec<(String, String)> {
    let mut result: std::collections::HashMap<String, String> = std::collections::HashMap::new();

    // 1. 从进程环境变量获取
    for (key, val) in std::env::vars() {
        if key == "CUDA_PATH" || key.starts_with("CUDA_PATH_V") || key == "CUDA_HOME" {
            result.insert(key, val);
        }
    }

    // 2. 如果进程中没有 CUDA_PATH，尝试从注册表读取
    if !result.contains_key("CUDA_PATH") {
        if let Some(val) = read_env_from_registry("CUDA_PATH") {
            result.insert("CUDA_PATH".to_string(), val);
        }
    }
    // 注册表中的 CUDA_PATH_V* 变量
    for suffix in &["V12_9", "V12_8", "V12_6", "V12_4", "V12_2", "V12_1", "V12_0"] {
        let key = format!("CUDA_PATH_{}", suffix);
        if !result.contains_key(&key) {
            if let Some(val) = read_env_from_registry(&key) {
                result.insert(key, val);
            }
        }
    }

    result.into_iter().collect()
}

/// 将目录下的所有子目录添加到 PATH 字符串中（用于 cuDNN 9.x 的 bin/12.x 结构）
#[cfg(target_os = "windows")]
pub fn add_subdirs_to_path(dir: &str, path: &mut String) {
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                let sub = entry.path();
                let sub_str = sub.to_string_lossy().to_string();
                if !path.contains(&sub_str) {
                    *path = format!("{};{}", sub_str, path);
                }
            }
        }
    }
}

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
                    if next.is_ascii_alphabetic() { break; }
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

/// 公开接口：检测 Apple GPU (macOS)
pub fn detect_apple_gpu_pub(lines: &mut Vec<String>) {
    detect_apple_gpu(lines)
}

/// 检测 Apple Silicon GPU 信息 (macOS)
fn detect_apple_gpu(lines: &mut Vec<String>) {
    // 获取芯片型号
    if let Ok(output) = Command::new("sysctl").args(["-n", "machdep.cpu.brand_string"]).output() {
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

    match run_hidden_cmd(&smi_path, &["--query-gpu=name,driver_version", "--format=csv,noheader,nounits"]) {
        Ok(stdout) => {
            for line in stdout.lines() {
                let line = line.trim();
                if line.is_empty() { continue; }
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
        for (_key, val) in get_cuda_env_vars() {
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
            "Quality" => TagCategory::Quality,
            "Model" => TagCategory::Model,
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
            if output.contains("Python 3")
                && run_hidden_cmd(name, &["-c", "import onnxruntime"]).is_ok() {
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


    // 启动 Python 子进程
    let mut cmd = Command::new(&python);
    cmd.arg(script.to_string_lossy().as_ref())
       .stdin(Stdio::piped())
       .stdout(Stdio::piped())
       .stderr(Stdio::piped())
       .env("NO_COLOR", "1")
       .env("PYTHONUNBUFFERED", "1");

    // Windows: 确保 CUDA/cuDNN DLL 路径在 PATH 中
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW

        if options.use_gpu {
            let mut path = std::env::var("PATH").unwrap_or_default();

            // 辅助函数：添加目录到 PATH（含子目录扫描）
            let mut add_dir = |dir: &str, _label: &str| {
                if std::path::Path::new(dir).exists() && !path.contains(dir) {
                    path = format!("{};{}", dir, path);
                }
            };

            // 1. CUDA 路径：从环境变量读取（含注册表回退）
            for (key, val) in get_cuda_env_vars() {
                let bin = format!(r"{}\bin", val);
                let bin_x64 = format!(r"{}\bin\x64", val); // cuDNN 9.x
                let lib = format!(r"{}\lib\x64", val);
                add_dir(&bin, &key);
                add_dir(&bin_x64, &key);
                add_dir(&lib, &key);
            }

            // 2. cuDNN 路径：从环境变量读取
            //    CUDNN_PATH 可能指向独立安装目录
            if let Ok(cudnn_path) = std::env::var("CUDNN_PATH") {
                let bin = format!(r"{}\bin", cudnn_path);
                add_dir(&bin, "CUDNN");
                // cuDNN 9.x 在 bin 下有 12.x 子目录
                add_subdirs_to_path(&bin, &mut path);
                // lib 下也可能有 12.x 子目录
                let lib = format!(r"{}\lib", cudnn_path);
                add_subdirs_to_path(&lib, &mut path);
            }

            // 3. 扫描 PATH 中已有的目录，找到包含 cuDNN DLL 的目录
            //    并自动添加其子目录（cuDNN 9.x 结构）
            let current_path = path.clone();
            for dir in current_path.split(';') {
                if let Ok(entries) = std::fs::read_dir(dir) {
                    let has_cudnn = entries.into_iter().flatten().any(|e| {
                        let name = e.file_name().to_string_lossy().to_lowercase();
                        name.contains("cudnn") && name.ends_with(".dll")
                    });
                    if has_cudnn {
                        // 如果有子目录也加上
                        add_subdirs_to_path(dir, &mut path);
                    }
                }
            }

            cmd.env("PATH", &path);
        }
    }

    #[cfg(not(target_os = "windows"))]
    {
        let _ = &options; // avoid unused warning
    }

    let mut child = cmd.spawn()
        .map_err(|e| format!("启动 Python 进程失败: {}", e))?;

    let mut stdin = child.stdin.take().ok_or("无法获取 Python stdin")?;
    let stdout = child.stdout.take().ok_or("无法获取 Python stdout")?;
    let stderr = child.stderr.take().ok_or("无法获取 Python stderr")?;

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
                        let line = String::from_utf8(buf.clone())
                            .unwrap_or_else(|_| String::from_utf8_lossy(&buf).to_string());
                        buf.clear();
                        let clean = strip_ansi_codes(&line);
                        let clean = clean.trim();
                        if clean.is_empty() { continue; }
                        let lower = clean.to_lowercase();
                        if lower.contains("context leak")
                            || lower.contains("msgtracer")
                            || lower.contains("number of partitions supported by coreml") {
                            continue;
                        }
                        let _ = app_err.emit("tagger-progress", ProgressEvent {
                            current: 0, total: 0, filename: String::new(),
                            status: "warning".to_string(),
                            message: format!("[Python] {}", clean),
                        });
                    } else if byte[0] != b'\r' {
                        buf.push(byte[0]);
                    }
                }
                Err(_) => break,
            }
        }
    });

    // 发送 init 命令
    let tags_path = model_path.parent()
        .unwrap_or(Path::new("."))
        .join(if tag_defs.is_empty() { "selected_tags.csv" } else {
            let dir = model_path.parent().unwrap_or(Path::new("."));
            if dir.join("tag_mapping.json").exists() {
                "tag_mapping.json"
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
    let _ = app.emit("tagger-progress", ProgressEvent {
        current: 0, total,
        filename: String::new(),
        status: "info".to_string(),
        message: format!("读取到 {} 张图片", total),
    });

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
            "exclude_tags": options.exclude_tags,
            "append_tags": options.append_tags,
            "append_position": options.append_position,
            "replace_underscore": options.replace_underscore,
            "output_format": options.output_format,
            "json_simplified": options.json_simplified,
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
