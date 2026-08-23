use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use tauri::Emitter;

use super::{ProcessResult, ProgressEvent};

/// 子进程 PID
static CHILD_PID: Mutex<Option<u32>> = Mutex::new(None);
static CANCEL_FLAG: AtomicBool = AtomicBool::new(false);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClusterOptions {
    pub input_path: String,
    pub output_path: String,
    pub algorithm: String,     // "kmeans" | "hdbscan"
    pub feature_type: String,  // "style" | "semantic" | "fusion"
    pub n_clusters: u32,       // K-Means 分组数
    pub min_cluster_size: u32, // HDBSCAN 最小簇大小
    pub device: String,        // "auto" | "cpu"
    pub weight_style: f64,     // 融合模式权重
    pub weight_semantic: f64,
    pub weight_color: f64,
    pub map_theme: String, // "light" | "dark"
    #[serde(default)]
    pub recursive: bool,
}

/// 获取聚类脚本路径
fn get_cluster_script() -> Result<PathBuf, String> {
    super::python_proc::find_script("image_cluster.py")
}

/// 获取聚类模型缓存目录 (models/cluster_models/)
fn get_cluster_model_dir() -> PathBuf {
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|p| p.to_path_buf()))
        .unwrap_or_else(|| PathBuf::from("."));

    let base = if cfg!(debug_assertions) {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or(exe_dir)
    } else {
        exe_dir
    };
    base.join("models").join("cluster_models")
}

/// 确保聚类所需 Python 依赖
async fn ensure_cluster_deps(app: &tauri::AppHandle) -> Result<(), String> {
    let emit_log = |msg: &str| {
        let _ = app.emit(
            "cluster-progress",
            ProgressEvent {
                current: 0,
                total: 0,
                filename: String::new(),
                status: "info".to_string(),
                message: msg.to_string(),
                ..Default::default()
            },
        );
    };

    emit_log("正在检查 Python 环境...");
    let python = super::python_env::setup_python_env(app, "cluster").await?;

    // 检查 torch（聚类的 ResNet50 特征提取依赖它）
    let has_torch = {
        let p = python.clone();
        tokio::task::spawn_blocking(move || {
            let mut cmd = std::process::Command::new(&p);
            cmd.args(["-c", "import torch, torchvision"]);
            #[cfg(target_os = "windows")]
            {
                use std::os::windows::process::CommandExt;
                cmd.creation_flags(0x08000000);
            }
            cmd.output().map(|o| o.status.success()).unwrap_or(false)
        })
        .await
        .unwrap_or(false)
    };

    if !has_torch {
        emit_log("正在安装 PyTorch（首次安装体积较大，请耐心等待）...");
        let p = python.clone();
        let app2 = app.clone();
        // 只装 PyPI 默认发行版，不指定 CUDA wheel index：
        // 使用本机既有的 CUDA 环境，不额外下载 CUDA 运行时
        tokio::task::spawn_blocking(move || {
            super::python_env::pip_install_with_python(&app2, &p, &["torch", "torchvision"])
        })
        .await
        .map_err(|e| format!("安装线程异常: {}", e))??;
        emit_log("PyTorch 安装完成");
    }

    // 探测 torch 能用的加速后端（只读，不安装）
    let _has_gpu = super::python_env::ensure_torch_gpu_runtime(app, &python, "cluster").await?;

    // 检查 sklearn
    let has_sklearn = {
        let p = python.clone();
        tokio::task::spawn_blocking(move || {
            let mut cmd = std::process::Command::new(&p);
            cmd.args(["-c", "import sklearn; print(sklearn.__version__)"]);
            #[cfg(target_os = "windows")]
            {
                use std::os::windows::process::CommandExt;
                cmd.creation_flags(0x08000000);
            }
            cmd.output().map(|o| o.status.success()).unwrap_or(false)
        })
        .await
        .unwrap_or(false)
    };

    if !has_sklearn {
        emit_log("正在安装 scikit-learn...");
        let p = python.clone();
        let app2 = app.clone();
        tokio::task::spawn_blocking(move || {
            super::python_env::pip_install_with_python(&app2, &p, &["scikit-learn"])
        })
        .await
        .map_err(|e| format!("安装线程异常: {}", e))??;
        emit_log("scikit-learn 安装完成");
    }

    // 检查 umap-learn（HDBSCAN 降维需要）
    let has_umap = {
        let p = python.clone();
        tokio::task::spawn_blocking(move || {
            let mut cmd = std::process::Command::new(&p);
            cmd.args(["-c", "import umap; print(umap.__version__)"]);
            #[cfg(target_os = "windows")]
            {
                use std::os::windows::process::CommandExt;
                cmd.creation_flags(0x08000000);
            }
            cmd.output().map(|o| o.status.success()).unwrap_or(false)
        })
        .await
        .unwrap_or(false)
    };

    if !has_umap {
        emit_log("正在安装 umap-learn（HDBSCAN 降维依赖）...");
        let p = python.clone();
        let app2 = app.clone();
        tokio::task::spawn_blocking(move || {
            super::python_env::pip_install_with_python(&app2, &p, &["umap-learn"])
        })
        .await
        .map_err(|e| format!("安装线程异常: {}", e))??;
        emit_log("umap-learn 安装完成");
    }

    // 检查 pillow (PIL)
    let has_pillow = {
        let p = python.clone();
        tokio::task::spawn_blocking(move || {
            let mut cmd = std::process::Command::new(&p);
            cmd.args(["-c", "from PIL import Image"]);
            #[cfg(target_os = "windows")]
            {
                use std::os::windows::process::CommandExt;
                cmd.creation_flags(0x08000000);
            }
            cmd.output().map(|o| o.status.success()).unwrap_or(false)
        })
        .await
        .unwrap_or(false)
    };

    if !has_pillow {
        emit_log("正在安装 Pillow...");
        let p = python.clone();
        let app2 = app.clone();
        tokio::task::spawn_blocking(move || {
            super::python_env::pip_install_with_python(&app2, &p, &["pillow"])
        })
        .await
        .map_err(|e| format!("安装线程异常: {}", e))??;
        emit_log("Pillow 安装完成");
    }

    emit_log("环境检查完成");
    Ok(())
}

