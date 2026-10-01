//! Python 子进程共享基础设施
//!
//! 五个 AI 功能（打标 / 超分 / 人物裁切 / 美学评分 / 聚类）都以子进程方式调用
//! 打包在 scripts/ 下的 Python 脚本，共用这里的：
//! 1. 子进程平台配置：`hidden_command` 只隐藏 Windows 控制台窗口，给短命令和探测用；
//!    `configure_python_command(_with_priority)` 给推理进程用，另外让子进程自成进程组
//!    （非 Windows，供按进程树终止）、可选降低优先级、GPU 模式注入 CUDA/cuDNN DLL 的 PATH
//!    （进程环境 + 注册表回退 + cuDNN 9.x 子目录）
//! 2. stdout 协议读取：`ProtocolReader` 在后台线程逐行解析 JSON，接收端带静默时限或总时限
//! 3. stderr 逐行读取与解码（UTF-16LE / UTF-8 / GBK，剥掉 ANSI 转义与控制字符）和噪音过滤
//! 4. 参数驱动、不读 stdin 的 JSON-lines 脚本运行器 `run_json_lines_script`，以及 PID 登记
//! 5. 推理脚本路径解析（开发 / exe 同级 scripts / NSIS exe 同级 / macOS Resources）

use std::ffi::OsStr;
use std::io::{BufRead, BufReader, Read};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Mutex};
use std::time::{Duration, Instant};

#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x08000000;

/// 协议读取的静默上限：超过该时长收不到任何协议消息，视为 Python 进程卡死。
/// 接收端不能裸 recv() 死等：Python 卡死（进程存活但不再输出）时任务会永久停滞；
/// 取消时进程树被杀、stdout 关闭，`ProtocolReader` 返回 `Recv::Closed` 正常收尾。
pub const PYTHON_SILENCE_LIMIT: Duration = Duration::from_secs(300);

/// 运行器检查取消标志的间隔
const CANCEL_POLL: Duration = Duration::from_millis(200);

/// 新建子进程命令；Windows 上设置 CREATE_NO_WINDOW，GUI 进程每起一个子进程就会闪一下控制台窗口。
///
/// 只管窗口，不设进程组、不注入 CUDA PATH，用于 reg / tar / pip / nvidia-smi / taskkill
/// 这类短命令和 `python -c` 探测。需要按进程树终止的推理进程用 `configure_python_command`。
/// Windows 的 creation_flags 是覆盖式设置，之后再调 configure_python_command 会整体替换这里的标志。
pub fn hidden_command(program: impl AsRef<OsStr>) -> Command {
    let cmd = Command::new(program);
    #[cfg(target_os = "windows")]
    let cmd = {
        use std::os::windows::process::CommandExt;
        let mut cmd = cmd;
        cmd.creation_flags(CREATE_NO_WINDOW);
        cmd
    };
    cmd
}

// ───────────────────────── stdout 协议读取 ─────────────────────────

/// `ProtocolReader` 一次接收的结果
#[derive(Debug, Clone, PartialEq)]
pub enum Recv {
    /// 一条协议消息：stdout 上的一行 JSON 对象
    Msg(serde_json::Value),
    /// stdout 已关闭（进程退出、被杀或读取出错），之前收到的消息都已取完
    Closed,
    /// 时限内没有新消息
    TimedOut,
}

/// Python 子进程 stdout 的 JSON-lines 协议读取器。
///
/// 后台线程按字节逐行读取（`read_until` + `from_utf8_lossy`）：
/// - 非 UTF-8 字节解成替换字符后照常解析。`BufRead::lines()` 会在第一行坏字节处报错停读，
///   之后管道写满，Python 永久阻塞在写输出上；
/// - 不套用 stderr 的 `decode_python_line`：`json.dumps(ensure_ascii=False)` 不转义
///   U+0080–U+009F，那里的控制字符过滤会改写路径；
/// - 只投递 JSON 对象行。其余行（ORT 打到 stdout 的 "EP Error"、第三方 .pth 的 print、
///   空行、裸数字）直接丢弃，所以静默计时只认协议消息；
/// - 读到 EOF 或读取出错即关闭通道，排空已收到的消息后 `recv` 返回 `Closed`。
///
/// 读取线程在 stdout 关闭时退出。调用方提前返回前要先终止子进程，否则线程会一直阻塞在读上。
pub struct ProtocolReader {
    rx: mpsc::Receiver<serde_json::Value>,
}

