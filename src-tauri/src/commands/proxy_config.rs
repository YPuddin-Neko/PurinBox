use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// 代理配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProxyConfig {
    /// 是否启用代理
    pub enabled: bool,
    /// LLM 相关功能是否使用代理
    #[serde(default)]
    pub llm_proxy: bool,
    /// 代理类型: "http" | "socks5"
    pub proxy_type: String,
    /// 代理地址（如 127.0.0.1）
    pub host: String,
    /// 代理端口（如 7890）
    pub port: u16,
    /// 代理用户名（可选）
    pub username: String,
    /// 代理密码（可选，base64 编码）
    pub password_encoded: String,
}

impl Default for ProxyConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            llm_proxy: false,
            proxy_type: "http".to_string(),
            host: "127.0.0.1".to_string(),
            port: 7890,
            username: String::new(),
            password_encoded: String::new(),
        }
    }
}

const CONFIG_FILE: &str = "proxy_config.json";

/// 配置文件读取路径（含旧 exe 同目录 config/ 的自动迁移）
fn config_path() -> PathBuf {
    super::config_paths::resolve_config_file(CONFIG_FILE)
}

fn encode(s: &str) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(s.as_bytes())
}

fn decode(s: &str) -> String {
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

/// 保存代理配置
#[tauri::command]
pub fn save_proxy_config(
    enabled: bool,
    llm_proxy: bool,
    proxy_type: String,
    host: String,
    port: u16,
    username: String,
    password: String,
) -> Result<(), String> {
    let dir = super::config_paths::user_config_dir();
    std::fs::create_dir_all(&dir).map_err(|e| format!("创建配置目录失败: {}", e))?;
    let config = ProxyConfig {
        enabled,
        llm_proxy,
        proxy_type,
        host,
        port,
        username,
        password_encoded: encode(&password),
    };
    let json = serde_json::to_string_pretty(&config).map_err(|e| format!("序列化失败: {}", e))?;
    std::fs::write(dir.join(CONFIG_FILE), json).map_err(|e| format!("写入代理配置失败: {}", e))?;
    Ok(())
}

/// 加载代理配置
#[tauri::command]
#[allow(clippy::type_complexity)]
pub fn load_proxy_config() -> Result<(bool, bool, String, String, u16, String, String), String> {
    let path = config_path();
    if !path.exists() {
        let d = ProxyConfig::default();
        return Ok((
            d.enabled,
            d.llm_proxy,
            d.proxy_type,
            d.host,
            d.port,
            d.username,
            String::new(),
        ));
    }
    let content = std::fs::read_to_string(&path).map_err(|e| format!("读取代理配置失败: {}", e))?;
    let config: ProxyConfig =
        serde_json::from_str(&content).map_err(|e| format!("解析代理配置失败: {}", e))?;
    Ok((
        config.enabled,
        config.llm_proxy,
        config.proxy_type,
        config.host,
        config.port,
        config.username,
        decode(&config.password_encoded),
    ))
}

/// 加载代理配置（内部使用，不是 tauri 命令）
pub fn load_proxy_config_internal() -> ProxyConfig {
    let path = config_path();
    if !path.exists() {
        return ProxyConfig::default();
    }
    std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

/// 为 ClientBuilder 应用代理配置
fn apply_proxy(builder: reqwest::ClientBuilder, cfg: &ProxyConfig) -> reqwest::ClientBuilder {
    if !cfg.enabled || cfg.host.is_empty() || cfg.port == 0 {
        return builder;
    }

    let is_socks = cfg.proxy_type == "socks5";
    let url = if is_socks {
        // SOCKS5 的认证必须放在 URL userinfo 中：Proxy::basic_auth 仅对 HTTP 代理生效
        if cfg.username.is_empty() {
            format!("socks5://{}:{}", cfg.host, cfg.port)
        } else {
            let password = decode(&cfg.password_encoded);
            format!(
                "socks5://{}:{}@{}:{}",
                urlencoding::encode(&cfg.username),
                urlencoding::encode(&password),
                cfg.host,
                cfg.port
            )
        }
    } else {
        format!("http://{}:{}", cfg.host, cfg.port)
    };

    match reqwest::Proxy::all(&url) {
        Ok(mut proxy) => {
            if !is_socks && !cfg.username.is_empty() {
                let password = decode(&cfg.password_encoded);
                proxy = proxy.basic_auth(&cfg.username, &password);
            }
            builder.proxy(proxy)
        }
        Err(e) => {
            // 代理无效时不能静默忽略：所有网络请求会绕过代理直连
            eprintln!(
                "[PurinBox] 代理配置无效，本次请求将直连（{} {}:{}）: {}",
                cfg.proxy_type, cfg.host, cfg.port, e
            );
            builder
        }
    }
}

/// 给需要联网的子进程（pip 等）注入应用内代理环境变量。
/// 应用内代理≠系统代理：clash 非系统代理模式下 reqwest 下载都正常，
/// pip 直连 PyPI 却会失败，表现为"下载都行、装依赖必挂"。
/// SOCKS5 不注入：pip 需要 pysocks 才认 socks 代理，注入反而让它报缺依赖错误。
pub fn apply_proxy_env(cmd: &mut std::process::Command) {
    let cfg = load_proxy_config_internal();
    if !cfg.enabled || cfg.host.is_empty() || cfg.port == 0 || cfg.proxy_type == "socks5" {
        return;
    }
    let url = if cfg.username.is_empty() {
        format!("http://{}:{}", cfg.host, cfg.port)
    } else {
        let password = decode(&cfg.password_encoded);
        format!(
            "http://{}:{}@{}:{}",
            urlencoding::encode(&cfg.username),
            urlencoding::encode(&password),
            cfg.host,
            cfg.port
        )
    };
    cmd.env("HTTP_PROXY", &url)
        .env("HTTPS_PROXY", &url)
        .env("http_proxy", &url)
        .env("https_proxy", &url);
}

/// 构建带代理的 reqwest Client（通用：翻译、模型下载等）
pub fn build_http_client() -> reqwest::ClientBuilder {
    let cfg = load_proxy_config_internal();
    apply_proxy(reqwest::Client::builder().user_agent("PurinBox"), &cfg)
}

/// 构建带代理的 reqwest Client（LLM 专用：仅当 llm_proxy 开启时使用代理）
///
/// 必须设置超时：reqwest 默认无限等待，网关/代理接受连接后不回包时
/// 请求会永久挂起，并发为 1 时整条打标/精修管线就此停摆。
/// read_timeout 取 300s 是因为非流式 LLM 请求在服务端生成完之前不回首字节。
pub fn build_http_client_for_llm() -> reqwest::ClientBuilder {
    let cfg = load_proxy_config_internal();
    let builder = reqwest::Client::builder()
        .user_agent("PurinBox")
        .connect_timeout(std::time::Duration::from_secs(20))
        .read_timeout(std::time::Duration::from_secs(300));
    if cfg.llm_proxy {
        apply_proxy(builder, &cfg)
    } else {
        builder
    }
}
