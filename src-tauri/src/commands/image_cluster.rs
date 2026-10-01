use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use super::python_proc::{self, hidden_command};
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

async fn ensure_cluster_deps(app: &tauri::AppHandle, algorithm: &str) -> Result<String, String> {
    let emit_log = |msg: String| {
        ProgressEvent::new("info", msg).emit(app, "cluster-progress");
    };
    emit_log("正在检查 Python 环境...".into());
    let python = super::python_env::setup_python_env(app, "cluster").await?;
    check_cancelled()?;
    super::python_env::ensure_torch_gpu_runtime(app, &python, "cluster").await?;
    check_cancelled()?;

    let dependencies = [
        (
            "import sklearn",
            "scikit-learn",
            &["scikit-learn"][..],
            false,
        ),
        ("import umap", "umap-learn", &["umap-learn"][..], true),
        ("from PIL import Image", "Pillow", &["pillow"][..], false),
    ];
    for (probe, label, packages, hdbscan_only) in dependencies {
        if hdbscan_only && algorithm != "hdbscan" {
            continue;
        }
        check_cancelled()?;
        if super::python_env::probe_python(&python, probe)
            .await
            .is_some()
        {
            continue;
        }
        check_cancelled()?;
        emit_log(format!("正在安装 {}...", label));
        super::python_env::pip_install_for(app, &python, packages, "cluster").await?;
        check_cancelled()?;
        emit_log(format!("{} 安装完成", label));
    }
    emit_log("环境检查完成".into());
    Ok(python)
}

fn check_cancelled() -> Result<(), String> {
    if CANCEL_FLAG.load(Ordering::SeqCst) {
        Err("已取消".into())
    } else {
        Ok(())
    }
}

#[tauri::command]
pub async fn start_image_cluster(
    app: tauri::AppHandle,
    options: ClusterOptions,
) -> Result<ProcessResult, String> {
    static RUNNING: AtomicBool = AtomicBool::new(false);
    let _busy = super::BusyGuard::acquire(&RUNNING, "聚类")?;
    CANCEL_FLAG.store(false, Ordering::SeqCst);
    let prepared = ensure_cluster_deps(&app, &options.algorithm).await;
    if CANCEL_FLAG.load(Ordering::SeqCst) {
        ProgressEvent::new("done", "已取消").emit(&app, "cluster-progress");
        return Ok(ProcessResult::default());
    }
    let python = prepared?;
    let script = python_proc::find_script("image_cluster.py")?;
    let model_dir = super::config_paths::models_dir("cluster_models");

    tokio::task::spawn_blocking(move || {
        let mut cmd = hidden_command(&python);
        cmd.arg(&script)
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
            .arg(&model_dir)
            .arg("--map-theme")
            .arg(&options.map_theme);
        if options.recursive {
            cmd.arg("--recursive");
        }
        let app_err = app.clone();
        let mut result = ProcessResult::default();
        let mut summary = None;
        let mut progress_total = 0;
        let mut progress_current = 0;
        let exit = python_proc::run_json_lines_script(
            cmd,
            options.device != "cpu",
            &CHILD_PID,
            &CANCEL_FLAG,
            move |line| {
                if !python_proc::is_runtime_noise(&line)
                    && !python_proc::is_python_library_noise(&line)
                {
                    ProgressEvent::new("warning", format!("[Python] {}", line))
                        .emit(&app_err, "cluster-progress");
                }
            },
            |msg| {
                match msg["type"].as_str().unwrap_or("") {
                    "log" => ProgressEvent::python_log(&msg, 0, 0).emit(&app, "cluster-progress"),
                    "error" => {
                        return Err(format!(
                            "聚类错误: {}",
                            msg["message"].as_str().unwrap_or("")
                        ))
                    }
                    "progress" => {
                        progress_current = msg["current"].as_u64().unwrap_or(0) as u32;
                        progress_total = msg["total"].as_u64().unwrap_or(0) as u32;
                        let status = msg["status"].as_str().unwrap_or("processing");
                        let message = msg["message"].as_str().unwrap_or("");
                        if status == "success" {
                            result.success_count += 1;
                        }
                        if status == "error" {
                            result.fail_count += 1;
                            result.errors.push(message.into());
                        }
                        // Python 的进度还含聚类和分布图步骤，不是图片总数。
                        result.total = progress_total.saturating_sub(2) / 2;
                        ProgressEvent::new(status, message)
                            .at(progress_current, progress_total)
                            .file(msg["filename"].as_str().unwrap_or(""))
                            .emit(&app, "cluster-progress");
                    }
                    "done" => {
                        result.success_count = msg["success_count"].as_u64().unwrap_or(0) as u32;
                        result.fail_count = msg["fail_count"].as_u64().unwrap_or(0) as u32;
                        result.total = msg["total"].as_u64().unwrap_or(0) as u32;
                        result.errors = msg["errors"]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .filter_map(|v| v.as_str().map(str::to_owned))
                            .collect();
                        summary = Some(msg["message"].as_str().unwrap_or("").to_owned());
                    }
                    _ => {}
                }
                Ok(())
            },
        );
        if CANCEL_FLAG.load(Ordering::SeqCst) {
            ProgressEvent::new(
                "done",
                format!(
                    "已取消: 已处理 {}, 共 {}",
                    result.success_count + result.fail_count,
                    result.total
                ),
            )
            .at(progress_current, progress_total)
            .emit(&app, "cluster-progress");
        } else {
            let exit = exit?;
            if let Some(summary) = summary {
                ProgressEvent::new("done", summary)
                    .at(progress_total, progress_total)
                    .emit(&app, "cluster-progress");
            } else {
                return Err(format!(
                    "聚类进程异常退出（退出码 {:?}），未返回结果；详见日志",
                    exit.code
                ));
            }
        }
        Ok(result)
    })
    .await
    .map_err(|e| format!("任务执行失败: {}", e))?
}

#[tauri::command]
pub fn cancel_image_cluster() {
    CANCEL_FLAG.store(true, Ordering::SeqCst);
    super::python_env::cancel_setup_for("cluster");
}

#[tauri::command]
pub fn force_cancel_image_cluster() {
    cancel_image_cluster();
    python_proc::kill_registered_pid(&CHILD_PID);
}
