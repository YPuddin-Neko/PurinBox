//! 配置文件路径解析。
//!
//! 配置统一存放在系统用户配置目录（Windows: `%APPDATA%\PurinBox`，
//! Linux: `~/.config/PurinBox`，macOS: `~/Library/Application Support/PurinBox`），
//! 避免按机安装（如 Program Files）时配置对所有本机用户可读、且普通用户无写权限的问题。
//!
//! 旧版本将配置写在 exe 同目录的 `config/` 下，读取时自动迁移（复制）到新位置；
//! 迁移失败不阻塞，回退读旧位置。

use serde::de::DeserializeOwned;
use serde::Serialize;
use std::path::{Path, PathBuf};

/// 应用数据根目录：release 为 exe 所在目录，debug 为仓库根目录。
/// `models/`、`env/` 和旧版 `config/` 都在它下面。
/// 和 `exe_root()` 不是一回事：macOS 上这里是 `.app/Contents/MacOS`，`exe_root()` 是 `.app` 的外层目录。
pub fn app_data_root() -> PathBuf {
    let exe_dir = || {
        std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(Path::to_path_buf))
            .unwrap_or_else(|| PathBuf::from("."))
    };
    if cfg!(debug_assertions) {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(exe_dir)
    } else {
        exe_dir()
    }
}

/// `<app_data_root>/models/<sub>`，如 `models_dir("tagger_models")`
pub fn models_dir(sub: &str) -> PathBuf {
    app_data_root().join("models").join(sub)
}

/// 旧配置目录（exe 同目录下的 config/；开发模式为仓库根目录下的 config/）。
/// 仅用于读取旧配置做迁移，不再写入。
fn legacy_config_dir() -> PathBuf {
    app_data_root().join("config")
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

/// 软件根目录（exe 所在目录，macOS .app 则取 bundle 外层）。
fn exe_root() -> PathBuf {
    let exe = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("."));
    let exe_dir = exe
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."))
        .to_path_buf();
    // macOS .app bundle: …/Foo.app/Contents/MacOS/exe → 取 Foo.app 所在目录
    if cfg!(target_os = "macos") {
        if let Some(contents) = exe_dir.parent() {
            if let Some(app_bundle) = contents.parent() {
                if app_bundle.extension().map(|e| e == "app").unwrap_or(false) {
                    return app_bundle.parent().unwrap_or(&exe_dir).to_path_buf();
                }
            }
        }
    }
    exe_dir
}

/// 旧标签缓存目录（exe 同目录）。仅用于判断已有数据位置，新安装不再默认写入：
/// 按机安装（如 Program Files）时普通用户对 exe 目录没有写权限。
fn legacy_tagcache_dir() -> PathBuf {
    if cfg!(target_os = "windows") {
        exe_root().join("data").join("tagcache")
    } else {
        exe_root().join("tagcache")
    }
}

/// 标签缓存目录（翻译缓存库与 Danbooru 标签库共用）。
/// 默认落在用户数据目录（Windows: `%LOCALAPPDATA%\PurinBox\tagcache`，
/// Linux: `~/.local/share/PurinBox/tagcache`，macOS: `~/Library/Application Support/PurinBox/tagcache`）；
/// 检测到旧版 exe 同目录缓存时继续沿用旧位置，已下载的数据不搬家。
pub fn default_tagcache_dir() -> PathBuf {
    let new_dir = dirs::data_local_dir()
        .map(|p| p.join("PurinBox").join("tagcache"))
        .unwrap_or_else(legacy_tagcache_dir);
    let legacy = legacy_tagcache_dir();
    if legacy != new_dir && legacy.exists() {
        return legacy;
    }
    new_dir
}

/// 原子写入文件：先写同目录临时文件再 rename，避免进程在写入中途被杀时留下截断文件
pub fn write_file_atomic(path: &std::path::Path, contents: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    use std::sync::atomic::{AtomicU32, Ordering};

    static TMP_SEQ: AtomicU32 = AtomicU32::new(0);
    let file_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("config");
    let tmp = path.with_file_name(format!(
        ".{}.tmp-{}-{}",
        file_name,
        std::process::id(),
        TMP_SEQ.fetch_add(1, Ordering::Relaxed)
    ));

    let write_result = (|| {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(contents)?;
        f.sync_all()
    })();
    if let Err(e) = write_result {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    if let Err(e) = std::fs::rename(&tmp, path) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    Ok(())
}

/// 配置里存密码/令牌用的 base64（只为不让明文直接出现在文件里，不是加密）
pub fn b64_encode(s: &str) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(s.as_bytes())
}

/// `b64_encode` 的逆操作；空串、非法 base64 或解出来不是 UTF-8 时返回空串
pub fn b64_decode(s: &str) -> String {
    use base64::Engine;
    if s.is_empty() {
        return String::new();
    }
    base64::engine::general_purpose::STANDARD
        .decode(s)
        .ok()
        .and_then(|b| String::from_utf8(b).ok())
        .unwrap_or_default()
}

