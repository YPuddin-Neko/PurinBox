//! Python 子进程共享基础设施
//!
//! 五个 AI 功能（打标 / 超分 / 人物裁切 / 美学评分 / 聚类）都以子进程方式调用
//! 打包在 scripts/ 下的 Python 脚本，此模块收敛两块原先各自复制的逻辑：
//! 1. 推理脚本路径解析（开发 / exe 同级 scripts / NSIS exe 同级 / macOS Resources）
//! 2. Windows 下 CUDA/cuDNN DLL 的 PATH 注入（进程环境 + 注册表回退 + cuDNN 9.x 子目录）

use std::io::{BufRead, BufReader, Read};
use std::path::PathBuf;
use std::process::Command;

/// 逐行读取 Python 子进程的 stderr，每行解码、清理后回调（空行不回调）。
///
/// 五个功能模块的 stderr 线程共用这一个入口：读到 EOF 或管道出错即返回。
/// 不能用 `BufRead::lines()`——它遇到非 UTF-8 字节直接返回 Err，
/// 配合 `map_while(Result::ok)` 会让整个线程在第一行 GBK 输出处停止读取，
/// 之后管道缓冲区一满 Python 就永久阻塞在写日志上。
pub fn for_each_stderr_line<R: Read>(reader: R, mut on_line: impl FnMut(String)) {
    let mut reader = BufReader::new(reader);
    let mut buf = Vec::new();
    loop {
        buf.clear();
        match reader.read_until(b'\n', &mut buf) {
            Ok(0) => break,
            Ok(_) => {
                let line = decode_python_line(&buf);
                if !line.is_empty() {
                    on_line(line);
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => break,
        }
    }
}

/// 解码 Python 子进程输出的一行字节，并剥掉 ANSI 转义序列与控制字符。
///
/// Windows 上 onnxruntime 的 `CLogSink` 走 `std::wclog` 宽字符流，而 Python 会把
/// CRT 的 stderr 设成二进制模式，于是 ORT 的每条日志都以 UTF-16LE 裸字节进了管道：
/// `[W:onnxruntime:, transformer_memcpy.cc:111 ...]` 到这里就是每个字符夹一个 NUL。
/// 按 UTF-8 读它不会报错（NUL 是合法 UTF-8），所以 UTF-16 判定必须放在 UTF-8 之前。
/// 其余情况按 UTF-8 解，失败再按中文 Windows 的 GBK 解。
pub fn decode_python_line(bytes: &[u8]) -> String {
    // 行尾的 `\n` 是 read_until 的分隔符本身；UTF-16 下它的高位 NUL 已落到下一行开头
    let bytes = bytes.strip_suffix(b"\n").unwrap_or(bytes);
    let text = decode_bytes(bytes);
    let text = strip_ansi_sequences(&text);
    let text: String = text
        .chars()
        .filter(|c| !c.is_control() || *c == '\t')
        .collect();
    text.trim().to_string()
}

fn decode_bytes(bytes: &[u8]) -> String {
    let bytes = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
    if let Some(rest) = bytes.strip_prefix(&[0xFF, 0xFE]) {
        return encoding_rs::UTF_16LE
            .decode_without_bom_handling(rest)
            .0
            .into_owned();
    }
    if let Some(rest) = bytes.strip_prefix(&[0xFE, 0xFF]) {
        return encoding_rs::UTF_16BE
            .decode_without_bom_handling(rest)
            .0
            .into_owned();
    }
    match detect_utf16(bytes) {
        Some(Utf16Layout::Le) => encoding_rs::UTF_16LE
            .decode_without_bom_handling(bytes)
            .0
            .into_owned(),
        Some(Utf16Layout::LeAfterStrayNul) => encoding_rs::UTF_16LE
            .decode_without_bom_handling(&bytes[1..])
            .0
            .into_owned(),
        Some(Utf16Layout::Be) => encoding_rs::UTF_16BE
            .decode_without_bom_handling(bytes)
            .0
            .into_owned(),
        None => match std::str::from_utf8(bytes) {
            Ok(s) => s.to_owned(),
            Err(_) => encoding_rs::GBK
                .decode_without_bom_handling(bytes)
                .0
                .into_owned(),
        },
    }
}

enum Utf16Layout {
    /// `XX 00 XX 00 ...`
    Le,
    /// `00 XX 00 XX 00 ...`：UTF-16LE 流按 `\n`（0A）切行时，
    /// 上一行换行符的高位 NUL 被留到了本行开头，长度必为奇数
    LeAfterStrayNul,
    /// `00 XX 00 XX`，偶数长度
    Be,
}

/// 无 BOM 的 UTF-16 识别：UTF-8/GBK 文本里根本不会出现 NUL，
/// 所以半数以上的码元在同一侧是 NUL 就足以判定。
/// 只看前 64 字节——ORT 日志前缀（颜色码 + 时间戳 + 位置）全是 ASCII，
/// 即便后面跟着中文路径也不影响判定。
fn detect_utf16(bytes: &[u8]) -> Option<Utf16Layout> {
    if bytes.len() < 4 {
        return None;
    }
    let sample = &bytes[..bytes.len().min(64)];
    let pairs = sample.len() / 2;
    let even_nul = sample.iter().step_by(2).filter(|&&b| b == 0).count();
    let odd_nul = sample.iter().skip(1).step_by(2).filter(|&&b| b == 0).count();
    if odd_nul * 2 >= pairs && even_nul * 4 <= pairs {
        Some(Utf16Layout::Le)
    } else if even_nul * 2 >= pairs && odd_nul * 4 <= pairs {
        if bytes.len() % 2 == 1 {
            Some(Utf16Layout::LeAfterStrayNul)
        } else {
            Some(Utf16Layout::Be)
        }
    } else {
        None
    }
}

/// 剥掉 ANSI CSI 转义序列（`ESC [ 参数… 终止字节`，终止字节 0x40–0x7E）。
/// 必须在剥控制字符之前做：先把 ESC 当控制字符删掉，`[0;93m` 就会作为正文残留。
fn strip_ansi_sequences(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\x1b' {
            out.push(c);
            continue;
        }
        if chars.peek() == Some(&'[') {
            chars.next();
            for next in chars.by_ref() {
                if ('\x40'..='\x7e').contains(&next) {
                    break;
                }
            }
        }
    }
    out
}

