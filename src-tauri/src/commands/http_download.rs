//! 流式 HTTP 下载：打标模型、美学/裁切模型、超分引擎与权重、独立版 Python 共用。
//!
//! 一次下载的固定流程在 `download_to_file` 里：写 `{dest}.part` → 分块写入、随时检查取消 →
//! 每 500ms 上报一次进度 → flush → 校验字节数后原子替换为 `dest`；任何失败（含取消）都删除
//! .part 残件，已有的 `dest` 只在下载完整成功后才被替换。
//! 事件名、开始/完成文案、错误是否另发 error 事件、取消后的善后由各模块自己决定。

use futures_util::StreamExt;
use serde::Serialize;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use tokio::io::AsyncWriteExt;

/// 建立连接的超时
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);
/// 停滞超时：连续这么久读不到任何数据才算失败。不设总时长上限——
/// 慢速网络下几百 MB 的模型要下十几分钟，总时长超时会在中途把它掐断
pub const STALL_TIMEOUT: Duration = Duration::from_secs(120);
/// 进度上报间隔
const REPORT_INTERVAL: Duration = Duration::from_millis(500);
/// 检查取消标志的间隔：服务器停滞不发数据时取消也要及时生效
const CANCEL_POLL: Duration = Duration::from_millis(200);
const MIB: f64 = 1_048_576.0;

/// 下载类进度事件的 payload（各模块的 `*-download` 事件）。
/// 前端 appendDownloadLog 只读 status / message / percent / speed_mbps
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct DownloadProgress {
    /// 附带的文件名，空时不序列化
    #[serde(skip_serializing_if = "String::is_empty")]
    pub filename: String,
    /// 0–100
    pub percent: f32,
    /// 平均速度（MB/s），非下载阶段为 0
    pub speed_mbps: f64,
    /// "downloading" | "extracting" | "done" | "cancelled" | "error"
    pub status: String,
    pub message: String,
}

impl DownloadProgress {
    pub fn new(status: &str, percent: f32, message: impl Into<String>) -> Self {
        DownloadProgress {
            filename: String::new(),
            percent,
            speed_mbps: 0.0,
            status: status.to_string(),
            message: message.into(),
        }
    }

    /// 开始下载 `label`：0% 的进度条，文案 "正在下载 {label}"。
    /// `download_to_file` 不发开始事件，需要的调用方在调用前自己发这一条
    pub fn starting(label: &str) -> Self {
        Self::new("downloading", 0.0, format!("正在下载 {}", label))
    }

    /// 完成，进度 100%（前端收到后移除进度条）
    pub fn done(message: impl Into<String>) -> Self {
        Self::new("done", 100.0, message)
    }

    /// 已取消（前端收到后移除进度条）
    pub fn cancelled(message: impl Into<String>) -> Self {
        Self::new("cancelled", 0.0, message)
    }

    /// 失败（前端收到后移除进度条并记一条错误）
    pub fn error(message: impl Into<String>) -> Self {
        Self::new("error", 0.0, message)
    }

    pub fn with_filename(mut self, filename: impl Into<String>) -> Self {
        self.filename = filename.into();
        self
    }
}

/// `download_to_file` 的失败原因。Display 即给用户看的文案
#[derive(Debug)]
pub enum DownloadError {
    /// 取消标志置位（.part 已删除）
    Cancelled,
    /// 服务器返回非 2xx（尚未写任何文件）
    Status {
        status: reqwest::StatusCode,
        url: String,
    },
    /// 网络、写盘、字节数校验等其他失败（.part 已删除）
    Other(String),
}

impl std::fmt::Display for DownloadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DownloadError::Cancelled => f.write_str("下载已取消"),
            DownloadError::Status { status, url } => write!(f, "HTTP {}: {}", status, url),
            DownloadError::Other(message) => f.write_str(message),
        }
    }
}

impl std::error::Error for DownloadError {}

impl From<DownloadError> for String {
    fn from(e: DownloadError) -> String {
        e.to_string()
    }
}

/// 下载用的 HTTP 客户端：应用内代理 + 连接 30s / 停滞 120s 超时，不设总时长超时
pub fn download_client() -> Result<reqwest::Client, String> {
    super::proxy_config::build_http_client()
        .connect_timeout(CONNECT_TIMEOUT)
        .read_timeout(STALL_TIMEOUT)
        .build()
        .map_err(|e| format!("创建 HTTP 客户端失败: {}", e))
}

