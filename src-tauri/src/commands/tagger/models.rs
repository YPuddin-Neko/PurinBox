use super::get_models_dir;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// 模型定义
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelDefinition {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub requires_token: bool,
    #[serde(default)]
    pub heavy_gpu: bool,
    pub repo_id: String,
    pub model_filename: String,
    pub tags_filename: String,
    #[serde(default)]
    pub extra_files: Vec<String>,
    pub input_size: u32,
    pub is_builtin: bool,
    #[serde(default = "default_preprocess_mode")]
    pub preprocess_mode: String,
    /// 输出语义：auto（旧启发式，NCHW 视为 logits）| probability（概率，多输出时取
    /// prediction 节点）| logits（原始分，需 sigmoid，多输出时取 logits 节点）
    #[serde(default = "default_output_kind")]
    pub output_kind: String,
    /// 官方推荐的通用/角色阈值（None = 用工具箱统一默认值）
    #[serde(default)]
    pub general_threshold: Option<f32>,
    #[serde(default)]
    pub character_threshold: Option<f32>,
    #[serde(default)]
    pub category_thresholds: BTreeMap<String, f32>,
}

fn default_preprocess_mode() -> String {
    "auto".to_string()
}

fn default_output_kind() -> String {
    "auto".to_string()
}

pub(super) fn basename(path: &str) -> String {
    std::path::Path::new(path)
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string()
}

impl ModelDefinition {
    pub fn tags_basename(&self) -> String {
        basename(&self.tags_filename)
    }

    pub fn required_local_files(&self) -> Vec<String> {
        let mut files = vec!["model.onnx".to_string(), self.tags_basename()];
        for extra in &self.extra_files {
            let name = basename(extra);
            if !name.is_empty() && !files.contains(&name) {
                files.push(name);
            }
        }
        files
    }

    /// 模型目录里所需文件是否齐全
    pub fn is_downloaded(&self) -> bool {
        let dir = super::get_model_dir(&self.id);
        self.required_local_files()
            .iter()
            .all(|filename| dir.join(filename).exists())
    }
}

fn builtin(
    id: &str,
    name: &str,
    repo_id: &str,
    tags_filename: &str,
    input_size: u32,
) -> ModelDefinition {
    ModelDefinition {
        id: id.into(),
        name: name.into(),
        requires_token: false,
        heavy_gpu: false,
        repo_id: repo_id.into(),
        model_filename: "model.onnx".into(),
        tags_filename: tags_filename.into(),
        extra_files: vec![],
        input_size,
        is_builtin: true,
        preprocess_mode: "wd".into(),
        output_kind: "auto".into(),
        general_threshold: None,
        character_threshold: None,
        category_thresholds: BTreeMap::new(),
    }
}