/// 解析打包的 Python 脚本路径。
///
/// 按顺序尝试四个候选位置，全部未命中时在错误信息中列出搜索路径。
pub fn find_script(script_name: &str) -> Result<PathBuf, String> {
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|p| p.to_path_buf()))
        .unwrap_or_else(|| PathBuf::from("."));

    let candidates = vec![
        // 开发模式: CARGO_MANIFEST_DIR/scripts/
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!("scripts/{}", script_name)),
        // 生产模式 Windows/Linux: exe 同级 scripts/
        exe_dir.join(format!("scripts/{}", script_name)),
        // 生产模式 Windows NSIS: exe 同级
        exe_dir.join(script_name),
        // macOS .app bundle: Resources/scripts/
        exe_dir.join(format!("../Resources/scripts/{}", script_name)),
    ];

    for path in &candidates {
        if path.exists() {
            return Ok(path.canonicalize().unwrap_or_else(|_| path.clone()));
        }
    }

    let paths_str = candidates
        .iter()
        .enumerate()
        .map(|(i, p)| format!("  {}. {}", i + 1, p.display()))
        .collect::<Vec<_>>()
        .join("\n");
    Err(format!(
        "推理脚本 {} 未找到。\n搜索路径:\n{}",
        script_name, paths_str
    ))
}