impl ProtocolReader {
    /// 在后台线程读取 `stdout`（一般是 `ChildStdout`；测试里可以是内存 reader）
    pub fn spawn<R: Read + Send + 'static>(stdout: R) -> Self {
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            let mut buf = Vec::new();
            loop {
                buf.clear();
                match reader.read_until(b'\n', &mut buf) {
                    Ok(0) => break,
                    Ok(_) => {
                        if let Some(msg) = parse_protocol_line(&buf) {
                            if tx.send(msg).is_err() {
                                break;
                            }
                        }
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                    Err(_) => break,
                }
            }
        });
        ProtocolReader { rx }
    }

    /// 等下一条消息，最多等 `silence`。每次调用重新计时，循环调用就是静默超时：
    /// 只要持续有消息（包括 log）就不会超时。
    pub fn recv(&self, silence: Duration) -> Recv {
        match self.rx.recv_timeout(silence) {
            Ok(msg) => Recv::Msg(msg),
            Err(mpsc::RecvTimeoutError::Timeout) => Recv::TimedOut,
            Err(mpsc::RecvTimeoutError::Disconnected) => Recv::Closed,
        }
    }

    /// 等下一条消息，最晚等到 `deadline`。循环调用时截止时刻不变，就是总时限。
    /// 截止时刻已过时仍会取走已在队列里的消息，队列为空才返回 `TimedOut`。
    pub fn recv_until(&self, deadline: Instant) -> Recv {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if !remaining.is_zero() {
            return self.recv(remaining);
        }
        match self.rx.try_recv() {
            Ok(msg) => Recv::Msg(msg),
            Err(mpsc::TryRecvError::Empty) => Recv::TimedOut,
            Err(mpsc::TryRecvError::Disconnected) => Recv::Closed,
        }
    }
}

/// 一行 stdout → 协议消息；不是 JSON 对象就返回 None
fn parse_protocol_line(bytes: &[u8]) -> Option<serde_json::Value> {
    let text = String::from_utf8_lossy(bytes);
    let text = text.strip_prefix('\u{feff}').unwrap_or(&text);
    match serde_json::from_str::<serde_json::Value>(text) {
        Ok(msg @ serde_json::Value::Object(_)) => Some(msg),
        _ => None,
    }
}

// ───────────────────────── stderr ─────────────────────────

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

/// CUDA/cuDNN DLL 探测与 macOS CoreML 的提示（小写匹配）。
/// CoreML 的 context leak / msgtracer 不受 ORT 日志级别控制，只能在这里兜底
const RUNTIME_NOISE_KEYWORDS: [&str; 7] = [
    "cudnn",
    "cuda_path",
    "could not load",
    "loaded library",
    "context leak",
    "msgtracer",
    "number of partitions supported by coreml",
];

/// stderr 行是不是推理运行时（onnxruntime / CUDA / CoreML）的噪音，各模块的 stderr 线程共用。
///
/// ORT 自己的日志只滤 Info / Warning 级（`[I:onnxruntime:` / `[W:onnxruntime:`）。
/// Error / Fatal 级（`[E:` / `[F:`）一律放行，即使行里带 cudnn 之类的关键词：
/// 会话已压到 Error 级，这时还冒出来的就是真错误（例如 cuDNN 找不到导致 GPU 回退 CPU）。
pub fn is_runtime_noise(line: &str) -> bool {
    if line.contains("[E:onnxruntime:") || line.contains("[F:onnxruntime:") {
        return false;
    }
    if line.contains("[W:onnxruntime:") || line.contains("[I:onnxruntime:") {
        return true;
    }
    let lower = line.to_lowercase();
    RUNTIME_NOISE_KEYWORDS.iter().any(|k| lower.contains(k))
}

/// stderr 行是不是 Python 库的告警或进度条噪音（聚类用：sklearn / umap / torch 的 warnings、
/// tqdm 进度条、torchvision 下载权重的输出）。与 `is_runtime_noise` 叠加使用。
///
/// tqdm 用 `\r` 刷新同一行，换行前的整串刷新会作为一行到达，所以按 `%` 的个数判断。
pub fn is_python_library_noise(line: &str) -> bool {
    line.matches('%').count() > 3
        || line.contains("UserWarning")
        || line.contains("FutureWarning")
        || line.contains("RuntimeWarning")
        || line.starts_with("Downloading:")
        || line.starts_with("100%")
        || line == "warn("
        || line.starts_with("warnings.warn(")
        // warnings 模块回显的源码位置行，例如 `.../site-packages/sklearn/x.py:12: ...`
        || (line.contains(".py:")
            && (line.contains("site-packages/") || line.contains("site-packages\\")))
        || line.starts_with("eigenvalues")
        || line.starts_with("scipy.")
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
    let odd_nul = sample
        .iter()
        .skip(1)
        .step_by(2)
        .filter(|&&b| b == 0)
        .count();
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

// ───────────────────────── PID 登记与脚本运行器 ─────────────────────────

/// 子进程 PID 登记：存活期间槽里是该 PID，供取消命令按进程树终止；drop 时清除。
/// 只在槽里仍是自己的 PID 时才清除——取消命令已经取走、或已被后来者覆盖时不动。
pub struct PidRegistration<'a> {
    slot: &'a Mutex<Option<u32>>,
    pid: u32,
}

