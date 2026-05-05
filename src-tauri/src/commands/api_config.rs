use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// API 配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiConfig {
    /// 预设类型: "openai" | "gemini" | "deepseek" | "custom"
    pub preset: String,
    /// 自定义端点 URL（仅 preset="custom" 时使用）
    pub custom_endpoint: String,
    /// API Key（base64 编码存储）
    pub api_key_encoded: String,
}

impl Default for ApiConfig {
    fn default() -> Self {
        Self {
            preset: "openai".to_string(),
            custom_endpoint: String::new(),
            api_key_encoded: String::new(),
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
    get_config_dir().join("api_config.json")
}

/// 编码 API Key（base64）
fn encode_key(key: &str) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(key.as_bytes())
}

/// 解码 API Key
fn decode_key(encoded: &str) -> String {
    use base64::Engine;
    if encoded.is_empty() {
        return String::new();
    }
    base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .ok()
        .and_then(|bytes| String::from_utf8(bytes).ok())
        .unwrap_or_default()
}

/// 保存 API 配置
#[tauri::command]
pub fn save_api_config(preset: String, custom_endpoint: String, api_key: String) -> Result<(), String> {
    let dir = get_config_dir();
    if !dir.exists() {
        std::fs::create_dir_all(&dir).map_err(|e| format!("创建配置目录失败: {}", e))?;
    }

    let config = ApiConfig {
        preset,
        custom_endpoint,
        api_key_encoded: encode_key(&api_key),
    };

    let json = serde_json::to_string_pretty(&config)
        .map_err(|e| format!("序列化失败: {}", e))?;
    std::fs::write(config_path(), json)
        .map_err(|e| format!("写入配置失败: {}", e))?;
    Ok(())
}

/// 加载 API 配置（返回解码后的 key）
#[tauri::command]
pub fn load_api_config() -> Result<(String, String, String), String> {
    let path = config_path();
    if !path.exists() {
        return Ok(("openai".to_string(), String::new(), String::new()));
    }

    let content = std::fs::read_to_string(&path)
        .map_err(|e| format!("读取配置失败: {}", e))?;
    let config: ApiConfig = serde_json::from_str(&content)
        .map_err(|e| format!("解析配置失败: {}", e))?;

    Ok((config.preset, config.custom_endpoint, decode_key(&config.api_key_encoded)))
}