/// 获取内置模型列表
pub fn get_builtin_models() -> Vec<ModelDefinition> {
    vec![
        builtin(
            "wd-swinv2-tagger-v3",
            "WD SwinV2 Tagger v3",
            "SmilingWolf/wd-swinv2-tagger-v3",
            "selected_tags.csv",
            448,
        ),
        builtin(
            "wd-vit-tagger-v3",
            "WD ViT Tagger v3",
            "SmilingWolf/wd-vit-tagger-v3",
            "selected_tags.csv",
            448,
        ),
        builtin(
            "wd-convnext-tagger-v3",
            "WD ConvNeXt Tagger v3",
            "SmilingWolf/wd-convnext-tagger-v3",
            "selected_tags.csv",
            448,
        ),
        builtin(
            "wd-eva02-large-tagger-v3",
            "WD EVA02 Large Tagger v3",
            "SmilingWolf/wd-eva02-large-tagger-v3",
            "selected_tags.csv",
            448,
        ),
        ModelDefinition {
            // 该 ONNX 使用 timm 的 NCHW + RGB 归一化预处理；SmilingWolf 导出使用 NHWC + BGR。
            preprocess_mode: "wd_nchw".into(),
            output_kind: "probability".into(),
            ..builtin(
                "wd-eva02-tagger-2026-canary",
                "WD EVA02 Tagger 2026 Canary",
                "Misaka41Z/wd-eva02-tagger-2026-canary-onnx-v2",
                "selected_tags.csv",
                448,
            )
        },
        ModelDefinition {
            extra_files: vec!["model.onnx.data".into()],
            preprocess_mode: "pixai_v1".into(),
            heavy_gpu: true,
            output_kind: "logits".into(),
            general_threshold: Some(0.17),
            character_threshold: Some(0.27),
            category_thresholds: [
                ("general", 0.17),
                ("character", 0.27),
                ("style", 0.15),
                ("copyright", 0.24),
                ("meta", 0.17),
                ("rating", 0.41),
            ]
            .into_iter()
            .map(|(name, threshold)| (name.into(), threshold))
            .collect(),
            ..builtin(
                "pixai-tagger-v1.0",
                "PixAI Tagger v1.0",
                "noaione/pixai-tagger-v1.0-onnx",
                "tags.json",
                1008,
            )
        },
        ModelDefinition {
            preprocess_mode: "pixai".into(),
            output_kind: "probability".into(),
            // 官方推荐阈值（deepghs thresholds.csv / README）：general 0.3, character 0.85
            general_threshold: Some(0.3),
            character_threshold: Some(0.85),
            ..builtin(
                "pixai-tagger-v0.9",
                "PixAI Tagger v0.9",
                "deepghs/pixai-tagger-v0.9-onnx",
                "selected_tags.csv",
                448,
            )
        },
        builtin(
            "wd-v1-4-moat-tagger-v2",
            "WD MOAT Tagger v2",
            "SmilingWolf/wd-v1-4-moat-tagger-v2",
            "selected_tags.csv",
            448,
        ),
        ModelDefinition {
            model_filename: "v2_01a/model.onnx".into(),
            requires_token: true,
            extra_files: vec!["v2_01a/model.onnx.data".into()],
            preprocess_mode: "siglip2".into(),
            ..builtin(
                "cl-tagger-v2-01a",
                "CL Tagger v2.01a",
                "cella110n/cl_tagger_v2",
                "v2_01a/model_vocabulary.json",
                384,
            )
        },
        ModelDefinition {
            model_filename: "cl_tagger_1_02/model.onnx".into(),
            ..builtin(
                "cl-tagger-1-02",
                "CL Tagger v1.02",
                "cella110n/cl_tagger",
                "cl_tagger_1_02/tag_mapping.json",
                448,
            )
        },
    ]
}

/// 自定义模型配置文件路径
fn custom_models_path() -> std::path::PathBuf {
    get_models_dir().join("custom_models.json")
}

/// 加载自定义模型列表
pub fn load_custom_models() -> Result<Vec<ModelDefinition>, String> {
    let path = custom_models_path();
    if !path.exists() {
        return Ok(Vec::new());
    }
    let content =
        std::fs::read_to_string(&path).map_err(|e| format!("读取自定义模型配置失败: {}", e))?;
    serde_json::from_str(&content).map_err(|e| format!("解析自定义模型配置失败: {}", e))
}

/// 保存自定义模型列表
fn save_custom_models(models: &[ModelDefinition]) -> Result<(), String> {
    let dir = custom_models_path().parent().unwrap().to_path_buf();
    if !dir.exists() {
        std::fs::create_dir_all(&dir).map_err(|e| format!("创建目录失败: {}", e))?;
    }
    let json = serde_json::to_string_pretty(models).map_err(|e| format!("序列化失败: {}", e))?;
    crate::commands::config_paths::write_file_atomic(&custom_models_path(), json.as_bytes())
        .map_err(|e| format!("写入配置失败: {}", e))?;
    Ok(())
}

