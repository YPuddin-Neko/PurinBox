//! 常驻 Python 会话类功能（美学评分、人物裁切）的任务控制：取消标志与进程登记、
//! 环境准备、会话的启动与收尾、命令的终态事件

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use tauri::{AppHandle, Runtime};

use super::batch::{self, BatchCounts};
use super::python_proc::{self, PythonCommand, PythonSession, SessionError, StderrNoise};
use super::{ProcessResult, ProgressEvent};

/// 常驻脚本的退出命令
const QUIT_COMMAND: &str = r#"{"cmd":"quit"}"#;

/// 常驻 Python 会话的取消标志与进程登记。
///
/// - 普通取消：环境部署和模型下载随之停止；进程还在加载模型时直接结束它（这时还没写任何文件）；
///   开始处理文件后不再发新的图片，等手头的图片写完再发退出命令让进程自己退出，
///   输出目录里不会留下写到一半的临时文件；
/// - 强制取消：立即按进程树结束进程。
pub(crate) struct SessionControls {
    /// 环境部署（`python_env`）的 owner
    owner: &'static str,
    cancelled: AtomicBool,
    /// 交给 `PythonSession` 的取消标志：置位即结束进程
    forced: AtomicBool,
    /// 已开始处理文件
    working: AtomicBool,
    process: Mutex<Option<u32>>,
}

impl SessionControls {
    pub(crate) const fn new(owner: &'static str) -> Self {
        SessionControls {
            owner,
            cancelled: AtomicBool::new(false),
            forced: AtomicBool::new(false),
            working: AtomicBool::new(false),
            process: Mutex::new(None),
        }
    }

    /// 新一轮任务开始时调用（已拿到功能的互斥锁）
    pub(crate) fn reset(&self) {
        self.cancelled.store(false, Ordering::SeqCst);
        self.forced.store(false, Ordering::SeqCst);
        self.working.store(false, Ordering::SeqCst);
        super::python_env::clear_pending_cancel(self.owner);
    }

    /// 普通取消的标志（下载用）
    pub(crate) fn cancel_flag(&self) -> &AtomicBool {
        &self.cancelled
    }

    pub(crate) fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }

    pub(crate) fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
        super::python_env::cancel_setup_for(self.owner);
        if !self.working.load(Ordering::SeqCst) {
            python_proc::kill_registered_pid(&self.process);
        }
    }

    pub(crate) fn force_cancel(&self) {
        self.forced.store(true, Ordering::SeqCst);
        self.cancel();
        python_proc::kill_registered_pid(&self.process);
    }

    /// 部署 Python 环境，按需换装 onnxruntime GPU 版，返回解释器路径。每一步之后检查取消
    pub(crate) async fn prepare_python<R: Runtime>(
        &self,
        app: &AppHandle<R>,
    ) -> Result<String, String> {
        let python = super::python_env::setup_python_env(app, self.owner).await?;
        if self.is_cancelled() {
            return Err("已取消".into());
        }
        super::python_env::ensure_onnx_gpu_runtime(app, &python, self.owner).await?;
        if self.is_cancelled() {
            return Err("已取消".into());
        }
        Ok(python)
    }

    /// 启动会话、发 `init`、等脚本加载完模型回 ready，加载期间的日志按进度 0/`total` 转发。
    /// 返回后的普通取消等手头的图片写完。启动或加载期间取消了返回 Ok(None)
    pub(crate) fn open_session<R: Runtime>(
        &self,
        app: &AppHandle<R>,
        event: &'static str,
        cmd: PythonCommand,
        init: &serde_json::Value,
        ready_timeout: Duration,
        total: u32,
    ) -> Result<Option<PythonSession<'_>>, String> {
        let on_stderr = python_proc::stderr_warnings(app, event, StderrNoise::Runtime, None);
        let mut session = match PythonSession::start(cmd, &self.process, &self.forced, on_stderr) {
            // 取消命令赶在 PID 登记之前到达时没能结束进程，drop 会话补上
            Ok(_) if self.is_cancelled() => return Ok(None),
            Ok(session) => session,
            Err(SessionError::Cancelled) => return Ok(None),
            Err(e) => return Err(e.into()),
        };
        let ready = session.send(init).and_then(|()| {
            session.wait_ready(ready_timeout, |msg| {
                ProgressEvent::python_log(msg, 0, total).emit(app, event)
            })
        });
        match ready {
            Ok(()) => {}
            Err(_) if self.is_cancelled() => return Ok(None),
            Err(SessionError::Script(message)) => return Err(message),
            Err(SessionError::TimedOut(limit)) => {
                return Err(format!("模型加载超时（{} 秒）", limit.as_secs()))
            }
            Err(e) => return Err(format!("模型加载失败: {}", e)),
        }
        self.working.store(true, Ordering::SeqCst);
        Ok(Some(session))
    }

    #[cfg(all(test, unix))]
    pub(crate) fn has_registered_process(&self) -> bool {
        self.process.lock().unwrap().is_some()
    }

    /// 处理循环结束后收尾会话。`stop` 是循环提前结束的原因：
    /// - 没有，或用户取消了：发退出命令，进程处理完手头的命令后自己退出；
    /// - 进程崩溃、无响应、脚本级错误（不带 image_path 的 error）：没处理完的文件都记失败，
    ///   发一条带原因的 error 事件，返回 Err，终态按出错处理（与超分、聚类一致）
    pub(crate) fn finish_session<R: Runtime>(
        &self,
        mut session: PythonSession<'_>,
        stop: Option<SessionError>,
        app: &AppHandle<R>,
        event: &str,
        label: &str,
        result: &ProcessResult,
    ) -> Result<(), String> {
        let reason = match stop {
            Some(reason) if reason != SessionError::Cancelled && !self.is_cancelled() => reason,
            _ => {
                session.shutdown(Some(QUIT_COMMAND));
                return Ok(());
            }
        };
        if matches!(reason, SessionError::TimedOut(_)) {
            session.kill();
        }
        session.shutdown(Some(QUIT_COMMAND));
        let remaining = result
            .total
            .saturating_sub(result.success_count + result.fail_count);
        let error = format!("{}中断: {}", label, reason);
        ProgressEvent::new(
            "error",
            format!("{}；未处理的 {} 张记为失败", error, remaining),
        )
        .at(result.total, result.total)
        .emit(app, event);
        Err(error)
    }

    /// 命令收尾（见 [`finish_command`]），是否取消看本轮的取消标志
    pub(crate) fn finish_command<R: Runtime>(
        &self,
        app: &AppHandle<R>,
        event: &str,
        total: u32,
        outcome: Result<ProcessResult, String>,
        summary: &str,
    ) -> Result<ProcessResult, String> {
        finish_command(app, event, total, outcome, self.is_cancelled(), summary)
    }
}

