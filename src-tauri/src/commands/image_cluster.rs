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
    pub map_theme: String,     // "light" | "dark"
}

/// 获取聚类脚本路径
fn get_cluster_script() -> Result<PathBuf, String> {
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|p| p.to_path_buf()))
        .unwrap_or_else(|| PathBuf::from("."));

    let candidates = vec![
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("scripts/image_cluster.py"),
        exe_dir.join("scripts/image_cluster.py"),
        exe_dir.join("image_cluster.py"),
        #[cfg(target_os = "macos")]
        exe_dir.join("../Resources/scripts/image_cluster.py"),
    ];

    for path in &candidates {
        if path.exists() {
            return Ok(path.canonicalize().unwrap_or_else(|_| path.clone()));
        }
    }
    Err("聚类脚本 image_cluster.py 未找到".into())
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
        let _ = app.emit("cluster-progress", ProgressEvent {
            current: 0, total: 0, filename: String::new(),
            status: "info".to_string(), message: msg.to_string(),
        });
    };

    emit_log("正在检查 Python 环境...");
    let python = super::python_env::setup_python_env(app).await?;

    // 检查 torch
    let has_torch = {
        let p = python.clone();
        tokio::task::spawn_blocking(move || {
            let mut cmd = std::process::Command::new(&p);
            cmd.args(["-c", "import torch; print(torch.__version__)"]);
            #[cfg(target_os = "windows")]
            {
                use std::os::windows::process::CommandExt;
                cmd.creation_flags(0x08000000);
            }
            cmd.output().map(|o| o.status.success()).unwrap_or(false)
        }).await.unwrap_or(false)
    };

    if !has_torch {
        emit_log("正在安装 PyTorch（首次安装约 500MB-2GB）...");
        let p = python.clone();
        let app2 = app.clone();
        tokio::task::spawn_blocking(move || {
            super::python_env::pip_install_with_python(&app2, &p, &["torch", "torchvision"])
        }).await
        .map_err(|e| format!("安装线程异常: {}", e))??;
        emit_log("PyTorch 安装完成");
    }

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
        }).await.unwrap_or(false)
    };

    if !has_sklearn {
        emit_log("正在安装 scikit-learn...");
        let p = python.clone();
        let app2 = app.clone();
        tokio::task::spawn_blocking(move || {
            super::python_env::pip_install_with_python(&app2, &p, &["scikit-learn"])
        }).await
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
        }).await.unwrap_or(false)
    };

    if !has_umap {
        emit_log("正在安装 umap-learn（HDBSCAN 降维依赖）...");
        let p = python.clone();
        let app2 = app.clone();
        tokio::task::spawn_blocking(move || {
            super::python_env::pip_install_with_python(&app2, &p, &["umap-learn"])
        }).await
        .map_err(|e| format!("安装线程异常: {}", e))??;
        emit_log("umap-learn 安装完成");
    }

    emit_log("环境检查完成");
    Ok(())
}

