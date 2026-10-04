//! 配置文件路径解析。
//!
//! 配置统一存放在系统用户配置目录（Windows: `%APPDATA%\PurinBox`，
//! Linux: `~/.config/PurinBox`，macOS: `~/Library/Application Support/PurinBox`），
//! 避免按机安装（如 Program Files）时配置对所有本机用户可读、且普通用户无写权限的问题。
//!
//! 旧版本将配置写在 exe 同目录的 `config/` 下，读取时自动迁移（复制）到新位置；
//! 迁移失败不阻塞，回退读旧位置。
//!
//! 单元测试不读写用户真实的配置和数据：配置目录、旧配置目录和标签缓存目录都换成
//! 本测试进程独占的临时目录（见 `test_profile_dir`）。

use serde::de::DeserializeOwned;
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// 应用数据根目录，`models/`、`env/` 和旧版 `config/` 都在它下面：
/// - debug：仓库根目录；
/// - release：exe 所在目录。Linux 安装包（deb / rpm 装在 `/usr/bin`，AppImage 挂载为只读）
///   的 exe 目录不可写，这时改用用户数据目录下的 `PurinBox/`（`~/.local/share/PurinBox`）。
///
/// 和 `exe_root()` 不是一回事：macOS 上这里是 `.app/Contents/MacOS`，`exe_root()` 是 `.app` 的外层目录。
pub fn app_data_root() -> PathBuf {
    static ROOT: OnceLock<PathBuf> = OnceLock::new();
    ROOT.get_or_init(resolve_app_data_root).clone()
}

fn resolve_app_data_root() -> PathBuf {
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(Path::to_path_buf))
        .unwrap_or_else(|| PathBuf::from("."));
    if cfg!(debug_assertions) {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or(exe_dir)
    } else if cfg!(target_os = "linux") {
        let writable = dir_is_writable(&exe_dir);
        release_data_root(exe_dir, writable, dirs::data_local_dir())
    } else {
        exe_dir
    }
}

/// Linux 发行版的数据根目录：exe 目录可写（解压即用）时就用它，否则用 `<data_local_dir>/PurinBox`
fn release_data_root(
    exe_dir: PathBuf,
    exe_dir_writable: bool,
    data_local_dir: Option<PathBuf>,
) -> PathBuf {
    match data_local_dir {
        Some(dir) if !exe_dir_writable => dir.join("PurinBox"),
        _ => exe_dir,
    }
}

/// 在 `dir` 里试建再删一个文件。权限位判断不了当前用户能不能写（`/usr/bin` 对属主 root 可写），
/// 只读挂载也看不出来，只能实际试一次
fn dir_is_writable(dir: &Path) -> bool {
    let probe = dir.join(format!(".purinbox-write-test-{}", std::process::id()));
    match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&probe)
    {
        Ok(file) => {
            drop(file);
            std::fs::remove_file(&probe).is_ok()
        }
        // 上次试探没删掉的残留：能删掉它就说明目录可写
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            std::fs::remove_file(&probe).is_ok()
        }
        Err(_) => false,
    }
}

/// `<app_data_root>/models/<sub>`，如 `models_dir("tagger_models")`
pub fn models_dir(sub: &str) -> PathBuf {
    app_data_root().join("models").join(sub)
}

/// 单元测试构型下替代用户配置目录、旧配置目录和标签缓存目录的临时目录（整个测试进程共用）。
/// 里面没有配置文件时一切按默认值处理：代理不启用，翻译缓存和标签库为空
fn test_profile_dir() -> Option<PathBuf> {
    cfg!(test)
        .then(|| std::env::temp_dir().join(format!("purinbox_test_profile_{}", std::process::id())))
}

/// 旧配置目录（exe 同目录下的 config/；开发模式为仓库根目录下的 config/）。
/// 仅用于读取旧配置做迁移，不再写入。
fn legacy_config_dir() -> PathBuf {
    if let Some(dir) = test_profile_dir() {
        return dir.join("legacy_config");
    }
    app_data_root().join("config")
}