impl<'a> PidRegistration<'a> {
    pub fn new(slot: &'a Mutex<Option<u32>>, pid: u32) -> Self {
        *slot.lock().unwrap_or_else(|e| e.into_inner()) = Some(pid);
        PidRegistration { slot, pid }
    }
}

impl Drop for PidRegistration<'_> {
    fn drop(&mut self) {
        let mut guard = self.slot.lock().unwrap_or_else(|e| e.into_inner());
        if *guard == Some(self.pid) {
            *guard = None;
        }
    }
}

/// 取走槽里登记的 PID 并按进程树终止（取消命令用）；没有登记时什么也不做
pub fn kill_registered_pid(slot: &Mutex<Option<u32>>) {
    let pid = slot.lock().unwrap_or_else(|e| e.into_inner()).take();
    if let Some(pid) = pid {
        super::kill_process_tree(pid);
    }
}

/// `run_json_lines_script` 的结束状态
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScriptExit {
    /// 子进程退出码。Unix 上被信号终止时为 None；Windows 上被强杀时一般是 1，
    /// 所以判断是不是取消要看 `cancelled`
    pub code: Option<i32>,
    /// 结束时取消标志是否置位。运行中检测到取消时已按进程树终止子进程
    pub cancelled: bool,
}

/// 运行参数驱动、stdout 逐行输出 JSON、不读 stdin 的 Python 脚本（超分 Real-ESRGAN、聚类）。
///
/// 调用方只需填好程序、参数和业务环境变量，这里负责：
/// - stdin 置空，stdout/stderr 走管道，设 `PYTHONUNBUFFERED=1`、`PYTHONIOENCODING=utf-8`；
/// - `configure_python_command(cmd, use_gpu)`：Windows 隐藏窗口（use_gpu 时注入 CUDA PATH），
///   其他平台自成进程组，取消时才能连同 worker 一起按组终止；
/// - 在 `pid_slot` 登记 PID，返回前清除（取消命令可用 `kill_registered_pid` 强杀）；
/// - stderr 由后台线程逐行解码后交给 `on_stderr`，stdout 经 `ProtocolReader` 逐条交给 `on_message`；
/// - 每 200ms 检查一次 `cancel`，脚本长时间不输出时取消也能生效。
///
/// 结束方式：
/// - `on_message` 返回 Err：终止进程树并原样返回这个 Err（脚本报 error 消息时用）；
/// - 检测到取消：终止进程树，返回 `ScriptExit { cancelled: true, .. }`；
/// - stdout 关闭：等子进程退出后返回退出码。没收到终态消息时怎么上报由调用方决定。
///
/// 阻塞函数，调用方放进 `spawn_blocking`。启动失败返回 "启动 Python 失败: …"。
pub fn run_json_lines_script(
    mut cmd: Command,
    use_gpu: bool,
    pid_slot: &Mutex<Option<u32>>,
    cancel: &AtomicBool,
    on_stderr: impl FnMut(String) + Send + 'static,
    mut on_message: impl FnMut(serde_json::Value) -> Result<(), String>,
) -> Result<ScriptExit, String> {
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env("PYTHONUNBUFFERED", "1")
        .env("PYTHONIOENCODING", "utf-8");
    configure_python_command(&mut cmd, use_gpu);

    let mut child = cmd
        .spawn()
        .map_err(|e| format!("启动 Python 失败: {}", e))?;
    let _registration = PidRegistration::new(pid_slot, child.id());

    let (Some(stdout), Some(stderr)) = (child.stdout.take(), child.stderr.take()) else {
        terminate(&mut child);
        return Err("无法获取 Python 进程管道".into());
    };
    std::thread::spawn(move || for_each_stderr_line(stderr, on_stderr));
    let reader = ProtocolReader::spawn(stdout);

    loop {
        if cancel.load(Ordering::SeqCst) {
            terminate(&mut child);
            break;
        }
        match reader.recv(CANCEL_POLL) {
            Recv::Msg(msg) => {
                if let Err(e) = on_message(msg) {
                    terminate(&mut child);
                    return Err(e);
                }
            }
            Recv::TimedOut => {}
            Recv::Closed => break,
        }
    }

    let code = child.wait().ok().and_then(|status| status.code());
    Ok(ScriptExit {
        code,
        cancelled: cancel.load(Ordering::SeqCst),
    })
}