/// Hugging Face 仓库文件的直链：`https://huggingface.co/{repo}/resolve/main/{path}`
pub fn huggingface_url(repo: &str, path: &str) -> String {
    format!("https://huggingface.co/{}/resolve/main/{}", repo, path)
}

/// 把 `request` 的响应体流式写到 `dest`，返回写入的字节数。
///
/// - `request`：调用方拼好的请求，一般是 `download_client()?.get(url)`，需要鉴权时自己加头
///   （例如 `huggingface_config::apply_huggingface_auth`）；
/// - `label`：进度文案里的名字。收到数据后每 500ms、以及写满 Content-Length 时调一次
///   `on_progress`，文案 `"{label} — x/y MB (z MB/s)"`（大小未知时 `"{label} — x MB (z MB/s)"`），
///   速度是平均速度，status 恒为 "downloading"；调用方按需改写 percent 后发自己的事件。
///   这里不发开始事件：要 "正在下载 {label}" 就在调用前发 `DownloadProgress::starting(label)`；
/// - `cancel`：请求前、等待响应和读数据期间（停滞时每 200ms）都会检查，置位即返回 `Cancelled`；
/// - `dest` 的父目录不存在时自动创建；下载完整后才替换已有的 `dest`。
///
/// 取消只在数据收完之前生效：数据收完后再取消，文件照常落盘，善后由调用方决定。
pub async fn download_to_file(
    request: reqwest::RequestBuilder,
    dest: &Path,
    label: &str,
    cancel: &AtomicBool,
    mut on_progress: impl FnMut(DownloadProgress),
) -> Result<u64, DownloadError> {
    if cancel.load(Ordering::SeqCst) {
        return Err(DownloadError::Cancelled);
    }

    let (client, request) = request.build_split();
    let request = request.map_err(|e| DownloadError::Other(format!("下载请求失败: {}", e)))?;
    let url = request.url().to_string();

    let mut cancel_tick = tokio::time::interval(CANCEL_POLL);
    cancel_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    let send = client.execute(request);
    tokio::pin!(send);
    let response = loop {
        tokio::select! {
            sent = &mut send => {
                break sent.map_err(|e| {
                    DownloadError::Other(format!("下载请求失败 ({}): {}", url, e))
                })?;
            }
            _ = cancel_tick.tick() => {
                if cancel.load(Ordering::SeqCst) {
                    return Err(DownloadError::Cancelled);
                }
            }
        }
    };

    let status = response.status();
    if !status.is_success() {
        return Err(DownloadError::Status { status, url });
    }
    let total = response.content_length().unwrap_or(0);

    if let Some(parent) = dest.parent().filter(|p| !p.as_os_str().is_empty()) {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| DownloadError::Other(format!("创建目录失败: {}", e)))?;
    }
    let part = super::prepare_part_file(dest);
    let mut file = tokio::fs::File::create(&part)
        .await
        .map_err(|e| DownloadError::Other(format!("创建文件失败: {}", e)))?;

    let written: Result<u64, DownloadError> = async {
        let mut stream = response.bytes_stream();
        let start = Instant::now();
        let mut last_report = start;
        let mut downloaded: u64 = 0;
        loop {
            if cancel.load(Ordering::SeqCst) {
                return Err(DownloadError::Cancelled);
            }
            let chunk = tokio::select! {
                next = stream.next() => match next {
                    Some(chunk) => chunk
                        .map_err(|e| DownloadError::Other(format!("下载数据失败: {}", e)))?,
                    None => break,
                },
                _ = cancel_tick.tick() => continue,
            };
            file.write_all(&chunk)
                .await
                .map_err(|e| DownloadError::Other(format!("写入文件失败: {}", e)))?;
            downloaded += chunk.len() as u64;

            let now = Instant::now();
            if now.duration_since(last_report) >= REPORT_INTERVAL
                || (total > 0 && downloaded >= total)
            {
                last_report = now;
                on_progress(progress_tick(label, downloaded, total, start.elapsed()));
            }
        }
        file.flush()
            .await
            .map_err(|e| DownloadError::Other(format!("写入文件失败: {}", e)))?;
        Ok(downloaded)
    }
    .await;
    // 先关闭文件句柄：Windows 上不能 rename / 删除仍打开着的文件
    drop(file);

    match written {
        Ok(downloaded) => {
            super::finalize_part_file(&part, dest, downloaded, total)
                .map_err(DownloadError::Other)?;
            Ok(downloaded)
        }
        Err(e) => {
            let _ = tokio::fs::remove_file(&part).await;
            Err(e)
        }
    }
}

