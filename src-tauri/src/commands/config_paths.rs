//! 配置文件路径解析。
//!
//! 配置统一存放在系统用户配置目录（Windows: `%APPDATA%\PurinBox`，
//! Linux: `~/.config/PurinBox`，macOS: `~/Library/Application Support/PurinBox`），
//! 避免按机安装（如 Program Files）时配置对所有本机用户可读、且普通用户无写权限的问题。
//!
//! 旧版本将配置写在 exe 同目录的 `config/` 下，读取时自动迁移（复制）到新位置；
//! 迁移失败不阻塞，回退读旧位置。

use std::path::PathBuf;

/// 旧配置目录（exe 同目录下的 config/；开发模式为仓库根目录下的 config/）。
/// 仅用于读取旧配置做迁移，不再写入。
fn legacy_config_dir() -> PathBuf {
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

    base.join("config")
}

/// 新配置目录：系统用户配置目录下的 PurinBox/。
/// 极端情况下取不到用户配置目录时回退到旧目录（保证仍可读写）。
pub fn user_config_dir() -> PathBuf {
    dirs::config_dir()
        .map(|p| p.join("PurinBox"))
        .unwrap_or_else(legacy_config_dir)
}

/// 解析配置文件的实际读取路径，必要时自动从旧位置迁移：
/// 1. 新位置已有该文件 → 直接用新位置；
/// 2. 新位置没有但旧位置有 → 尝试复制到新位置（成功用新位置，失败回退旧位置，不阻塞）；
/// 3. 两边都没有 → 返回新位置（调用方按"配置不存在"处理）。
pub fn resolve_config_file(file_name: &str) -> PathBuf {
    let dir = user_config_dir();
    let new_path = dir.join(file_name);
    if new_path.exists() {
        return new_path;
    }

    let old_path = legacy_config_dir().join(file_name);
    if old_path.exists() {
        let migrated = std::fs::create_dir_all(&dir)
            .and_then(|_| std::fs::copy(&old_path, &new_path))
            .is_ok();
        if migrated {
            return new_path;
        }
        return old_path;
    }

    new_path
}