/// 按进程树终止并回收子进程
fn terminate(child: &mut Child) {
    super::kill_process_tree(child.id());
    let _ = child.kill();
    let _ = child.wait();
}

// ───────────────────────── 脚本路径与平台配置 ─────────────────────────

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

/// 为 Python 子进程应用平台配置（不调整优先级）：
/// - Windows: CREATE_NO_WINDOW；use_gpu 时把 CUDA/cuDNN DLL 目录注入 PATH
/// - 其他平台: 让子进程自成进程组，use_gpu 不起作用
pub fn configure_python_command(cmd: &mut Command, use_gpu: bool) {
    configure_python_command_with_priority(cmd, use_gpu, false);
}

/// 同 configure_python_command；background_priority 时为需要大量 GPU 计算的
/// 后台任务降低进程调度优先级。
/// Windows 桌面合成仍可优先获得 CPU/GPU 调度时间，推理空闲时吞吐不变。
#[cfg(target_os = "windows")]
pub fn configure_python_command_with_priority(
    cmd: &mut Command,
    use_gpu: bool,
    background_priority: bool,
) {
    use std::os::windows::process::CommandExt;
    const BELOW_NORMAL_PRIORITY_CLASS: u32 = 0x00004000;
    let priority = if background_priority {
        BELOW_NORMAL_PRIORITY_CLASS
    } else {
        0
    };
    cmd.creation_flags(CREATE_NO_WINDOW | priority);

    if use_gpu {
        cmd.env("PATH", build_cuda_enhanced_path());
    }
}

#[cfg(not(target_os = "windows"))]
pub fn configure_python_command_with_priority(
    cmd: &mut Command,
    _use_gpu: bool,
    _background_priority: bool,
) {
    // 让子进程自成进程组：kill_process_tree 的 `kill -9 -PID` 需要 PGID==PID 才能
    // 连同 torch/onnxruntime 派生的 worker 一起杀掉，否则永远走单杀回退留下孤儿
    use std::os::unix::process::CommandExt;
    cmd.process_group(0);
}

/// 从 Windows 注册表读取系统/用户环境变量（GUI 进程可能没有最新环境变量）
#[cfg(target_os = "windows")]
fn read_env_from_registry(name: &str) -> Option<String> {
    const HIVES: [&str; 2] = [
        r"HKLM\SYSTEM\CurrentControlSet\Control\Session Manager\Environment",
        r"HKCU\Environment",
    ];
    HIVES.iter().find_map(|hive| {
        let output = hidden_command("reg")
            .args(["query", hive, "/v", name])
            .output()
            .ok()?;
        parse_reg_query_value(&String::from_utf8_lossy(&output.stdout), name)
    })
}

/// 从 `reg query <hive> /v <name>` 的输出中取值。
/// 格式: "    CUDA_PATH    REG_SZ    J:\NVIDIA\CUDA"；第三列起是值，路径可能含空格，按空格拼回
#[cfg(any(target_os = "windows", test))]
fn parse_reg_query_value(stdout: &str, name: &str) -> Option<String> {
    stdout
        .lines()
        .map(str::trim)
        .filter(|line| line.starts_with(name))
        .find_map(|line| {
            let parts: Vec<&str> = line.split_whitespace().collect();
            (parts.len() >= 3).then(|| parts[2..].join(" "))
        })
}

