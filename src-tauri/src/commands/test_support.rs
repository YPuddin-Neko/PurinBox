//! 测试辅助：一次性本地 HTTP 服务器和临时目录。
//! 两者都由测试持有，drop 时自行收尾，测试结束（含断言失败）不留后台线程、连接和目录

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
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

/// 系统临时目录下的独占目录，drop 时连同内容删除。
/// 目录名带进程号和进程内序号：并行的测试即使用了相同的 tag 也不会互删目录
pub(crate) struct TempDir(PathBuf);

impl TempDir {
    pub(crate) fn new(tag: &str) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "purinbox_{}_{}_{}",
            tag,
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
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

/// 跑真实 Python 脚本的测试用的解释器：仓库里部署过的 venv，没有就用系统的 python3
pub(crate) fn test_python() -> String {
    let venv = Path::new(env!("CARGO_MANIFEST_DIR")).join("../env/python/venv/bin/python3");
    if venv.exists() {
        venv.to_string_lossy().into_owned()
    } else {
        "python3".into()
    }
}

/// 可执行的 sh 脚本，冒充 NCNN 引擎或 Python 解释器
#[cfg(unix)]
pub(crate) fn fake_program(dir: &Path, name: &str, body: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let path = dir.join(name);
    std::fs::write(&path, format!("#!/bin/sh\n{}", body)).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
    path
}

/// 进程是否还在（`kill -0`）；已回收或不存在的 PID 返回 false。
/// 经 `spawn_exclusive` 启动，不会顺带继承别的测试正在建的管道
#[cfg(unix)]
pub(crate) fn process_alive(pid: u32) -> bool {
    let mut cmd = std::process::Command::new("kill");
    cmd.args(["-0", &pid.to_string()])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    super::python_proc::spawn_exclusive(&mut cmd)
        .and_then(|mut child| child.wait())
        .map(|status| status.success())
        .unwrap_or(false)
}

/// 等 `pid` 消失，最多 5 秒；返回是否已消失。
/// 被杀的孙进程由 init 回收，回收前 `kill -0` 仍会成功
#[cfg(unix)]
pub(crate) fn wait_process_gone(pid: u32) -> bool {
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while process_alive(pid) {
        if std::time::Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn temp_dirs_with_the_same_tag_are_distinct_and_removed_on_drop() {
        let first = TempDir::new("support_same_tag");
        let second = TempDir::new("support_same_tag");
        assert_ne!(&*first, &*second);
        std::fs::write(first.join("a.txt"), "a").unwrap();
        let path = first.to_path_buf();
        drop(first);
        assert!(!path.exists());
        assert!(second.is_dir());
    }

    #[test]
    fn server_records_request_and_replies() {
        let server = serve_once(|stream, _, _| {
            write_response(stream, "200 OK", "application/json", b"{\"ok\":true}")
        });
        let mut stream = TcpStream::connect(server.url.trim_start_matches("http://")).unwrap();
        stream
            .write_all(b"POST /path?q=1 HTTP/1.1\r\nHost: x\r\nX-Test: 1\r\nContent-Length: 7\r\n\r\n{\"a\":1}")
            .unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        assert!(response.starts_with("HTTP/1.1 200 OK"), "{response}");
        assert!(response.ends_with("{\"ok\":true}"), "{response}");

        let request = server.requests.recv().unwrap();
        assert_eq!(request.method, "POST");
        assert_eq!(request.path, "/path?q=1");
        assert_eq!(request.header("x-test"), Some("1"));
        assert_eq!(request.json(), serde_json::json!({"a": 1}));
    }
}
