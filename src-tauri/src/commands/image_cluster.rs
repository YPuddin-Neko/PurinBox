use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use super::batch::{self, BatchCounts};
use super::python_proc::{
    self, abnormal_exit, ManifestFile, PythonCommand, PythonTempTag, StderrNoise,
};
use super::{ProcessResult, ProgressEvent};

const EVENT: &str = "cluster-progress";

static CHILD_PID: Mutex<Option<u32>> = Mutex::new(None);
static CANCEL_FLAG: AtomicBool = AtomicBool::new(false);
static RUNNING: AtomicBool = AtomicBool::new(false);

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
        ProgressEvent::new("info", msg).emit(app, EVENT);
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
    let _busy = super::BusyGuard::acquire(&RUNNING, "聚类")?;
    CANCEL_FLAG.store(false, Ordering::SeqCst);
    super::python_env::clear_pending_cancel("cluster");
    let run_id = super::begin_run(EVENT);
    let scan = options.clone();
    let files = tokio::task::spawn_blocking(move || {
        super::collect_image_files_with_recursive_excluding(
            Path::new(&scan.input_path),
            scan.recursive,
            Some(Path::new(&scan.output_path)),
        )
    })
    .await
    .map_err(|e| format!("读取图片失败: {}", e))??;
    if files.is_empty() {
        return Err("未找到图片文件".into());
    }
    let prepared = ensure_cluster_deps(&app, &options.algorithm).await;
    if CANCEL_FLAG.load(Ordering::SeqCst) {
        let result = ProcessResult {
            total: files.len() as u32,
            ..Default::default()
        };
        batch::finish_run(&app, EVENT, &BatchCounts::from(&result), true, |_| {
            String::new()
        });
        return Ok(result);
    }
    let python = prepared?;
    let script = python_proc::find_script("image_cluster.py")?;
    let model_dir = super::config_paths::models_dir("cluster_models");
    tokio::task::spawn_blocking(move || {
        run_cluster(
            &app,
            PythonCommand::new(python)
                .arg(script)
                .arg("--model-dir")
                .arg(model_dir),
            &options,
            &files,
            &PythonTempTag::for_run(run_id),
            &CANCEL_FLAG,
            &CHILD_PID,
        )
    })
    .await
    .map_err(|e| format!("任务执行失败: {}", e))?
}

/// 交给聚类脚本的 [图片路径, 相对输入目录的子目录]（分组目录下保留这层子目录）
fn cluster_entry(input: &Path, file: &Path, recursive: bool) -> Result<[String; 2], String> {
    let relative = super::relative_dir_for_input(input, file, recursive).unwrap_or_default();
    match (file.to_str(), relative.to_str()) {
        (Some(path), Some(relative)) => Ok([path.to_owned(), relative.to_owned()]),
        // 清单是 UTF-8 的 JSON，写不进这种路径
        _ => Err("路径含有非 UTF-8 字符，聚类脚本无法处理".into()),
    }
}

