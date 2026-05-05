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
            llm_proxy: true,
            proxy_type: "http".to_string(),
            host: "127.0.0.1".to_string(),
            port: 7890,
            username: String::new(),
            password_encoded: String::new(),
        }
    }
}

/// 获取配置目录
fn get_config_dir() -> PathBuf {
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

fn config_path() -> PathBuf {
    get_config_dir().join("proxy_config.json")
}

fn encode(s: &str) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(s.as_bytes())
}

fn decode(s: &str) -> String {
    use base64::Engine;
    if s.is_empty() { return String::new(); }
    base64::engine::general_purpose::STANDARD
        .decode(s).ok()
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
    let dir = get_config_dir();
    if !dir.exists() {
        std::fs::create_dir_all(&dir).map_err(|e| format!("创建配置目录失败: {}", e))?;
    }
    let config = ProxyConfig {
        enabled,
        llm_proxy,
        proxy_type,
        host,
        port,
        username,
        password_encoded: encode(&password),
    };
    let json = serde_json::to_string_pretty(&config)
        .map_err(|e| format!("序列化失败: {}", e))?;
    std::fs::write(config_path(), json)
        .map_err(|e| format!("写入代理配置失败: {}", e))?;
    Ok(())
}

/// 加载代理配置
#[tauri::command]
pub fn load_proxy_config() -> Result<(bool, bool, String, String, u16, String, String), String> {
    let path = config_path();
    if !path.exists() {
        let d = ProxyConfig::default();
        return Ok((d.enabled, d.llm_proxy, d.proxy_type, d.host, d.port, d.username, String::new()));
    }
    let content = std::fs::read_to_string(&path)
        .map_err(|e| format!("读取代理配置失败: {}", e))?;
    let config: ProxyConfig = serde_json::from_str(&content)
        .map_err(|e| format!("解析代理配置失败: {}", e))?;
    Ok((config.enabled, config.llm_proxy, config.proxy_type, config.host, config.port, config.username, decode(&config.password_encoded)))
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

    let url = match cfg.proxy_type.as_str() {
        "socks5" => format!("socks5://{}:{}", cfg.host, cfg.port),
        _ => format!("http://{}:{}", cfg.host, cfg.port),
    };

    if let Ok(mut proxy) = reqwest::Proxy::all(&url) {
        if !cfg.username.is_empty() {
            let password = decode(&cfg.password_encoded);
            proxy = proxy.basic_auth(&cfg.username, &password);
        }
        builder.proxy(proxy)
    } else {
        builder
    }
}

/// 构建带代理的 reqwest Client（通用：翻译、模型下载等）
pub fn build_http_client() -> reqwest::ClientBuilder {
    let cfg = load_proxy_config_internal();
    apply_proxy(reqwest::Client::builder(), &cfg)
}

/// 构建带代理的 reqwest Client（LLM 专用：仅当 llm_proxy 开启时使用代理）
pub fn build_http_client_for_llm() -> reqwest::ClientBuilder {
    let cfg = load_proxy_config_internal();
    if cfg.llm_proxy {
        apply_proxy(reqwest::Client::builder(), &cfg)
    } else {
        reqwest::Client::builder()
    }
}
