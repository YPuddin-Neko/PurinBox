use serde::{Deserialize, Serialize};
use std::path::Path;

/// Workflow 文件信息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowInfo {
    /// 工作流名称（不含扩展名）
    pub name: String,
    /// 文件完整路径
    pub path: String,
    /// 最后修改时间（Unix 时间戳，秒）
    pub modified: u64,
}

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

/// 列出目录中的 .purin 工作流文件
#[tauri::command]
pub async fn list_workflows(dir: String) -> Result<Vec<WorkflowInfo>, String> {
    let dir_path = Path::new(&dir);

    if !dir_path.exists() || !dir_path.is_dir() {
        return Ok(Vec::new());
    }

    let mut workflows = Vec::new();

    let entries = std::fs::read_dir(dir_path)
        .map_err(|e| format!("读取目录失败: {}", e))?;

    for entry in entries.filter_map(|e| e.ok()) {
        let p = entry.path();
        if !p.is_file() {
            continue;
        }

        let ext_match = p
            .extension()
            .map(|ext| ext.to_string_lossy().to_lowercase() == "purin")
            .unwrap_or(false);

        if !ext_match {
            continue;
        }

        let name = p
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();

        let modified = p
            .metadata()
            .and_then(|m| m.modified())
            .map(|t| {
                t.duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs()
            })
            .unwrap_or(0);

        workflows.push(WorkflowInfo {
            name,
            path: p.to_string_lossy().to_string(),
            modified,
        });
    }

    // 按修改时间倒序排列（最新的在前）
    workflows.sort_by(|a, b| b.modified.cmp(&a.modified));

    Ok(workflows)
}

/// 清理工作流临时目录，返回释放的字节数
#[tauri::command]
pub async fn cleanup_workflow_temp(dir: String) -> Result<u64, String> {
    let temp_dir = Path::new(&dir).join(".workflow_temp");

    if !temp_dir.exists() {
        return Ok(0);
    }

    let bytes_freed = dir_size(&temp_dir);

    std::fs::remove_dir_all(&temp_dir)
        .map_err(|e| format!("清理临时目录失败: {}", e))?;

    Ok(bytes_freed)
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