#[tauri::command]
pub async fn start_image_cluster(app: tauri::AppHandle, options: ClusterOptions) -> Result<ProcessResult, String> {
    CANCEL_FLAG.store(false, Ordering::SeqCst);

    // 确保依赖
    ensure_cluster_deps(&app).await?;

    let python = super::python_env::get_python_exe()
        .ok_or("Python 环境未就绪")?;
    let script = get_cluster_script()?;
    let model_dir = get_cluster_model_dir();

    let app_clone = app.clone();

    tokio::task::spawn_blocking(move || {
        use std::io::BufRead;

        let mut cmd = std::process::Command::new(&python);
        cmd.arg(script.to_string_lossy().as_ref())
            .arg("--input").arg(&options.input_path)
            .arg("--output").arg(&options.output_path)
            .arg("--algorithm").arg(&options.algorithm)
            .arg("--feature").arg(&options.feature_type)
            .arg("--n-clusters").arg(options.n_clusters.to_string())
            .arg("--min-cluster-size").arg(options.min_cluster_size.to_string())
            .arg("--device").arg(&options.device)
            .arg("--weight-style").arg(format!("{:.2}", options.weight_style))
            .arg("--weight-semantic").arg(format!("{:.2}", options.weight_semantic))
            .arg("--weight-color").arg(format!("{:.2}", options.weight_color))
            .arg("--model-dir").arg(model_dir.to_string_lossy().as_ref())
            .arg("--map-theme").arg(&options.map_theme)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .env("PYTHONUNBUFFERED", "1");

        #[cfg(target_os = "windows")]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x08000000);
        }

        let mut child = cmd.spawn()
            .map_err(|e| format!("启动 Python 失败: {}", e))?;

        if let Ok(mut guard) = CHILD_PID.lock() {
            *guard = Some(child.id());
        }

        let stdout = child.stdout.take().ok_or("无法获取 stdout")?;
        let stderr = child.stderr.take().ok_or("无法获取 stderr")?;

        // stderr 线程
        let app_err = app_clone.clone();
        std::thread::spawn(move || {
            let reader = std::io::BufReader::new(stderr);
            for line in reader.lines().flatten() {
                let clean = line.trim();
                if clean.is_empty() { continue; }
                // 跳过 PyTorch 模型下载进度条（包含大量 % 符号的行）
                if clean.matches('%').count() > 3 { continue; }
                // 跳过常见无害警告
                if clean.contains("UserWarning") || clean.contains("FutureWarning") { continue; }
                if clean.contains("RuntimeWarning") { continue; }
                if clean.starts_with("Downloading:") || clean.starts_with("100%") { continue; }
                // 跳过 Python warnings 模块的碎片行
                if clean == "warn(" || clean.starts_with("warnings.warn(") { continue; }
                if clean.contains("site-packages/") && clean.contains(".py:") { continue; }
                if clean.starts_with("eigenvalues") || clean.starts_with("scipy.") { continue; }
                let _ = app_err.emit("cluster-progress", ProgressEvent {
                    current: 0, total: 0, filename: String::new(),
                    status: "warning".to_string(),
                    message: format!("[Python] {}", clean),
                });
            }
        });

        // 解析 stdout JSON
        let reader = std::io::BufReader::new(stdout);
        let mut success_count = 0u32;
        let mut fail_count = 0u32;
        let mut total = 0u32;
        let mut errors = Vec::new();

        for line in reader.lines().flatten() {
            if CANCEL_FLAG.load(Ordering::SeqCst) {
                let _ = child.kill();
                break;
            }

            if let Ok(msg) = serde_json::from_str::<serde_json::Value>(&line) {
                let msg_type = msg.get("type").and_then(|v| v.as_str()).unwrap_or("");
                match msg_type {
                    "log" => {
                        let text = msg.get("message").and_then(|v| v.as_str()).unwrap_or("");
                        let _ = app_clone.emit("cluster-progress", ProgressEvent {
                            current: 0, total: 0, filename: String::new(),
                            status: "info".to_string(),
                            message: text.to_string(),
                        });
                    }
                    "error" => {
                        let text = msg.get("message").and_then(|v| v.as_str()).unwrap_or("");
                        return Err(format!("聚类错误: {}", text));
                    }
                    "progress" => {
                        let cur = msg.get("current").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
                        let tot = msg.get("total").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
                        let fname = msg.get("filename").and_then(|v| v.as_str()).unwrap_or("");
                        let status = msg.get("status").and_then(|v| v.as_str()).unwrap_or("processing");
                        let message = msg.get("message").and_then(|v| v.as_str()).unwrap_or("");
                        total = tot;

                        if status == "success" {
                            success_count += 1;
                        } else if status == "error" {
                            fail_count += 1;
                            errors.push(message.to_string());
                        }

                        let _ = app_clone.emit("cluster-progress", ProgressEvent {
                            current: cur, total: tot,
                            filename: fname.to_string(),
                            status: status.to_string(),
                            message: message.to_string(),
                        });
                    }
                    "done" => {
                        let text = msg.get("message").and_then(|v| v.as_str()).unwrap_or("");
                        let _ = app_clone.emit("cluster-progress", ProgressEvent {
                            current: total, total,
                            filename: String::new(),
                            status: "done".to_string(),
                            message: text.to_string(),
                        });
                    }
                    "result" => {
                        success_count = msg.get("success_count").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
                        fail_count = msg.get("fail_count").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
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

        let _ = child.wait();
        if let Ok(mut guard) = CHILD_PID.lock() {
            *guard = None;
        }

        Ok(ProcessResult { success_count, fail_count, total, errors })
    }).await
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
            #[cfg(unix)]
            {
                let _ = std::process::Command::new("kill")
                    .args(["-9", &pid.to_string()])
                    .output();
            }
            #[cfg(windows)]
            {
                let _ = std::process::Command::new("taskkill")
                    .args(["/F", "/PID", &pid.to_string(), "/T"])
                    .output();
            }
        }
    }
}
