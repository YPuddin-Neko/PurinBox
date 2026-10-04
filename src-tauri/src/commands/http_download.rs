//! HTTP 客户端与流式下载：打标模型、美学/裁切模型、超分引擎与权重、独立版 Python 共用。
//!
//! 一次下载的固定流程在 `download_to_file` 里：写 `{dest}.part` → 分块写入、随时检查取消 →
//! 每 500ms 上报一次进度 → flush → 校验字节数后原子替换为 `dest`；任何失败（含取消）都删除
//! .part 残件，已有的 `dest` 只在下载完整成功后才被替换。多个文件依次下载用 `download_files`。
//! 事件名、开始/完成文案、取消后的善后由各模块自己决定；下载失败的终态事件用
//! `DownloadProgress::from_error` 生成。
//!
//! 客户端：大文件下载用 `download_client`（只设停滞超时），API 等小请求用 `api_client`（有总时长）。

use futures_util::StreamExt;
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use tokio::io::AsyncWriteExt;

/// 建立连接的超时
const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);
/// 停滞超时：连续这么久读不到任何数据才算失败。不设总时长上限——
/// 慢速网络下几百 MB 的模型要下十几分钟，总时长超时会在中途把它掐断
const STALL_TIMEOUT: Duration = Duration::from_secs(120);
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

    /// 下载失败对应的终态事件：被取消（`err` 是 `Cancelled`，或 `cancel` 已置位）时为 cancelled、
    /// 文案「已取消下载」，否则为 error、文案是 `err` 的 Display。
    /// 也看 `cancel`：取消与网络错误同时发生时，用户点过取消就不再报失败
    pub(crate) fn from_error(err: &DownloadError, cancel: &AtomicBool) -> Self {
        if matches!(err, DownloadError::Cancelled) || cancel.load(Ordering::SeqCst) {
            Self::cancelled(DownloadError::Cancelled.to_string())
        } else {
            Self::error(err.to_string())
        }
    }

    /// 换掉文案，例如把 HTTP 401/403 换成具体的处理办法
    pub(crate) fn with_message(mut self, message: impl Into<String>) -> Self {
        self.message = message.into();
        self
    }
}

