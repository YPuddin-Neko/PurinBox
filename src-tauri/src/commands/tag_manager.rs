use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

fn image_display_name(root: &Path, path: &Path, recursive: bool) -> String {
    if recursive {
        if let Ok(relative) = path.strip_prefix(root) {
            return relative.to_string_lossy().replace('\\', "/");
        }
    }
    path.file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string()
}

/// 校验目录后扫描数据集图片，按扫描顺序把 (图片路径, 显示名) 交给 `make` 生成条目。
/// 失败图副本目录在收集时已剪掉，同一张图不会在编辑器列表里出现两次
fn scan_dataset<T>(
    folder: &str,
    recursive: Option<bool>,
    mut make: impl FnMut(PathBuf, String) -> T,
) -> Result<Vec<T>, String> {
    let dir = Path::new(folder);
    if !dir.is_dir() {
        return Err(format!("目录不存在: {}", folder));
    }
    let recursive = recursive.unwrap_or(false);
    Ok(super::collect_image_files_with_recursive(dir, recursive)?
        .into_iter()
        .map(|p| {
            let filename = image_display_name(dir, &p, recursive);
            make(p, filename)
        })
        .collect())
}

/// 写与图片同名、换了扩展名的旁路文件
fn write_sidecar(image: &Path, ext: &str, content: &str) -> Result<(), String> {
    let path = image.with_extension(ext);
    std::fs::write(&path, content).map_err(|e| format!("写入失败 {}: {}", path.display(), e))
}

fn existing_image(image_path: &str) -> Result<&Path, String> {
    let img = Path::new(image_path);
    if !img.exists() {
        return Err(format!("图片不存在: {}", image_path));
    }
    Ok(img)
}