/// `cmd` 是带脚本和模型缓存目录的聚类命令，其余参数在这里补上
fn run_cluster<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    cmd: PythonCommand,
    options: &ClusterOptions,
    files: &[PathBuf],
    temp_tag: &PythonTempTag,
    cancel: &AtomicBool,
    child_slot: &Mutex<Option<u32>>,
) -> Result<ProcessResult, String> {
    let input = Path::new(&options.input_path);
    let mut result = ProcessResult {
        total: files.len() as u32,
        ..Default::default()
    };
    let mut entries = Vec::new();
    for file in files {
        match cluster_entry(input, file, options.recursive) {
            Ok(entry) => entries.push(entry),
            Err(error) => {
                result.fail_count += 1;
                let filename = super::file_name_lossy(file);
                let message = format!("{}: {}", filename, error);
                result.errors.push(message.clone());
                ProgressEvent::new("error", message)
                    .file(filename)
                    .emit(app, EVENT);
            }
        }
    }
    let manifest = ManifestFile::write("purinbox-cluster", &entries)?;
    let cmd = cmd
        .arg("--files")
        .arg(manifest.path())
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
        .arg("--map-theme")
        .arg(&options.map_theme)
        .use_gpu(options.device != "cpu");
    let rejected = result.fail_count;
    let initial_errors = result.errors.clone();
    let mut clusters = None;
    let exit = python_proc::run_json_lines_script_with(
        temp_tag.apply(cmd),
        None,
        child_slot,
        cancel,
        python_proc::stderr_warnings(app, EVENT, StderrNoise::RuntimeAndLibraries, None),
        |msg| {
            match msg["type"].as_str().unwrap_or("") {
                "log" => ProgressEvent::python_log(&msg, 0, 0).emit(app, EVENT),
                "error" => {
                    return Err(format!(
                        "聚类错误: {}",
                        msg["message"].as_str().unwrap_or("")
                    ))
                }
                "progress" => {
                    let status = msg["status"].as_str().unwrap_or("processing");
                    let message = msg["message"].as_str().unwrap_or("");
                    if status == "success" {
                        result.success_count += 1;
                    }
                    if status == "error" {
                        result.fail_count += 1;
                        result.errors.push(message.into());
                    }
                    // Python 的进度还含聚类和分布图步骤，current/total 不是图片数
                    ProgressEvent::new(status, message)
                        .at(
                            msg["current"].as_u64().unwrap_or(0) as u32,
                            msg["total"].as_u64().unwrap_or(0) as u32,
                        )
                        .file(msg["filename"].as_str().unwrap_or(""))
                        .emit(app, EVENT);
                }
                "done" => {
                    result.success_count = msg["success_count"].as_u64().unwrap_or(0) as u32;
                    result.fail_count = rejected + msg["fail_count"].as_u64().unwrap_or(0) as u32;
                    result.errors.clone_from(&initial_errors);
                    result.errors.extend(
                        msg["errors"]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .filter_map(|v| v.as_str().map(str::to_owned)),
                    );
                    clusters = Some(msg["clusters"].as_u64().unwrap_or(0));
                }
                _ => {}
            }
            Ok(())
        },
    );
    if clusters.is_none() {
        // 没正常结束（被取消、报错，或自己崩溃、被系统杀掉）时可能有复制到一半的临时文件
        temp_tag.remove_under(Path::new(&options.output_path));
    }
    if cancel.load(Ordering::SeqCst) {
        batch::finish_run(app, EVENT, &BatchCounts::from(&result), true, |_| {
            String::new()
        });
        return Ok(result);
    }
    let exit = exit?;
    let Some(clusters) = clusters else {
        return Err(abnormal_exit("聚类进程", &exit));
    };
    batch::finish_run(app, EVENT, &BatchCounts::from(&result), false, |c| {
        format!(
            "完成: {} 个分组, 成功 {}, 失败 {}, 共 {}",
            clusters, c.success, c.failed, c.total
        )
    });
    Ok(result)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::test_support::TempDir;

    #[test]
    fn entries_keep_the_subdirectory_only_when_recursive() {
        let root = TempDir::new("cluster_entries");
        std::fs::create_dir_all(root.join("sub")).unwrap();
        let nested = root.join("sub/a.png");
        let entry = cluster_entry(&root, &nested, true).unwrap();
        assert_eq!(
            entry,
            [nested.to_str().unwrap().to_owned(), "sub".to_owned()]
        );
        assert_eq!(cluster_entry(&root, &nested, false).unwrap()[1], "");
        assert_eq!(cluster_entry(&nested, &nested, true).unwrap()[1], "");
    }

    #[cfg(unix)]
    #[test]
    fn non_utf8_paths_are_rejected_one_by_one() {
        use std::os::unix::ffi::OsStrExt;
        let root = TempDir::new("cluster_non_utf8");
        let bad = root.join(std::ffi::OsStr::from_bytes(b"bad\xff.png"));
        assert!(cluster_entry(&root, &bad, false)
            .unwrap_err()
            .contains("UTF-8"));
    }
}

#[cfg(all(test, unix))]
mod script_tests {
    use super::*;
    use crate::commands::batch::capture_events;
    use crate::commands::test_support::{fake_program, TempDir};
    use std::sync::Arc;

    fn options(root: &Path) -> ClusterOptions {
        ClusterOptions {
            input_path: root.join("in").to_string_lossy().into_owned(),
            output_path: root.join("out").to_string_lossy().into_owned(),
            algorithm: "hdbscan".into(),
            feature_type: "semantic".into(),
            n_clusters: 8,
            min_cluster_size: 5,
            device: "cpu".into(),
            weight_style: 0.5,
            weight_semantic: 0.5,
            weight_color: 0.0,
            map_theme: "light".into(),
            recursive: true,
        }
    }

    fn cluster(
        root: &Path,
        body: &str,
        files: &[PathBuf],
        cancel: &AtomicBool,
        app: &tauri::App<tauri::test::MockRuntime>,
    ) -> Result<ProcessResult, String> {
        let python = fake_program(root, "fake-python", body);
        run_cluster(
            app.handle(),
            PythonCommand::new(python),
            &options(root),
            files,
            &PythonTempTag::for_run(5),
            cancel,
            &Mutex::new(None),
        )
    }

    fn done_events(events: &Mutex<Vec<serde_json::Value>>) -> Vec<serde_json::Value> {
        events
            .lock()
            .unwrap()
            .iter()
            .filter(|e| e["status"] == "done")
            .cloned()
            .collect()
    }