/// 新配置目录：系统用户配置目录下的 PurinBox/。
/// 极端情况下取不到用户配置目录时回退到旧目录（保证仍可读写）。
pub fn user_config_dir() -> PathBuf {
    if let Some(dir) = test_profile_dir() {
        return dir.join("config");
    }
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

/// 标签缓存的默认目录：Danbooru 标签库固定放在这里；翻译缓存库默认也在这里，
/// 用户在设置页改了缓存路径后翻译缓存库随之移走，标签库不动。
/// 默认落在用户数据目录（Windows: `%LOCALAPPDATA%\PurinBox\tagcache`，
/// Linux: `~/.local/share/PurinBox/tagcache`，macOS: `~/Library/Application Support/PurinBox/tagcache`）；
/// 检测到旧版 exe 同目录缓存时继续沿用旧位置，已下载的数据不搬家。
pub fn default_tagcache_dir() -> PathBuf {
    if let Some(dir) = test_profile_dir() {
        return dir.join("tagcache");
    }
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
/// 文件不存在时返回默认值；读取或解析失败时报 "{read_failed}: …" / "{parse_failed}: …"，
/// 两段文案由调用方写全，如 "读取代理配置失败"。
pub fn load_json_config<T: DeserializeOwned + Default>(
    file_name: &str,
    read_failed: &str,
    parse_failed: &str,
) -> Result<T, String> {
    read_json_file(&resolve_config_file(file_name), read_failed, parse_failed)
}

/// 同 `load_json_config`，但读取或解析失败也返回默认值
pub fn load_json_config_or_default<T: DeserializeOwned + Default>(file_name: &str) -> T {
    load_json_config(file_name, "", "").unwrap_or_default()
}

/// 把配置格式化成带缩进的 JSON，原子写入用户配置目录（只写新位置）。
/// 失败时报"创建配置目录失败: …"、"序列化失败: …"或 "{write_failed}: …"（如 "写入代理配置失败"）。
pub fn save_json_config<T: Serialize>(
    file_name: &str,
    value: &T,
    write_failed: &str,
) -> Result<(), String> {
    write_json_file(&user_config_dir(), file_name, value, write_failed)
}

fn read_json_file<T: DeserializeOwned + Default>(
    path: &Path,
    read_failed: &str,
    parse_failed: &str,
) -> Result<T, String> {
    if !path.exists() {
        return Ok(T::default());
    }
    let content = std::fs::read_to_string(path).map_err(|e| format!("{}: {}", read_failed, e))?;
    serde_json::from_str(&content).map_err(|e| format!("{}: {}", parse_failed, e))
}

fn write_json_file<T: Serialize>(
    dir: &Path,
    file_name: &str,
    value: &T,
    write_failed: &str,
) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("创建配置目录失败: {}", e))?;
    let json = serde_json::to_string_pretty(value).map_err(|e| format!("序列化失败: {}", e))?;
    write_file_atomic(&dir.join(file_name), json.as_bytes())
        .map_err(|e| format!("{}: {}", write_failed, e))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::test_support::TempDir;
    use serde::Deserialize;

    #[derive(Debug, Default, PartialEq, Serialize, Deserialize)]
    struct Sample {
        name: String,
        port: u16,
    }

    #[test]
    fn app_data_root_is_repo_root_in_debug() {
        let repo = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        assert_eq!(app_data_root(), repo);
        assert_eq!(
            models_dir("tagger_models"),
            repo.join("models").join("tagger_models")
        );
    }

    #[test]
    fn tests_never_resolve_into_real_user_dirs() {
        let profile = test_profile_dir().unwrap();
        assert!(profile.starts_with(std::env::temp_dir()));
        let resolved = [
            user_config_dir(),
            legacy_config_dir(),
            default_tagcache_dir(),
            resolve_config_file("proxy_config.json"),
            resolve_config_file("api_config.json"),
        ];
        for path in &resolved {
            assert!(path.starts_with(&profile), "{}", path.display());
        }
        let real_dirs = [dirs::config_dir(), dirs::data_local_dir()];
        for real in real_dirs.iter().flatten() {
            for path in &resolved {
                assert!(!path.starts_with(real), "{}", path.display());
            }
        }
        // 开发构型的旧配置目录（仓库 config/）不参与迁移
        let repo_config = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .join("config");
        assert!(!legacy_config_dir().starts_with(repo_config));
    }

    #[test]
    fn linux_release_uses_data_dir_only_when_exe_dir_is_read_only() {
        let exe = PathBuf::from("/usr/bin");
        let data = Some(PathBuf::from("/home/u/.local/share"));
        assert_eq!(
            release_data_root(exe.clone(), false, data.clone()),
            PathBuf::from("/home/u/.local/share/PurinBox")
        );
        assert_eq!(release_data_root(exe.clone(), true, data), exe);
        assert_eq!(release_data_root(exe.clone(), false, None), exe);
    }

    #[test]
    fn writability_is_probed_by_creating_a_file() {
        let dir = TempDir::new("config_paths_writable");
        assert!(dir_is_writable(&dir));
        // 试探文件不留在目录里，上次残留的也被清掉
        let leftover = dir.join(format!(".purinbox-write-test-{}", std::process::id()));
        std::fs::write(&leftover, "").unwrap();
        assert!(dir_is_writable(&dir));
        assert_eq!(std::fs::read_dir(&*dir).unwrap().count(), 0);
        assert!(!dir_is_writable(&dir.join("missing")));
    }

    #[cfg(unix)]
    #[test]
    fn read_only_dir_is_not_writable() {
        use std::os::unix::fs::PermissionsExt;
        let dir = TempDir::new("config_paths_read_only");
        let locked = dir.join("locked");
        std::fs::create_dir(&locked).unwrap();
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o555)).unwrap();
        // root 不受目录权限限制，这时模拟不出只读目录
        let simulated = std::fs::write(locked.join("probe"), "").is_err();
        let writable = dir_is_writable(&locked);
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).unwrap();
        if simulated {
            assert!(!writable);
        }
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
    fn json_config_write_then_read() {
        let dir = TempDir::new("config_paths_rw");
        let value = Sample {
            name: "a".into(),
            port: 7890,
        };
        write_json_file(&dir.join("nested"), "s.json", &value, "写入测试配置失败").unwrap();
        let path = dir.join("nested").join("s.json");
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            serde_json::to_string_pretty(&value).unwrap()
        );
        assert_eq!(
            read_json_file::<Sample>(&path, "读取测试配置失败", "解析测试配置失败").unwrap(),
            value
        );
    }

    #[test]
    fn json_config_missing_or_broken() {
        let dir = TempDir::new("config_paths_broken");
        let missing = dir.join("missing.json");
        assert_eq!(
            read_json_file::<Sample>(&missing, "读取代理配置失败", "解析代理配置失败").unwrap(),
            Sample::default()
        );

        let broken = dir.join("broken.json");
        std::fs::write(&broken, "{ not json").unwrap();
        let err =
            read_json_file::<Sample>(&broken, "读取代理配置失败", "解析代理配置失败").unwrap_err();
        assert!(err.starts_with("解析代理配置失败: "), "{}", err);

        // 路径是目录：存在但读不出来
        let err =
            read_json_file::<Sample>(&dir, "读取代理配置失败", "解析代理配置失败").unwrap_err();
        assert!(err.starts_with("读取代理配置失败: "), "{}", err);

        // 配置目录的位置被一个普通文件占着
        let err =
            write_json_file(&broken, "s.json", &Sample::default(), "写入代理配置失败").unwrap_err();
        assert!(err.starts_with("创建配置目录失败: "), "{}", err);

        // 目标位置是目录：写入这一步失败
        std::fs::create_dir(dir.join("taken.json")).unwrap();
        let err = write_json_file(
            &dir,
            "taken.json",
            &Sample::default(),
            "写入 Hugging Face 配置失败",
        )
        .unwrap_err();
        assert!(err.starts_with("写入 Hugging Face 配置失败: "), "{}", err);
    }
}