fn progress_tick(label: &str, downloaded: u64, total: u64, elapsed: Duration) -> DownloadProgress {
    let secs = elapsed.as_secs_f64();
    let speed_mbps = if secs > 0.0 {
        downloaded as f64 / secs / MIB
    } else {
        0.0
    };
    let percent = if total > 0 {
        (downloaded as f64 / total as f64 * 100.0) as f32
    } else {
        0.0
    };
    DownloadProgress {
        filename: String::new(),
        percent,
        speed_mbps,
        status: "downloading".to_string(),
        message: progress_message(label, downloaded, total, speed_mbps),
    }
}

fn progress_message(label: &str, downloaded: u64, total: u64, speed_mbps: f64) -> String {
    let done = downloaded as f64 / MIB;
    if total > 0 {
        format!(
            "{} — {:.1}/{:.1} MB ({:.1} MB/s)",
            label,
            done,
            total as f64 / MIB,
            speed_mbps
        )
    } else {
        format!("{} — {:.1} MB ({:.1} MB/s)", label, done, speed_mbps)
    }
}

/// 测试辅助：一次性本地 HTTP 服务器和临时目录（llm_client、tag_sort 的测试也用）。
/// 两者都由测试持有，drop 时自行收尾，测试结束（含断言失败）不留后台线程、连接和目录
#[cfg(test)]
pub(crate) mod test_support {
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::path::{Path, PathBuf};
    use std::sync::mpsc;
    use std::time::Duration;

    /// 服务器收到的请求
    #[derive(Debug, Clone)]
    pub(crate) struct Request {
        pub method: String,
        pub path: String,
        /// 头名称已转小写
        pub headers: Vec<(String, String)>,
        pub body: Vec<u8>,
    }

    impl Request {
        pub(crate) fn header(&self, name: &str) -> Option<&str> {
            let name = name.to_ascii_lowercase();
            self.headers
                .iter()
                .find(|(k, _)| *k == name)
                .map(|(_, v)| v.as_str())
        }

        pub(crate) fn json(&self) -> serde_json::Value {
            serde_json::from_slice(&self.body).expect("请求体应是 JSON")
        }
    }

    /// 响应函数拿到的停止信号：测试结束（`TestServer` 被 drop）时触发
    pub(crate) struct StopSignal(mpsc::Receiver<()>);

    impl StopSignal {
        /// 挂住连接，直到测试结束或超过 `max`（模拟服务器停滞）
        pub(crate) fn wait(&self, max: Duration) {
            let _ = self.0.recv_timeout(max);
        }

        fn stopped_within(&self, timeout: Duration) -> bool {
            matches!(
                self.0.recv_timeout(timeout),
                Err(mpsc::RecvTimeoutError::Disconnected)
            )
        }
    }

    /// 只接一次连接的本地 HTTP/1.1 服务器，由测试持有。
    /// drop 时停止等待连接、放开挂住的响应并回收服务线程
    pub(crate) struct TestServer {
        /// `http://127.0.0.1:端口`，路径由调用方拼
        pub url: String,
        /// 服务器收到的请求
        pub requests: mpsc::Receiver<Request>,
        stop: Option<mpsc::Sender<()>>,
        thread: Option<std::thread::JoinHandle<()>>,
    }

    impl Drop for TestServer {
        fn drop(&mut self) {
            drop(self.stop.take());
            if let Some(thread) = self.thread.take() {
                let _ = thread.join();
            }
        }
    }

