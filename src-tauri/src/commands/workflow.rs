use std::path::Path;

/// 保存工作流 JSON 到指定路径
#[tauri::command]
pub async fn save_workflow(path: String, data: String) -> Result<(), String> {
    let file_path = Path::new(&path);

    // 自动创建父目录
    if let Some(parent) = file_path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("创建目录失败: {}", e))?;
    }

    std::fs::write(file_path, data)
        .map_err(|e| format!("写入工作流失败: {}", e))?;

    Ok(())
}

/// 加载工作流 JSON
#[tauri::command]
pub async fn load_workflow(path: String) -> Result<String, String> {
    let file_path = Path::new(&path);

    if !file_path.exists() {
        return Err(format!("工作流文件不存在: {}", path));
    }

    std::fs::read_to_string(file_path)
        .map_err(|e| format!("读取工作流失败: {}", e))
}

/// 清理工作流临时目录（{dir}/.workflow_temp），返回释放的字节数。
///
/// 取消工作流时刚强杀完当前节点的子进程（Python/NCNN），Windows 上其打开的
/// 文件句柄可能尚未释放，首次删除会报拒绝访问——失败后短暂等待并重试。
#[tauri::command]
pub async fn cleanup_workflow_temp(dir: String) -> Result<u64, String> {
    tokio::task::spawn_blocking(move || {
        let temp_dir = Path::new(&dir).join(".workflow_temp");

        if !temp_dir.exists() {
            return Ok(0);
        }

        let bytes_freed = dir_size(&temp_dir);

        let mut last_err = String::new();
        for attempt in 0..3 {
            if attempt > 0 {
                std::thread::sleep(std::time::Duration::from_millis(400));
            }
            match std::fs::remove_dir_all(&temp_dir) {
                Ok(_) => return Ok(bytes_freed),
                Err(e) => last_err = e.to_string(),
            }
            if !temp_dir.exists() {
                return Ok(bytes_freed);
            }
        }
        Err(format!("清理临时目录失败: {}", last_err))
    })
    .await
    .map_err(|e| format!("清理任务执行失败: {}", e))?
}

/// 递归计算目录大小（字节）
fn dir_size(path: &Path) -> u64 {
    if path.is_file() {
        return path.metadata().map(|m| m.len()).unwrap_or(0);
    }

    let mut total = 0u64;
    if let Ok(entries) = std::fs::read_dir(path) {
        for entry in entries.filter_map(|e| e.ok()) {
            total += dir_size(&entry.path());
        }
    }
    total
}

/// 把输入目录里与输出目录图片同名（stem）的标签文件（.txt/.json/.caption）带到输出目录。
/// 图像处理节点只搬图片；打标节点在上游时，标签会被留在临时目录里随清理丢失。
/// 幂等：目标已存在则跳过；失败不阻断工作流（调用方 catch）。
#[tauri::command]
pub fn carry_tag_sidecars(
    input_path: String,
    output_path: String,
    recursive: bool,
) -> Result<u32, String> {
    let input = Path::new(&input_path);
    let output = Path::new(&output_path);
    if !input.is_dir() || !output.is_dir() {
        return Ok(0);
    }
    // 先给输入侧的标签文件按相对路径建索引：纯图片数据集（最常见）直接零开销返回，
    // 避免对输出目录每张图做 3 次存在性探测
    let mut avail: std::collections::HashSet<std::path::PathBuf> = std::collections::HashSet::new();
    let walker = if recursive {
        walkdir::WalkDir::new(input)
    } else {
        walkdir::WalkDir::new(input).max_depth(1)
    };
    for entry in walker.into_iter().filter_map(|e| e.ok()) {
        let p = entry.path();
        if p.is_file()
            && matches!(
                p.extension().and_then(|e| e.to_str()),
                Some("txt") | Some("json") | Some("caption")
            )
        {
            if let Ok(rel) = p.strip_prefix(input) {
                avail.insert(rel.to_path_buf());
            }
        }
    }
    if avail.is_empty() {
        return Ok(0);
    }

    let images = super::collect_image_files_with_recursive(output, recursive)?;
    let mut copied = 0u32;
    for img in images {
        // 输出图片相对输出根的位置，映射回输入根找同名标签
        let rel = img.strip_prefix(output).unwrap_or(&img);
        for ext in ["txt", "json", "caption"] {
            let rel_sc = rel.with_extension(ext);
            if !avail.contains(&rel_sc) {
                continue;
            }
            let src = input.join(&rel_sc);
            let dst = img.with_extension(ext);
            if !dst.exists() && src != dst && std::fs::copy(&src, &dst).is_ok() {
                copied += 1;
            }
        }
    }
    Ok(copied)
}
