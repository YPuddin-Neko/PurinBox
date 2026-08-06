//! 分辨率分布分析 — 统计数据集中各分辨率的图片数量，
//! 并定位数量稀少（疑似异常）的分辨率对应的具体文件

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::Emitter;

use super::{collect_image_files_with_recursive, ProgressEvent};

static CANCEL_FLAG: AtomicBool = AtomicBool::new(false);

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
    /// 宽高比（保留两位小数）
    pub aspect_ratio: f64,
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

/// 按文件头猜测格式读取尺寸，避免后缀与实际格式不一致导致失败
fn read_image_dimensions(path: &Path) -> image::ImageResult<(u32, u32)> {
    image::ImageReader::open(path)?
        .with_guessed_format()?
        .into_dimensions()
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
pub async fn analyze_resolutions(
    app: tauri::AppHandle,
    options: ResolutionAnalyzeOptions,
) -> Result<ResolutionAnalyzeResult, String> {
    CANCEL_FLAG.store(false, Ordering::SeqCst);
    tokio::task::spawn_blocking(move || analyze_sync(&app, &options))
        .await
        .map_err(|e| format!("任务执行失败: {}", e))?
}

#[tauri::command]
pub fn cancel_resolution_analyze() {
    CANCEL_FLAG.store(true, Ordering::SeqCst);
}

fn analyze_sync(
    app: &tauri::AppHandle,
    options: &ResolutionAnalyzeOptions,
) -> Result<ResolutionAnalyzeResult, String> {
    let input = Path::new(&options.input_path);
    if !input.exists() || !input.is_dir() {
        return Err(format!("输入目录不存在: {}", options.input_path));
    }

    let files: Vec<PathBuf> = collect_image_files_with_recursive(input, options.recursive)?;
    if files.is_empty() {
        return Err("未找到图片文件".into());
    }
    let total = files.len() as u32;

    analyze_files(&files, options.rare_threshold, |current, status, message, filename| {
        let _ = app.emit(
            "resolution-analyze-progress",
            ProgressEvent {
                current,
                total,
                filename,
                status: status.to_string(),
                message,
                ..Default::default()
            },
        );
    })
}

/// 分析核心：与 Tauri 解耦，便于单元测试。
/// `on_progress(current, status, message, filename)` 用于上报进度。
fn analyze_files<F>(
    files: &[PathBuf],
    rare_threshold: u32,
    mut on_progress: F,
) -> Result<ResolutionAnalyzeResult, String>
where
    F: FnMut(u32, &str, String, String),
{
    let total = files.len() as u32;
    let mut emit = |current: u32, status: &str, message: String, filename: String| {
        on_progress(current, status, message, filename);
    };

    let mut dist: HashMap<(u32, u32), Vec<String>> = HashMap::new();
    let mut failed_files: Vec<String> = Vec::new();
    let (mut min_w, mut max_w, mut min_h, mut max_h) = (u32::MAX, 0u32, u32::MAX, 0u32);

    for (i, path) in files.iter().enumerate() {
        if CANCEL_FLAG.load(Ordering::SeqCst) {
            return Err("已取消".into());
        }

        let fname = path
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();

        match read_image_dimensions(path) {
            Ok((w, h)) => {
                dist.entry((w, h))
                    .or_default()
                    .push(path.to_string_lossy().to_string());
                min_w = min_w.min(w);
                max_w = max_w.max(w);
                min_h = min_h.min(h);
                max_h = max_h.max(h);
                emit(
                    i as u32 + 1,
                    "processing",
                    format!("{} — {}x{}", fname, w, h),
                    fname,
                );
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
                aspect_ratio: ((w as f64 / h.max(1) as f64) * 100.0).round() / 100.0,
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

    emit(
        total,
        "done",
        format!(
            "分析完成：{} 张图片，{} 种分辨率",
            valid_total, distinct_count
        ),
        String::new(),
    );

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

/// 导出分析结果到文件（支持 txt / csv / json）
#[tauri::command]
pub async fn export_resolution_report(
    result: ResolutionAnalyzeResult,
    format: String,
    save_path: String,
    input_path: String,
) -> Result<String, String> {
    let content = match format.as_str() {
        "json" => serde_json::to_string_pretty(&result)
            .map_err(|e| format!("序列化失败: {}", e))?,
        "csv" => build_csv(&result),
        _ => build_txt(&result, &input_path),
    };

    std::fs::write(&save_path, content).map_err(|e| format!("写入失败: {}", e))?;
    Ok(save_path)
}

fn build_csv(r: &ResolutionAnalyzeResult) -> String {
    let mut out = String::from("width,height,count,percent,aspect_ratio,aspect_label,is_rare\n");
    for g in &r.groups {
        out.push_str(&format!(
            "{},{},{},{:.2},{:.2},{},{}\n",
            g.width,
            g.height,
            g.count,
            g.percent,
            g.aspect_ratio,
            g.aspect_label,
            g.is_rare
        ));
    }
    // 稀有分辨率的文件清单单独附在末尾，保持主表可直接导入表格软件
    let rare: Vec<&ResolutionGroup> = r.groups.iter().filter(|g| g.is_rare).collect();
    if !rare.is_empty() {
        out.push_str("\n# rare resolution files\n");
        out.push_str("width,height,file\n");
        for g in rare {
            for f in &g.files {
                // CSV 转义：路径可能含逗号或引号
                let escaped = f.replace('"', "\"\"");
                out.push_str(&format!("{},{},\"{}\"\n", g.width, g.height, escaped));
            }
        }
    }
    out
}

fn build_txt(r: &ResolutionAnalyzeResult, input_path: &str) -> String {
    let mut out = String::new();
    out.push_str(&"=".repeat(56));
    out.push_str("\n 图片分辨率分布统计\n");
    out.push_str(&"=".repeat(56));
    out.push('\n');
    if !input_path.is_empty() {
        out.push_str(&format!("\n扫描目录: {}\n", input_path));
    }
    out.push_str(&format!(
        "\n共 {} 张图片，{} 种分辨率\n",
        r.total_images, r.distinct_count
    ));
    out.push_str(&format!(
        "宽度范围: {} - {} px    高度范围: {} - {} px\n",
        r.min_width, r.max_width, r.min_height, r.max_height
    ));

    for g in &r.groups {
        let label = if g.aspect_label.is_empty() {
            format!("{:.2}", g.aspect_ratio)
        } else {
            g.aspect_label.clone()
        };
        out.push_str(&format!(
            "\n▶ {} x {}  =>  {} 张  ({:.1}%, {})\n",
            g.width, g.height, g.count, g.percent, label
        ));
        if g.is_rare && !g.files.is_empty() {
            out.push_str("  [数量偏少，文件路径如下：]\n");
            for f in &g.files {
                out.push_str(&format!("    - {}\n", f));
            }
        }
    }

    if r.failed_count > 0 {
        out.push_str(&format!("\n\n无法读取的文件 ({}):\n", r.failed_count));
        for f in &r.failed_files {
            out.push_str(&format!("    - {}\n", f));
        }
    }

    out.push_str(&format!("\n{}\n", "=".repeat(56)));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_png(dir: &Path, name: &str, w: u32, h: u32) -> PathBuf {
        let p = dir.join(name);
        image::RgbImage::new(w, h).save(&p).unwrap();
        p
    }

    fn tmpdir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("purinbox_res_test_{}_{}", tag, std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn groups_sorted_by_count_and_marks_rare() {
        let d = tmpdir("sort");
        let mut files = Vec::new();
        // 3 张 100x100，1 张 50x50
        for i in 0..3 {
            files.push(write_png(&d, &format!("a{}.png", i), 100, 100));
        }
        files.push(write_png(&d, "b.png", 50, 50));

        let r = analyze_files(&files, 1, |_, _, _, _| {}).unwrap();

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
        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn reports_min_max_extents() {
        let d = tmpdir("extent");
        let files = vec![
            write_png(&d, "a.png", 100, 400),
            write_png(&d, "b.png", 300, 200),
        ];
        let r = analyze_files(&files, 0, |_, _, _, _| {}).unwrap();
        assert_eq!((r.min_width, r.max_width), (100, 300));
        assert_eq!((r.min_height, r.max_height), (200, 400));
        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn unreadable_files_counted_not_fatal() {
        let d = tmpdir("bad");
        let good = write_png(&d, "good.png", 64, 64);
        let bad = d.join("broken.png");
        std::fs::write(&bad, b"this is not an image").unwrap();

        let r = analyze_files(&[good, bad], 10, |_, _, _, _| {}).unwrap();
        assert_eq!(r.total_images, 1, "坏文件不应计入有效总数");
        assert_eq!(r.failed_count, 1);
        assert_eq!(r.groups.len(), 1);
        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn all_unreadable_is_error() {
        let d = tmpdir("allbad");
        let bad = d.join("x.png");
        std::fs::write(&bad, b"nope").unwrap();
        assert!(analyze_files(&[bad], 10, |_, _, _, _| {}).is_err());
        std::fs::remove_dir_all(&d).ok();
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

    #[test]
    fn csv_escapes_quotes_in_paths() {
        let r = ResolutionAnalyzeResult {
            total_images: 1,
            failed_count: 0,
            failed_files: vec![],
            distinct_count: 1,
            groups: vec![ResolutionGroup {
                width: 10,
                height: 20,
                count: 1,
                percent: 100.0,
                aspect_ratio: 0.5,
                aspect_label: "1:2".into(),
                is_rare: true,
                files: vec!["/a/we\"ird,name.png".into()],
            }],
            min_width: 10,
            max_width: 10,
            min_height: 20,
            max_height: 20,
        };
        let csv = build_csv(&r);
        assert!(csv.contains("\"/a/we\"\"ird,name.png\""), "引号需转义为双引号");
        assert!(csv.starts_with("width,height,count"));
    }

    #[test]
    fn txt_lists_rare_files_only() {
        let r = ResolutionAnalyzeResult {
            total_images: 5,
            failed_count: 0,
            failed_files: vec![],
            distinct_count: 2,
            groups: vec![
                ResolutionGroup { width:100,height:100,count:4,percent:80.0,aspect_ratio:1.0,
                    aspect_label:"1:1".into(),is_rare:false,files:vec![] },
                ResolutionGroup { width:7,height:9,count:1,percent:20.0,aspect_ratio:0.78,
                    aspect_label:String::new(),is_rare:true,files:vec!["/x/rare.png".into()] },
            ],
            min_width: 7, max_width: 100, min_height: 9, max_height: 100,
        };
        let txt = build_txt(&r, "/data/set");
        assert!(txt.contains("/data/set"));
        assert!(txt.contains("100 x 100"));
        assert!(txt.contains("/x/rare.png"), "稀有分组应列出文件");
        assert!(txt.contains("0.78"), "无常见比例标签时回退显示数值");
    }
}