/// 添加自定义模型（本地导入）
pub fn add_local_model(
    name: String,
    model_path: String,
    tags_path: String,
    input_size: u32,
) -> Result<String, String> {
    let id = format!(
        "custom-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()
    );

    let mut models = load_custom_models().unwrap_or_default();
    if models.iter().any(|m| m.name == name) {
        return Err(format!("名称 '{}' 已存在", name));
    }

    let model_dir = super::get_model_dir(&id);
    if !model_dir.exists() {
        std::fs::create_dir_all(&model_dir).map_err(|e| format!("创建目录失败: {}", e))?;
    }

    let src_model = std::path::Path::new(&model_path);
    let dest_model = model_dir.join("model.onnx");
    std::fs::copy(src_model, &dest_model).map_err(|e| format!("复制模型文件失败: {}", e))?;

    // 标签文件保留原始扩展名：加载时按扩展名区分 CSV 与 JSON 词表
    let src_tags = std::path::Path::new(&tags_path);
    let tags_ext = src_tags
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("csv");
    let tags_dest_name = format!("tags.{}", tags_ext);
    let dest_tags = model_dir.join(&tags_dest_name);
    std::fs::copy(src_tags, &dest_tags).map_err(|e| format!("复制标签文件失败: {}", e))?;

    models.push(ModelDefinition {
        id: id.clone(),
        name,
        requires_token: false,
        heavy_gpu: false,
        repo_id: String::new(),
        model_filename: "model.onnx".into(),
        tags_filename: tags_dest_name,
        extra_files: vec![],
        input_size,
        is_builtin: false,
        preprocess_mode: "auto".into(),
        output_kind: "auto".into(),
        general_threshold: None,
        character_threshold: None,
        category_thresholds: BTreeMap::new(),
    });

    save_custom_models(&models)?;
    Ok(id)
}

/// 删除自定义模型
pub fn remove_custom_model(id: &str) -> Result<(), String> {
    let mut models = load_custom_models().unwrap_or_default();
    let orig_len = models.len();
    models.retain(|m| m.id != id);
    if models.len() == orig_len {
        return Err("模型不存在".into());
    }
    save_custom_models(&models)?;

    // 删除模型文件目录
    let model_dir = super::get_model_dir(id);
    if model_dir.exists() {
        let _ = std::fs::remove_dir_all(&model_dir);
    }
    Ok(())
}

/// 根据 ID 查找模型（含内置和自定义）
pub fn find_model(id: &str) -> Option<ModelDefinition> {
    let all = get_builtin_models();
    if let Some(m) = all.into_iter().find(|m| m.id == id) {
        return Some(m);
    }
    let custom = load_custom_models().unwrap_or_default();
    custom.into_iter().find(|m| m.id == id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pixai_v1_requires_external_weights_and_keeps_v09_available() {
        let model = find_model("pixai-tagger-v1.0").unwrap();
        assert_eq!(
            model.required_local_files(),
            ["model.onnx", "tags.json", "model.onnx.data"]
        );
        assert_eq!(model.category_thresholds.len(), 6);
        assert_eq!(model.category_thresholds["style"], 0.15);
        let old = find_model("pixai-tagger-v0.9").unwrap();
        assert_eq!(old.input_size, 448);
        assert!(old.category_thresholds.is_empty());
    }

    #[test]
    fn old_model_configs_default_to_legacy_thresholds() {
        let mut value = serde_json::to_value(find_model("pixai-tagger-v0.9").unwrap()).unwrap();
        value.as_object_mut().unwrap().remove("category_thresholds");
        value.as_object_mut().unwrap().remove("heavy_gpu");
        value.as_object_mut().unwrap().remove("requires_token");
        value["input_format"] = serde_json::json!("NCHW");
        value["description"] = serde_json::json!("legacy description");
        let restored: ModelDefinition = serde_json::from_value(value).unwrap();
        assert!(restored.category_thresholds.is_empty());
        assert!(!restored.heavy_gpu);
        assert!(!restored.requires_token);
        assert_eq!(restored.character_threshold, Some(0.85));
        let serialized = serde_json::to_value(restored).unwrap();
        assert!(serialized.get("input_format").is_none());
        assert!(serialized.get("description").is_none());
    }

    #[test]
    fn builtin_capabilities_are_explicit() {
        let models = get_builtin_models();
        assert_eq!(
            models
                .iter()
                .filter(|m| m.heavy_gpu)
                .map(|m| m.id.as_str())
                .collect::<Vec<_>>(),
            ["pixai-tagger-v1.0"]
        );
        assert_eq!(
            models
                .iter()
                .filter(|m| m.requires_token)
                .map(|m| m.id.as_str())
                .collect::<Vec<_>>(),
            ["cl-tagger-v2-01a"]
        );
    }
}