/// 为 Python 子进程应用平台配置：
/// - Windows: CREATE_NO_WINDOW；use_gpu 时把 CUDA/cuDNN DLL 目录注入 PATH
/// - 其他平台: 无操作
#[cfg(target_os = "windows")]
pub fn configure_python_command(cmd: &mut Command, use_gpu: bool) {
    use std::os::windows::process::CommandExt;
    cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW

    if use_gpu {
        cmd.env("PATH", build_cuda_enhanced_path());
    }
}

#[cfg(not(target_os = "windows"))]
pub fn configure_python_command(cmd: &mut Command, _use_gpu: bool) {
    // 让子进程自成进程组：kill_process_tree 的 `kill -9 -PID` 需要 PGID==PID 才能
    // 连同 torch/onnxruntime 派生的 worker 一起杀掉，否则永远走单杀回退留下孤儿
    use std::os::unix::process::CommandExt;
    cmd.process_group(0);
}

/// 从 Windows 注册表读取系统环境变量（GUI 进程可能没有最新环境变量）
#[cfg(target_os = "windows")]
fn read_env_from_registry(name: &str) -> Option<String> {
    use std::os::windows::process::CommandExt;
    // 使用 reg query 读取系统环境变量
    let output = Command::new("reg")
        .args([
            "query",
            r"HKLM\SYSTEM\CurrentControlSet\Control\Session Manager\Environment",
            "/v",
            name,
        ])
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
    for suffix in &[
        "V12_9", "V12_8", "V12_6", "V12_4", "V12_2", "V12_1", "V12_0",
    ] {
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
fn add_subdirs_to_path(dir: &str, path: &mut String) {
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

/// 构建注入了 CUDA/cuDNN DLL 目录的 PATH 值
#[cfg(target_os = "windows")]
fn build_cuda_enhanced_path() -> String {
    let mut path = std::env::var("PATH").unwrap_or_default();

    // 辅助闭包：目录存在且未加入时前置到 PATH
    let mut add_dir = |dir: &str| {
        if std::path::Path::new(dir).exists() && !path.contains(dir) {
            path = format!("{};{}", dir, path);
        }
    };

    // 1. CUDA 路径：从环境变量读取（含注册表回退）
    for (_key, val) in get_cuda_env_vars() {
        let bin = format!(r"{}\bin", val);
        let bin_x64 = format!(r"{}\bin\x64", val); // cuDNN 9.x
        let lib = format!(r"{}\lib\x64", val);
        add_dir(&bin);
        add_dir(&bin_x64);
        add_dir(&lib);
    }

    // 2. cuDNN 路径：CUDNN_PATH 可能指向独立安装目录
    if let Ok(cudnn_path) = std::env::var("CUDNN_PATH") {
        let bin = format!(r"{}\bin", cudnn_path);
        add_dir(&bin);
        // cuDNN 9.x 在 bin/lib 下有 12.x 子目录
        add_subdirs_to_path(&bin, &mut path);
        let lib = format!(r"{}\lib", cudnn_path);
        add_subdirs_to_path(&lib, &mut path);
    }

    // 3. 扫描 PATH 中已含 cuDNN DLL 的目录，自动加其子目录（cuDNN 9.x 结构）
    let current_path = path.clone();
    for dir in current_path.split(';') {
        if let Ok(entries) = std::fs::read_dir(dir) {
            let has_cudnn = entries.into_iter().flatten().any(|e| {
                let name = e.file_name().to_string_lossy().to_lowercase();
                name.contains("cudnn") && name.ends_with(".dll")
            });
            if has_cudnn {
                add_subdirs_to_path(dir, &mut path);
            }
        }
    }

    path
}

#[cfg(test)]
mod decode_tests {
    use super::*;

    /// Windows 上 ORT 实际写进管道的形态：UTF-16LE 裸字节，含颜色码与时间戳
    const ORT_WARNING: &str = "\x1b[0;93m2026-09-16 08:38:45.1234567 [W:onnxruntime:, transformer_memcpy.cc:111 onnxruntime::MemcpyTransformer::ApplyImpl] 11 Memcpy nodes are added to the graph main_graph for CUDAExecutionProvider. It might have negative impact on performance (including unable to run CUDA graph). Set session_options.log_severity_level=1 to see the detail logs before this message.\x1b[m";

    fn utf16le(s: &str) -> Vec<u8> {
        s.encode_utf16().flat_map(|u| u.to_le_bytes()).collect()
    }

    fn collect_lines(stream: &[u8]) -> Vec<String> {
        let mut out = Vec::new();
        for_each_stderr_line(std::io::Cursor::new(stream), |l| out.push(l));
        out
    }

    #[test]
    fn utf16le_ort_warning_becomes_readable_text() {
        let line = decode_python_line(&utf16le(&format!("{}\n", ORT_WARNING)));
        assert!(line.starts_with("2026-09-16 08:38:45.1234567 [W:onnxruntime:, transformer_memcpy.cc:111"), "{line}");
        assert!(line.ends_with("before this message."), "{line}");
        assert!(!line.contains('\0') && !line.contains('\u{FFFD}') && !line.contains("[0;93m"), "{line}");
        // 各模块按这个关键词过滤 ORT 噪音，解码后必须认得出来
        assert!(line.to_lowercase().contains("onnxruntime"));
    }

    #[test]
    fn utf16le_stream_splits_into_clean_lines_despite_stray_nul() {
        // 按 0A 切行后，第二行开头会带上一行换行符的高位 NUL；第三行是 \r\n 结尾
        let stream = utf16le(&format!("{}\nsecond line 第二行\r\nthird\n", ORT_WARNING));
        let lines = collect_lines(&stream);
        assert_eq!(lines.len(), 3, "{lines:?}");
        assert!(lines[0].contains("Memcpy nodes are added"));
        assert_eq!(lines[1], "second line 第二行");
        assert_eq!(lines[2], "third");
    }

    #[test]
    fn utf16le_with_cjk_path_keeps_cjk() {
        let text = "2026-09-16 08:38:45.000 [E:onnxruntime:, inference_session.cc:2 Load] 加载 D:\\模型\\wd-eva02.onnx 失败";
        assert_eq!(decode_python_line(&utf16le(text)), text);
    }

    #[test]
    fn utf16le_bom_is_honoured() {
        let mut bytes = vec![0xFF, 0xFE];
        bytes.extend(utf16le("with bom"));
        assert_eq!(decode_python_line(&bytes), "with bom");
    }

    #[test]
    fn plain_utf8_and_gbk_still_decode() {
        assert_eq!(decode_python_line("警告: 模型 abc.onnx 加载慢\n".as_bytes()), "警告: 模型 abc.onnx 加载慢");
        let (gbk, _, _) = encoding_rs::GBK.encode("模型文件不存在: 中文路径");
        assert_eq!(decode_python_line(&gbk), "模型文件不存在: 中文路径");
    }

    #[test]
    fn ansi_sequences_stripped_but_bracketed_text_kept() {
        assert_eq!(decode_python_line(b"\x1b[0;93mwarn\x1b[m text\x1b[1;31m!\x1b[0m"), "warn text!");
        assert_eq!(decode_python_line(b"[W:onnxruntime:Default, x.cc:1] [1m 30s] done"), "[W:onnxruntime:Default, x.cc:1] [1m 30s] done");
    }

    #[test]
    fn control_chars_dropped_tab_kept_blank_lines_skipped() {
        assert_eq!(decode_python_line(b"a\x00b\tc\r\n"), "ab\tc");
        assert!(collect_lines(b"\n   \n\r\n\x00\x00\n").is_empty());
    }

    #[test]
    fn invalid_utf8_does_not_stop_the_reader() {
        let mut stream = Vec::new();
        stream.extend_from_slice(b"first\n");
        stream.extend_from_slice(&encoding_rs::GBK.encode("第二行是 GBK").0);
        stream.extend_from_slice(b"\nthird\n");
        assert_eq!(collect_lines(&stream), vec!["first", "第二行是 GBK", "third"]);
    }
}