    #[test]
    fn summary_counts_rejected_files_and_manifest_reaches_the_script() {
        use std::os::unix::ffi::OsStrExt;
        let root = TempDir::new("cluster_summary");
        std::fs::create_dir_all(root.join("in/sub")).unwrap();
        let files = [
            root.join("in/a.png"),
            root.join("in/sub/b.png"),
            root.join(std::ffi::OsStr::from_bytes(b"in/bad\xff.png")),
        ];
        let body = format!(
            "while [ $# -gt 0 ]; do case \"$1\" in --files) cp \"$2\" '{manifest}';; --output) echo \"$2\" > '{output}';; esac; shift; done\nprintf '%s\\n' '{{\"type\":\"progress\",\"current\":3,\"total\":6,\"filename\":\"a.png\",\"status\":\"success\",\"message\":\"ok\"}}' '{{\"type\":\"done\",\"clusters\":2,\"success_count\":2,\"fail_count\":0,\"total\":2,\"errors\":[]}}'\n",
            manifest = root.join("manifest.json").display(),
            output = root.join("output.txt").display()
        );
        let app = tauri::test::mock_app();
        let events = capture_events(app.handle(), EVENT);
        let result = cluster(&root, &body, &files, &AtomicBool::new(false), &app).unwrap();
        assert_eq!(
            (result.success_count, result.fail_count, result.total),
            (2, 1, 3)
        );
        let manifest: Vec<[String; 2]> =
            serde_json::from_slice(&std::fs::read(root.join("manifest.json")).unwrap()).unwrap();
        assert_eq!(
            manifest,
            [
                [files[0].to_str().unwrap().to_owned(), String::new()],
                [files[1].to_str().unwrap().to_owned(), "sub".to_owned()],
            ]
        );
        assert_eq!(
            std::fs::read_to_string(root.join("output.txt"))
                .unwrap()
                .trim(),
            root.join("out").to_str().unwrap()
        );
        let done = done_events(&events);
        assert_eq!(done.len(), 1);
        assert_eq!(done[0]["message"], "完成: 2 个分组, 成功 2, 失败 1, 共 3");
    }

    #[test]
    fn cancel_removes_half_written_copies() {
        use tauri::Listener;
        let root = TempDir::new("cluster_cancel");
        std::fs::create_dir_all(root.join("out/cluster_0")).unwrap();
        let body = format!(
            "touch '{}'.\"$PURIN_TEMP_TAG\".tmp\nprintf '%s\\n' '{{\"type\":\"progress\",\"current\":4,\"total\":6,\"filename\":\"a.png\",\"status\":\"processing\",\"message\":\"copying\"}}'\nsleep 30\n",
            root.join("out/cluster_0/a.png").display()
        );
        let cancel = Arc::new(AtomicBool::new(false));
        let app = tauri::test::mock_app();
        let events = capture_events(app.handle(), EVENT);
        let flag = cancel.clone();
        app.listen_any(EVENT, move |event| {
            if event.payload().contains("copying") {
                flag.store(true, Ordering::SeqCst);
            }
        });
        let result = cluster(&root, &body, &[root.join("in/a.png")], &cancel, &app).unwrap();
        assert_eq!(result.success_count, 0);
        assert!(!root.join("out/cluster_0/a.png.purin-r5.tmp").exists());
        let done = done_events(&events);
        assert_eq!(done.len(), 1);
        assert_eq!(done[0]["cancelled"], true);
    }

    /// 脚本自己崩溃（没被我们终止）也会留下复制到一半的临时文件
    #[test]
    fn crash_without_done_removes_half_written_copies() {
        let root = TempDir::new("cluster_crash");
        std::fs::create_dir_all(root.join("out/cluster_0")).unwrap();
        let body = format!(
            "touch '{}'.\"$PURIN_TEMP_TAG\".tmp\necho 'Segmentation fault' >&2\nexit 139\n",
            root.join("out/cluster_0/a.png").display()
        );
        let app = tauri::test::mock_app();
        let error = cluster(
            &root,
            &body,
            &[root.join("in/a.png")],
            &AtomicBool::new(false),
            &app,
        )
        .unwrap_err();
        assert_eq!(error, "聚类进程异常退出（退出码 139）: Segmentation fault");
        assert!(!root.join("out/cluster_0/a.png.purin-r5.tmp").exists());
    }

    #[test]
    fn script_error_and_missing_done_are_errors() {
        let root = TempDir::new("cluster_errors");
        let app = tauri::test::mock_app();
        let events = capture_events(app.handle(), EVENT);
        let error = cluster(
            &root,
            "printf '%s\\n' '{\"type\":\"error\",\"message\":\"有效图片不足 2 张，无法聚类\"}'\nsleep 30\n",
            &[root.join("in/a.png")],
            &AtomicBool::new(false),
            &app,
        )
        .unwrap_err();
        assert_eq!(error, "聚类错误: 有效图片不足 2 张，无法聚类");
        let error = cluster(
            &root,
            "echo 'ModuleNotFoundError: torch' >&2\nexit 1\n",
            &[root.join("in/a.png")],
            &AtomicBool::new(false),
            &app,
        )
        .unwrap_err();
        assert_eq!(
            error,
            "聚类进程异常退出（退出码 1）: ModuleNotFoundError: torch"
        );
        assert!(done_events(&events).is_empty());
    }
}