/// 读取 JSON 配置（经 `resolve_config_file` 定位，含旧位置迁移）。
/// 文件不存在时返回默认值；读取或解析失败时报"读取{what}失败: …"/"解析{what}失败: …"。
pub fn load_json_config<T: DeserializeOwned + Default>(
    file_name: &str,
    what: &str,
) -> Result<T, String> {
    read_json_file(&resolve_config_file(file_name), what)
}

/// 同 `load_json_config`，但读取或解析失败也返回默认值
pub fn load_json_config_or_default<T: DeserializeOwned + Default>(file_name: &str) -> T {
    load_json_config(file_name, "").unwrap_or_default()
}

/// 把配置格式化成带缩进的 JSON，原子写入用户配置目录（只写新位置）。
/// 失败时报"创建配置目录失败: …"、"序列化失败: …"或"写入{what}失败: …"。
pub fn save_json_config<T: Serialize>(
    file_name: &str,
    value: &T,
    what: &str,
) -> Result<(), String> {
    write_json_file(&user_config_dir(), file_name, value, what)
}

fn read_json_file<T: DeserializeOwned + Default>(path: &Path, what: &str) -> Result<T, String> {
    if !path.exists() {
        return Ok(T::default());
    }
    let content = std::fs::read_to_string(path)
        .map_err(|e| format!("{}: {}", failure_text("读取", what), e))?;
    serde_json::from_str(&content).map_err(|e| format!("{}: {}", failure_text("解析", what), e))
}

fn write_json_file<T: Serialize>(
    dir: &Path,
    file_name: &str,
    value: &T,
    what: &str,
) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("创建配置目录失败: {}", e))?;
    let json = serde_json::to_string_pretty(value).map_err(|e| format!("序列化失败: {}", e))?;
    write_file_atomic(&dir.join(file_name), json.as_bytes())
        .map_err(|e| format!("{}: {}", failure_text("写入", what), e))
}

/// "{verb}{what}失败"；`what` 首尾是 ASCII 字母数字时与中文之间补一个空格，
/// 如 "写入 Hugging Face 配置失败"
fn failure_text(verb: &str, what: &str) -> String {
    let gap = |c: Option<char>| {
        if c.is_some_and(|c| c.is_ascii_alphanumeric()) {
            " "
        } else {
            ""
        }
    };
    format!(
        "{}{}{}{}失败",
        verb,
        gap(what.chars().next()),
        what,
        gap(what.chars().last())
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;

    #[derive(Debug, Default, PartialEq, Serialize, Deserialize)]
    struct Sample {
        name: String,
        port: u16,
    }

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "purinbox_config_paths_{}_{}",
            tag,
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn app_data_root_is_repo_root_in_debug() {
        let repo = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        assert_eq!(app_data_root(), repo);
        assert_eq!(
            models_dir("tagger_models"),
            repo.join("models").join("tagger_models")
        );
        assert_eq!(legacy_config_dir(), repo.join("config"));
    }

    #[test]
    fn b64_round_trip_and_fallbacks() {
        assert_eq!(b64_encode("p@ss 密码"), "cEBzcyDlr4bnoIE=");
        assert_eq!(b64_decode(&b64_encode("p@ss 密码")), "p@ss 密码");
        assert_eq!(b64_decode(""), "");
        assert_eq!(b64_decode("不是base64"), "");
        // 合法 base64 但不是 UTF-8
        assert_eq!(b64_decode("/w=="), "");
    }

    #[test]
    fn failure_text_spaces_latin_labels() {
        assert_eq!(failure_text("写入", "代理配置"), "写入代理配置失败");
        assert_eq!(failure_text("读取", "配置"), "读取配置失败");
        assert_eq!(
            failure_text("写入", "Hugging Face 配置"),
            "写入 Hugging Face 配置失败"
        );
        assert_eq!(failure_text("写入", "API Key"), "写入 API Key 失败");
    }

    #[test]
    fn json_config_write_then_read() {
        let dir = temp_dir("rw");
        let value = Sample {
            name: "a".into(),
            port: 7890,
        };
        write_json_file(&dir.join("nested"), "s.json", &value, "测试配置").unwrap();
        let path = dir.join("nested").join("s.json");
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            serde_json::to_string_pretty(&value).unwrap()
        );
        assert_eq!(read_json_file::<Sample>(&path, "测试配置").unwrap(), value);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn json_config_missing_or_broken() {
        let dir = temp_dir("broken");
        std::fs::create_dir_all(&dir).unwrap();
        let missing = dir.join("missing.json");
        assert_eq!(
            read_json_file::<Sample>(&missing, "代理配置").unwrap(),
            Sample::default()
        );

        let broken = dir.join("broken.json");
        std::fs::write(&broken, "{ not json").unwrap();
        let err = read_json_file::<Sample>(&broken, "代理配置").unwrap_err();
        assert!(err.starts_with("解析代理配置失败: "), "{}", err);

        // 路径是目录：存在但读不出来
        let err = read_json_file::<Sample>(&dir, "代理配置").unwrap_err();
        assert!(err.starts_with("读取代理配置失败: "), "{}", err);

        // 配置目录的位置被一个普通文件占着
        let err = write_json_file(&broken, "s.json", &Sample::default(), "代理配置").unwrap_err();
        assert!(err.starts_with("创建配置目录失败: "), "{}", err);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
