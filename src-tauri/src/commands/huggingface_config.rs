use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct HuggingFaceConfig {
    #[serde(default)]
    pub token_encoded: String,
}

const CONFIG_FILE: &str = "huggingface_config.json";

fn config_path() -> PathBuf {
    super::config_paths::resolve_config_file(CONFIG_FILE)
}

fn encode(value: &str) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(value.as_bytes())
}

fn decode(value: &str) -> String {
    use base64::Engine;
    if value.is_empty() {
        return String::new();
    }
    base64::engine::general_purpose::STANDARD
        .decode(value)
        .ok()
        .and_then(|bytes| String::from_utf8(bytes).ok())
        .unwrap_or_default()
}

#[tauri::command]
pub fn save_huggingface_config(token: String) -> Result<(), String> {
    let dir = super::config_paths::user_config_dir();
    std::fs::create_dir_all(&dir).map_err(|e| format!("创建配置目录失败: {}", e))?;

    let config = HuggingFaceConfig {
        token_encoded: encode(token.trim()),
    };
    let json = serde_json::to_string_pretty(&config).map_err(|e| format!("序列化失败: {}", e))?;
    std::fs::write(dir.join(CONFIG_FILE), json)
        .map_err(|e| format!("写入 Hugging Face 配置失败: {}", e))?;
    Ok(())
}

#[tauri::command]
pub fn load_huggingface_config() -> Result<String, String> {
    Ok(load_huggingface_token_internal())
}

pub fn load_huggingface_token_internal() -> String {
    let path = config_path();
    if !path.exists() {
        return String::new();
    }

    std::fs::read_to_string(&path)
        .ok()
        .and_then(|content| serde_json::from_str::<HuggingFaceConfig>(&content).ok())
        .map(|config| decode(&config.token_encoded))
        .unwrap_or_default()
}

pub fn apply_huggingface_auth(request: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
    let token = load_huggingface_token_internal();
    let token = token.trim();
    if token.is_empty() {
        request
    } else {
        request.bearer_auth(token)
    }
}
