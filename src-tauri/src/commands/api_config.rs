use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

/// API 配置（磁盘存储格式）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiConfig {
    /// 预设类型: "openai" | "gemini" | "deepseek" | "custom"
    pub preset: String,
    /// 自定义端点 URL（仅 preset="custom" 时使用）
    pub custom_endpoint: String,
    /// 各预设的 API Key（base64 编码存储），key = preset 名称
    #[serde(default)]
    pub api_keys: HashMap<String, String>,
    // ---- 兼容旧版：单一 api_key_encoded ----
    /// 旧字段（迁移后不再写入，仅用于读取旧配置）
    #[serde(default, skip_serializing)]
    api_key_encoded: String,
}

impl Default for ApiConfig {
    fn default() -> Self {
        Self {
            preset: "openai".to_string(),
            custom_endpoint: String::new(),
            api_keys: HashMap::new(),
            api_key_encoded: String::new(),
        }
    }
}

/// 返回给前端的配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiConfigResponse {
    pub preset: String,
    pub custom_endpoint: String,
    /// 各预设的 API Key（已解码明文），key = preset 名称
    pub api_keys: HashMap<String, String>,
}

const CONFIG_FILE: &str = "api_config.json";

/// 配置文件读取路径（含旧 exe 同目录 config/ 的自动迁移）
fn config_path() -> PathBuf {
    super::config_paths::resolve_config_file(CONFIG_FILE)
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
pub fn save_api_config(
    preset: String,
    custom_endpoint: String,
    api_keys: HashMap<String, String>,
) -> Result<(), String> {
    let dir = super::config_paths::user_config_dir();
    std::fs::create_dir_all(&dir).map_err(|e| format!("创建配置目录失败: {}", e))?;

    // 与已存储的 key 合并：调用方（如精修/辅助打标 Tab）可能只传当前预设一把 key，
    // 整体替换会抹掉其他预设已保存的 key。传空字符串表示显式清除该预设。
    let mut encoded_keys: HashMap<String, String> = std::fs::read_to_string(config_path())
        .ok()
        .and_then(|c| serde_json::from_str::<ApiConfig>(&c).ok())
        .map(|old| old.api_keys)
        .unwrap_or_default();
    for (k, v) in api_keys {
        if v.is_empty() {
            encoded_keys.remove(&k);
        } else {
            encoded_keys.insert(k, encode_key(&v));
        }
    }

    let config = ApiConfig {
        preset,
        custom_endpoint,
        api_keys: encoded_keys,
        api_key_encoded: String::new(),
    };

    let json = serde_json::to_string_pretty(&config).map_err(|e| format!("序列化失败: {}", e))?;
    std::fs::write(dir.join(CONFIG_FILE), json).map_err(|e| format!("写入配置失败: {}", e))?;
    Ok(())
}

/// 加载 API 配置
#[tauri::command]
pub fn load_api_config() -> Result<ApiConfigResponse, String> {
    let path = config_path();
    if !path.exists() {
        return Ok(ApiConfigResponse {
            preset: "openai".to_string(),
            custom_endpoint: String::new(),
            api_keys: HashMap::new(),
        });
    }

    let content = std::fs::read_to_string(&path).map_err(|e| format!("读取配置失败: {}", e))?;
    let config: ApiConfig =
        serde_json::from_str(&content).map_err(|e| format!("解析配置失败: {}", e))?;

    // 解码所有 key
    let mut decoded_keys: HashMap<String, String> = config
        .api_keys
        .iter()
        .map(|(k, v)| (k.clone(), decode_key(v)))
        .filter(|(_, v)| !v.is_empty())
        .collect();

    // 兼容旧版：如果有 api_key_encoded 但 api_keys 为空，迁移到当前 preset
    if decoded_keys.is_empty() && !config.api_key_encoded.is_empty() {
        let old_key = decode_key(&config.api_key_encoded);
        if !old_key.is_empty() {
            decoded_keys.insert(config.preset.clone(), old_key);
        }
    }

    Ok(ApiConfigResponse {
        preset: config.preset,
        custom_endpoint: config.custom_endpoint,
        api_keys: decoded_keys,
    })
}