#[tauri::command]
pub async fn start_image_cluster(
    app: tauri::AppHandle,
    options: ClusterOptions,
) -> Result<ProcessResult, String> {
    CANCEL_FLAG.store(false, Ordering::SeqCst);

    // 确保依赖
    ensure_cluster_deps(&app).await?;

    let python = super::python_env::get_python_exe().ok_or("Python 环境未就绪")?;
    let script = get_cluster_script()?;
    let model_dir = get_cluster_model_dir();

    let app_clone = app.clone();

    tokio::task::spawn_blocking(move || {
        use std::io::BufRead;

        let mut cmd = std::process::Command::new(&python);
        cmd.arg(script.to_string_lossy().as_ref())
            .arg("--input")
            .arg(&options.input_path)
            .arg("--output")
            .arg(&options.output_path)
            .arg("--algorithm")
            .arg(&options.algorithm)
            .arg("--feature")
            .arg(&options.feature_type)
            .arg("--n-clusters")
            .arg(options.n_clusters.to_string())
            .arg("--min-cluster-size")
            .arg(options.min_cluster_size.to_string())
            .arg("--device")
            .arg(&options.device)
            .arg("--weight-style")
            .arg(format!("{:.2}", options.weight_style))
            .arg("--weight-semantic")
            .arg(format!("{:.2}", options.weight_semantic))
            .arg("--weight-color")
            .arg(format!("{:.2}", options.weight_color))
            .arg("--model-dir")
            .arg(model_dir.to_string_lossy().as_ref())
            .arg("--map-theme")
            .arg(&options.map_theme)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .env("PYTHONUNBUFFERED", "1")
            .env("PYTHONIOENCODING", "utf-8");

        if options.recursive {
            cmd.arg("--recursive");
        }

        #[cfg(target_os = "windows")]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x08000000);
        }

        let mut child = cmd
            .spawn()
            .map_err(|e| format!("启动 Python 失败: {}", e))?;

        if let Ok(mut guard) = CHILD_PID.lock() {
            *guard = Some(child.id());
        }

        let (stdout, stderr) = match (child.stdout.take(), child.stderr.take()) {
            (Some(o), Some(e)) => (o, e),
            _ => {
                // 取管道失败：杀掉并回收子进程，清空 PID 记录
                let _ = child.kill();
                let _ = child.wait();
                if let Ok(mut guard) = CHILD_PID.lock() {
                    *guard = None;
                }
                return Err("无法获取 Python 进程管道".into());
            }
        };

        // stderr 线程
        let app_err = app_clone.clone();
        std::thread::spawn(move || {
            let mut reader = std::io::BufReader::new(stderr);
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
                            let clean = line.trim();
                            if clean.is_empty() {
                                continue;
                            }
                            // 去除 ANSI 转义序列
                            let clean = clean
                                .replace('\x1b', "")
                                .replace("[0m", "")
                                .replace("[1m", "")
                                .replace("[31m", "")
                                .replace("[33m", "");
                            let clean = clean.trim();
                            if clean.is_empty() {
                                continue;
                            }
                            if clean.matches('%').count() > 3 {
                                continue;
                            }
                            if clean.contains("UserWarning") || clean.contains("FutureWarning") {
                                continue;
                            }
                            if clean.contains("RuntimeWarning") {
                                continue;
                            }
                            if clean.starts_with("Downloading:") || clean.starts_with("100%") {
                                continue;
                            }
                            if clean == "warn(" || clean.starts_with("warnings.warn(") {
                                continue;
                            }
                            if clean.contains("site-packages/") && clean.contains(".py:") {
                                continue;
                            }
                            if clean.starts_with("eigenvalues") || clean.starts_with("scipy.") {
                                continue;
                            }
                            // 过滤 cuDNN/CUDA/onnxruntime 加载警告和编码乱码
                            let lower = clean.to_lowercase();
                            if lower.contains("cudnn")
                                || lower.contains("cuda_path")
                                || lower.contains("onnxruntime")
                                || lower.contains("could not load")
                                || lower.contains("loaded library")
                            {
                                continue;
                            }

                            let _ = app_err.emit(
                                "cluster-progress",
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

        // 解析 stdout JSON
        let reader = std::io::BufReader::new(stdout);
        let mut success_count = 0u32;
        let mut fail_count = 0u32;
        let mut total = 0u32;
        let mut errors = Vec::new();
        let mut got_result = false;

        for line in reader.lines().map_while(Result::ok) {
            if CANCEL_FLAG.load(Ordering::SeqCst) {
                let _ = child.kill();
                break;
            }

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
                        let _ = app_clone.emit(
                            "cluster-progress",
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
                        // 出错时先杀掉并回收子进程，再清空 PID 记录，
                        // 避免僵尸进程以及之后"强制取消"对陈旧 PID 误杀无关进程
                        let _ = child.kill();
                        let _ = child.wait();
                        if let Ok(mut guard) = CHILD_PID.lock() {
                            *guard = None;
                        }
                        return Err(format!("聚类错误: {}", text));
                    }
                    "progress" => {
                        let cur = msg.get("current").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
                        let tot = msg.get("total").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
                        let fname = msg.get("filename").and_then(|v| v.as_str()).unwrap_or("");
                        let status = msg
                            .get("status")
                            .and_then(|v| v.as_str())
                            .unwrap_or("processing");
                        let message = msg.get("message").and_then(|v| v.as_str()).unwrap_or("");
                        total = tot;

                        if status == "success" {
                            success_count += 1;
                        } else if status == "error" {
                            fail_count += 1;
                            errors.push(message.to_string());
                        }

                        let _ = app_clone.emit(
                            "cluster-progress",
                            ProgressEvent {
                                current: cur,
                                total: tot,
                                filename: fname.to_string(),
                                status: status.to_string(),
                                message: message.to_string(),
                                ..Default::default()
                            },
                        );
                    }
                    "done" => {
                        let text = msg.get("message").and_then(|v| v.as_str()).unwrap_or("");
                        let _ = app_clone.emit(
                            "cluster-progress",
                            ProgressEvent {
                                current: total,
                                total,
                                filename: String::new(),
                                status: "done".to_string(),
                                message: text.to_string(),
                                ..Default::default()
                            },
                        );
                    }
                    "result" => {
                        got_result = true;
                        success_count = msg
                            .get("success_count")
                            .and_then(|v| v.as_u64())
                            .unwrap_or(0) as u32;
                        fail_count =
                            msg.get("fail_count").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
                        total = msg.get("total").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
                        if let Some(errs) = msg.get("errors").and_then(|v| v.as_array()) {
                            for e in errs {
                                if let Some(s) = e.as_str() {
                                    errors.push(s.to_string());
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
        }

        let exit_status = child.wait();
        if let Ok(mut guard) = CHILD_PID.lock() {
            *guard = None;
        }

        // Python 没发 result 行就退出（import 失败/崩溃/被杀）时，
        // 不能默默返回"成功 0/0"——按取消或失败如实上报
        if !got_result && !CANCEL_FLAG.load(Ordering::SeqCst) {
            let code = exit_status.ok().and_then(|s| s.code());
            return Err(format!(
                "聚类进程异常退出（退出码 {:?}），未返回结果；详见日志",
                code
            ));
        }

        Ok(ProcessResult {
            success_count,
            fail_count,
            total,
            errors,
        })
    })
    .await
    .map_err(|e| format!("任务执行失败: {}", e))?
}

#[tauri::command]
pub fn cancel_image_cluster() {
    CANCEL_FLAG.store(true, Ordering::SeqCst);
}

#[tauri::command]
pub fn force_cancel_image_cluster() {
    CANCEL_FLAG.store(true, Ordering::SeqCst);
    if let Ok(mut guard) = CHILD_PID.lock() {
        if let Some(pid) = guard.take() {
            super::kill_process_tree(pid);
        }
    }
}