/// 获取 CUDA 相关环境变量：进程环境优先，缺的再查注册表
#[cfg(target_os = "windows")]
fn get_cuda_env_vars() -> Vec<(String, String)> {
    use std::collections::hash_map::{Entry, HashMap};

    let mut result: HashMap<String, String> = std::env::vars()
        .filter(|(key, _)| {
            key == "CUDA_PATH" || key.starts_with("CUDA_PATH_V") || key == "CUDA_HOME"
        })
        .collect();

    for key in [
        "CUDA_PATH",
        "CUDA_PATH_V12_9",
        "CUDA_PATH_V12_8",
        "CUDA_PATH_V12_6",
        "CUDA_PATH_V12_4",
        "CUDA_PATH_V12_2",
        "CUDA_PATH_V12_1",
        "CUDA_PATH_V12_0",
    ] {
        if let Entry::Vacant(slot) = result.entry(key.to_string()) {
            if let Some(val) = read_env_from_registry(key) {
                slot.insert(val);
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
        assert!(
            line.starts_with(
                "2026-09-16 08:38:45.1234567 [W:onnxruntime:, transformer_memcpy.cc:111"
            ),
            "{line}"
        );
        assert!(line.ends_with("before this message."), "{line}");
        assert!(
            !line.contains('\0') && !line.contains('\u{FFFD}') && !line.contains("[0;93m"),
            "{line}"
        );
        // 噪音过滤按 `[W:onnxruntime:` 前缀识别 ORT 告警，解码后必须认得出来
        assert!(is_runtime_noise(&line));
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
        assert_eq!(
            decode_python_line("警告: 模型 abc.onnx 加载慢\n".as_bytes()),
            "警告: 模型 abc.onnx 加载慢"
        );
        let (gbk, _, _) = encoding_rs::GBK.encode("模型文件不存在: 中文路径");
        assert_eq!(decode_python_line(&gbk), "模型文件不存在: 中文路径");
    }

    #[test]
    fn ansi_sequences_stripped_but_bracketed_text_kept() {
        assert_eq!(
            decode_python_line(b"\x1b[0;93mwarn\x1b[m text\x1b[1;31m!\x1b[0m"),
            "warn text!"
        );
        assert_eq!(
            decode_python_line(b"[W:onnxruntime:Default, x.cc:1] [1m 30s] done"),
            "[W:onnxruntime:Default, x.cc:1] [1m 30s] done"
        );
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
        assert_eq!(
            collect_lines(&stream),
            vec!["first", "第二行是 GBK", "third"]
        );
    }
}

#[cfg(test)]
mod protocol_tests {
    use super::*;
    use serde_json::json;
    use std::io::Cursor;

    const WAIT: Duration = Duration::from_secs(5);

    /// 由测试逐块喂数据的 stdout：没有数据时阻塞，发送端 drop 即 EOF
    struct ChunkReader {
        rx: mpsc::Receiver<Vec<u8>>,
        pending: Vec<u8>,
    }

    impl Read for ChunkReader {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            while self.pending.is_empty() {
                match self.rx.recv() {
                    Ok(chunk) => self.pending = chunk,
                    Err(_) => return Ok(0),
                }
            }
            let n = buf.len().min(self.pending.len());
            buf[..n].copy_from_slice(&self.pending[..n]);
            self.pending.drain(..n);
            Ok(n)
        }
    }

    fn chunked() -> (mpsc::Sender<Vec<u8>>, ProtocolReader) {
        let (tx, rx) = mpsc::channel();
        let reader = ProtocolReader::spawn(ChunkReader {
            rx,
            pending: Vec::new(),
        });
        (tx, reader)
    }

    /// 先给一行合法 JSON，再报读取错误
    struct FailingReader {
        sent: bool,
    }

    impl Read for FailingReader {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            if self.sent {
                return Err(std::io::Error::other("pipe broken"));
            }
            self.sent = true;
            let line = b"{\"type\":\"ready\"}\n";
            buf[..line.len()].copy_from_slice(line);
            Ok(line.len())
        }
    }

    #[test]
    fn delivers_messages_in_order_then_closes_on_eof() {
        let reader = ProtocolReader::spawn(Cursor::new(
            b"{\"type\":\"ready\"}\n{\"type\":\"log\",\"message\":\"hi\"}\r\n".to_vec(),
        ));
        assert_eq!(reader.recv(WAIT), Recv::Msg(json!({"type": "ready"})));
        assert_eq!(
            reader.recv(WAIT),
            Recv::Msg(json!({"type": "log", "message": "hi"}))
        );
        assert_eq!(reader.recv(WAIT), Recv::Closed);
        // 关闭后保持 Closed，不会变成超时
        assert_eq!(reader.recv(Duration::from_millis(10)), Recv::Closed);
    }

    /// stdout 上混入的非协议行（ORT 的 EP Error、空行、裸数字、数组）都不算消息
    #[test]
    fn non_json_and_non_object_lines_are_dropped() {
        let reader = ProtocolReader::spawn(Cursor::new(
            b"EP Error: CUDA failed\n\n42\n[1,2]\n\"text\"\n{\"type\":\"ready\"}\n{broken json\n"
                .to_vec(),
        ));
        assert_eq!(reader.recv(WAIT), Recv::Msg(json!({"type": "ready"})));
        assert_eq!(reader.recv(WAIT), Recv::Closed);
    }

    /// 坏字节按替换字符解码，后面的行照常读到
    #[test]
    fn invalid_utf8_is_decoded_lossily_and_reading_continues() {
        let mut stream = Vec::new();
        stream.extend_from_slice(b"{\"type\":\"log\",\"message\":\"bad \xff\xfe bytes\"}\n");
        stream.extend_from_slice(&encoding_rs::GBK.encode("GBK 写的非协议行").0);
        stream.extend_from_slice(b"\n{\"type\":\"result\",\"score\":0.5}\n");
        let reader = ProtocolReader::spawn(Cursor::new(stream));
        match reader.recv(WAIT) {
            Recv::Msg(msg) => {
                let text = msg["message"].as_str().unwrap();
                assert!(
                    text.starts_with("bad ") && text.contains('\u{FFFD}'),
                    "{text}"
                );
            }
            other => panic!("应收到 log 消息: {other:?}"),
        }
        assert_eq!(
            reader.recv(WAIT),
            Recv::Msg(json!({"type": "result", "score": 0.5}))
        );
        assert_eq!(reader.recv(WAIT), Recv::Closed);
    }

    /// json.dumps(ensure_ascii=False) 原样输出 U+0080–U+009F，路径里的这类字符不能被滤掉
    #[test]
    fn c1_control_chars_and_bom_survive() {
        let line = "\u{feff}{\"image_path\":\"D:\\\\数据\\\\a\u{85}b.png\"}\n";
        let reader = ProtocolReader::spawn(Cursor::new(line.as_bytes().to_vec()));
        assert_eq!(
            reader.recv(WAIT),
            Recv::Msg(json!({"image_path": "D:\\数据\\a\u{85}b.png"}))
        );
    }

    #[test]
    fn silence_timeout_only_counts_protocol_messages() {
        let (tx, reader) = chunked();
        assert_eq!(reader.recv(Duration::from_millis(50)), Recv::TimedOut);

        tx.send(b"{\"type\":\"log\",".to_vec()).unwrap();
        // 半行不算消息
        assert_eq!(reader.recv(Duration::from_millis(50)), Recv::TimedOut);
        tx.send(b"\"message\":\"loading\"}\n".to_vec()).unwrap();
        assert_eq!(
            reader.recv(WAIT),
            Recv::Msg(json!({"type": "log", "message": "loading"}))
        );

        // 非协议输出不重置静默计时
        tx.send(b"some print() noise\n".to_vec()).unwrap();
        assert_eq!(reader.recv(Duration::from_millis(100)), Recv::TimedOut);

        drop(tx);
        assert_eq!(reader.recv(WAIT), Recv::Closed);
    }

    #[test]
    fn recv_until_keeps_deadline_and_drains_queue_after_it() {
        let (tx, reader) = chunked();
        let start = Instant::now();
        let deadline = start + Duration::from_millis(80);
        assert_eq!(reader.recv_until(deadline), Recv::TimedOut);
        assert!(start.elapsed() >= Duration::from_millis(80));

        // 截止时刻已过，但已在队列里的消息仍要取走
        tx.send(b"{\"type\":\"ready\"}\n".to_vec()).unwrap();
        std::thread::sleep(Duration::from_millis(100));
        assert_eq!(
            reader.recv_until(deadline),
            Recv::Msg(json!({"type": "ready"}))
        );
        assert_eq!(reader.recv_until(deadline), Recv::TimedOut);

        drop(tx);
        std::thread::sleep(Duration::from_millis(100));
        assert_eq!(reader.recv_until(deadline), Recv::Closed);
    }

    #[test]
    fn read_error_closes_after_queued_messages() {
        let reader = ProtocolReader::spawn(FailingReader { sent: false });
        assert_eq!(reader.recv(WAIT), Recv::Msg(json!({"type": "ready"})));
        assert_eq!(reader.recv(WAIT), Recv::Closed);
    }
}