/// 批量保存：单个失败只打日志、不中断，返回成功数
fn save_each<T>(items: &[T], mut save: impl FnMut(&T) -> Result<(), String>) -> u32 {
    let mut saved = 0u32;
    for item in items {
        match save(item) {
            Ok(()) => saved += 1,
            Err(e) => eprintln!("{}", e),
        }
    }
    saved
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TagImageItem {
    pub path: String,
    pub filename: String,
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TagDataset {
    pub images: Vec<TagImageItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SaveTagItem {
    pub path: String,
    pub tags: Vec<String>,
}

/// 加载标签数据集：扫描文件夹中的图片，读取对应 .txt 文件的标签
#[tauri::command]
pub fn load_tag_dataset(folder: String, recursive: Option<bool>) -> Result<TagDataset, String> {
    let images = scan_dataset(&folder, recursive, |p, filename| {
        let tags = std::fs::read_to_string(p.with_extension("txt"))
            .map(|content| {
                content
                    .split(',')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect()
            })
            .unwrap_or_default();
        TagImageItem {
            path: p.to_string_lossy().to_string(),
            filename,
            tags,
        }
    })?;
    Ok(TagDataset { images })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CaptionImageItem {
    pub path: String,
    pub filename: String,
    pub caption: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CaptionDataset {
    pub images: Vec<CaptionImageItem>,
}

/// 自然语言描述数据集加载（读取原始文本，不按逗号分割）
#[tauri::command]
pub fn load_caption_dataset(
    folder: String,
    recursive: Option<bool>,
) -> Result<CaptionDataset, String> {
    let images = scan_dataset(&folder, recursive, |p, filename| CaptionImageItem {
        caption: std::fs::read_to_string(p.with_extension("txt")).unwrap_or_default(),
        path: p.to_string_lossy().to_string(),
        filename,
    })?;
    Ok(CaptionDataset { images })
}

/// 保存单个图片的标签到 .txt 文件
#[tauri::command]
pub fn save_single_tag_file(image_path: String, tags: Vec<String>) -> Result<(), String> {
    save_caption_file(image_path, tags.join(", "))
}

/// 批量保存多个图片的标签
#[tauri::command]
pub fn save_all_tag_files(items: Vec<SaveTagItem>) -> Result<u32, String> {
    Ok(save_each(&items, |item| {
        write_sidecar(Path::new(&item.path), "txt", &item.tags.join(", "))
    }))
}

/// 保存单个图片的自然语言描述到 .txt 文件
#[tauri::command]
pub fn save_caption_file(image_path: String, content: String) -> Result<(), String> {
    write_sidecar(existing_image(&image_path)?, "txt", &content)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SaveCaptionItem {
    pub path: String,
    pub content: String,
}

/// 批量保存多个图片的自然语言描述
#[tauri::command]
pub fn save_all_caption_files(items: Vec<SaveCaptionItem>) -> Result<u32, String> {
    Ok(save_each(&items, |item| {
        write_sidecar(Path::new(&item.path), "txt", &item.content)
    }))
}

// ============================================================
// JSON 结构化标签管理 — AnimaLoraStudio 完整格式
// ============================================================

/// 空 Option 字段序列化为 "" 占位：完整 schema 恒定输出，没有数据也不省略键
fn ser_opt_str_as_empty<S: serde::Serializer>(v: &Option<String>, s: S) -> Result<S::Ok, S::Error> {
    s.serialize_str(v.as_deref().unwrap_or(""))
}

/// fixed: 固定字段，不参与 shuffle
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct JsonFixed {
    #[serde(serialize_with = "ser_opt_str_as_empty")]
    pub quality: Option<String>,
    #[serde(serialize_with = "ser_opt_str_as_empty")]
    pub series: Option<String>,
    #[serde(serialize_with = "ser_opt_str_as_empty")]
    pub artist: Option<String>,
    /// schema 外的未知字段原样保留，整文件重写时不丢用户自定义内容
    #[serde(flatten, skip_serializing_if = "serde_json::Map::is_empty")]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

/// character: 角色信息
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct JsonCharacter {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub variant: String,
    #[serde(flatten, skip_serializing_if = "serde_json::Map::is_empty")]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

/// from_path: 从目录路径自动提取的外观标签
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct JsonFromPath {
    #[serde(default, deserialize_with = "deserialize_string_or_array")]
    pub appearance: Vec<String>,
    #[serde(flatten, skip_serializing_if = "serde_json::Map::is_empty")]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

/// 自定义反序列化：支持 JSON 数组 ["a","b"] 或逗号字符串 "a, b"
fn deserialize_string_or_array<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de;
    struct StringOrArray;
    impl<'de> de::Visitor<'de> for StringOrArray {
        type Value = Vec<String>;
        fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
            f.write_str("a string or array of strings")
        }
        fn visit_str<E: de::Error>(self, v: &str) -> Result<Vec<String>, E> {
            Ok(v.split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect())
        }
        fn visit_seq<A: de::SeqAccess<'de>>(self, mut seq: A) -> Result<Vec<String>, A::Error> {
            let mut v = Vec::new();
            while let Some(s) = seq.next_element::<String>()? {
                v.push(s);
            }
            Ok(v)
        }
        // null 字段视为"字段为空"，而不是让整个文件解析失败
        fn visit_unit<E: de::Error>(self) -> Result<Vec<String>, E> {
            Ok(Vec::new())
        }
    }
    deserializer.deserialize_any(StringOrArray)
}

/// ai_output: VLM/Tagger 打标输出
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct JsonAiOutput {
    #[serde(serialize_with = "ser_opt_str_as_empty")]
    pub count: Option<String>,
    #[serde(default, deserialize_with = "deserialize_string_or_array")]
    pub appearance: Vec<String>,
    #[serde(default, deserialize_with = "deserialize_string_or_array")]
    pub tags: Vec<String>,
    #[serde(default, deserialize_with = "deserialize_string_or_array")]
    pub environment: Vec<String>,
    #[serde(serialize_with = "ser_opt_str_as_empty")]
    pub nl: Option<String>,
    #[serde(flatten, skip_serializing_if = "serde_json::Map::is_empty")]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

/// 完整 JSON 标签结构
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct JsonTagData {
    #[serde(default)]
    pub fixed: JsonFixed,
    #[serde(default)]
    pub character: JsonCharacter,
    #[serde(default)]
    pub from_path: JsonFromPath,
    #[serde(default)]
    pub ai_output: JsonAiOutput,
    /// 顶层未知字段（如用户自定义 rating 等）随读随写，不因编辑而丢失
    #[serde(flatten, skip_serializing_if = "serde_json::Map::is_empty")]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonImageItem {
    pub path: String,
    pub filename: String,
    pub data: JsonTagData,
    pub has_json: bool,
    /// JSON 存在但解析失败：data 是空默认值，前端必须排除在批量操作/保存之外
    #[serde(default)]
    pub parse_failed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonDataset {
    pub images: Vec<JsonImageItem>,
    /// "full" | "simplified" | "unknown"
    pub detected_format: String,
}

/// 加载 JSON 标签数据集
#[tauri::command]
pub fn load_json_dataset(folder: String, recursive: Option<bool>) -> Result<JsonDataset, String> {
    let mut detected_format = "unknown";
    let images = scan_dataset(&folder, recursive, |p, filename| {
        let (data, has_json, fmt, parse_failed) =
            match std::fs::read_to_string(p.with_extension("json")) {
                Ok(content) => match serde_json::from_str::<serde_json::Value>(&content) {
                    Ok(v) if is_full_json(&v) => match serde_json::from_value::<JsonTagData>(v) {
                        Ok(d) => (d, true, "full", false),
                        Err(_) => (JsonTagData::default(), true, "unknown", true),
                    },
                    Ok(v) => match parse_simplified_format(&v) {
                        Some(d) => (d, true, "simplified", false),
                        None => (JsonTagData::default(), true, "unknown", true),
                    },
                    Err(_) => (JsonTagData::default(), true, "unknown", true),
                },
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    (JsonTagData::default(), false, "unknown", false)
                }
                // 文件存在但读不了：同样按解析失败保护，避免空数据反写覆盖
                Err(_) => (JsonTagData::default(), false, "unknown", true),
            };

        if has_json && detected_format == "unknown" {
            detected_format = fmt;
        }

        JsonImageItem {
            path: p.to_string_lossy().to_string(),
            filename,
            data,
            has_json,
            parse_failed,
        }
    })?;

    Ok(JsonDataset {
        images,
        detected_format: detected_format.to_string(),
    })
}

/// 判断 JSON 是否为完整格式（顶层含结构化字段）。
/// 完整格式的 character 是对象 {name, variant, ...}；简化格式顶层 character 是字符串，
/// 含同名字符串键不算完整格式
pub(crate) fn is_full_json(v: &serde_json::Value) -> bool {
    ["ai_output", "fixed", "from_path", "character"]
        .iter()
        .any(|key| v.get(key).is_some_and(serde_json::Value::is_object))
}

#[test]
fn full_json_requires_structured_fields() {
    for key in ["ai_output", "fixed", "from_path", "character"] {
        assert!(!is_full_json(&serde_json::json!({key: "text"})));
        assert!(!is_full_json(&serde_json::json!({key: null})));
        assert!(!is_full_json(&serde_json::json!({key: []})));
        assert!(is_full_json(&serde_json::json!({key: {}})));
    }
}

/// 解析简化格式 JSON（扁平结构）转为完整格式
fn parse_simplified_format(v: &serde_json::Value) -> Option<JsonTagData> {
    let obj = v.as_object()?;
    let text = |key: &str| obj.get(key).and_then(|v| v.as_str()).map(str::to_string);

    Some(JsonTagData {
        fixed: JsonFixed {
            quality: text("quality"),
            series: text("series"),
            artist: text("artist"),
            ..Default::default()
        },
        // character — 简化格式中是 string
        character: JsonCharacter {
            name: text("character").unwrap_or_default(),
            ..Default::default()
        },
        ai_output: JsonAiOutput {
            count: text("count"),
            appearance: extract_string_array(obj.get("appearance")),
            tags: extract_string_array(obj.get("tags")),
            environment: extract_string_array(obj.get("environment")),
            nl: text("nl"),
            ..Default::default()
        },
        ..Default::default()
    })
}

fn extract_string_array(v: Option<&serde_json::Value>) -> Vec<String> {
    match v {
        Some(serde_json::Value::Array(arr)) => arr
            .iter()
            .filter_map(|v| v.as_str().map(|s| s.to_string()))
            .collect(),
        // 兼容：值是逗号分隔的字符串而非数组
        Some(serde_json::Value::String(s)) => s
            .split(',')
            .map(|t| t.trim().to_string())
            .filter(|t| !t.is_empty())
            .collect(),
        _ => Vec::new(),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SaveJsonItem {
    pub path: String,
    pub data: JsonTagData,
}

/// 将完整格式转为简化格式 JSON Value（9 个键恒定输出，空值占位不省略）
fn to_simplified(data: &JsonTagData) -> serde_json::Value {
    fn s(v: &Option<String>) -> &str {
        v.as_deref().unwrap_or("")
    }

    // from_path.appearance 与 ai_output.appearance 合并（简化格式无 from_path 字段）
    let mut appearance: Vec<String> = data.from_path.appearance.clone();
    for t in &data.ai_output.appearance {
        if !appearance.contains(t) {
            appearance.push(t.clone());
        }
    }

    serde_json::json!({
        "quality": s(&data.fixed.quality),
        "series": s(&data.fixed.series),
        "artist": s(&data.fixed.artist),
        "character": data.character.name,
        "count": s(&data.ai_output.count),
        "appearance": appearance,
        "tags": data.ai_output.tags,
        "environment": data.ai_output.environment,
        "nl": s(&data.ai_output.nl),
    })
}

fn serialize_json(data: &JsonTagData, simplified: bool) -> Result<String, String> {
    if simplified {
        serde_json::to_string_pretty(&to_simplified(data))
    } else {
        serde_json::to_string_pretty(data)
    }
    .map_err(|e| format!("JSON 序列化失败: {}", e))
}

#[tauri::command]
pub fn save_single_json_file(
    image_path: String,
    data: JsonTagData,
    simplified: bool,
) -> Result<(), String> {
    let img = existing_image(&image_path)?;
    write_sidecar(img, "json", &serialize_json(&data, simplified)?)
}

#[tauri::command]
pub fn save_all_json_files(items: Vec<SaveJsonItem>, simplified: bool) -> Result<u32, String> {
    Ok(save_each(&items, |item| {
        let content = serialize_json(&item.data, simplified)?;
        write_sidecar(Path::new(&item.path), "json", &content)
    }))
}

#[cfg(test)]
mod json_extra_tests {
    use super::*;

    /// schema 外字段必须原样往返：整文件重写不能把用户自定义字段丢掉
    #[test]
    fn full_json_roundtrip_preserves_unknown_fields() {
        let raw = r#"{
            "fixed": {"quality": "masterpiece", "custom_note": "keep me"},
            "character": {"name": "miku"},
            "from_path": {"appearance": ["blue hair"]},
            "ai_output": {"tags": ["1girl"], "rating": "safe"},
            "my_extension": {"a": 1}
        }"#;
        let data: JsonTagData = serde_json::from_str(raw).unwrap();
        assert_eq!(
            data.fixed.extra.get("custom_note").and_then(|v| v.as_str()),
            Some("keep me")
        );
        assert!(data.extra.contains_key("my_extension"));
        let out = serde_json::to_string(&data).unwrap();
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["fixed"]["custom_note"], "keep me");
        assert_eq!(v["ai_output"]["rating"], "safe");
        assert_eq!(v["my_extension"]["a"], 1);
        assert_eq!(v["ai_output"]["tags"][0], "1girl");
    }

    /// 加载时先解析成 Value 再 from_value，结果必须与直接 from_str 一致
    /// （未知字段、null 数组、逗号串数组都要原样处理）
    #[test]
    fn from_value_matches_from_str() {
        let raw = r#"{
            "fixed": {"quality": "masterpiece", "custom_note": "keep me"},
            "character": {"name": "miku", "variant": "winter", "age": 16},
            "from_path": {"appearance": "blue hair, twintails"},
            "ai_output": {"appearance": null, "tags": ["1girl"], "nl": null, "rating": "safe"},
            "my_extension": {"a": [1, 2.5, "x"]}
        }"#;
        let via_str: JsonTagData = serde_json::from_str(raw).unwrap();
        let value: serde_json::Value = serde_json::from_str(raw).unwrap();
        assert!(is_full_json(&value));
        let via_value: JsonTagData = serde_json::from_value(value).unwrap();
        assert_eq!(
            serde_json::to_string(&via_value).unwrap(),
            serde_json::to_string(&via_str).unwrap()
        );
        assert_eq!(
            via_value.from_path.appearance,
            vec!["blue hair".to_string(), "twintails".to_string()]
        );
    }

    /// 完整 schema 恒定输出：没有数据的字段以空值占位，不省略键
    #[test]
    fn full_json_serializes_complete_schema_with_empty_placeholders() {
        let out = serde_json::to_string(&JsonTagData::default()).unwrap();
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["fixed"]["quality"], "");
        assert_eq!(v["fixed"]["series"], "");
        assert_eq!(v["fixed"]["artist"], "");
        assert_eq!(v["character"]["name"], "");
        assert_eq!(v["character"]["variant"], "");
        assert_eq!(v["from_path"]["appearance"], serde_json::json!([]));
        assert_eq!(v["ai_output"]["count"], "");
        assert_eq!(v["ai_output"]["appearance"], serde_json::json!([]));
        assert_eq!(v["ai_output"]["tags"], serde_json::json!([]));
        assert_eq!(v["ai_output"]["environment"], serde_json::json!([]));
        assert_eq!(v["ai_output"]["nl"], "");
    }

    /// 简化格式同样恒定输出 9 个键
    #[test]
    fn simplified_json_serializes_complete_schema() {
        let v = to_simplified(&JsonTagData::default());
        let obj = v.as_object().unwrap();
        for key in [
            "quality",
            "series",
            "artist",
            "character",
            "count",
            "appearance",
            "tags",
            "environment",
            "nl",
        ] {
            assert!(obj.contains_key(key), "简化格式缺少字段: {}", key);
        }
        assert_eq!(v["nl"], "");
        assert_eq!(v["tags"], serde_json::json!([]));
    }

    /// 简化格式的键顺序固定，from_path.appearance 并入 appearance 且去重
    #[test]
    fn simplified_json_keeps_key_order_and_merges_appearance() {
        let raw = r#"{
            "fixed": {"quality": "masterpiece", "artist": "@wlop"},
            "character": {"name": "miku", "variant": "winter"},
            "from_path": {"appearance": ["twintails", "blue hair"]},
            "ai_output": {"count": "1girl", "appearance": ["blue hair", "smile"],
                          "tags": ["standing"], "environment": ["outdoors"], "nl": "A girl."}
        }"#;
        let data: JsonTagData = serde_json::from_str(raw).unwrap();
        assert_eq!(
            serde_json::to_string(&to_simplified(&data)).unwrap(),
            r#"{"quality":"masterpiece","series":"","artist":"@wlop","character":"miku","count":"1girl","appearance":["twintails","blue hair","smile"],"tags":["standing"],"environment":["outdoors"],"nl":"A girl."}"#
        );
    }

    /// 简化格式读入：字符串字段原样取，数组字段兼容逗号串
    #[test]
    fn simplified_format_parses_flat_fields() {
        let v: serde_json::Value = serde_json::from_str(
            r#"{"quality": "best", "character": "miku", "count": "1girl",
                "appearance": "long hair, blue eyes", "tags": ["smile"], "nl": "desc"}"#,
        )
        .unwrap();
        assert!(!is_full_json(&v));
        let data = parse_simplified_format(&v).unwrap();
        assert_eq!(data.fixed.quality.as_deref(), Some("best"));
        assert_eq!(data.fixed.series, None);
        assert_eq!(data.character.name, "miku");
        assert_eq!(data.ai_output.count.as_deref(), Some("1girl"));
        assert_eq!(
            data.ai_output.appearance,
            vec!["long hair".to_string(), "blue eyes".to_string()]
        );
        assert_eq!(data.ai_output.tags, vec!["smile".to_string()]);
        assert!(data.ai_output.environment.is_empty());
        assert_eq!(data.ai_output.nl.as_deref(), Some("desc"));
        // 顶层不是对象的不算简化格式
        assert!(parse_simplified_format(&serde_json::json!(["a"])).is_none());
    }

    /// null 数组字段视为空，不拖垮整个文件解析
    #[test]
    fn null_array_fields_parse_as_empty() {
        let raw = r#"{
            "ai_output": {"appearance": null, "tags": ["1girl"], "environment": null, "nl": null},
            "from_path": {"appearance": null}
        }"#;
        let data: JsonTagData = serde_json::from_str(raw).unwrap();
        assert!(data.ai_output.appearance.is_empty());
        assert_eq!(data.ai_output.tags, vec!["1girl".to_string()]);
        assert!(data.ai_output.environment.is_empty());
        assert_eq!(data.ai_output.nl, None);
        assert!(data.from_path.appearance.is_empty());
    }

    /// character 对象按完整格式识别；同名字符串键仍是简化格式
    #[test]
    fn character_object_detected_as_full_format() {
        let full: serde_json::Value =
            serde_json::from_str(r#"{"character": {"name": "hakurei reimu", "variant": ""}}"#)
                .unwrap();
        assert!(is_full_json(&full));
        let simplified: serde_json::Value =
            serde_json::from_str(r#"{"character": "hakurei reimu", "tags": ["1girl"]}"#).unwrap();
        assert!(!is_full_json(&simplified));

        // 角色对象能按完整结构解析保留
        let data: JsonTagData = serde_json::from_str(
            r#"{"character": {"name": "hakurei reimu", "variant": "winter"}}"#,
        )
        .unwrap();
        assert_eq!(data.character.name, "hakurei reimu");
        assert_eq!(data.character.variant, "winter");
    }
}