    /// 在 127.0.0.1 的随机端口起服务器。读完请求后先把它发进 `requests`，再交给 `respond`
    /// 写响应——可以只写一半就返回（断流），或用 `StopSignal::wait` 挂住（停滞）
    pub(crate) fn serve_once<F>(respond: F) -> TestServer
    where
        F: FnOnce(&mut TcpStream, &Request, &StopSignal) + Send + 'static,
    {
        let listener = TcpListener::bind("127.0.0.1:0").expect("绑定本地端口");
        listener.set_nonblocking(true).expect("设置非阻塞监听");
        let url = format!("http://{}", listener.local_addr().expect("本地地址"));
        let (request_tx, requests) = mpsc::channel();
        let (stop_tx, stop_rx) = mpsc::channel();
        let thread = std::thread::spawn(move || {
            let stop = StopSignal(stop_rx);
            let mut stream = loop {
                match listener.accept() {
                    Ok((stream, _)) => break stream,
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        if stop.stopped_within(Duration::from_millis(10)) {
                            return;
                        }
                    }
                    Err(_) => return,
                }
            };
            drop(listener);
            // macOS 上 accept 出来的连接会继承监听端的非阻塞标志
            let _ = stream.set_nonblocking(false);
            let _ = stream.set_read_timeout(Some(Duration::from_secs(10)));
            let _ = stream.set_write_timeout(Some(Duration::from_secs(10)));
            let Some(request) = read_request(&mut stream) else {
                return;
            };
            let _ = request_tx.send(request.clone());
            respond(&mut stream, &request, &stop);
        });
        TestServer {
            url,
            requests,
            stop: Some(stop_tx),
            thread: Some(thread),
        }
    }

    /// 写一个完整的响应（带 Content-Length，写完关闭连接）
    pub(crate) fn write_response(
        stream: &mut TcpStream,
        status: &str,
        content_type: &str,
        body: &[u8],
    ) {
        let head = format!(
            "HTTP/1.1 {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            status,
            content_type,
            body.len()
        );
        let _ = stream.write_all(head.as_bytes());
        let _ = stream.write_all(body);
        let _ = stream.flush();
    }

    fn read_request(stream: &mut TcpStream) -> Option<Request> {
        let mut buf = Vec::new();
        let mut tmp = [0u8; 8192];
        let header_end = loop {
            if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                break pos + 4;
            }
            let n = stream.read(&mut tmp).ok()?;
            if n == 0 {
                return None;
            }
            buf.extend_from_slice(&tmp[..n]);
        };
        let head = String::from_utf8_lossy(&buf[..header_end]).to_string();
        let mut lines = head.split("\r\n");
        let mut request_line = lines.next()?.split_whitespace();
        let method = request_line.next()?.to_string();
        let path = request_line.next()?.to_string();
        let headers: Vec<(String, String)> = lines
            .filter_map(|line| line.split_once(':'))
            .map(|(k, v)| (k.trim().to_ascii_lowercase(), v.trim().to_string()))
            .collect();
        let content_length = headers
            .iter()
            .find(|(k, _)| k == "content-length")
            .and_then(|(_, v)| v.parse::<usize>().ok())
            .unwrap_or(0);
        let mut body = buf[header_end..].to_vec();
        while body.len() < content_length {
            let n = stream.read(&mut tmp).ok()?;
            if n == 0 {
                break;
            }
            body.extend_from_slice(&tmp[..n]);
        }
        Some(Request {
            method,
            path,
            headers,
            body,
        })
    }

    /// 系统临时目录下的独占目录，drop 时连同内容删除
    pub(crate) struct TempDir(PathBuf);

    impl TempDir {
        pub(crate) fn new(tag: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("purinbox_{}_{}", tag, std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).expect("创建临时目录");
            TempDir(dir)
        }
    }

    impl std::ops::Deref for TempDir {
        type Target = Path;
        fn deref(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::{serve_once, write_response, TempDir};
    use super::*;
    use std::io::Write;
    use std::path::PathBuf;
    use std::sync::Arc;

    fn client() -> reqwest::Client {
        reqwest::Client::builder().no_proxy().build().unwrap()
    }

    fn part_of(dest: &Path) -> PathBuf {
        let mut os = dest.as_os_str().to_os_string();
        os.push(".part");
        PathBuf::from(os)
    }

    fn payload(len: usize) -> Vec<u8> {
        (0..len).map(|i| (i % 251) as u8).collect()
    }

    #[tokio::test]
    async fn downloads_body_replaces_old_file_and_reports_progress() {
        let body = payload(300_000);
        let expected = body.clone();
        let server = serve_once(move |stream, _, _| {
            write_response(stream, "200 OK", "application/octet-stream", &body)
        });
        let dir = TempDir::new("dl_ok");
        let dest = dir.join("sub").join("model.onnx");
        std::fs::create_dir_all(dest.parent().unwrap()).unwrap();
        std::fs::write(&dest, b"old").unwrap();
        std::fs::write(part_of(&dest), b"stale leftover").unwrap();

        let cancel = AtomicBool::new(false);
        let mut ticks = Vec::new();
        let n = download_to_file(
            client().get(format!("{}/file.bin", server.url)),
            &dest,
            "model.onnx",
            &cancel,
            |p| ticks.push(p),
        )
        .await
        .unwrap();

        assert_eq!(n, 300_000);
        assert_eq!(std::fs::read(&dest).unwrap(), expected);
        assert!(!part_of(&dest).exists());
        assert_eq!(server.requests.recv().unwrap().path, "/file.bin");

        let last = ticks.last().unwrap();
        assert_eq!(last.status, "downloading");
        assert_eq!(last.percent, 100.0);
        assert!(
            last.message.starts_with("model.onnx — 0.3/0.3 MB (")
                && last.message.ends_with(" MB/s)"),
            "{}",
            last.message
        );
        assert!(last.speed_mbps > 0.0);
    }

    #[tokio::test]
    async fn creates_missing_parent_dir() {
        let server =
            serve_once(|stream, _, _| write_response(stream, "200 OK", "text/plain", b"hello"));
        let dir = TempDir::new("dl_mkdir");
        let dest = dir.join("a").join("b").join("meta.json");
        let cancel = AtomicBool::new(false);
        download_to_file(
            client().get(&server.url),
            &dest,
            "meta.json",
            &cancel,
            |_| {},
        )
        .await
        .unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), b"hello");
    }

    #[tokio::test]
    async fn http_error_writes_nothing() {
        let server = serve_once(|stream, _, _| {
            write_response(stream, "404 Not Found", "text/plain", b"nope")
        });
        let dir = TempDir::new("dl_404");
        let dest = dir.join("missing.bin");
        let cancel = AtomicBool::new(false);
        let mut ticks = 0;
        let url = format!("{}/missing.bin", server.url);
        let err = download_to_file(client().get(&url), &dest, "missing.bin", &cancel, |_| {
            ticks += 1
        })
        .await
        .unwrap_err();

        match &err {
            DownloadError::Status { status, url: u } => {
                assert_eq!(status.as_u16(), 404);
                assert_eq!(u, &url);
            }
            other => panic!("应为 HTTP 状态错误: {other:?}"),
        }
        assert_eq!(err.to_string(), format!("HTTP 404 Not Found: {}", url));
        assert_eq!(ticks, 0);
        assert!(!dest.exists() && !part_of(&dest).exists());
    }

    /// 声明 1000 字节只发 400 就断开：报错、删残件、旧文件原样保留
    #[tokio::test]
    async fn truncated_body_cleans_up_and_keeps_old_file() {
        let server = serve_once(|stream, _, _| {
            let _ = stream.write_all(
                b"HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nContent-Length: 1000\r\nConnection: close\r\n\r\n",
            );
            let _ = stream.write_all(&payload(400));
        });
        let dir = TempDir::new("dl_truncated");
        let dest = dir.join("model.onnx");
        std::fs::write(&dest, b"old").unwrap();
        let cancel = AtomicBool::new(false);
        let err = download_to_file(
            client().get(&server.url),
            &dest,
            "model.onnx",
            &cancel,
            |_| {},
        )
        .await
        .unwrap_err();

        assert!(matches!(err, DownloadError::Other(_)), "{err}");
        assert!(!part_of(&dest).exists(), "残件应已删除");
        assert_eq!(std::fs::read(&dest).unwrap(), b"old");
    }

    /// 服务器发完一块就停滞：取消要在下一块到来之前生效，并删掉残件
    #[tokio::test]
    async fn cancel_while_stalled_removes_part_file() {
        let server = serve_once(|stream, _, stop| {
            let _ = stream.write_all(
                b"HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nContent-Length: 10000000\r\nConnection: close\r\n\r\n",
            );
            let _ = stream.write_all(&payload(65_536));
            let _ = stream.flush();
            stop.wait(Duration::from_secs(10));
        });
        let dir = TempDir::new("dl_cancel");
        let dest = dir.join("big.bin");
        let part = part_of(&dest);
        let cancel = Arc::new(AtomicBool::new(false));
        let part_existed = Arc::new(AtomicBool::new(false));
        let canceller = {
            let (flag, seen, part) = (cancel.clone(), part_existed.clone(), part.clone());
            std::thread::spawn(move || {
                // 等首块数据写进 .part、服务器进入停滞后再取消
                let deadline = Instant::now() + Duration::from_secs(3);
                while !part.exists() && Instant::now() < deadline {
                    std::thread::sleep(Duration::from_millis(10));
                }
                seen.store(part.exists(), Ordering::SeqCst);
                std::thread::sleep(Duration::from_millis(300));
                flag.store(true, Ordering::SeqCst);
            })
        };
        let start = Instant::now();

        let err = download_to_file(client().get(&server.url), &dest, "big.bin", &cancel, |_| {})
            .await
            .unwrap_err();
        canceller.join().unwrap();

        assert!(matches!(err, DownloadError::Cancelled), "{err}");
        assert!(part_existed.load(Ordering::SeqCst), "取消前 .part 应已创建");
        assert!(
            start.elapsed() < Duration::from_secs(5),
            "停滞时取消也应很快生效"
        );
        assert!(!part.exists() && !dest.exists());
    }

    #[tokio::test]
    async fn cancelled_before_start_does_not_connect() {
        let dir = TempDir::new("dl_precancel");
        let dest = dir.join("x.bin");
        let cancel = AtomicBool::new(true);
        let err = download_to_file(
            client().get("http://127.0.0.1:9/never"),
            &dest,
            "x.bin",
            &cancel,
            |_| panic!("取消后不该再有进度"),
        )
        .await
        .unwrap_err();
        assert!(matches!(err, DownloadError::Cancelled));
        assert_eq!(String::from(err), "下载已取消");
        assert!(!dest.exists() && !part_of(&dest).exists());
    }

    #[test]
    fn progress_message_formats() {
        let mib = 1_048_576;
        assert_eq!(
            progress_message("model.onnx", 5 * mib, 10 * mib, 2.5),
            "model.onnx — 5.0/10.0 MB (2.5 MB/s)"
        );
        assert_eq!(
            progress_message("Python", 3 * mib, 0, 1.3),
            "Python — 3.0 MB (1.3 MB/s)"
        );
        let tick = progress_tick("a", mib, 4 * mib, Duration::from_secs(2));
        assert_eq!(tick.percent, 25.0);
        assert_eq!(tick.speed_mbps, 0.5);
        assert_eq!(tick.message, "a — 1.0/4.0 MB (0.5 MB/s)");
    }

    #[test]
    fn progress_payload_skips_empty_filename() {
        assert_eq!(
            DownloadProgress::starting("model.onnx"),
            DownloadProgress::new("downloading", 0.0, "正在下载 model.onnx")
        );
        assert_eq!(
            serde_json::to_value(DownloadProgress::done("ok")).unwrap(),
            serde_json::json!({"percent": 100.0, "speed_mbps": 0.0, "status": "done", "message": "ok"})
        );
        let v = serde_json::to_value(DownloadProgress::error("x").with_filename("a.onnx")).unwrap();
        assert_eq!(v["filename"], "a.onnx");
        assert_eq!(v["status"], "error");
        assert_eq!(DownloadProgress::cancelled("c").percent, 0.0);
    }

    #[test]
    fn huggingface_url_points_at_resolve_main() {
        assert_eq!(
            huggingface_url("deepghs/anime_aesthetic", "swinv2pv3_v0_448_ls0.2_x/model.onnx"),
            "https://huggingface.co/deepghs/anime_aesthetic/resolve/main/swinv2pv3_v0_448_ls0.2_x/model.onnx"
        );
    }
}