#[cfg(test)]
mod noise_tests {
    use super::*;

    #[test]
    fn ort_info_and_warning_are_noise_but_errors_pass() {
        assert!(is_runtime_noise("2026-09-16 08:38:45 [W:onnxruntime:, transformer_memcpy.cc:111 ApplyImpl] 11 Memcpy nodes"));
        assert!(is_runtime_noise(
            "[I:onnxruntime:Default, session_state.cc:1] info"
        ));
        // Error/Fatal 级即使带 cudnn / could not load 也放行
        assert!(!is_runtime_noise(
            "[E:onnxruntime:Default, provider_bridge_ort.cc:2022] Could not load cudnn64_9.dll"
        ));
        assert!(!is_runtime_noise(
            "[F:onnxruntime:, inference_session.cc:1] fatal"
        ));
    }

    #[test]
    fn cuda_and_coreml_chatter_is_noise() {
        for line in [
            "Could not locate cudnn_ops64_9.dll. Please make sure it is in your library path!",
            "CUDA_PATH is set but CUDA wasn't able to be loaded.",
            "Loaded library cublasLt64_12.dll",
            "Context leak detected, msgtracer returned -1",
            "CoreMLExecutionProvider::GetCapability, number of partitions supported by CoreML: 12",
        ] {
            assert!(is_runtime_noise(line), "应视为噪音: {line}");
        }
    }

