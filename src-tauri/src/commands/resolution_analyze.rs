//! 分辨率分布分析 — 统计数据集中各分辨率的图片数量，
//! 并定位数量稀少（疑似异常）的分辨率对应的具体文件

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use super::batch::{BatchCounts, BatchJob, RunEvents};
use super::{
    collect_image_files_with_recursive, collect_image_files_with_recursive_excluding, same_path,
    unique_destination, NameSuffix, ProgressEvent,
};

static ANALYZE_JOB: BatchJob = BatchJob::new("分辨率分析");
static AGGREGATE_JOB: BatchJob = BatchJob::new("分辨率聚合导出");

const EVENT: &str = "resolution-analyze-progress";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResolutionAnalyzeOptions {
    pub input_path: String,
    /// 图片数量 <= 此阈值的分辨率会被标记为异常，并附带文件路径列表
    #[serde(default = "default_rare_threshold")]
    pub rare_threshold: u32,
    #[serde(default)]
    pub recursive: bool,
}

fn default_rare_threshold() -> u32 {
    10
}

/// 单个分辨率的统计项
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResolutionGroup {
    pub width: u32,
    pub height: u32,
    pub count: u32,
    /// 占总数百分比
    pub percent: f64,
    /// 常见宽高比标签，如 "16:9"，无法归类时为空
    pub aspect_label: String,
    /// 是否为稀有分辨率（count <= rare_threshold）
    pub is_rare: bool,
    /// 仅稀有分辨率携带完整文件路径，避免大数据集下返回体过大
    pub files: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResolutionAnalyzeResult {
    pub total_images: u32,
    /// 无法读取尺寸的文件数
    pub failed_count: u32,
    pub failed_files: Vec<String>,
    /// 不同分辨率的种类数
    pub distinct_count: u32,
    pub groups: Vec<ResolutionGroup>,
    /// 最小/最大边长统计，便于快速判断数据集规模
    pub min_width: u32,
    pub max_width: u32,
    pub min_height: u32,
    pub max_height: u32,
}

/// 归类常见宽高比
fn aspect_label_for(w: u32, h: u32) -> String {
    if w == 0 || h == 0 {
        return String::new();
    }
    let ratio = w as f64 / h as f64;
    const KNOWN: &[(f64, &str)] = &[
        (1.0, "1:1"),
        (4.0 / 3.0, "4:3"),
        (3.0 / 4.0, "3:4"),
        (3.0 / 2.0, "3:2"),
        (2.0 / 3.0, "2:3"),
        (16.0 / 9.0, "16:9"),
        (9.0 / 16.0, "9:16"),
        (5.0 / 4.0, "5:4"),
        (4.0 / 5.0, "4:5"),
        (21.0 / 9.0, "21:9"),
        (2.0, "2:1"),
        (0.5, "1:2"),
    ];
    for (target, label) in KNOWN {
        if (ratio - target).abs() < 0.01 {
            return (*label).to_string();
        }
    }
    String::new()
}

#[tauri::command]
pub async fn analyze_resolutions<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    options: ResolutionAnalyzeOptions,
) -> Result<ResolutionAnalyzeResult, String> {
    ANALYZE_JOB.run(move || analyze_sync(&app, &options)).await
}

#[tauri::command]
pub fn cancel_resolution_analyze() {
    ANALYZE_JOB.cancel();
}

fn analyze_sync<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    options: &ResolutionAnalyzeOptions,
) -> Result<ResolutionAnalyzeResult, String> {
    let input = Path::new(&options.input_path);
    if !input.is_dir() {
        return Err(format!("输入目录不存在: {}", options.input_path));
    }

    let files: Vec<PathBuf> = collect_image_files_with_recursive(input, options.recursive)?;
    if files.is_empty() {
        return Err("未找到图片文件".into());
    }

    analyze_files(
        &files,
        options.rare_threshold,
        ANALYZE_JOB.cancel_flag(),
        &RunEvents::begin(app, EVENT),
    )
}

