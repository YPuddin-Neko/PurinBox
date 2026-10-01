use super::config_paths::{
    b64_decode, b64_encode, load_json_config, load_json_config_or_default, save_json_config,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// API 配置（磁盘存储格式）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiConfig {
    /// 预设类型: "openai" | "gemini" | "deepseek" | "custom"
    pub preset: String,
    /// 自定义端点 URL（仅 preset="custom" 时使用）
    pub custom_endpoint: String,
    /// 各预设 API Key 的落盘存储值（密钥环标记或 base64），key = preset 名称
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

/// 系统密钥环中的服务名
const KEYRING_SERVICE: &str = "PurinBox";

/// 落盘标记值：真实 key 在系统密钥环里，配置文件只留标记
const KEYRING_MARKER: &str = "@keyring";

fn keyring_entry(preset: &str) -> Option<keyring::Entry> {
    keyring::Entry::new(KEYRING_SERVICE, &format!("api-key:{}", preset)).ok()
}

/// 优先写入系统密钥环（Windows 凭据管理器 / macOS 钥匙串 / Linux Secret Service）;
/// 密钥环不可用时回退配置文件 base64。返回落盘存储值
fn store_key(preset: &str, key: &str) -> String {
    if let Some(entry) = keyring_entry(preset) {
        if entry.set_password(key).is_ok() {
            return KEYRING_MARKER.to_string();
        }
    }
    b64_encode(key)
}

/// 读取 API Key：密钥环标记走系统密钥环，其余按 base64 解码（兼容旧配置）
fn load_key(preset: &str, stored: &str) -> String {
    if stored == KEYRING_MARKER {
        return keyring_entry(preset)
            .and_then(|entry| entry.get_password().ok())
            .unwrap_or_default();
    }
    b64_decode(stored)
}

/// 显式清除某个预设的 key 时同步删除密钥环条目
fn delete_key(preset: &str) {
    if let Some(entry) = keyring_entry(preset) {
        let _ = entry.delete_credential();
    }
}

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
    let mut stored_keys = load_json_config_or_default::<ApiConfig>(CONFIG_FILE).api_keys;
    for (k, v) in api_keys {
        if v.is_empty() {
            stored_keys.remove(&k);
            delete_key(&k);
        } else {
            let stored = store_key(&k, &v);
            stored_keys.insert(k, stored);
        }
    }

    let config = ApiConfig {
        preset,
        custom_endpoint,
        api_keys: stored_keys,
        api_key_encoded: String::new(),
    };

    save_json_config(CONFIG_FILE, &config, "配置")
}

#[tauri::command]
pub fn load_api_config() -> Result<ApiConfigResponse, String> {
    let config: ApiConfig = load_json_config(CONFIG_FILE, "配置")?;

    let mut decoded_keys: HashMap<String, String> = config
        .api_keys
        .iter()
        .map(|(k, v)| (k.clone(), load_key(k, v)))
        .filter(|(_, v)| !v.is_empty())
        .collect();

    // 兼容旧版：如果有 api_key_encoded 但 api_keys 为空，迁移到当前 preset
    if decoded_keys.is_empty() && !config.api_key_encoded.is_empty() {
        let old_key = b64_decode(&config.api_key_encoded);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_fallback_keys_keep_legacy_decoding() {
        for key in ["", "test-key", "密钥"] {
            assert_eq!(load_key("custom", &b64_encode(key)), key);
        }
        assert_eq!(load_key("custom", "invalid base64"), "");
    }

    #[test]
    fn legacy_config_can_be_read_without_reserializing_old_key_field() {
        let encoded = b64_encode("old-key");
        let config: ApiConfig = serde_json::from_value(serde_json::json!({
            "preset": "custom", "custom_endpoint": "http://localhost/v1",
            "api_key_encoded": encoded,
        }))
        .unwrap();
        assert_eq!(b64_decode(&config.api_key_encoded), "old-key");
        assert!(config.api_keys.is_empty());
        let saved = serde_json::to_value(config).unwrap();
        assert!(saved.get("api_key_encoded").is_none());
        assert_eq!(ApiConfig::default().preset, "openai");
    }
}
