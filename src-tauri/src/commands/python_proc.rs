//! Python 子进程共享基础设施
//!
//! 五个 AI 功能（打标 / 超分 / 人物裁切 / 美学评分 / 聚类）和环境部署都以子进程方式调用 Python，
//! 共用这里的：
//! 1. 子进程构造：`PythonCommand` 统一 stdio 管道、输出相关的环境变量和平台配置（Windows 隐藏
//!    控制台窗口、可选降低优先级、GPU 模式注入 CUDA/cuDNN DLL 的 PATH（进程环境 + 注册表回退 +
//!    cuDNN 9.x 子目录）；其他平台让子进程自成进程组，供按进程树终止），各运行方式都经
//!    `spawn_exclusive` 启动它。要同时收集 stdout 和 stderr 的非 Python 命令（tar 等）也用它。
//!    `hidden_command` 只设隐藏窗口标志，给 stdio 自己配的短命令用（nvidia-smi、taskkill、reg）
//! 2. stdout 协议读取：`ProtocolReader` 在后台线程逐行解析 JSON，接收端带静默时限或总时限
//! 3. stderr 逐行读取与解码（UTF-16LE / UTF-8 / GBK，剥掉 ANSI 转义与控制字符）、噪音过滤，
//!    以及把 stderr 转成 `[Python] …` 警告事件的回调 `stderr_warnings`
//! 4. 进程终止与登记：`kill_and_reap`；PID 槽（`PidRegistration` 登记，取消命令用
//!    `kill_registered_pid` 强杀）
//! 5. 三种运行方式：可取消的一次性命令 `output_cancellable`（pip 等）；参数驱动、不读 stdin 的
//!    JSON-lines 脚本 `run_json_lines_script_with`（超分、聚类、标签转换）；常驻、按 stdin
//!    收命令的会话 `PythonSession`（打标、美学评分、人物裁切）
//! 6. 推理脚本路径解析（开发源码目录 / Tauri 资源目录 / exe 同级 scripts / NSIS exe 同级 /
//!    macOS Resources）

use std::collections::VecDeque;
use std::ffi::OsStr;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Output, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x08000000;

/// 协议读取的静默上限：超过该时长收不到任何协议消息，视为 Python 进程卡死。
/// 接收端不能裸 recv() 死等：Python 卡死（进程存活但不再输出）时任务会永久停滞；
/// 取消时进程树被杀、stdout 关闭，`ProtocolReader` 返回 `Recv::Closed` 正常收尾。
pub const PYTHON_SILENCE_LIMIT: Duration = Duration::from_secs(300);

/// 运行器和会话检查取消标志的间隔
const CANCEL_POLL: Duration = Duration::from_millis(200);

/// `output_cancellable` 检查取消、等输出读完的间隔
const OUTPUT_POLL: Duration = Duration::from_millis(50);

/// 错误信息附带的 stderr 行数：Python 的关键异常信息在最后，整段 traceback 灌进错误提示没法看
const STDERR_TAIL_LINES: usize = 4;

/// 进程结束后等 stderr 读完的上限：派生的进程继承了 stderr 又迟迟不退出时不能一直等
const STDERR_DRAIN_LIMIT: Duration = Duration::from_secs(1);

/// 会话收尾时等进程自行退出的时间，超过就按进程树终止
const SHUTDOWN_GRACE: Duration = Duration::from_secs(2);

/// 新建子进程命令；Windows 上设置 CREATE_NO_WINDOW，GUI 进程每起一个子进程就会闪一下控制台窗口。
///
/// 只管窗口，不配 stdio、不设进程组、不注入 CUDA PATH，stdio 由调用方配置（nvidia-smi、taskkill、
/// Windows 的 reg）；在 Unix 上要经 `spawn_exclusive` 启动。
/// Python 子进程，以及要同时收集 stdout 和 stderr 的命令（tar 等）用 `PythonCommand`：
/// 它配好两路管道和进程组，各运行方式都经 `spawn_exclusive` 启动。
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

/// Python 子进程的命令构造器。`build()` 统一设置：
/// - stdin 置空（`stdin_piped()` 时走管道），stdout / stderr 走管道；
/// - `NO_COLOR=1`、`PYTHONUNBUFFERED=1`、`PYTHONIOENCODING=utf-8`：输出不带颜色码、逐行实时到达，
///   编码不随系统代码页变化；
/// - `configure_python_command_with_priority`：Windows 隐藏控制台窗口（窗口标志只设这一次）、
///   可选降低优先级，`use_gpu` 时注入 CUDA/cuDNN DLL 的 PATH；其他平台让子进程自成进程组。
///
/// 工作目录、代理环境变量等其余设置直接改 `build()` 返回的 `Command`。
/// `output_cancellable`、`run_json_lines_script_with`、`PythonSession::start` 都接收它，
/// 也接收填好程序和参数的 `Command`（按不用 GPU、不降优先级处理）。
pub(crate) struct PythonCommand {
    cmd: Command,
    use_gpu: bool,
    background_priority: bool,
    stdin_piped: bool,
}

impl PythonCommand {
    /// `python` 是解释器路径；脚本和参数用 `arg` / `args` 依次追加
    pub(crate) fn new(python: impl AsRef<OsStr>) -> Self {
        Command::new(python).into()
    }

    pub(crate) fn arg(mut self, arg: impl AsRef<OsStr>) -> Self {
        self.cmd.arg(arg);
        self
    }

    pub(crate) fn args<I, S>(mut self, args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        self.cmd.args(args);
        self
    }

    pub(crate) fn env(mut self, key: impl AsRef<OsStr>, value: impl AsRef<OsStr>) -> Self {
        self.cmd.env(key, value);
        self
    }

    /// GPU 推理：Windows 上把 CUDA/cuDNN DLL 目录注入 PATH，其他平台不起作用
    pub(crate) fn use_gpu(mut self, use_gpu: bool) -> Self {
        self.use_gpu = use_gpu;
        self
    }

    /// 以低于正常的优先级运行（Windows BELOW_NORMAL，其他平台不起作用）。
    /// 给大量占用 GPU 的推理用（打标 PixAI v1 GPU）：桌面合成仍能优先拿到 CPU/GPU 时间，
    /// 推理空闲时吞吐不变
    pub(crate) fn background_priority(mut self, background: bool) -> Self {
        self.background_priority = background;
        self
    }

    /// stdin 走管道。`PythonSession::start` 会自动设置
    pub(crate) fn stdin_piped(mut self) -> Self {
        self.stdin_piped = true;
        self
    }