/// 统计分辨率分布并逐图上报进度。取消时发带 `cancelled` 的终态 done，返回「已取消」
fn analyze_files<R: tauri::Runtime>(
    files: &[PathBuf],
    rare_threshold: u32,
    cancel: &AtomicBool,
    run: &RunEvents<'_, R>,
) -> Result<ResolutionAnalyzeResult, String> {
    let total = files.len() as u32;
    let emit = |current: u32, status: &str, message: String, filename: String| {
        run.emit(
            ProgressEvent::new(status, message)
                .at(current, total)
                .file(filename),
        );
    };

    let mut dist: HashMap<(u32, u32), Vec<String>> = HashMap::new();
    let mut failed_files: Vec<String> = Vec::new();
    let (mut min_w, mut max_w, mut min_h, mut max_h) = (u32::MAX, 0u32, u32::MAX, 0u32);

    for (i, path) in files.iter().enumerate() {
        if cancel.load(Ordering::SeqCst) {
            return run.finish_cancelled(&BatchCounts {
                success: i as u32 - failed_files.len() as u32,
                failed: failed_files.len() as u32,
                total,
                ..Default::default()
            });
        }

        let fname = path
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();

        match super::image_io::read_dimensions(path) {
            Ok((w, h)) => {
                dist.entry((w, h))
                    .or_default()
                    .push(path.to_string_lossy().to_string());
                min_w = min_w.min(w);
                max_w = max_w.max(w);
                min_h = min_h.min(h);
                max_h = max_h.max(h);
                // 每文件一次 IPC 事件在万级数据集上会拖垮前端渲染，成功分支节流（错误分支保留逐条）
                let current = i as u32 + 1;
                if current.is_multiple_of(50) || current as usize == files.len() {
                    emit(
                        current,
                        "processing",
                        format!("{} — {}x{}", fname, w, h),
                        fname,
                    );
                }
            }
            Err(e) => {
                failed_files.push(format!("{}: {}", path.to_string_lossy(), e));
                emit(
                    i as u32 + 1,
                    "error",
                    format!("无法读取: {} ({})", fname, e),
                    fname,
                );
            }
        }
    }

    let valid_total: u32 = dist.values().map(|v| v.len() as u32).sum();
    if valid_total == 0 {
        emit(
            files.len() as u32,
            "error",
            "没有可成功读取尺寸的图片".to_string(),
            String::new(),
        );
        return Err("没有可成功读取尺寸的图片".into());
    }

    let mut groups: Vec<ResolutionGroup> = dist
        .into_iter()
        .map(|((w, h), paths)| {
            let count = paths.len() as u32;
            let is_rare = count <= rare_threshold;
            ResolutionGroup {
                width: w,
                height: h,
                count,
                percent: (count as f64 / valid_total as f64) * 100.0,
                aspect_label: aspect_label_for(w, h),
                is_rare,
                // 仅稀有分辨率保留路径，控制返回体大小
                files: if is_rare { paths } else { Vec::new() },
            }
        })
        .collect();

    // 数量降序；数量相同时按像素面积降序，输出稳定
    groups.sort_by(|a, b| {
        b.count
            .cmp(&a.count)
            .then((b.width * b.height).cmp(&(a.width * a.height)))
    });

    let distinct_count = groups.len() as u32;

    let counts = BatchCounts {
        success: valid_total,
        failed: failed_files.len() as u32,
        total,
        ..Default::default()
    };
    run.finish(&counts, false, |_| {
        format!(
            "分析完成：{} 张图片，{} 种分辨率",
            valid_total, distinct_count
        )
    });

    Ok(ResolutionAnalyzeResult {
        total_images: valid_total,
        failed_count: failed_files.len() as u32,
        failed_files,
        distinct_count,
        groups,
        min_width: if min_w == u32::MAX { 0 } else { min_w },
        max_width: max_w,
        min_height: if min_h == u32::MAX { 0 } else { min_h },
        max_height: max_h,
    })
}

// ═══════════════ 分辨率聚合导出 ═══════════════

/// 聚合计划中的一个目标文件夹：命中 resolutions 中任一分辨率的图片会被复制进 folder
#[derive(Debug, Clone, Deserialize)]
pub struct AggregatePlanEntry {
    /// 目标文件夹名（通常为 "宽x高"，如 "1920x1080"）
    pub folder: String,
    /// 归入该文件夹的成员分辨率列表
    pub resolutions: Vec<(u32, u32)>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ResolutionAggregateOptions {
    pub input_path: String,
    pub recursive: bool,
    pub output_path: String,
    pub plan: Vec<AggregatePlanEntry>,
}

#[tauri::command]
pub fn cancel_resolution_aggregate() {
    AGGREGATE_JOB.cancel();
}

/// 按聚合计划把图片复制到以目标分辨率命名的文件夹。
///
/// 分析结果为控制体积只保留稀有分组的文件路径，因此导出时重新扫描目录、
/// 逐图读取尺寸（仅解析文件头，开销小）后按计划归组。
#[tauri::command]
pub async fn export_resolution_aggregation<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    options: ResolutionAggregateOptions,
) -> Result<String, String> {
    AGGREGATE_JOB
        .run(move || aggregate_sync(&app, &options, AGGREGATE_JOB.cancel_flag()))
        .await
}