/// 命令收尾：用户取消了时，中途因取消失败的步骤不算出错，结果只计已处理的；
/// 没取消时原样返回错误。之后发终态事件
pub(crate) fn finish_command<R: Runtime>(
    app: &AppHandle<R>,
    event: &str,
    total: u32,
    outcome: Result<ProcessResult, String>,
    cancelled: bool,
    summary: &str,
) -> Result<ProcessResult, String> {
    let result = if cancelled {
        outcome.unwrap_or(ProcessResult {
            total,
            ..Default::default()
        })
    } else {
        outcome?
    };
    batch::finish_run(app, event, &BatchCounts::from(&result), cancelled, |c| {
        c.summary(summary)
    });
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::batch::capture_events;

    fn done_events(events: &std::sync::Mutex<Vec<serde_json::Value>>) -> Vec<serde_json::Value> {
        let events = events.lock().unwrap();
        events
            .iter()
            .filter(|e| e["status"] == "done")
            .cloned()
            .collect()
    }

    #[test]
    fn finish_command_turns_cancelled_failures_into_partial_results() {
        const EVENT: &str = "python-task-finish-test";
        let app = tauri::test::mock_app();
        let events = capture_events(app.handle(), EVENT);
        let result =
            finish_command(app.handle(), EVENT, 3, Err("已取消".into()), true, "完成").unwrap();
        assert_eq!((result.total, result.success_count), (3, 0));
        let done = done_events(&events);
        assert_eq!(done.len(), 1);
        assert_eq!(done[0]["cancelled"], true);

        let error = finish_command(app.handle(), EVENT, 3, Err("boom".into()), false, "完成");
        assert_eq!(error.unwrap_err(), "boom");
        assert_eq!(
            done_events(&events).len(),
            1,
            "出错时不发 done：命令返回的 Err 就是终态"
        );

        let ok = ProcessResult {
            total: 2,
            success_count: 2,
            ..Default::default()
        };
        finish_command(app.handle(), EVENT, 2, Ok(ok), false, "完成").unwrap();
        let done = done_events(&events);
        assert_eq!(done[1]["message"], "完成: 成功 2, 失败 0, 共 2");
        assert_ne!(done[1]["cancelled"], true);
    }
}

/// 美学评分、人物裁切的会话测试共用：模拟的 app、收集的进度事件、放伪 Python 的临时目录
#[cfg(all(test, unix))]
pub(crate) mod session_test {
    use std::path::Path;
    use std::sync::{Arc, Mutex};

    use tauri::Listener;

    use super::SessionControls;
    use crate::commands::batch::capture_events;
    use crate::commands::test_support::{fake_program, TempDir};

    pub(crate) struct SessionRun {
        pub(crate) root: TempDir,
        pub(crate) controls: Arc<SessionControls>,
        pub(crate) app: tauri::App<tauri::test::MockRuntime>,
        pub(crate) events: Arc<Mutex<Vec<serde_json::Value>>>,
        event: &'static str,
    }

    impl SessionRun {
        /// `owner` 是测试自己的环境部署 owner，不和正式功能共用取消状态
        pub(crate) fn new(tag: &str, event: &'static str, owner: &'static str) -> Self {
            let app = tauri::test::mock_app();
            let events = capture_events(app.handle(), event);
            SessionRun {
                root: TempDir::new(tag),
                controls: Arc::new(SessionControls::new(owner)),
                app,
                events,
                event,
            }
        }

        /// 收到 message 含 `needle` 的事件时调用 `action`
        pub(crate) fn on_event(&self, needle: &'static str, action: fn(&SessionControls)) {
            let controls = self.controls.clone();
            self.app.listen_any(self.event, move |event| {
                if event.payload().contains(needle) {
                    action(&controls);
                }
            });
        }

        /// 在临时目录写出冒充 Python 解释器的 sh 脚本，返回它的路径
        pub(crate) fn fake_python(&self, body: &str) -> String {
            fake_program(&self.root, "fake-python", body)
                .to_string_lossy()
                .into_owned()
        }

        /// 状态为 `status` 的事件的 message，按发出顺序
        pub(crate) fn messages(&self, status: &str) -> Vec<String> {
            self.events
                .lock()
                .unwrap()
                .iter()
                .filter(|e| e["status"] == status)
                .map(|e| e["message"].as_str().unwrap().to_string())
                .collect()
        }
    }

    /// 读到退出命令时留下标记文件：证明进程是收到 quit 后自己退出的
    pub(crate) fn quit_marker(marker: &Path) -> String {
        format!(
            "read -r line\ncase \"$line\" in *'\"cmd\":\"quit\"'*) touch '{}';; esac\n",
            marker.display()
        )
    }
}