    pub(crate) fn build(self) -> Command {
        let PythonCommand {
            mut cmd,
            use_gpu,
            background_priority,
            stdin_piped,
        } = self;
        cmd.stdin(if stdin_piped {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env("NO_COLOR", "1")
        .env("PYTHONUNBUFFERED", "1")
        .env("PYTHONIOENCODING", "utf-8");
        configure_python_command_with_priority(&mut cmd, use_gpu, background_priority);
        cmd
    }

    /// 运行到结束并收集输出（`python -c` 探测、`python -m venv` 这类短命令）。
    /// 需要中途取消的用 `output_cancellable`
    pub(crate) fn output(self) -> std::io::Result<Output> {
        spawn_exclusive(&mut self.build())?.wait_with_output()
    }
}

impl From<Command> for PythonCommand {
    fn from(cmd: Command) -> Self {
        PythonCommand {
            cmd,
            use_gpu: false,
            background_priority: false,
            stdin_piped: false,
        }
    }
}

// ───────────────────────── stdout 协议读取 ─────────────────────────

/// `ProtocolReader` 一次接收的结果
#[derive(Debug, Clone, PartialEq)]
enum Recv {
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
struct ProtocolReader {
    rx: mpsc::Receiver<serde_json::Value>,
}

impl ProtocolReader {
    /// 在后台线程读取 `stdout`（一般是 `ChildStdout`；测试里可以是内存 reader）
    fn spawn<R: Read + Send + 'static>(stdout: R) -> Self {
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
    fn recv(&self, silence: Duration) -> Recv {
        match self.rx.recv_timeout(silence) {
            Ok(msg) => Recv::Msg(msg),
            Err(mpsc::RecvTimeoutError::Timeout) => Recv::TimedOut,
            Err(mpsc::RecvTimeoutError::Disconnected) => Recv::Closed,
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
/// `StderrCapture` 的读取线程用它：读到 EOF 或管道出错即返回。
/// 不能用 `BufRead::lines()`——它遇到非 UTF-8 字节直接返回 Err，
/// 配合 `map_while(Result::ok)` 会让整个线程在第一行 GBK 输出处停止读取，
/// 之后管道缓冲区一满 Python 就永久阻塞在写日志上。
fn for_each_stderr_line<R: Read>(reader: R, mut on_line: impl FnMut(String)) {
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
fn is_runtime_noise(line: &str) -> bool {
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
fn is_python_library_noise(line: &str) -> bool {
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
fn decode_python_line(bytes: &[u8]) -> String {
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

/// `stderr_warnings` 过滤掉的噪音
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StderrNoise {
    /// 推理运行时（onnxruntime / CUDA / CoreML）的噪音，见 `is_runtime_noise`
    Runtime,
    /// 另加 Python 库的告警与进度条（聚类的 sklearn / umap / torch），见 `is_python_library_noise`
    RuntimeAndLibraries,
}

impl StderrNoise {
    fn matches(self, line: &str) -> bool {
        is_runtime_noise(line)
            || (self == StderrNoise::RuntimeAndLibraries && is_python_library_noise(line))
    }
}

/// stderr 回调：每行过滤噪音后作为 `[Python] …` 的 warning 进度事件发到 `event`，
/// 交给 `run_json_lines_script_with` / `PythonSession::start` 的 `on_stderr`。
///
/// `mute` 置位后不再转发：取消时进程被强杀，随后冒出的 BrokenPipe、KeyboardInterrupt 之类
/// 报错不该作为警告进日志。传 None 则始终转发。
pub(crate) fn stderr_warnings<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    event: &'static str,
    noise: StderrNoise,
    mute: Option<&'static AtomicBool>,
) -> impl FnMut(String) + Send + 'static {
    let app = app.clone();
    move |line| {
        if noise.matches(&line) || mute.is_some_and(|flag| flag.load(Ordering::SeqCst)) {
            return;
        }
        super::ProgressEvent::new("warning", format!("[Python] {}", line)).emit(&app, event);
    }
}

/// stderr 读取线程：每行交给回调，同时保留最后几行作错误信息；可以等它读到 EOF
struct StderrCapture {
    tail: Arc<Mutex<VecDeque<String>>>,
    /// 读取线程结束时断开
    finished: mpsc::Receiver<()>,
}

impl StderrCapture {
    fn spawn<R: Read + Send + 'static>(
        stderr: R,
        mut on_line: impl FnMut(String) + Send + 'static,
    ) -> Self {
        let tail = Arc::new(Mutex::new(VecDeque::with_capacity(STDERR_TAIL_LINES)));
        let (finished_tx, finished) = mpsc::channel::<()>();
        let sink = tail.clone();
        std::thread::spawn(move || {
            let _finished = finished_tx;
            for_each_stderr_line(stderr, |line| {
                {
                    let mut tail = sink.lock().unwrap_or_else(|e| e.into_inner());
                    if tail.len() == STDERR_TAIL_LINES {
                        tail.pop_front();
                    }
                    tail.push_back(line.clone());
                }
                on_line(line);
            });
        });
        StderrCapture { tail, finished }
    }

    /// 等读取线程读到 EOF、处理完所有行，最多等 `STDERR_DRAIN_LIMIT`
    fn drain(&self) {
        let _ = self.finished.recv_timeout(STDERR_DRAIN_LIMIT);
    }

    /// 最后几行，以 " | " 连接；没有输出时为空串
    fn tail(&self) -> String {
        let tail = self.tail.lock().unwrap_or_else(|e| e.into_inner());
        tail.iter()
            .map(String::as_str)
            .collect::<Vec<_>>()
            .join(" | ")
    }
}

// ───────────────────────── 启动、终止与 PID 登记 ─────────────────────────

/// 启动子进程，同一时刻只允许一个启动。
///
/// macOS 没有 pipe2，标准库先建管道、再给它设 CLOEXEC。两步之间若别的线程恰好启动子进程，
/// 管道端会被那个子进程继承：长期运行的进程握着别人 stdout 的写端，读的一方等不到 EOF，
/// 进程退出也察觉不到，`Command::output()` 会一直等下去。
/// 本模块的启动都经过这里；别处会建管道（`output()`、piped stdio）或会长期运行的启动
/// （NCNN 超分、`kill_process_tree` 里的 kill / taskkill）也应经过这里。
pub(crate) fn spawn_exclusive(cmd: &mut Command) -> std::io::Result<Child> {
    static SPAWN_LOCK: Mutex<()> = Mutex::new(());
    let _guard = SPAWN_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    cmd.spawn()
}

/// 按进程树终止并回收子进程。
///
/// `kill_process_tree` 连同 Python 派生的 worker 一起杀（单杀直接子进程会留下孤儿进程），
/// 之后 kill + wait 回收句柄，不留僵尸进程。
/// 调用前不能已经 wait / try_wait 到退出状态：回收后 PID 可能被系统复用，再按 PID 杀树会误伤别的进程。
pub(crate) fn kill_and_reap(child: &mut Child) {
    super::kill_process_tree(child.id());
    let _ = child.kill();
    let _ = child.wait();
}

/// 子进程 PID 登记：存活期间槽里是该 PID，供取消命令按进程树终止；drop 时清除。
/// 只在槽里仍是自己的 PID 时才清除——取消命令已经取走、或已被后来者覆盖时不动。
///
/// 槽是各功能自己的 `static Mutex<Option<u32>>`：运行器和会话启动时登记、结束时清除，
/// 取消命令调 `kill_registered_pid` 强杀。登记之前到达的取消由运行器 / 会话轮询取消标志兜底。
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

/// 取走槽里登记的 PID 并按进程树终止（取消命令用）；没有登记时什么也不做。
/// 只杀不回收：回收由持有 `Child` 的运行器 / 会话在 stdout 关闭后完成
pub fn kill_registered_pid(slot: &Mutex<Option<u32>>) {
    let pid = slot.lock().unwrap_or_else(|e| e.into_inner()).take();
    if let Some(pid) = pid {
        super::kill_process_tree(pid);
    }
}

// ───────────────────────── 一次性命令 ─────────────────────────

/// 运行到结束并收集 stdout / stderr，`cancel` 置位时按进程树终止（pip 安装等可能很久的命令）。
///
/// - 按 `PythonCommand` 配置子进程，stdin 置空；`cancel` 已置位时不启动；
/// - 两路输出各由一个线程读到 EOF，任一路写满管道都不会让子进程阻塞；
/// - 每 50ms 检查一次 `cancel`。输出读完后才回收进程，取消时按 PID 杀树不会误伤复用的 PID；
/// - 取消返回 `ErrorKind::Interrupted`、文案「已取消」的错误。
pub(crate) fn output_cancellable(
    cmd: impl Into<PythonCommand>,
    cancel: &AtomicBool,
) -> std::io::Result<Output> {
    let cancelled_error = || std::io::Error::new(std::io::ErrorKind::Interrupted, "已取消");
    if cancel.load(Ordering::SeqCst) {
        return Err(cancelled_error());
    }
    let mut cmd = cmd.into().build();
    cmd.stdin(Stdio::null());
    let mut child = spawn_exclusive(&mut cmd)?;
    let (Some(stdout), Some(stderr)) = (child.stdout.take(), child.stderr.take()) else {
        kill_and_reap(&mut child);
        return Err(std::io::Error::other("无法获取子进程管道"));
    };
    std::thread::scope(|scope| {
        fn read(mut stream: impl Read) -> std::io::Result<Vec<u8>> {
            let mut bytes = Vec::new();
            stream.read_to_end(&mut bytes)?;
            Ok(bytes)
        }
        let stdout = scope.spawn(move || read(stdout));
        let stderr = scope.spawn(move || read(stderr));
        let status = loop {
            if cancel.load(Ordering::SeqCst) {
                kill_and_reap(&mut child);
                break Err(cancelled_error());
            }
            if stdout.is_finished() && stderr.is_finished() {
                match child.try_wait() {
                    Ok(Some(status)) => break Ok(status),
                    Ok(None) => {}
                    Err(e) => {
                        kill_and_reap(&mut child);
                        break Err(e);
                    }
                }
            }
            std::thread::sleep(OUTPUT_POLL);
        };
        let stdout = stdout
            .join()
            .map_err(|_| std::io::Error::other("读取 stdout 线程异常"))?;
        let stderr = stderr
            .join()
            .map_err(|_| std::io::Error::other("读取 stderr 线程异常"))?;
        Ok(Output {
            status: status?,
            stdout: stdout?,
            stderr: stderr?,
        })
    })
}

// ───────────────────────── JSON-lines 脚本运行器 ─────────────────────────

/// `run_json_lines_script_with` 的结束状态
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScriptExit {
    /// 子进程退出码。Unix 上被信号终止时为 None；Windows 上被强杀时一般是 1，
    /// 所以判断是不是取消要看 `cancelled`
    pub code: Option<i32>,
    /// 结束时取消标志是否置位。运行中检测到取消时已按进程树终止子进程
    pub cancelled: bool,
    /// 超过静默时限没有协议消息，已按进程树终止（只在设了 `silence_limit` 时出现）
    pub timed_out: bool,
    /// stderr 最后几行，以 " | " 连接，没有输出时为空。脚本没发终态消息就退出时可附在错误里
    pub stderr_tail: String,
}

/// 运行参数驱动、stdout 逐行输出 JSON、不读 stdin 的 Python 脚本。
///
/// - 按 `cmd`（`PythonCommand`，GPU / 优先级由它决定）启动，stdin 置空；
/// - 在 `pid_slot` 登记 PID，返回前清除（取消命令可用 `kill_registered_pid` 强杀）；
/// - stderr 由后台线程逐行解码后交给 `on_stderr`，stdout 经 `ProtocolReader` 逐条交给 `on_message`；
/// - 每 200ms 检查一次 `cancel`，脚本长时间不输出时取消也能生效；
/// - `silence_limit`：超过这么久收不到任何协议消息就按进程树终止，返回 `timed_out: true`。
///   None 不限时（脚本自己保证会结束时用）。
///
/// 结束方式：
/// - `on_message` 返回 Err：终止进程树并原样返回这个 Err（脚本报 error 消息时用）；
/// - 检测到取消：终止进程树，返回 `ScriptExit { cancelled: true, .. }`；
/// - 静默超时：终止进程树，返回 `ScriptExit { timed_out: true, .. }`；
/// - stdout 关闭：等子进程退出后返回退出码。没收到终态消息时怎么上报由调用方决定，
///   可附上 `stderr_tail`。
///
/// 返回前会等 stderr 读完（最多 1 秒），调用方随后发的终态事件不会被迟到的 stderr 警告插队。
/// 阻塞函数，调用方放进 `spawn_blocking`。启动失败返回 "启动 Python 失败: …"。
pub(crate) fn run_json_lines_script_with(
    cmd: impl Into<PythonCommand>,
    silence_limit: Option<Duration>,
    pid_slot: &Mutex<Option<u32>>,
    cancel: &AtomicBool,
    on_stderr: impl FnMut(String) + Send + 'static,
    mut on_message: impl FnMut(serde_json::Value) -> Result<(), String>,
) -> Result<ScriptExit, String> {
    let mut cmd = cmd.into().build();
    cmd.stdin(Stdio::null());
    let mut child = spawn_exclusive(&mut cmd).map_err(|e| format!("启动 Python 失败: {}", e))?;
    let mut registration = Some(PidRegistration::new(pid_slot, child.id()));

    let (Some(stdout), Some(stderr)) = (child.stdout.take(), child.stderr.take()) else {
        registration.take();
        kill_and_reap(&mut child);
        return Err("无法获取 Python 进程管道".into());
    };
    let stderr = StderrCapture::spawn(stderr, on_stderr);
    let reader = ProtocolReader::spawn(stdout);

    let mut last_message = Instant::now();
    let mut timed_out = false;
    let mut terminate = false;
    loop {
        if cancel.load(Ordering::SeqCst) {
            terminate = true;
            break;
        }
        match reader.recv(CANCEL_POLL) {
            Recv::Msg(msg) => {
                last_message = Instant::now();
                if let Err(e) = on_message(msg) {
                    registration.take();
                    kill_and_reap(&mut child);
                    stderr.drain();
                    return Err(e);
                }
            }
            Recv::TimedOut => {
                if silence_limit.is_some_and(|limit| last_message.elapsed() >= limit) {
                    timed_out = true;
                    terminate = true;
                    break;
                }
            }
            Recv::Closed => break,
        }
    }

    // 先清登记再回收：回收后 PID 可能被复用，不能再留给取消命令去杀，之后的取消由这里处理
    registration.take();
    if !terminate {
        // stdout 关闭后进程一般随即退出；卡在退出清理里时也要响应取消
        loop {
            match child.try_wait() {
                Ok(Some(_)) => break,
                Ok(None) if !cancel.load(Ordering::SeqCst) => std::thread::sleep(OUTPUT_POLL),
                _ => {
                    terminate = true;
                    break;
                }
            }
        }
    }
    if terminate {
        kill_and_reap(&mut child);
    }
    let code = child.wait().ok().and_then(|status| status.code());
    stderr.drain();
    Ok(ScriptExit {
        code,
        cancelled: cancel.load(Ordering::SeqCst),
        timed_out,
        stderr_tail: stderr.tail(),
    })
}

// ───────────────────────── 一次性脚本的共用件（超分、聚类、标签转换） ─────────────────────────

/// Python 写文件期间临时文件名里的标记（`purin_proto.temp_path`：`<目标>.<标记>.tmp`），每轮任务一个。
/// 脚本没发终态消息就结束时（被我们终止，或自己崩溃、被系统杀掉）可能正写到一半，
/// 按标记找到这些临时文件删掉
pub(crate) struct PythonTempTag(String);

impl PythonTempTag {
    /// 与 `purin_proto.TEMP_TAG_ENV` 一致
    const ENV: &'static str = "PURIN_TEMP_TAG";

    pub(crate) fn for_run(run_id: u64) -> Self {
        PythonTempTag(format!("purin-r{}", run_id))
    }

    /// 让子进程按这个标记命名临时文件
    pub(crate) fn apply(&self, cmd: PythonCommand) -> PythonCommand {
        cmd.env(Self::ENV, &self.0)
    }

    fn suffix(&self) -> String {
        format!(".{}.tmp", self.0)
    }

    /// `target` 写入期间的临时文件
    pub(crate) fn temp_file(&self, target: &Path) -> PathBuf {
        let mut name = target.as_os_str().to_os_string();
        name.push(self.suffix());
        PathBuf::from(name)
    }

    /// 删掉 `root` 下（含各级子目录）带这个标记的临时文件
    pub(crate) fn remove_under(&self, root: &Path) {
        let suffix = self.suffix();
        for entry in walkdir::WalkDir::new(root).into_iter().flatten() {
            if entry.file_type().is_file() && entry.file_name().to_string_lossy().ends_with(&suffix)
            {
                let _ = std::fs::remove_file(entry.path());
            }
        }
    }
}

/// 交给 Python 的文件清单（系统临时目录下的 JSON），drop 时删除
pub(crate) struct ManifestFile(PathBuf);

impl ManifestFile {
    pub(crate) fn write(prefix: &str, value: &impl serde::Serialize) -> Result<Self, String> {
        // 先在内存里序列化：直接写未缓冲的 File，上万条路径会变成几万次小写入
        let bytes = serde_json::to_vec(value).map_err(|e| format!("生成文件清单失败: {}", e))?;
        // 时间戳在部分平台只精确到微秒，同一微秒内的两次写入靠序号区分
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "{}-{}-{}-{}.json",
            prefix,
            std::process::id(),
            stamp,
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(|e| format!("创建文件清单失败: {}", e))?;
        let manifest = ManifestFile(path);
        file.write_all(&bytes)
            .map_err(|e| format!("写入文件清单失败: {}", e))?;
        Ok(manifest)
    }

    pub(crate) fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for ManifestFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// 脚本没发终态消息就退出时的错误
pub(crate) fn abnormal_exit(what: &str, exit: &ScriptExit) -> String {
    let code = exit.code.map_or_else(
        || "被系统终止".to_string(),
        |code| format!("退出码 {}", code),
    );
    if exit.stderr_tail.is_empty() {
        format!("{}异常退出（{}），未返回结果", what, code)
    } else {
        format!("{}异常退出（{}）: {}", what, code, exit.stderr_tail)
    }
}

// ───────────────────────── 常驻会话 ─────────────────────────

/// `PythonSession` 的失败原因，Display 是给用户看的文案
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SessionError {
    /// 取消标志已置位，进程树已终止
    Cancelled,
    /// 启动失败或拿不到管道
    Spawn(String),
    /// 等待就绪时脚本回了 `{"type":"error"}`，内容是其中的 message
    Script(String),
    /// stdout 已关闭或 stdin 写不进去（进程退出、崩溃或被外部终止），进程已回收。
    /// `stderr` 是 stderr 最后几行，以 " | " 连接，可能为空
    Exited { stderr: String },
    /// 时限内没有协议消息。进程仍在运行，会话 drop 或 shutdown 时终止
    TimedOut(Duration),
}

impl std::fmt::Display for SessionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SessionError::Cancelled => f.write_str("已取消"),
            SessionError::Spawn(e) => write!(f, "启动 Python 进程失败: {}", e),
            SessionError::Script(message) => f.write_str(message),
            SessionError::Exited { stderr } if stderr.is_empty() => {
                f.write_str("Python 进程已退出")
            }
            SessionError::Exited { stderr } => write!(f, "Python 进程已退出: {}", stderr),
            SessionError::TimedOut(limit) => {
                write!(f, "Python 进程无响应（{} 秒）", limit.as_secs())
            }
        }
    }
}

impl std::error::Error for SessionError {}

impl From<SessionError> for String {
    fn from(e: SessionError) -> String {
        e.to_string()
    }
}

/// 常驻 Python 进程：stdin 逐行收 JSON 命令，stdout 逐行回 JSON 消息（打标、美学评分、人物裁切）。
///
/// - `start` 按 `PythonCommand` 启动（stdin 走管道），在 `pid_slot` 登记 PID，取消命令可用
///   `kill_registered_pid` 强杀；stderr 由后台线程逐行交给 `on_stderr`，同时保留最后几行作错误信息；
/// - `send` 发命令；`recv` 收下一条消息；`wait_ready` 等 `{"type":"ready"}`；
/// - `send` / `recv` / `wait_ready` 都会检查 `cancel`（等待中每 200ms 一次）：置位即按进程树终止并
///   返回 `Cancelled`，取消命令赶在 PID 登记之前到达时也能及时生效；
/// - stdout 关闭或 stdin 写不进去时返回 `Exited`（进程已回收，附 stderr 尾部），
///   由取消引起的返回 `Cancelled`；
/// - `shutdown` 正常收尾；drop 时进程若还在，按进程树终止并回收。
///
/// 阻塞式接口，调用方放进 `spawn_blocking`。
pub(crate) struct PythonSession<'a> {
    child: Child,
    stdin: Option<ChildStdin>,
    reader: ProtocolReader,
    stderr: StderrCapture,
    cancel: &'a AtomicBool,
    registration: Option<PidRegistration<'a>>,
    /// 子进程已回收；之后不能再按 PID 杀树
    reaped: bool,
}

impl<'a> PythonSession<'a> {
    /// 启动进程。`cancel` 已置位时不启动，返回 `Cancelled`
    pub(crate) fn start(
        cmd: impl Into<PythonCommand>,
        pid_slot: &'a Mutex<Option<u32>>,
        cancel: &'a AtomicBool,
        on_stderr: impl FnMut(String) + Send + 'static,
    ) -> Result<Self, SessionError> {
        if cancel.load(Ordering::SeqCst) {
            return Err(SessionError::Cancelled);
        }
        let cmd: PythonCommand = cmd.into();
        let mut child = spawn_exclusive(&mut cmd.stdin_piped().build())
            .map_err(|e| SessionError::Spawn(e.to_string()))?;
        let registration = PidRegistration::new(pid_slot, child.id());
        let (Some(stdin), Some(stdout), Some(stderr)) =
            (child.stdin.take(), child.stdout.take(), child.stderr.take())
        else {
            drop(registration);
            kill_and_reap(&mut child);
            return Err(SessionError::Spawn("无法获取进程管道".into()));
        };
        Ok(PythonSession {
            child,
            stdin: Some(stdin),
            reader: ProtocolReader::spawn(stdout),
            stderr: StderrCapture::spawn(stderr, on_stderr),
            cancel,
            registration: Some(registration),
            reaped: false,
        })
    }

    #[cfg(all(test, unix))]
    pub(crate) fn pid(&self) -> u32 {
        self.child.id()
    }

    /// 发一条命令：`command` 序列化成一行 JSON 写入 stdin
    pub(crate) fn send(&mut self, command: &serde_json::Value) -> Result<(), SessionError> {
        if self.cancel.load(Ordering::SeqCst) {
            self.terminate();
            return Err(SessionError::Cancelled);
        }
        match self.write_line(&command.to_string()) {
            Ok(()) => Ok(()),
            // 管道写不进去说明 Python 已关闭 stdin（多半已退出），会话不再可用
            Err(_) => Err(self.closed_error()),
        }
    }

    /// 等下一条消息，最多等 `timeout`。每次调用重新计时，循环调用就是静默时限。
    /// log、result、error 等消息都原样返回，由调用方分派
    pub(crate) fn recv(&mut self, timeout: Duration) -> Result<serde_json::Value, SessionError> {
        let deadline = Instant::now().checked_add(timeout);
        loop {
            if self.cancel.load(Ordering::SeqCst) {
                self.terminate();
                return Err(SessionError::Cancelled);
            }
            let wait = deadline.map_or(CANCEL_POLL, |deadline| {
                deadline
                    .saturating_duration_since(Instant::now())
                    .min(CANCEL_POLL)
            });
            match self.reader.recv(wait) {
                Recv::Msg(msg) => return Ok(msg),
                Recv::Closed => return Err(self.closed_error()),
                Recv::TimedOut => {
                    if deadline.is_some_and(|deadline| Instant::now() >= deadline) {
                        return Err(SessionError::TimedOut(timeout));
                    }
                }
            }
        }
    }

    /// 等脚本发 `{"type":"ready"}`，从调用起最多等 `timeout`（总时限）。
    /// 期间的 log 消息交给 `on_log`，其他类型的消息忽略；脚本回 error 时返回 `Script(message)`，
    /// 进程提前退出时返回带 stderr 尾部的 `Exited`
    pub(crate) fn wait_ready(
        &mut self,
        timeout: Duration,
        mut on_log: impl FnMut(&serde_json::Value),
    ) -> Result<(), SessionError> {
        let deadline = Instant::now().checked_add(timeout);
        loop {
            let remaining = deadline.map_or(Duration::MAX, |deadline| {
                deadline.saturating_duration_since(Instant::now())
            });
            let msg = match self.recv(remaining) {
                Ok(msg) => msg,
                Err(SessionError::TimedOut(_)) => return Err(SessionError::TimedOut(timeout)),
                Err(e) => return Err(e),
            };
            match msg["type"].as_str().unwrap_or("") {
                "ready" => return Ok(()),
                "log" => on_log(&msg),
                "error" => {
                    return Err(SessionError::Script(
                        msg["message"].as_str().unwrap_or("").to_string(),
                    ))
                }
                _ => {}
            }
        }
    }

    /// stderr 最后几行，以 " | " 连接（没有输出时为空）
    pub(crate) fn stderr_tail(&self) -> String {
        self.stderr.tail()
    }

    /// 立即按进程树终止并回收，不等进程自行退出（超时或协议出错后放弃会话时用）。
    /// 之后 `send` / `recv` 返回 `Exited`，`shutdown` 只等 stderr 读完
    pub(crate) fn kill(&mut self) {
        self.terminate();
    }

    /// 结束会话：写入退出命令 `exit_line`（原样加换行；None 不写）→ 关闭 stdin → 最多等 2 秒
    /// 让进程自行退出 → 仍未退出就按进程树终止。等待期间取消标志置位则立即终止。
    /// 返回前等 stderr 读完（最多 1 秒），调用方随后发的终态事件不会被迟到的 stderr 警告插队
    pub(crate) fn shutdown(mut self, exit_line: Option<&str>) {
        if !self.reaped {
            if let Some(line) = exit_line {
                let _ = self.write_line(line);
            }
            self.stdin.take();
            self.registration.take();
            let deadline = Instant::now() + SHUTDOWN_GRACE;
            loop {
                match self.child.try_wait() {
                    Ok(Some(_)) => break,
                    Ok(None)
                        if Instant::now() < deadline && !self.cancel.load(Ordering::SeqCst) =>
                    {
                        std::thread::sleep(Duration::from_millis(20));
                    }
                    _ => {
                        kill_and_reap(&mut self.child);
                        break;
                    }
                }
            }
            self.reaped = true;
        }
        self.stderr.drain();
    }

    fn write_line(&mut self, line: &str) -> std::io::Result<()> {
        let stdin = self
            .stdin
            .as_mut()
            .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::BrokenPipe, "stdin 已关闭"))?;
        let mut bytes = Vec::with_capacity(line.len() + 1);
        bytes.extend_from_slice(line.as_bytes());
        bytes.push(b'\n');
        stdin.write_all(&bytes)?;
        stdin.flush()
    }

    /// 管道断开后的错误：取消引起的算取消；否则进程已不可用，终止并回收后附上 stderr 尾部
    fn closed_error(&mut self) -> SessionError {
        let cancelled = self.cancel.load(Ordering::SeqCst);
        self.terminate();
        if cancelled {
            return SessionError::Cancelled;
        }
        self.stderr.drain();
        SessionError::Exited {
            stderr: self.stderr.tail(),
        }
    }

    /// 按进程树终止并回收；已回收时什么也不做
    fn terminate(&mut self) {
        if self.reaped {
            return;
        }
        self.registration.take();
        self.stdin.take();
        kill_and_reap(&mut self.child);
        self.reaped = true;
    }
}

impl Drop for PythonSession<'_> {
    fn drop(&mut self) {
        self.terminate();
    }
}

// ───────────────────────── 脚本路径与平台配置 ─────────────────────────

/// Tauri 资源目录，由 `set_resource_dir` 登记
static RESOURCE_DIR: OnceLock<PathBuf> = OnceLock::new();

/// 开发构建的源码目录（`CARGO_MANIFEST_DIR`）。发布版不带：见 `script_candidates`
#[cfg(debug_assertions)]
const DEV_MANIFEST_DIR: Option<&str> = Some(env!("CARGO_MANIFEST_DIR"));
#[cfg(not(debug_assertions))]
const DEV_MANIFEST_DIR: Option<&str> = None;

/// 登记 Tauri 资源目录（`app.path().resource_dir()`），启动时调用一次，之后再调用不生效。
/// Linux 的 deb / rpm / AppImage 把脚本装在资源目录（`/usr/lib/<产品名>/scripts`），
/// 只按 exe 位置推算找不到
pub fn set_resource_dir(dir: PathBuf) {
    let _ = RESOURCE_DIR.set(dir);
}

/// 解析打包的 Python 脚本路径。
///
/// 按 `script_candidates` 的顺序取第一个存在的路径，全部未命中时在错误信息中列出搜索路径。
pub fn find_script(script_name: &str) -> Result<PathBuf, String> {
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|p| p.to_path_buf()))
        .unwrap_or_else(|| PathBuf::from("."));
    let candidates = script_candidates(
        script_name,
        DEV_MANIFEST_DIR.map(Path::new),
        RESOURCE_DIR.get().map(PathBuf::as_path),
        &exe_dir,
    );
    locate_script(script_name, &candidates)
}

