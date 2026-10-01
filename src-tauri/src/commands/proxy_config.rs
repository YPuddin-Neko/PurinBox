use serde::{Deserialize, Serialize};

use super::config_paths::{
    b64_decode, b64_encode, load_json_config, load_json_config_or_default, save_json_config,
};

const USER_AGENT: &str = concat!("PurinBox/", env!("CARGO_PKG_VERSION"));

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
    let config = ProxyConfig {
        enabled,
        llm_proxy,
        proxy_type,
        host,
        port,
        username,
        password_encoded: b64_encode(&password),
    };
    save_json_config(CONFIG_FILE, &config, "代理配置")
}

/// 加载代理配置
#[tauri::command]
#[allow(clippy::type_complexity)]
pub fn load_proxy_config() -> Result<(bool, bool, String, String, u16, String, String), String> {
    let config: ProxyConfig = load_json_config(CONFIG_FILE, "代理配置")?;
    Ok((
        config.enabled,
        config.llm_proxy,
        config.proxy_type,
        config.host,
        config.port,
        config.username,
        b64_decode(&config.password_encoded),
    ))
}

/// 读取代理配置；文件缺失、读取或解析失败时返回默认配置（不启用代理）
pub fn load_proxy_config_internal() -> ProxyConfig {
    load_json_config_or_default(CONFIG_FILE)
}

fn usable(cfg: &ProxyConfig) -> bool {
    cfg.enabled && !cfg.host.is_empty() && cfg.port != 0
}

fn proxy_url(cfg: &ProxyConfig, scheme: &str, credentials: bool) -> String {
    if !credentials || cfg.username.is_empty() {
        format!("{}://{}:{}", scheme, cfg.host, cfg.port)
    } else {
        format!(
            "{}://{}:{}@{}:{}",
            scheme,
            urlencoding::encode(&cfg.username),
            urlencoding::encode(&b64_decode(&cfg.password_encoded)),
            cfg.host,
            cfg.port
        )
    }
}

/// 为 ClientBuilder 应用代理配置
fn apply_proxy(builder: reqwest::ClientBuilder, cfg: &ProxyConfig) -> reqwest::ClientBuilder {
    if !usable(cfg) {
        return builder;
    }

    let is_socks = cfg.proxy_type == "socks5";
    // socks5h 由代理解析域名；HTTP 凭据单独通过 basic_auth 传递。
    let url = proxy_url(cfg, if is_socks { "socks5h" } else { "http" }, is_socks);

    match reqwest::Proxy::all(&url) {
        Ok(mut proxy) => {
            if !is_socks && !cfg.username.is_empty() {
                let password = b64_decode(&cfg.password_encoded);
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
    if !usable(&cfg) || cfg.proxy_type == "socks5" {
        return;
    }
    let url = proxy_url(&cfg, "http", true);
    cmd.env("HTTP_PROXY", &url)
        .env("HTTPS_PROXY", &url)
        .env("http_proxy", &url)
        .env("https_proxy", &url);
}

/// 构建带代理的 reqwest Client（通用：翻译、模型下载等）
pub fn build_http_client() -> reqwest::ClientBuilder {
    let cfg = load_proxy_config_internal();
    apply_proxy(reqwest::Client::builder().user_agent(USER_AGENT), &cfg)
}

/// 构建带代理的 reqwest Client（LLM 专用：仅当 llm_proxy 开启时使用代理）
///
/// 必须设置超时：reqwest 默认无限等待，网关/代理接受连接后不回包时
/// 请求会永久挂起，并发为 1 时整条打标/精修管线就此停摆。
/// read_timeout 取 300s 是因为非流式 LLM 请求在服务端生成完之前不回首字节。
pub fn build_http_client_for_llm() -> reqwest::ClientBuilder {
    let cfg = load_proxy_config_internal();
    let builder = reqwest::Client::builder()
        .user_agent(USER_AGENT)
        .connect_timeout(std::time::Duration::from_secs(20))
        .read_timeout(std::time::Duration::from_secs(300));
    if cfg.llm_proxy {
        apply_proxy(builder, &cfg)
    } else {
        builder
    }
}

#[cfg(test)]
mod proxy_helper_tests {
    use super::*;

    #[test]
    fn proxy_url_preserves_auth_encoding_and_scheme() {
        let mut config = ProxyConfig {
            enabled: true,
            username: "user@example".into(),
            password_encoded: b64_encode("p:a ss"),
            ..Default::default()
        };
        assert!(usable(&config));
        assert_eq!(
            proxy_url(&config, "socks5h", true),
            "socks5h://user%40example:p%3Aa%20ss@127.0.0.1:7890"
        );
        assert_eq!(proxy_url(&config, "http", false), "http://127.0.0.1:7890");
        config.username.clear();
        assert_eq!(proxy_url(&config, "http", true), "http://127.0.0.1:7890");
        config.port = 0;
        assert!(!usable(&config));
        config.port = 7890;
        config.enabled = false;
        assert!(!usable(&config));
    }
}

#[cfg(test)]
mod proxy_e2e_tests {
    use super::*;

    /// 测试期间写入真实配置文件，结束时（含断言失败）恢复原内容
    struct RestoreConfig(Option<Vec<u8>>);

    fn config_path() -> std::path::PathBuf {
        crate::commands::config_paths::resolve_config_file(CONFIG_FILE)
    }

    impl Drop for RestoreConfig {
        fn drop(&mut self) {
            let path = config_path();
            match &self.0 {
                Some(original) => {
                    let _ = std::fs::write(&path, original);
                }
                None => {
                    let _ = std::fs::remove_file(&path);
                }
            }
        }
    }

    /// 端到端：保存 socks5 配置(落盘到真实配置文件) → 用与下载相同的客户端构建
    /// 路径走代理拉取 HF 文件。同时验证"保存真的写盘了"和"reqwest 能走通 socks5"。
    /// 依赖局域网 socks5 代理和外网，且会临时改写真实配置，只在显式指定时运行：
    /// cargo test -- --ignored save_then_download_via_socks5
    #[tokio::test]
    #[ignore = "依赖局域网 socks5 代理和外网，且会临时改写真实代理配置"]
    async fn save_then_download_via_socks5() {
        let _restore = RestoreConfig(std::fs::read(config_path()).ok());
        save_proxy_config(
            true,
            false,
            "socks5".into(),
            "192.168.0.25".into(),
            7897,
            String::new(),
            String::new(),
        )
        .unwrap();
        // 读回验证落盘
        let (enabled, _, ptype, host, port, _, _) = load_proxy_config().unwrap();
        assert!(enabled);
        assert_eq!(ptype, "socks5");
        assert_eq!(host, "192.168.0.25");
        assert_eq!(port, 7897);

        let client = build_http_client()
            .connect_timeout(std::time::Duration::from_secs(10))
            .read_timeout(std::time::Duration::from_secs(30))
            .build()
            .unwrap();
        let resp = client
            .get("https://huggingface.co/SmilingWolf/wd-vit-tagger-v3/resolve/main/selected_tags.csv")
            .send()
            .await;
        match resp {
            Ok(r) => assert!(r.status().is_success(), "HTTP 错误: {}", r.status()),
            Err(e) => panic!("socks5 代理下载失败: {e}"),
        }
    }
}