    #[test]
    fn real_python_errors_are_kept() {
        for line in [
            "Traceback (most recent call last):",
            "ValueError: cannot reshape array of size 0",
            "加载模型失败: 文件不存在",
            "EP Error: CUDA_ERROR_OUT_OF_MEMORY when using ['CUDAExecutionProvider']",
        ] {
            assert!(!is_runtime_noise(line), "不该过滤: {line}");
            assert!(!is_python_library_noise(line), "不该过滤: {line}");
        }
    }

    #[test]
    fn python_library_warnings_and_progress_bars_are_noise() {
        for line in [
            "C:\\env\\Lib\\site-packages\\sklearn\\cluster\\_kmeans.py:1416: FutureWarning: n_init",
            "/env/lib/python3.12/site-packages/umap/umap_.py:1952: UserWarning: n_jobs value 1",
            "/env/lib/python3.12/site-packages/x/y.py:10: DeprecationWarning",
            "warnings.warn(",
            "warn(",
            "Downloading: \"https://download.pytorch.org/models/resnet50-11ad3fa6.pth\"",
            "100%|██████████| 97.8M/97.8M [00:03<00:00, 30.1MB/s]",
            "0%|  | 0/10 1%|  | 1/10 5%|  | 5/10 9%|█ | 9/10",
            "eigenvalues did not converge",
            "scipy.sparse.linalg warning",
        ] {
            assert!(is_python_library_noise(line), "应视为噪音: {line}");
        }
    }
}

#[cfg(test)]
mod registry_tests {
    use super::*;

    #[test]
    fn reg_query_value_keeps_spaces_in_path() {
        let out = "\r\nHKEY_LOCAL_MACHINE\\SYSTEM\\CurrentControlSet\\Control\\Session Manager\\Environment\r\n    CUDA_PATH    REG_SZ    C:\\Program Files\\NVIDIA GPU Computing Toolkit\\CUDA\\v12.4\r\n\r\n";
        assert_eq!(
            parse_reg_query_value(out, "CUDA_PATH").as_deref(),
            Some("C:\\Program Files\\NVIDIA GPU Computing Toolkit\\CUDA\\v12.4")
        );
        assert_eq!(parse_reg_query_value("ERROR: not found", "CUDA_PATH"), None);
        assert_eq!(
            parse_reg_query_value("    CUDA_PATH    REG_SZ", "CUDA_PATH"),
            None
        );
    }
}

#[cfg(all(test, unix))]
mod runner_tests {
    use super::*;
    use serde_json::json;
    use std::sync::Arc;

    fn sh(script: &str) -> Command {
        let mut cmd = Command::new("sh");
        cmd.args(["-c", script]);
        cmd
    }