/// 脚本的候选路径，按查找顺序：
/// 1. 开发构建的 `CARGO_MANIFEST_DIR/scripts/`（改脚本不用重新构建）。发布版不查：那是编译机上的
///    绝对路径，在用户机器上可能被其他本地用户建出来并放入同名脚本；
/// 2. Tauri 资源目录的 `scripts/`（Windows 即 exe 所在目录，macOS 是 `.app/Contents/Resources`，
///    Linux 安装包是 `/usr/lib/<产品名>`）；
/// 3. exe 同级 `scripts/`；4. exe 同级（NSIS）；5. macOS `.app` 的 `../Resources/scripts/`。
///
/// 重复的路径只保留第一个。
fn script_candidates(
    script_name: &str,
    dev_dir: Option<&Path>,
    resource_dir: Option<&Path>,
    exe_dir: &Path,
) -> Vec<PathBuf> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    let mut push = |path: PathBuf| {
        if !candidates.contains(&path) {
            candidates.push(path);
        }
    };
    if let Some(dir) = dev_dir {
        push(dir.join("scripts").join(script_name));
    }
    if let Some(dir) = resource_dir {
        push(dir.join("scripts").join(script_name));
    }
    push(exe_dir.join("scripts").join(script_name));
    push(exe_dir.join(script_name));
    push(exe_dir.join("../Resources/scripts").join(script_name));
    candidates
}