/// 取消时发带 `cancelled` 的终态 done，返回以「已取消」开头的错误（带已复制数）
fn aggregate_sync<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    options: &ResolutionAggregateOptions,
    cancel: &AtomicBool,
) -> Result<String, String> {
    let input = Path::new(&options.input_path);
    if !input.is_dir() {
        return Err(format!("输入目录不存在: {}", options.input_path));
    }
    if options.plan.is_empty() {
        return Err("聚合计划为空".into());
    }

    let out_root = Path::new(&options.output_path);
    std::fs::create_dir_all(out_root).map_err(|e| format!("创建输出目录失败: {}", e))?;

    // 输出目录==输入目录时，收集排除逻辑会整体失效（excluded != input 不成立），
    // 上次导出的产物会被再次归组，每次重导出文件数近似翻倍，直接拒绝
    if same_path(input, out_root) {
        return Err(
            "输出目录不能与输入目录相同：导出产物会在下次导出/分析时被当作输入重复归组".into(),
        );
    }

    // 分辨率 → 计划条目索引；文件夹名剥掉路径分隔符防止逃逸
    let mut lookup: HashMap<(u32, u32), usize> = HashMap::new();
    let mut folder_names: Vec<String> = Vec::with_capacity(options.plan.len());
    for (idx, entry) in options.plan.iter().enumerate() {
        let safe: String = entry
            .folder
            .chars()
            .map(|c| {
                if matches!(c, '/' | '\\' | ':') {
                    '_'
                } else {
                    c
                }
            })
            .collect();
        let safe = safe.trim().trim_matches('.').to_string();
        if safe.is_empty() {
            return Err(format!("非法的文件夹名: {}", entry.folder));
        }
        folder_names.push(safe);
        for &(w, h) in &entry.resolutions {
            lookup.insert((w, h), idx);
        }
    }

    // 输出目录可能位于输入目录内，收集时排除，避免把导出产物再当输入
    let files =
        collect_image_files_with_recursive_excluding(input, options.recursive, Some(out_root))?;
    if files.is_empty() {
        return Err("未找到图片文件".into());
    }
    let total = files.len() as u32;
    let run = RunEvents::begin(app, EVENT);

    let emit = |current: u32, status: &str, message: String| {
        run.emit(ProgressEvent::new(status, message).at(current, total));
    };

    let mut copied = 0u32;
    let mut unmatched = 0u32;
    let mut failed: Vec<String> = Vec::new();
    let mut used_folders: std::collections::HashSet<usize> = std::collections::HashSet::new();
    let counts = |copied: u32, unmatched: u32, failed: usize| BatchCounts {
        success: copied,
        skipped: unmatched,
        failed: failed as u32,
        total,
        ..Default::default()
    };

    for (i, path) in files.iter().enumerate() {
        if cancel.load(Ordering::SeqCst) {
            // 汇总信息放进错误消息（前端 catch 统一记录，避免与事件日志重复）
            return run
                .finish_cancelled(&counts(copied, unmatched, failed.len()))
                .map_err(|e| format!("{}，已复制 {} 个文件", e, copied));
        }

        match super::image_io::read_dimensions(path) {
            Ok((w, h)) => {
                if let Some(&idx) = lookup.get(&(w, h)) {
                    let dir = out_root.join(&folder_names[idx]);
                    if !used_folders.contains(&idx) {
                        if let Err(e) = std::fs::create_dir_all(&dir) {
                            let msg = format!("创建目录失败 {}: {}", dir.display(), e);
                            emit(i as u32, "error", msg.clone());
                            return Err(msg);
                        }
                        used_folders.insert(idx);
                    }
                    let filename = path
                        .file_name()
                        .map(|s| s.to_string_lossy().to_string())
                        .unwrap_or_else(|| format!("image_{}", i));
                    let dst =
                        unique_destination(&dir, &filename, NameSuffix::Counter, Path::exists);
                    match std::fs::copy(path, &dst) {
                        Ok(_) => copied += 1,
                        Err(e) => failed.push(format!("{}: {}", path.display(), e)),
                    }
                } else {
                    // 分析之后新增/变动的分辨率不在计划内，跳过
                    unmatched += 1;
                }
            }
            Err(_) => unmatched += 1,
        }

        let current = i as u32 + 1;
        if current.is_multiple_of(20) || current == total {
            emit(
                current,
                "processing",
                format!("正在聚合 {}/{}", current, total),
            );
        }
    }

    let summary = format!(
        "聚合导出完成：{} 个文件 → {} 个文件夹{}{}",
        copied,
        used_folders.len(),
        if unmatched > 0 {
            format!("，跳过 {} 个未匹配", unmatched)
        } else {
            String::new()
        },
        if failed.is_empty() {
            String::new()
        } else {
            format!("，{} 个复制失败", failed.len())
        },
    );
    // 详细汇总由命令返回值带回前端记录日志；done 事件负责把全局任务面板收尾
    run.finish(&counts(copied, unmatched, failed.len()), false, |_| {
        "聚合导出完成".to_string()
    });
    Ok(summary)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::batch::capture_raw_events;
    use crate::commands::test_support::TempDir;

    fn write_png(dir: &Path, name: &str, w: u32, h: u32) -> PathBuf {
        let p = dir.join(name);
        image::RgbImage::new(w, h).save(&p).unwrap();
        p
    }

    fn analyze(files: &[PathBuf], rare_threshold: u32) -> Result<ResolutionAnalyzeResult, String> {
        let app = tauri::test::mock_app();
        analyze_files(
            files,
            rare_threshold,
            &AtomicBool::new(false),
            &RunEvents::begin(app.handle(), EVENT),
        )
    }

    #[test]
    fn groups_sorted_by_count_and_marks_rare() {
        let d = TempDir::new("res_sort");
        let mut files = Vec::new();
        // 3 张 100x100，1 张 50x50
        for i in 0..3 {
            files.push(write_png(&d, &format!("a{}.png", i), 100, 100));
        }
        files.push(write_png(&d, "b.png", 50, 50));

        let r = analyze(&files, 1).unwrap();

        assert_eq!(r.total_images, 4);
        assert_eq!(r.distinct_count, 2);
        // 数量降序：100x100 在前
        assert_eq!((r.groups[0].width, r.groups[0].count), (100, 3));
        assert_eq!((r.groups[1].width, r.groups[1].count), (50, 1));
        // 阈值=1：仅 50x50 稀有，且携带路径
        assert!(!r.groups[0].is_rare);
        assert!(r.groups[0].files.is_empty(), "非稀有分组不应携带路径");
        assert!(r.groups[1].is_rare);
        assert_eq!(r.groups[1].files.len(), 1);
        // 百分比
        assert!((r.groups[0].percent - 75.0).abs() < 1e-6);
    }

    #[test]
    fn reports_min_max_extents() {
        let d = TempDir::new("res_extent");
        let files = vec![
            write_png(&d, "a.png", 100, 400),
            write_png(&d, "b.png", 300, 200),
        ];
        let r = analyze(&files, 0).unwrap();
        assert_eq!((r.min_width, r.max_width), (100, 300));
        assert_eq!((r.min_height, r.max_height), (200, 400));
    }

    #[test]
    fn unreadable_files_counted_not_fatal() {
        let d = TempDir::new("res_bad");
        let good = write_png(&d, "good.png", 64, 64);
        let bad = d.join("broken.png");
        std::fs::write(&bad, b"this is not an image").unwrap();

        let r = analyze(&[good, bad], 10).unwrap();
        assert_eq!(r.total_images, 1, "坏文件不应计入有效总数");
        assert_eq!(r.failed_count, 1);
        assert_eq!(r.groups.len(), 1);
    }

    #[test]
    fn all_unreadable_is_error() {
        let d = TempDir::new("res_allbad");
        let bad = d.join("x.png");
        std::fs::write(&bad, b"nope").unwrap();
        assert!(analyze(&[bad], 10).is_err());
    }

    /// 取消：终态 done 带 cancelled、用统一取消文案，返回「已取消」；正常完成的 done 不带
    #[test]
    fn cancelled_analysis_ends_with_a_cancelled_done() {
        let d = TempDir::new("res_cancel");
        let files = vec![write_png(&d, "a.png", 8, 8)];
        let app = tauri::test::mock_app();
        let log = capture_raw_events(app.handle(), EVENT);
        let run = RunEvents::begin(app.handle(), EVENT);
        let err = analyze_files(&files, 10, &AtomicBool::new(true), &run).unwrap_err();
        assert_eq!(err, "已取消");
        let done = log.lock().unwrap().last().cloned().unwrap();
        assert_eq!(done["cancelled"], true);
        assert_eq!(done["message"], "已取消: 已处理 0/1, 成功 0, 失败 0");
        assert_eq!(done["run_id"], run.run_id());

        let run = RunEvents::begin(app.handle(), EVENT);
        analyze_files(&files, 10, &AtomicBool::new(false), &run).unwrap();
        let done = log.lock().unwrap().last().cloned().unwrap();
        assert_eq!(done["message"], "分析完成：1 张图片，1 种分辨率");
        assert!(done.get("cancelled").is_none());
    }

    /// 聚合导出：同名文件追加序号不覆盖；取消时 done 带 cancelled
    #[test]
    fn aggregation_copies_without_overwriting_and_reports_cancel() {
        let root = TempDir::new("res_aggregate");
        let (input, output) = (root.join("in"), root.join("out"));
        std::fs::create_dir_all(input.join("sub")).unwrap();
        write_png(&input, "a.png", 8, 8);
        write_png(&input.join("sub"), "a.png", 8, 8);
        let options = ResolutionAggregateOptions {
            input_path: input.to_string_lossy().into_owned(),
            recursive: true,
            output_path: output.to_string_lossy().into_owned(),
            plan: vec![AggregatePlanEntry {
                folder: "8x8".into(),
                resolutions: vec![(8, 8)],
            }],
        };
        let app = tauri::test::mock_app();
        let log = capture_raw_events(app.handle(), EVENT);
        let summary = aggregate_sync(app.handle(), &options, &AtomicBool::new(false)).unwrap();
        assert_eq!(summary, "聚合导出完成：2 个文件 → 1 个文件夹");
        assert!(output.join("8x8/a.png").is_file() && output.join("8x8/a_1.png").is_file());

        let err = aggregate_sync(app.handle(), &options, &AtomicBool::new(true)).unwrap_err();
        assert_eq!(err, "已取消，已复制 0 个文件");
        let done = log.lock().unwrap().last().cloned().unwrap();
        assert_eq!(done["cancelled"], true);
        assert_eq!(done["message"], "已取消: 已处理 0/2, 成功 0, 失败 0");
    }

    /// 输出目录就是输入目录（含 `in/.`、只差大小写的写法）：开始前拒绝，不复制、不发事件
    #[test]
    fn aggregation_into_the_input_dir_is_rejected() {
        let root = TempDir::new("res_aggregate_same");
        let input = root.join("in");
        std::fs::create_dir_all(&input).unwrap();
        write_png(&input, "a.png", 8, 8);
        let app = tauri::test::mock_app();
        let log = capture_raw_events(app.handle(), EVENT);
        for output in [input.clone(), input.join("."), root.join("IN")] {
            let options = ResolutionAggregateOptions {
                input_path: input.to_string_lossy().into_owned(),
                recursive: true,
                output_path: output.to_string_lossy().into_owned(),
                plan: vec![AggregatePlanEntry {
                    folder: "8x8".into(),
                    resolutions: vec![(8, 8)],
                }],
            };
            let err = aggregate_sync(app.handle(), &options, &AtomicBool::new(false)).unwrap_err();
            assert!(err.starts_with("输出目录不能与输入目录相同"), "{err}");
        }
        assert!(!input.join("8x8").exists());
        assert!(log.lock().unwrap().is_empty());
    }

    #[test]
    fn aspect_labels() {
        assert_eq!(aspect_label_for(1024, 1024), "1:1");
        assert_eq!(aspect_label_for(1920, 1080), "16:9");
        assert_eq!(aspect_label_for(1080, 1920), "9:16");
        assert_eq!(aspect_label_for(512, 768), "2:3");
        // 非常见比例返回空
        assert_eq!(aspect_label_for(333, 777), "");
        // 防御除零
        assert_eq!(aspect_label_for(100, 0), "");
    }
}