/// `download_to_file` 的失败原因。Display 即给用户看的文案
#[derive(Debug)]
pub enum DownloadError {
    /// 取消标志置位（.part 已删除）。文案以「已取消」开头：前端据此把返回的 Err 认作用户取消
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
            DownloadError::Cancelled => f.write_str("已取消下载"),
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

/// API 等小请求用的 HTTP 客户端（GitHub API 与更新检查、翻译服务商、标签库的小请求）：
/// 应用内代理（与 `download_client` 相同，socks5 由代理解析域名），连接超时 30 秒，
/// 整个请求（连接、等响应、读完响应体）最长 `timeout`。服务器接受连接后不回包时请求不会一直挂着。
/// 大文件下载用 `download_client`：总时长限制会把慢速网络下的长下载掐断
pub(crate) fn api_client(timeout: Duration) -> Result<reqwest::Client, String> {
    with_api_timeouts(super::proxy_config::build_http_client(), timeout)
}

fn with_api_timeouts(
    builder: reqwest::ClientBuilder,
    timeout: Duration,
) -> Result<reqwest::Client, String> {
    builder
        .connect_timeout(CONNECT_TIMEOUT)
        .timeout(timeout)
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

/// `download_files` 下载的一个文件
pub(crate) struct DownloadFile {
    /// 调用方拼好的请求，一般是 `client.get(url)`；需要鉴权时自己加头
    pub request: reqwest::RequestBuilder,
    pub dest: PathBuf,
    /// 进度文案里的名字
    pub label: String,
}

impl DownloadFile {
    pub(crate) fn new(
        request: reqwest::RequestBuilder,
        dest: impl Into<PathBuf>,
        label: impl Into<String>,
    ) -> Self {
        DownloadFile {
            request,
            dest: dest.into(),
            label: label.into(),
        }
    }
}

/// `download_files` 的选项，默认全关
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub(crate) struct DownloadFilesOptions {
    /// 目标已是文件就跳过，只补缺的；跳过时发一条「{label} 已存在，跳过」的进度
    pub skip_existing: bool,
    /// 每个文件下完发一条 done「{label} — 下载完成 ✓」：前端每收到 done 记一行日志并收起进度条
    pub done_each: bool,
    /// 把进度换算到整体区间 `(起点, 终点)`：n 个文件均分，第 i 个占
    /// `[起点 + i·份, 起点 + (i+1)·份]`。None 时每个文件各自 0–100
    pub percent_range: Option<(f32, f32)>,
}

/// 依次下载多个文件（模型权重、词表、配置），返回实际下载的个数（不含跳过的）。
///
/// 进度经 `emit` 发出：每个文件开始时发 `DownloadProgress::starting(label)`，之后转发
/// `download_to_file` 的进度；跳过和逐个完成见 `DownloadFilesOptions`。
/// 不发整体完成和失败事件：完成文案各模块不同；失败时调用方用
/// `DownloadProgress::from_error(&err, cancel)` 发终态。
///
/// 每个文件开始前和全部下完后都检查 `cancel`，置位即返回 `Cancelled`。已下完的文件保留，
/// 配合 `skip_existing` 重试时只补缺的；任一文件失败即停止，后面的不再下载。
pub(crate) async fn download_files(
    files: Vec<DownloadFile>,
    options: DownloadFilesOptions,
    cancel: &AtomicBool,
    mut emit: impl FnMut(DownloadProgress),
) -> Result<usize, DownloadError> {
    let count = files.len();
    let mut downloaded = 0;
    for (index, file) in files.into_iter().enumerate() {
        if cancel.load(Ordering::SeqCst) {
            return Err(DownloadError::Cancelled);
        }
        let scale = |percent: f32| match options.percent_range {
            Some((start, end)) => {
                start + (index as f32 + percent / 100.0) * (end - start) / count as f32
            }
            None => percent,
        };
        if options.skip_existing && file.dest.is_file() {
            emit(DownloadProgress::new(
                "downloading",
                scale(100.0),
                format!("{} 已存在，跳过", file.label),
            ));
            continue;
        }
        let mut starting = DownloadProgress::starting(&file.label);
        starting.percent = scale(0.0);
        emit(starting);
        download_to_file(
            file.request,
            &file.dest,
            &file.label,
            cancel,
            |mut progress| {
                progress.percent = scale(progress.percent);
                emit(progress);
            },
        )
        .await?;
        downloaded += 1;
        if options.done_each {
            emit(DownloadProgress::done(format!(
                "{} — 下载完成 ✓",
                file.label
            )));
        }
    }
    if cancel.load(Ordering::SeqCst) {
        return Err(DownloadError::Cancelled);
    }
    Ok(downloaded)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::test_support::{serve_once, write_response, TempDir, TestServer};
    use std::io::Write;
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
        assert_eq!(String::from(err), "已取消下载");
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

    #[test]
    fn error_events_follow_the_cancel_flag_and_keep_messages() {
        let idle = AtomicBool::new(false);
        let cancelled = AtomicBool::new(true);
        let status = DownloadError::Status {
            status: reqwest::StatusCode::NOT_FOUND,
            url: "http://127.0.0.1/a".into(),
        };
        assert_eq!(
            DownloadProgress::from_error(&status, &idle),
            DownloadProgress::error("HTTP 404 Not Found: http://127.0.0.1/a")
        );
        assert_eq!(
            DownloadProgress::from_error(&DownloadError::Cancelled, &idle),
            DownloadProgress::cancelled("已取消下载")
        );
        // 用户点过取消时，同时发生的网络错误也按取消处理
        assert_eq!(
            DownloadProgress::from_error(
                &DownloadError::Other("下载数据失败: reset".into()),
                &cancelled
            ),
            DownloadProgress::cancelled("已取消下载")
        );
        let replaced =
            DownloadProgress::from_error(&status, &idle).with_message("请先保存 Access Token");
        assert_eq!(replaced.status, "error");
        assert_eq!(replaced.message, "请先保存 Access Token");
    }

    #[tokio::test]
    async fn api_client_gives_up_on_a_stalled_server() {
        let server = serve_once(|stream, _, stop| {
            let _ = stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\n\r\npartial");
            let _ = stream.flush();
            stop.wait(Duration::from_secs(10));
        });
        let client = with_api_timeouts(
            reqwest::Client::builder().no_proxy(),
            Duration::from_millis(300),
        )
        .unwrap();
        let start = Instant::now();
        let err = match client.get(&server.url).send().await {
            Ok(response) => response.text().await.unwrap_err(),
            Err(e) => e,
        };
        assert!(err.is_timeout(), "{err}");
        assert!(start.elapsed() < Duration::from_secs(5));
    }

    #[tokio::test]
    async fn api_client_returns_normal_responses() {
        let server = serve_once(|stream, _, _| {
            write_response(
                stream,
                "200 OK",
                "application/json",
                b"{\"tag_name\":\"v1.2.3\"}",
            )
        });
        let client = with_api_timeouts(
            reqwest::Client::builder().no_proxy(),
            Duration::from_secs(5),
        )
        .unwrap();
        let json: serde_json::Value = client
            .get(&server.url)
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(json["tag_name"], "v1.2.3");
    }

    fn serve_bytes(body: &'static [u8]) -> TestServer {
        serve_once(move |stream, _, _| {
            write_response(stream, "200 OK", "application/octet-stream", body)
        })
    }

    #[tokio::test]
    async fn files_download_in_order_skip_existing_and_map_progress() {
        let first = serve_bytes(b"first");
        let second = serve_bytes(b"second");
        let dir = TempDir::new("dl_files");
        std::fs::write(dir.join("a.bin"), b"kept").unwrap();
        let cancel = AtomicBool::new(false);
        let mut events = Vec::new();
        let files = vec![
            DownloadFile::new(client().get(&first.url), dir.join("a.bin"), "a.bin"),
            DownloadFile::new(
                client().get(format!("{}/b", second.url)),
                dir.join("b.bin"),
                "b.bin",
            ),
        ];
        let options = DownloadFilesOptions {
            skip_existing: true,
            percent_range: Some((30.0, 98.0)),
            ..Default::default()
        };
        let downloaded = download_files(files, options, &cancel, |p| events.push(p))
            .await
            .unwrap();

        assert_eq!(downloaded, 1);
        assert_eq!(std::fs::read(dir.join("a.bin")).unwrap(), b"kept");
        assert_eq!(std::fs::read(dir.join("b.bin")).unwrap(), b"second");
        assert_eq!(second.requests.recv().unwrap().path, "/b");
        assert!(first.requests.try_recv().is_err(), "已存在的文件不该再请求");
        // 两个文件均分 30–98：第一个占 30–64，第二个占 64–98
        assert_eq!(
            events[0],
            DownloadProgress::new("downloading", 64.0, "a.bin 已存在，跳过")
        );
        assert_eq!(
            events[1],
            DownloadProgress::new("downloading", 64.0, "正在下载 b.bin")
        );
        let last = events.last().unwrap();
        assert_eq!(last.percent, 98.0);
        assert!(last.message.starts_with("b.bin — "), "{}", last.message);
        assert!(events.iter().all(|p| p.status == "downloading"));
    }

    #[tokio::test]
    async fn done_each_reports_every_file_with_its_own_progress() {
        let a = serve_bytes(b"aa");
        let b = serve_bytes(b"bb");
        let dir = TempDir::new("dl_files_each");
        let cancel = AtomicBool::new(false);
        let mut events = Vec::new();
        let files = vec![
            DownloadFile::new(client().get(&a.url), dir.join("a"), "[1/2] A"),
            DownloadFile::new(client().get(&b.url), dir.join("b"), "[2/2] B"),
        ];
        let options = DownloadFilesOptions {
            done_each: true,
            ..Default::default()
        };
        assert_eq!(
            download_files(files, options, &cancel, |p| events.push(p))
                .await
                .unwrap(),
            2
        );
        let done: Vec<&str> = events
            .iter()
            .filter(|p| p.status == "done")
            .map(|p| p.message.as_str())
            .collect();
        assert_eq!(done, ["[1/2] A — 下载完成 ✓", "[2/2] B — 下载完成 ✓"]);
        let starts: Vec<f32> = events
            .iter()
            .filter(|p| p.message.starts_with("正在下载"))
            .map(|p| p.percent)
            .collect();
        assert_eq!(starts, [0.0, 0.0]);
    }

    #[tokio::test]
    async fn failure_stops_the_sequence_and_keeps_finished_files() {
        let a = serve_bytes(b"aa");
        let missing = serve_once(|stream, _, _| {
            write_response(stream, "404 Not Found", "text/plain", b"nope")
        });
        let never = serve_bytes(b"cc");
        let dir = TempDir::new("dl_files_fail");
        let cancel = AtomicBool::new(false);
        let mut events = Vec::new();
        let files = vec![
            DownloadFile::new(client().get(&a.url), dir.join("a"), "a"),
            DownloadFile::new(client().get(&missing.url), dir.join("b"), "b"),
            DownloadFile::new(client().get(&never.url), dir.join("c"), "c"),
        ];
        let err = download_files(files, DownloadFilesOptions::default(), &cancel, |p| {
            events.push(p)
        })
        .await
        .unwrap_err();

        assert!(matches!(err, DownloadError::Status { .. }), "{err}");
        assert_eq!(std::fs::read(dir.join("a")).unwrap(), b"aa");
        assert!(!dir.join("b").exists() && !dir.join("c").exists());
        assert!(
            never.requests.try_recv().is_err(),
            "失败后不再下载后面的文件"
        );
        assert!(
            events.iter().all(|p| p.status == "downloading"),
            "失败的终态事件由调用方发"
        );
        assert_eq!(DownloadProgress::from_error(&err, &cancel).status, "error");
    }

    #[tokio::test]
    async fn files_cancelled_before_start_or_after_the_last_one() {
        let dir = TempDir::new("dl_files_cancel");
        let cancelled = AtomicBool::new(true);
        let err = download_files(
            vec![DownloadFile::new(
                client().get("http://127.0.0.1:9/never"),
                dir.join("x"),
                "x",
            )],
            DownloadFilesOptions::default(),
            &cancelled,
            |_| panic!("取消后不该再有进度"),
        )
        .await
        .unwrap_err();
        assert!(matches!(err, DownloadError::Cancelled));
        assert!(!dir.join("x").exists());

        // 最后一个文件下完才取消：文件保留，整体仍按取消返回
        let server = serve_bytes(b"done");
        let cancel = AtomicBool::new(false);
        let options = DownloadFilesOptions {
            done_each: true,
            ..Default::default()
        };
        let err = download_files(
            vec![DownloadFile::new(
                client().get(&server.url),
                dir.join("y"),
                "y",
            )],
            options,
            &cancel,
            |p| {
                if p.status == "done" {
                    cancel.store(true, Ordering::SeqCst);
                }
            },
        )
        .await
        .unwrap_err();
        assert!(matches!(err, DownloadError::Cancelled));
        assert_eq!(std::fs::read(dir.join("y")).unwrap(), b"done");
    }
}