fn locate_script(script_name: &str, candidates: &[PathBuf]) -> Result<PathBuf, String> {
    for path in candidates {
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
fn configure_python_command_with_priority(
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
fn configure_python_command_with_priority(
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
    use super::super::test_support::process_alive as is_alive;
    use super::*;
    use serde_json::json;
    use std::sync::Arc;

    fn sh(script: &str) -> Command {
        let mut cmd = Command::new("sh");
        cmd.args(["-c", script]);
        cmd
    }

    /// 等槽里出现登记的 PID（最多 5 秒）
    fn wait_for_pid(slot: &Mutex<Option<u32>>) -> Option<u32> {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let pid = *slot.lock().unwrap();
            if pid.is_some() || Instant::now() >= deadline {
                return pid;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
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

        let exit = run_json_lines_script_with(
            sh(r#"echo '{"type":"log","message":"a"}'; echo 'not json'; echo 'warn line' >&2; echo '{"type":"done","success":2}'; exit 3"#),
            None,
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
                cancelled: false,
                timed_out: false,
                stderr_tail: "warn line".into(),
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
        // 返回前已等 stderr 读完
        assert_eq!(*stderr_lines.lock().unwrap(), vec!["warn line".to_string()]);
    }

    #[test]
    fn message_error_kills_the_process_tree() {
        static SLOT: Mutex<Option<u32>> = Mutex::new(None);
        let cancel = AtomicBool::new(false);
        let mut pid = None;
        let start = Instant::now();
        let result = run_json_lines_script_with(
            sh(r#"echo '{"type":"error","message":"boom"}'; sleep 30"#),
            None,
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

    /// 调用方拿到 Err 后发终态事件，之前写出的 stderr 警告不能落在它后面
    #[test]
    fn message_error_returns_after_stderr_is_drained() {
        static SLOT: Mutex<Option<u32>> = Mutex::new(None);
        let cancel = AtomicBool::new(false);
        let order = Arc::new(Mutex::new(Vec::new()));
        let sink = order.clone();
        let result = run_json_lines_script_with(
            sh(
                r#"echo 'Traceback: boom' >&2; echo '{"type":"error","message":"boom"}'; exec sleep 30"#,
            ),
            None,
            &SLOT,
            &cancel,
            move |line| {
                // 处理得慢一点：不等 stderr 读完就返回的话，这一行会排在返回之后
                std::thread::sleep(Duration::from_millis(200));
                sink.lock().unwrap().push(line);
            },
            |msg| Err(msg["message"].as_str().unwrap_or("").to_string()),
        );
        order.lock().unwrap().push("returned".to_string());
        assert_eq!(result, Err("boom".to_string()));
        assert_eq!(*order.lock().unwrap(), ["Traceback: boom", "returned"]);
    }

    #[test]
    fn cancel_terminates_a_silent_script() {
        static SLOT: Mutex<Option<u32>> = Mutex::new(None);
        let cancel = Arc::new(AtomicBool::new(false));
        let flag = cancel.clone();
        // 等运行器登记 PID 后再取消，事后确认正是这个进程被终止
        let canceller = std::thread::spawn(move || {
            let pid = wait_for_pid(&SLOT);
            flag.store(true, Ordering::SeqCst);
            pid
        });
        let start = Instant::now();
        let exit =
            run_json_lines_script_with(sh("sleep 30"), None, &SLOT, &cancel, |_| {}, |_| Ok(()))
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
        let err = run_json_lines_script_with(
            Command::new("/nonexistent/purinbox-python"),
            None,
            &SLOT,
            &cancel,
            |_| {},
            |_| Ok(()),
        )
        .unwrap_err();
        assert!(err.starts_with("启动 Python 失败"), "{err}");
        assert_eq!(*SLOT.lock().unwrap(), None);
    }

    #[test]
    fn silence_limit_terminates_a_stalled_script() {
        static SLOT: Mutex<Option<u32>> = Mutex::new(None);
        let cancel = AtomicBool::new(false);
        let mut pid = None;
        let start = Instant::now();
        let exit = run_json_lines_script_with(
            sh(r#"echo '{"type":"log","message":"loading"}'; sleep 30"#),
            Some(Duration::from_millis(300)),
            &SLOT,
            &cancel,
            |_| {},
            |_| {
                pid = *SLOT.lock().unwrap();
                Ok(())
            },
        )
        .unwrap();
        assert!(exit.timed_out);
        assert!(!exit.cancelled);
        assert_eq!(exit.code, None, "被信号终止，没有退出码");
        assert!(start.elapsed() >= Duration::from_millis(300));
        assert!(start.elapsed() < Duration::from_secs(10));
        assert!(!is_alive(pid.expect("应登记 PID")), "进程应已被终止");
        assert_eq!(*SLOT.lock().unwrap(), None);
    }

    #[test]
    fn messages_reset_the_silence_limit() {
        static SLOT: Mutex<Option<u32>> = Mutex::new(None);
        let cancel = AtomicBool::new(false);
        let mut count = 0;
        // 总时长约 1.5 秒，超过静默时限；相邻消息只隔 0.1 秒
        let exit = run_json_lines_script_with(
            sh(r#"i=0; while [ $i -lt 15 ]; do echo '{"type":"progress"}'; sleep 0.1; i=$((i+1)); done"#),
            Some(Duration::from_secs(1)),
            &SLOT,
            &cancel,
            |_| {},
            |_| {
                count += 1;
                Ok(())
            },
        )
        .unwrap();
        assert!(!exit.timed_out);
        assert_eq!(exit.code, Some(0));
        assert_eq!(count, 15);
    }

    #[test]
    fn cancel_still_works_after_stdout_closes() {
        static SLOT: Mutex<Option<u32>> = Mutex::new(None);
        let cancel = Arc::new(AtomicBool::new(false));
        let reported = Arc::new(Mutex::new(None::<u32>));
        let canceller = {
            let (flag, reported) = (cancel.clone(), reported.clone());
            std::thread::spawn(move || {
                // stderr 上报 PID 时已登记；登记被清掉说明运行器已读到 stdout 关闭、在等进程退出
                let deadline = Instant::now() + Duration::from_secs(5);
                while (reported.lock().unwrap().is_none() || SLOT.lock().unwrap().is_some())
                    && Instant::now() < deadline
                {
                    std::thread::sleep(Duration::from_millis(5));
                }
                flag.store(true, Ordering::SeqCst);
            })
        };
        let sink = reported.clone();
        let start = Instant::now();
        let exit = run_json_lines_script_with(
            sh("echo $$ >&2; exec 1>&-; sleep 30"),
            None,
            &SLOT,
            &cancel,
            move |line| *sink.lock().unwrap() = line.parse().ok(),
            |_| Ok(()),
        )
        .unwrap();
        canceller.join().unwrap();
        let pid = reported.lock().unwrap().expect("脚本应已上报 PID");
        assert!(exit.cancelled);
        assert!(start.elapsed() < Duration::from_secs(10));
        assert!(!is_alive(pid));
    }

    #[test]
    fn stderr_tail_keeps_the_last_lines() {
        static SLOT: Mutex<Option<u32>> = Mutex::new(None);
        let cancel = AtomicBool::new(false);
        let exit = run_json_lines_script_with(
            PythonCommand::from(sh(
                "for i in 1 2 3 4 5 6; do echo \"line $i\" >&2; done; exit 1",
            )),
            None,
            &SLOT,
            &cancel,
            |_| {},
            |_| Ok(()),
        )
        .unwrap();
        assert_eq!(exit.code, Some(1));
        assert_eq!(exit.stderr_tail, "line 3 | line 4 | line 5 | line 6");
    }

    #[test]
    fn kill_and_reap_takes_down_the_process_group() {
        let dir = super::super::test_support::TempDir::new("kill_and_reap");
        let grandchild_pid = dir.join("grandchild");
        let mut child = spawn_exclusive(
            &mut PythonCommand::from(sh(&format!(
                "sleep 30 & echo $! > '{}'; wait",
                grandchild_pid.display()
            )))
            .build(),
        )
        .unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        let grandchild = loop {
            let text = std::fs::read_to_string(&grandchild_pid).unwrap_or_default();
            if let Ok(pid) = text.trim().parse::<u32>() {
                break pid;
            }
            assert!(Instant::now() < deadline, "后台进程应已启动");
            std::thread::sleep(Duration::from_millis(10));
        };
        let pid = child.id();
        kill_and_reap(&mut child);
        assert!(
            matches!(child.try_wait(), Ok(Some(_))),
            "直接子进程应已回收"
        );
        assert!(!is_alive(pid));
        assert!(
            super::super::test_support::wait_process_gone(grandchild),
            "同一进程组里的孙进程也应被终止"
        );
    }
}

#[cfg(all(test, unix))]
mod command_tests {
    use super::*;

    #[test]
    fn build_sets_pipes_env_and_own_process_group() {
        let output = PythonCommand::new("sh")
            .args([
                "-c",
                r#"read -r line; printf '%s|%s|%s|[%s]\n' "$NO_COLOR" "$PYTHONUNBUFFERED" "$PYTHONIOENCODING" "$line"; ps -o pgid= -p $$ | tr -d ' '; echo $$; echo err >&2"#,
            ])
            .env("PURIN_TEST", "1")
            .output()
            .unwrap();
        assert!(output.status.success());
        let stdout = String::from_utf8(output.stdout).unwrap();
        let lines: Vec<&str> = stdout.lines().collect();
        // stdin 置空：read 立即读到 EOF
        assert_eq!(lines[0], "1|1|utf-8|[]");
        // 自成进程组，按进程组杀树才能连同 worker 一起终止
        assert_eq!(lines[1], lines[2], "PGID 应等于 PID");
        assert_eq!(output.stderr, b"err\n");
    }

    #[test]
    fn stdin_piped_and_built_command_settings_reach_the_process() {
        let dir = super::super::test_support::TempDir::new("python_command_cwd");
        let mut cmd = PythonCommand::new("sh")
            .args(["-c", r#"read -r line; echo "got $line in $(pwd -P)""#])
            .stdin_piped()
            .build();
        cmd.current_dir(&*dir);
        let mut child = spawn_exclusive(&mut cmd).unwrap();
        child.stdin.take().unwrap().write_all(b"ping\n").unwrap();
        let output = child.wait_with_output().unwrap();
        let expected = format!("got ping in {}\n", dir.canonicalize().unwrap().display());
        assert_eq!(String::from_utf8(output.stdout).unwrap(), expected);
    }

    #[test]
    fn from_command_keeps_program_and_args() {
        let mut cmd = hidden_command("sh");
        cmd.args(["-c", "echo $0-$1", "a", "b"]);
        let output = PythonCommand::from(cmd).output().unwrap();
        assert_eq!(output.stdout, b"a-b\n");
    }
}

#[cfg(all(test, unix))]
mod output_tests {
    use super::super::test_support::{wait_process_gone, TempDir};
    use super::*;

    fn sh(script: &str) -> Command {
        let mut cmd = hidden_command("/bin/sh");
        cmd.args(["-c", script]);
        cmd
    }

    #[test]
    fn drains_both_pipes_and_preserves_invalid_utf8() {
        let output = output_cancellable(
            sh("head -c 131072 /dev/zero; head -c 131072 /dev/zero >&2; printf '\\377' >&2; exit 7"),
            &AtomicBool::new(false),
        )
        .unwrap();
        assert_eq!(output.status.code(), Some(7));
        assert_eq!(output.stdout.len(), 131072);
        assert_eq!(output.stderr.len(), 131073);
        assert_eq!(output.stderr.last(), Some(&255));
    }

    #[test]
    fn cancelled_before_start_never_spawns() {
        let root = TempDir::new("output_precancel");
        let marker = root.join("started");
        let error = output_cancellable(
            sh(&format!("touch '{}'", marker.display())),
            &AtomicBool::new(true),
        )
        .unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::Interrupted);
        assert_eq!(error.to_string(), "已取消");
        assert!(!marker.exists());
    }

    #[test]
    fn cancel_kills_the_running_process_tree() {
        let root = TempDir::new("output_cancel");
        let pids = root.join("pids");
        let cancel = Arc::new(AtomicBool::new(false));
        let canceller = {
            let (flag, pids) = (cancel.clone(), pids.clone());
            std::thread::spawn(move || {
                let deadline = Instant::now() + Duration::from_secs(5);
                loop {
                    let text = std::fs::read_to_string(&pids).unwrap_or_default();
                    let found: Vec<u32> = text
                        .split_whitespace()
                        .filter_map(|p| p.parse().ok())
                        .collect();
                    if found.len() == 2 || Instant::now() >= deadline {
                        flag.store(true, Ordering::SeqCst);
                        return found;
                    }
                    std::thread::sleep(Duration::from_millis(10));
                }
            })
        };
        let start = Instant::now();
        let error = output_cancellable(
            sh(&format!(
                "sleep 30 & echo \"$$ $!\" > '{}.tmp'; mv '{}.tmp' '{}'; wait",
                pids.display(),
                pids.display(),
                pids.display()
            )),
            &cancel,
        )
        .unwrap_err();
        let found = canceller.join().unwrap();
        assert_eq!(error.kind(), std::io::ErrorKind::Interrupted);
        assert!(start.elapsed() < Duration::from_secs(10));
        assert_eq!(found.len(), 2, "子进程应已写出 PID");
        for pid in found {
            assert!(
                wait_process_gone(pid),
                "shell 和它派生的进程都应被终止: {pid}"
            );
        }
    }
}

#[cfg(all(test, unix))]
mod session_tests {
    use super::super::test_support::process_alive as is_alive;
    use super::*;
    use serde_json::json;

    const WAIT: Duration = Duration::from_secs(5);

    fn script(body: &str) -> PythonCommand {
        PythonCommand::new("sh").args(["-c", body])
    }

    /// 读取 init 后先发 log、非协议行和其他消息，再报 ready；之后把每条命令包进 result 回显，
    /// 收到 quit 时往 stderr 写一行再退出
    const ECHO_SERVER: &str = r#"
read -r init
printf '%s\n' '{"type":"log","message":"loading"}' 'not json' '{"type":"other"}' '{"type":"ready"}'
while read -r line; do
  case "$line" in
    *quit*) echo bye >&2; exit 0;;
    *) printf '{"type":"result","echo":%s}\n' "$line";;
  esac
done
"#;

    #[test]
    fn round_trip_then_graceful_shutdown() {
        static SLOT: Mutex<Option<u32>> = Mutex::new(None);
        let cancel = AtomicBool::new(false);
        let stderr = Arc::new(Mutex::new(Vec::new()));
        let sink = stderr.clone();
        let mut session = PythonSession::start(script(ECHO_SERVER), &SLOT, &cancel, move |l| {
            sink.lock().unwrap().push(l)
        })
        .unwrap();
        let pid = session.pid();
        assert_eq!(*SLOT.lock().unwrap(), Some(pid));

        session.send(&json!({"cmd": "init"})).unwrap();
        let mut logs = Vec::new();
        session
            .wait_ready(WAIT, |msg| logs.push(msg.clone()))
            .unwrap();
        assert_eq!(logs, vec![json!({"type": "log", "message": "loading"})]);

        session.send(&json!({"n": 1})).unwrap();
        assert_eq!(
            session.recv(WAIT).unwrap(),
            json!({"type": "result", "echo": {"n": 1}})
        );

        let start = Instant::now();
        session.shutdown(Some(r#"{"cmd":"quit"}"#));
        assert!(start.elapsed() < SHUTDOWN_GRACE, "收到退出命令应自行退出");
        assert_eq!(*SLOT.lock().unwrap(), None);
        assert!(!is_alive(pid));
        assert_eq!(*stderr.lock().unwrap(), vec!["bye".to_string()]);
    }

    #[test]
    fn early_exit_reports_stderr_tail() {
        static SLOT: Mutex<Option<u32>> = Mutex::new(None);
        let cancel = AtomicBool::new(false);
        let mut session = PythonSession::start(
            script("read -r init; for i in 1 2 3 4 5 6; do echo \"line $i\" >&2; done; exit 3"),
            &SLOT,
            &cancel,
            |_| {},
        )
        .unwrap();
        session.send(&json!({"cmd": "init"})).unwrap();
        let error = session.wait_ready(WAIT, |_| {}).unwrap_err();
        assert_eq!(
            error,
            SessionError::Exited {
                stderr: "line 3 | line 4 | line 5 | line 6".into()
            }
        );
        assert_eq!(
            error.to_string(),
            "Python 进程已退出: line 3 | line 4 | line 5 | line 6"
        );
        assert_eq!(session.stderr_tail(), "line 3 | line 4 | line 5 | line 6");
        assert_eq!(*SLOT.lock().unwrap(), None, "进程退出后应清除登记");
        assert_eq!(session.send(&json!({"cmd": "next"})), Err(error.clone()));
        assert!(matches!(
            session.recv(WAIT),
            Err(SessionError::Exited { .. })
        ));
    }

    #[test]
    fn send_to_a_process_that_closed_stdin_reports_exit() {
        static SLOT: Mutex<Option<u32>> = Mutex::new(None);
        let cancel = AtomicBool::new(false);
        let (tx, rx) = mpsc::channel();
        let mut session = PythonSession::start(
            script("exec 0<&-; echo 'stdin closed' >&2; sleep 30"),
            &SLOT,
            &cancel,
            move |line| {
                let _ = tx.send(line);
            },
        )
        .unwrap();
        let pid = session.pid();
        assert_eq!(rx.recv_timeout(WAIT).unwrap(), "stdin closed");
        // 别的测试线程同时启动的子进程可能短暂继承了 stdin 的读端，那时写入还会成功
        let deadline = Instant::now() + WAIT;
        let error = loop {
            match session.send(&json!({"cmd": "x"})) {
                Err(error) => break error,
                Ok(()) => {
                    assert!(Instant::now() < deadline, "stdin 已关闭，写入应失败");
                    std::thread::sleep(Duration::from_millis(10));
                }
            }
        };
        assert_eq!(
            error,
            SessionError::Exited {
                stderr: "stdin closed".into()
            }
        );
        assert!(!is_alive(pid), "写不进去的会话应已终止");
        assert_eq!(*SLOT.lock().unwrap(), None);
    }

    #[test]
    fn script_error_and_timeout_while_waiting_for_ready() {
        static SLOT: Mutex<Option<u32>> = Mutex::new(None);
        let cancel = AtomicBool::new(false);
        let mut session = PythonSession::start(
            script(r#"read -r init; echo '{"type":"error","message":"模型加载失败"}'; sleep 30"#),
            &SLOT,
            &cancel,
            |_| {},
        )
        .unwrap();
        let pid = session.pid();
        session.send(&json!({"cmd": "init"})).unwrap();
        assert_eq!(
            session.wait_ready(WAIT, |_| {}),
            Err(SessionError::Script("模型加载失败".into()))
        );
        drop(session);
        assert!(!is_alive(pid), "drop 时应终止进程");
        assert_eq!(*SLOT.lock().unwrap(), None);

        let mut session = PythonSession::start(script("sleep 30"), &SLOT, &cancel, |_| {}).unwrap();
        let pid = session.pid();
        let start = Instant::now();
        let limit = Duration::from_millis(300);
        assert_eq!(
            session.wait_ready(limit, |_| {}),
            Err(SessionError::TimedOut(limit))
        );
        assert!(start.elapsed() >= limit);
        assert!(start.elapsed() < WAIT);
        assert!(is_alive(pid), "超时不终止进程，由调用方决定");
        drop(session);
        assert!(!is_alive(pid));
    }

    #[test]
    fn cancel_flag_stops_a_silent_wait_promptly() {
        static SLOT: Mutex<Option<u32>> = Mutex::new(None);
        let cancel = Arc::new(AtomicBool::new(false));
        let mut session = PythonSession::start(script("sleep 30"), &SLOT, &cancel, |_| {}).unwrap();
        let pid = session.pid();
        let flag = cancel.clone();
        let canceller = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(100));
            flag.store(true, Ordering::SeqCst);
            Instant::now()
        });
        assert_eq!(
            session.recv(Duration::from_secs(30)),
            Err(SessionError::Cancelled)
        );
        let cancelled_at = canceller.join().unwrap();
        assert!(
            cancelled_at.elapsed() < Duration::from_secs(2),
            "取消应在一个轮询间隔左右生效"
        );
        assert!(!is_alive(pid));
        assert_eq!(*SLOT.lock().unwrap(), None);
        assert_eq!(
            session.send(&json!({"cmd": "x"})),
            Err(SessionError::Cancelled)
        );
    }

    #[test]
    fn kill_through_the_pid_slot() {
        static SLOT: Mutex<Option<u32>> = Mutex::new(None);
        // 取消命令：先置取消标志再按登记的 PID 强杀 → 取消
        let cancel = Arc::new(AtomicBool::new(false));
        let mut session = PythonSession::start(script("sleep 30"), &SLOT, &cancel, |_| {}).unwrap();
        let pid = session.pid();
        cancel.store(true, Ordering::SeqCst);
        kill_registered_pid(&SLOT);
        assert_eq!(session.recv(WAIT), Err(SessionError::Cancelled));
        assert!(!is_alive(pid));
        drop(session);

        // 没有取消标志、进程被外部终止 → 进程退出。
        // 等 shell 派生完 sleep 再杀：进程组正在创建成员时 macOS 的按组终止会漏掉它，
        // 应用里取消命令同时置了取消标志，会话会再杀一次，这里模拟的是单纯的外部终止
        let cancel = AtomicBool::new(false);
        let (tx, started) = mpsc::channel();
        let mut session = PythonSession::start(
            script("sleep 30 & echo started >&2; wait"),
            &SLOT,
            &cancel,
            move |line| {
                let _ = tx.send(line);
            },
        )
        .unwrap();
        let pid = session.pid();
        assert_eq!(started.recv_timeout(WAIT).unwrap(), "started");
        kill_registered_pid(&SLOT);
        assert_eq!(
            session.recv(WAIT),
            Err(SessionError::Exited {
                stderr: "started".into()
            })
        );
        assert!(!is_alive(pid));
        assert_eq!(*SLOT.lock().unwrap(), None);
    }

    #[test]
    fn kill_after_timeout_skips_the_shutdown_grace() {
        static SLOT: Mutex<Option<u32>> = Mutex::new(None);
        let cancel = AtomicBool::new(false);
        let mut session =
            PythonSession::start(script("echo stuck >&2; sleep 30"), &SLOT, &cancel, |_| {})
                .unwrap();
        let pid = session.pid();
        assert!(matches!(
            session.recv(Duration::from_millis(100)),
            Err(SessionError::TimedOut(_))
        ));
        session.kill();
        assert!(!is_alive(pid));
        assert_eq!(*SLOT.lock().unwrap(), None);
        assert_eq!(
            session.recv(WAIT),
            Err(SessionError::Exited {
                stderr: "stuck".into()
            })
        );
        let start = Instant::now();
        session.shutdown(Some(r#"{"cmd":"quit"}"#));
        assert!(start.elapsed() < SHUTDOWN_GRACE);
    }

    #[test]
    fn shutdown_kills_a_process_that_does_not_exit() {
        static SLOT: Mutex<Option<u32>> = Mutex::new(None);
        let cancel = AtomicBool::new(false);
        let session = PythonSession::start(script("sleep 30"), &SLOT, &cancel, |_| {}).unwrap();
        let pid = session.pid();
        let start = Instant::now();
        session.shutdown(None);
        assert!(start.elapsed() >= SHUTDOWN_GRACE);
        assert!(start.elapsed() < SHUTDOWN_GRACE + WAIT);
        assert!(!is_alive(pid));
        assert_eq!(*SLOT.lock().unwrap(), None);
    }

    #[test]
    fn start_honours_cancel_and_reports_spawn_failure() {
        static SLOT: Mutex<Option<u32>> = Mutex::new(None);
        let cancelled = AtomicBool::new(true);
        assert!(matches!(
            PythonSession::start(script("exit 0"), &SLOT, &cancelled, |_| {}),
            Err(SessionError::Cancelled)
        ));
        let cancel = AtomicBool::new(false);
        let error = match PythonSession::start(
            PythonCommand::new("/nonexistent/purinbox-python"),
            &SLOT,
            &cancel,
            |_| {},
        ) {
            Err(error) => error,
            Ok(_) => panic!("不存在的程序不该启动成功"),
        };
        assert!(matches!(error, SessionError::Spawn(_)));
        assert!(
            error.to_string().starts_with("启动 Python 进程失败"),
            "{error}"
        );
        assert_eq!(*SLOT.lock().unwrap(), None);
    }

    #[test]
    fn error_messages() {
        assert_eq!(SessionError::Cancelled.to_string(), "已取消");
        assert_eq!(
            SessionError::Exited {
                stderr: String::new()
            }
            .to_string(),
            "Python 进程已退出"
        );
        assert_eq!(
            SessionError::TimedOut(PYTHON_SILENCE_LIMIT).to_string(),
            "Python 进程无响应（300 秒）"
        );
        assert_eq!(String::from(SessionError::Script("坏了".into())), "坏了");
    }
}

#[cfg(test)]
mod one_shot_tests {
    use super::super::test_support::TempDir;
    use super::*;

    #[test]
    fn temp_tag_matches_python_naming() {
        let root = TempDir::new("python_temp_tag");
        let tag = PythonTempTag::for_run(42);
        assert_eq!(
            tag.temp_file(&root.join("a.png")),
            root.join("a.png.purin-r42.tmp")
        );
        std::fs::create_dir_all(root.join("sub")).unwrap();
        for name in [
            "a.png.purin-r42.tmp",
            "sub/b.png.purin-r42.tmp",
            "c.png.purin-r7.tmp",
            "a.png",
        ] {
            std::fs::write(root.join(name), []).unwrap();
        }
        tag.remove_under(&root);
        assert!(!root.join("a.png.purin-r42.tmp").exists());
        assert!(!root.join("sub/b.png.purin-r42.tmp").exists());
        assert!(root.join("c.png.purin-r7.tmp").exists());
        assert!(root.join("a.png").exists());
    }

    #[test]
    fn abnormal_exit_explains_code_and_stderr() {
        let exit = |code, stderr: &str| ScriptExit {
            code,
            cancelled: false,
            timed_out: false,
            stderr_tail: stderr.into(),
        };
        assert_eq!(
            abnormal_exit("超分进程", &exit(Some(1), "")),
            "超分进程异常退出（退出码 1），未返回结果"
        );
        assert_eq!(
            abnormal_exit("超分进程", &exit(None, "MemoryError")),
            "超分进程异常退出（被系统终止）: MemoryError"
        );
    }
}

#[cfg(test)]
mod script_path_tests {
    use super::super::test_support::TempDir;
    use super::*;

    #[test]
    fn dev_dir_only_comes_first_when_given_and_duplicates_are_dropped() {
        let dev = Path::new("/src/src-tauri");
        let res = Path::new("/usr/lib/PurinBox");
        let exe = Path::new("/usr/bin");
        let all = script_candidates("a.py", Some(dev), Some(res), exe);
        assert_eq!(
            all,
            vec![
                dev.join("scripts").join("a.py"),
                res.join("scripts").join("a.py"),
                exe.join("scripts").join("a.py"),
                exe.join("a.py"),
                exe.join("../Resources/scripts").join("a.py"),
            ]
        );
        // 发布版没有开发目录：资源目录排第一
        let release = script_candidates("a.py", None, Some(res), exe);
        assert_eq!(release, all[1..].to_vec());
        // Windows 的资源目录就是 exe 目录，不重复列出
        let same = script_candidates("a.py", None, Some(exe), exe);
        assert_eq!(same, all[2..].to_vec());
        assert_eq!(
            script_candidates("a.py", None, None, exe),
            all[2..].to_vec()
        );
    }

    #[test]
    fn locate_takes_the_first_existing_candidate() {
        let root = TempDir::new("script_locate");
        let res = root.join("res");
        let exe = root.join("bin");
        for dir in [res.join("scripts"), exe.join("scripts")] {
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("a.py"), "").unwrap();
        }
        let candidates = script_candidates("a.py", None, Some(&res), &exe);
        assert_eq!(
            locate_script("a.py", &candidates).unwrap(),
            res.join("scripts/a.py").canonicalize().unwrap()
        );
        std::fs::remove_file(res.join("scripts/a.py")).unwrap();
        assert_eq!(
            locate_script("a.py", &candidates).unwrap(),
            exe.join("scripts/a.py").canonicalize().unwrap()
        );

        let error = locate_script(
            "missing.py",
            &script_candidates("missing.py", None, Some(&res), &exe),
        )
        .unwrap_err();
        assert!(
            error.starts_with("推理脚本 missing.py 未找到。\n搜索路径:\n  1. "),
            "{error}"
        );
        assert_eq!(error.lines().count(), 2 + 4);
    }

    #[test]
    fn bundled_scripts_are_found() {
        assert!(find_script("purin_proto.py").unwrap().is_file());
    }
}

#[cfg(test)]
mod stderr_warning_tests {
    use super::*;

    #[test]
    fn noise_is_filtered_and_lines_become_warnings() {
        static MUTE: AtomicBool = AtomicBool::new(false);
        let app = tauri::test::mock_app();
        let events = super::super::batch::capture_events(app.handle(), "stderr-test-progress");
        let mut runtime_only = stderr_warnings(
            app.handle(),
            "stderr-test-progress",
            StderrNoise::Runtime,
            None,
        );
        let mut with_libraries = stderr_warnings(
            app.handle(),
            "stderr-test-progress",
            StderrNoise::RuntimeAndLibraries,
            Some(&MUTE),
        );
        let runtime_noise = "[W:onnxruntime:, x.cc:1] warn".to_string();
        let library_noise =
            "/env/lib/python3.12/site-packages/umap/umap_.py:1952: UserWarning: n_jobs".to_string();
        runtime_only(runtime_noise.clone());
        runtime_only(library_noise.clone());
        runtime_only("Traceback (most recent call last):".into());
        with_libraries(runtime_noise);
        with_libraries(library_noise);
        with_libraries("ValueError: bad".into());
        MUTE.store(true, Ordering::SeqCst);
        with_libraries("BrokenPipeError: [Errno 32]".into());

        let events = events.lock().unwrap();
        let messages: Vec<&str> = events
            .iter()
            .map(|e| {
                assert_eq!(e["status"], "warning");
                e["message"].as_str().unwrap()
            })
            .collect();
        assert_eq!(
            messages,
            vec![
                "[Python] /env/lib/python3.12/site-packages/umap/umap_.py:1952: UserWarning: n_jobs",
                "[Python] Traceback (most recent call last):",
                "[Python] ValueError: bad",
            ]
        );
    }
}