    fn is_alive(pid: u32) -> bool {
        Command::new("kill")
            .args(["-0", &pid.to_string()])
            .stderr(Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }

    #[test]
    fn hidden_command_keeps_program_and_runs() {
        let mut cmd = hidden_command("sh");
        assert_eq!(cmd.get_program(), "sh");
        assert!(cmd.args(["-c", "exit 0"]).status().unwrap().success());
    }

    #[test]
    fn pid_registration_clears_only_its_own_pid() {
        static SLOT: Mutex<Option<u32>> = Mutex::new(None);
        {
            let _reg = PidRegistration::new(&SLOT, 41);
            assert_eq!(*SLOT.lock().unwrap(), Some(41));
        }
        assert_eq!(*SLOT.lock().unwrap(), None);

        let reg = PidRegistration::new(&SLOT, 42);
        *SLOT.lock().unwrap() = Some(43); // 后来者覆盖
        drop(reg);
        assert_eq!(*SLOT.lock().unwrap(), Some(43));
        *SLOT.lock().unwrap() = None;
        kill_registered_pid(&SLOT); // 空槽什么也不做
    }

    #[test]
    fn streams_messages_and_stderr_and_reports_exit_code() {
        static SLOT: Mutex<Option<u32>> = Mutex::new(None);
        let cancel = AtomicBool::new(false);
        let stderr_lines = Arc::new(Mutex::new(Vec::new()));
        let sink = stderr_lines.clone();
        let mut messages = Vec::new();
        let mut pid_seen = None;

        let exit = run_json_lines_script(
            sh(r#"echo '{"type":"log","message":"a"}'; echo 'not json'; echo 'warn line' >&2; echo '{"type":"done","success":2}'; exit 3"#),
            false,
            &SLOT,
            &cancel,
            move |line| sink.lock().unwrap().push(line),
            |msg| {
                pid_seen = *SLOT.lock().unwrap();
                messages.push(msg);
                Ok(())
            },
        )
        .unwrap();

        assert_eq!(
            exit,
            ScriptExit {
                code: Some(3),
                cancelled: false
            }
        );
        assert_eq!(
            messages,
            vec![
                json!({"type": "log", "message": "a"}),
                json!({"type": "done", "success": 2})
            ]
        );
        assert!(pid_seen.is_some(), "运行期间应登记 PID");
        assert_eq!(*SLOT.lock().unwrap(), None, "返回后应清除 PID");
        // stderr 线程是异步的，给它一点时间
        let deadline = Instant::now() + Duration::from_secs(5);
        while stderr_lines.lock().unwrap().is_empty() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(*stderr_lines.lock().unwrap(), vec!["warn line".to_string()]);
    }

    #[test]
    fn message_error_kills_the_process_tree() {
        static SLOT: Mutex<Option<u32>> = Mutex::new(None);
        let cancel = AtomicBool::new(false);
        let mut pid = None;
        let start = Instant::now();
        let result = run_json_lines_script(
            sh(r#"echo '{"type":"error","message":"boom"}'; sleep 30"#),
            false,
            &SLOT,
            &cancel,
            |_| {},
            |msg| {
                pid = *SLOT.lock().unwrap();
                Err(format!(
                    "脚本错误: {}",
                    msg["message"].as_str().unwrap_or("")
                ))
            },
        );
        assert_eq!(result, Err("脚本错误: boom".to_string()));
        assert!(start.elapsed() < Duration::from_secs(10));
        assert!(!is_alive(pid.expect("应登记 PID")), "进程应已被终止");
        assert_eq!(*SLOT.lock().unwrap(), None);
    }

    #[test]
    fn cancel_terminates_a_silent_script() {
        static SLOT: Mutex<Option<u32>> = Mutex::new(None);
        let cancel = Arc::new(AtomicBool::new(false));
        let flag = cancel.clone();
        // 记下运行器登记的 PID 再取消，事后确认正是这个进程被终止
        let canceller = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(300));
            let pid = *SLOT.lock().unwrap();
            flag.store(true, Ordering::SeqCst);
            pid
        });
        let start = Instant::now();
        let exit = run_json_lines_script(sh("sleep 30"), false, &SLOT, &cancel, |_| {}, |_| Ok(()))
            .unwrap();
        let pid = canceller.join().unwrap().expect("运行期间应登记 PID");
        assert!(exit.cancelled);
        assert_eq!(exit.code, None, "被信号终止，没有退出码");
        assert!(start.elapsed() < Duration::from_secs(10));
        assert!(!is_alive(pid), "登记的进程应已被终止");
        assert_eq!(*SLOT.lock().unwrap(), None);
    }

    #[test]
    fn spawn_failure_is_reported() {
        static SLOT: Mutex<Option<u32>> = Mutex::new(None);
        let cancel = AtomicBool::new(false);
        let err = run_json_lines_script(
            Command::new("/nonexistent/purinbox-python"),
            false,
            &SLOT,
            &cancel,
            |_| {},
            |_| Ok(()),
        )
        .unwrap_err();
        assert!(err.starts_with("启动 Python 失败"), "{err}");
        assert_eq!(*SLOT.lock().unwrap(), None);
    }
}
