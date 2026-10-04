use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use super::tag_text::{field_tags, join_tags, split_tags};

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

/// 批量保存里没保存成功的一项
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SaveFailure {
    /// 请求里这一项的图片路径，前端按它找回对应条目
    pub path: String,
    pub error: String,
}

/// 批量保存的结果：前端只清除保存成功的条目的未保存标记，失败项逐条提示
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct SaveAllResult {
    pub saved: u32,
    pub failed: Vec<SaveFailure>,
}

/// 批量保存：逐项保存，单项失败不中断，记下它的图片路径和原因
fn save_each<T>(
    items: &[T],
    path: impl Fn(&T) -> &str,
    mut save: impl FnMut(&T) -> Result<(), String>,
) -> SaveAllResult {
    let mut result = SaveAllResult::default();
    for item in items {
        match save(item) {
            Ok(()) => result.saved += 1,
            Err(error) => result.failed.push(SaveFailure {
                path: path(item).to_string(),
                error,
            }),
        }
    }
    result
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
            .map(|content| split_tags(&content))
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
    save_caption_file(image_path, join_tags(&tags))
}

/// 批量保存多个图片的标签
#[tauri::command]
pub fn save_all_tag_files(items: Vec<SaveTagItem>) -> SaveAllResult {
    save_each(
        &items,
        |item| &item.path,
        |item| write_sidecar(existing_image(&item.path)?, "txt", &join_tags(&item.tags)),
    )
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
pub fn save_all_caption_files(items: Vec<SaveCaptionItem>) -> SaveAllResult {
    save_each(
        &items,
        |item| &item.path,
        |item| write_sidecar(existing_image(&item.path)?, "txt", &item.content),
    )
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
    #[serde(default, deserialize_with = "deserialize_null_as_default")]
    pub name: String,
    #[serde(default, deserialize_with = "deserialize_null_as_default")]
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

/// 标签数组字段：收 JSON 数组 ["a","b"] 或逗号串 "a, b"，按 `tag_text::field_tags` 读
/// （去掉首尾空白、丢弃空项）。null 视为字段为空，不让整个文件解析失败；其他类型报错
fn deserialize_string_or_array<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::Error;
    use serde_json::Value;
    match Value::deserialize(deserializer)? {
        value @ (Value::String(_) | Value::Array(_) | Value::Null) => Ok(field_tags(&value)),
        other => Err(D::Error::custom(format!(
            "标签字段应为字符串或数组: {}",
            other
        ))),
    }
}

/// null 按默认值读（字符串为空串、段为空段）：别的工具常把空值写成 null，不能因此整份文件解析失败
fn deserialize_null_as_default<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de> + Default,
{
    Ok(Option::<T>::deserialize(deserializer)?.unwrap_or_default())
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
    #[serde(default, deserialize_with = "deserialize_null_as_default")]
    pub fixed: JsonFixed,
    #[serde(default, deserialize_with = "deserialize_null_as_default")]
    pub character: JsonCharacter,
    #[serde(default, deserialize_with = "deserialize_null_as_default")]
    pub from_path: JsonFromPath,
    #[serde(default, deserialize_with = "deserialize_null_as_default")]
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

/// 解析简化格式 JSON（扁平结构）转为完整格式
fn parse_simplified_format(v: &serde_json::Value) -> Option<JsonTagData> {
    let obj = v.as_object()?;
    let text = |key: &str| obj.get(key).and_then(|v| v.as_str()).map(str::to_string);
    let tags = |key: &str| obj.get(key).map(field_tags).unwrap_or_default();

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
            appearance: tags("appearance"),
            tags: tags("tags"),
            environment: tags("environment"),
            nl: text("nl"),
            ..Default::default()
        },
        ..Default::default()
    })
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
pub fn save_all_json_files(items: Vec<SaveJsonItem>, simplified: bool) -> SaveAllResult {
    save_each(
        &items,
        |item| &item.path,
        |item| {
            let content = serialize_json(&item.data, simplified)?;
            write_sidecar(existing_image(&item.path)?, "json", &content)
        },
    )
}

/// 把 VLM 回复的 JSON 规范成恒定骨架：`simplified` 为 false 时是完整格式
/// （fixed{quality,series,artist} / character{name,variant} / from_path{appearance} /
/// ai_output{count,appearance,tags,environment,nl}），为 true 时是 `to_simplified` 的 9 个扁平键。
///
/// - 两种格式的回复都能读；同一字段在嵌套段和扁平键里都给了时取嵌套段；
/// - 标签数组字段收数组或逗号串，单值字段收字符串或数组（多项以 ", " 连接），都去掉空白和空项；
/// - 画师逐位补 `@`，已带的不重复加；nl 原样保留；
/// - 不认识的字段原样保留：完整格式留在原来的层级；简化格式没有嵌套层级，
///   嵌套段里的未知字段提到顶层（不覆盖已有的键）；
/// - 回复不是 JSON 对象时整段放进 nl（字符串取原文，其他取 JSON 文本）。
pub(crate) fn normalize_tag_json(value: serde_json::Value, simplified: bool) -> serde_json::Value {
    use serde_json::{Map, Value};

    const FLAT_KEYS: [&str; 9] = [
        "quality",
        "series",
        "artist",
        "character",
        "count",
        "appearance",
        "tags",
        "environment",
        "nl",
    ];

    fn tags(value: Option<Value>) -> Vec<String> {
        value.map(|v| field_tags(&v)).unwrap_or_default()
    }
    fn single(value: Option<Value>) -> Option<String> {
        let text = match value? {
            Value::Number(n) => n.to_string(),
            Value::Bool(b) => b.to_string(),
            other => field_tags(&other).join(", "),
        };
        (!text.is_empty()).then_some(text)
    }
    fn raw_text(value: Option<Value>) -> Option<String> {
        match value? {
            Value::String(text) => Some(text),
            Value::Null => None,
            other => Some(other.to_string()),
        }
    }

    let mut data = JsonTagData::default();
    match value {
        Value::Object(top) => {
            let (mut fixed, mut character, mut from_path, mut ai) =
                (Map::new(), Map::new(), Map::new(), Map::new());
            let mut flat = Map::new();
            for (key, value) in top {
                let section = match key.as_str() {
                    "fixed" => Some(&mut fixed),
                    "from_path" => Some(&mut from_path),
                    "ai_output" => Some(&mut ai),
                    "character" if value.is_object() => Some(&mut character),
                    _ => None,
                };
                match (section, value) {
                    (Some(section), Value::Object(map)) => *section = map,
                    // 段名对应的值不是对象：归不了位，留着又会和骨架的同名段冲突
                    (Some(_), _) => {}
                    (None, value) if FLAT_KEYS.contains(&key.as_str()) => {
                        flat.insert(key, value);
                    }
                    (None, value) => {
                        data.extra.insert(key, value);
                    }
                }
            }
            let mut take = |section: &mut Map<String, Value>, key: &str, flat_key: &str| {
                section
                    .shift_remove(key)
                    .filter(|v| !v.is_null())
                    .or_else(|| flat.shift_remove(flat_key))
            };
            let artists: Vec<String> = tags(take(&mut fixed, "artist", "artist"))
                .into_iter()
                .map(|a| {
                    if a.starts_with('@') {
                        a
                    } else {
                        format!("@{}", a)
                    }
                })
                .collect();
            data.fixed.artist = (!artists.is_empty()).then(|| artists.join(", "));
            data.fixed.quality = single(take(&mut fixed, "quality", "quality"));
            data.fixed.series = single(take(&mut fixed, "series", "series"));
            data.character.name =
                single(take(&mut character, "name", "character")).unwrap_or_default();
            data.character.variant = single(character.shift_remove("variant")).unwrap_or_default();
            data.from_path.appearance = tags(from_path.shift_remove("appearance"));
            data.ai_output.count = single(take(&mut ai, "count", "count"));
            data.ai_output.appearance = tags(take(&mut ai, "appearance", "appearance"));
            data.ai_output.tags = tags(take(&mut ai, "tags", "tags"));
            data.ai_output.environment = tags(take(&mut ai, "environment", "environment"));
            data.ai_output.nl = raw_text(take(&mut ai, "nl", "nl"));
            data.fixed.extra = fixed;
            data.character.extra = character;
            data.from_path.extra = from_path;
            data.ai_output.extra = ai;
        }
        other => data.ai_output.nl = raw_text(Some(other)),
    }

    if !simplified {
        return serde_json::to_value(&data).expect("标签结构总能序列化成 JSON");
    }
    let mut out = to_simplified(&data);
    let obj = out.as_object_mut().expect("简化格式是 JSON 对象");
    let nested = [
        data.fixed.extra,
        data.character.extra,
        data.from_path.extra,
        data.ai_output.extra,
    ];
    for (key, value) in data.extra.into_iter().chain(nested.into_iter().flatten()) {
        obj.entry(key).or_insert(value);
    }
    out
}

#[cfg(test)]
mod normalize_tag_json_tests {
    use super::*;
    use serde_json::{json, Value};

    fn keys(v: &Value) -> Vec<&str> {
        v.as_object().unwrap().keys().map(String::as_str).collect()
    }

    const EMPTY_FULL: &str = r#"{"fixed":{"quality":"","series":"","artist":""},"character":{"name":"","variant":""},"from_path":{"appearance":[]},"ai_output":{"count":"","appearance":[],"tags":[],"environment":[],"nl":""}}"#;
    const EMPTY_SIMPLE: &str = r#"{"quality":"","series":"","artist":"","character":"","count":"","appearance":[],"tags":[],"environment":[],"nl":""}"#;

    /// 完整格式默认提示词只要求 ai_output 一段：其余三段补成空骨架，键序固定
    #[test]
    fn partial_full_reply_gets_complete_skeleton() {
        let reply = json!({"ai_output": {
            "count": "1girl", "appearance": ["long hair", " blue eyes "],
            "tags": "smile, standing", "environment": [], "nl": "A girl."
        }});
        let out = normalize_tag_json(reply, false);
        assert_eq!(
            out,
            json!({
                "fixed": {"quality": "", "series": "", "artist": ""},
                "character": {"name": "", "variant": ""},
                "from_path": {"appearance": []},
                "ai_output": {"count": "1girl", "appearance": ["long hair", "blue eyes"],
                              "tags": ["smile", "standing"], "environment": [], "nl": "A girl."}
            })
        );
        assert_eq!(keys(&out), ["fixed", "character", "from_path", "ai_output"]);
        assert_eq!(
            keys(&out["ai_output"]),
            ["count", "appearance", "tags", "environment", "nl"]
        );
        assert!(is_full_json(&out));
    }

    #[test]
    fn simplified_reply_keeps_nine_keys_in_order() {
        let reply = json!({"nl": "desc", "tags": ["smile"], "count": "1girl", "artist": "wlop"});
        let out = normalize_tag_json(reply, true);
        assert_eq!(
            serde_json::to_string(&out).unwrap(),
            r#"{"quality":"","series":"","artist":"@wlop","character":"","count":"1girl","appearance":[],"tags":["smile"],"environment":[],"nl":"desc"}"#
        );
    }

    /// 回复用了另一种格式：扁平键提进嵌套段，嵌套段摊平（from_path.appearance 并入 appearance）
    #[test]
    fn converts_between_layouts() {
        let flat = json!({"quality": "masterpiece", "character": "hatsune miku",
                          "appearance": "twintails, blue hair", "nl": "Miku."});
        let full = normalize_tag_json(flat, false);
        assert_eq!(full["fixed"]["quality"], "masterpiece");
        assert_eq!(
            full["character"],
            json!({"name": "hatsune miku", "variant": ""})
        );
        assert_eq!(
            full["ai_output"]["appearance"],
            json!(["twintails", "blue hair"])
        );
        assert_eq!(full["ai_output"]["nl"], "Miku.");

        let nested = json!({
            "fixed": {"series": "vocaloid"},
            "character": {"name": "miku", "variant": "winter"},
            "from_path": {"appearance": ["twintails"]},
            "ai_output": {"appearance": ["twintails", "scarf"], "tags": ["smile"]}
        });
        let simple = normalize_tag_json(nested, true);
        assert_eq!(simple["series"], "vocaloid");
        assert_eq!(simple["character"], "miku");
        assert_eq!(simple["appearance"], json!(["twintails", "scarf"]));
        assert_eq!(keys(&simple).len(), 9);
    }

    #[test]
    fn single_value_fields_accept_arrays_and_artists_get_one_at() {
        let reply = json!({
            "fixed": {"artist": ["wlop", " @sakimichan ", ""], "quality": ["masterpiece", "best quality"]},
            "ai_output": {"count": ["1girl", "1boy"]}
        });
        let out = normalize_tag_json(reply, false);
        assert_eq!(out["fixed"]["artist"], "@wlop, @sakimichan");
        assert_eq!(out["fixed"]["quality"], "masterpiece, best quality");
        assert_eq!(out["ai_output"]["count"], "1girl, 1boy");
        for (artist, expected) in [
            (json!("wlop，@ask,  "), "@wlop, @ask"),
            (json!("@wlop"), "@wlop"),
            (json!(""), ""),
            (json!(null), ""),
        ] {
            let out = normalize_tag_json(json!({ "artist": artist }), true);
            assert_eq!(out["artist"], expected);
        }
    }

    #[test]
    fn nl_is_kept_verbatim_and_nested_wins_over_flat() {
        let nl = "  Line one,\n  line two.  ";
        let reply = json!({"ai_output": {"nl": nl, "tags": ["a"]}, "tags": ["b"], "nl": "flat"});
        let out = normalize_tag_json(reply, false);
        assert_eq!(out["ai_output"]["nl"], nl);
        assert_eq!(out["ai_output"]["tags"], json!(["a"]));
        assert!(out.get("tags").is_none() && out.get("nl").is_none());
        // 嵌套段写了 null 时退回扁平键
        let out = normalize_tag_json(json!({"ai_output": {"nl": null}, "nl": "flat"}), false);
        assert_eq!(out["ai_output"]["nl"], "flat");
    }

    #[test]
    fn unknown_fields_are_kept() {
        let reply = json!({
            "rating": "safe",
            "fixed": {"quality": "best", "custom_note": "keep"},
            "character": {"name": "miku", "age": 16},
            "ai_output": {"tags": ["smile"], "score": 0.9, "quality": "shadowed"},
            "meta": {"model": "x"}
        });
        let full = normalize_tag_json(reply.clone(), false);
        assert_eq!(full["rating"], "safe");
        assert_eq!(full["meta"], json!({"model": "x"}));
        assert_eq!(full["fixed"]["custom_note"], "keep");
        assert_eq!(full["character"]["age"], 16);
        assert_eq!(full["ai_output"]["score"], 0.9);
        assert_eq!(
            keys(&full),
            [
                "fixed",
                "character",
                "from_path",
                "ai_output",
                "rating",
                "meta"
            ]
        );
        // 与编辑器的读入结构一致：额外字段进各层的 extra，往返不丢
        let data: JsonTagData = serde_json::from_value(full.clone()).unwrap();
        assert_eq!(serde_json::to_value(&data).unwrap(), full);

        let simple = normalize_tag_json(reply, true);
        assert_eq!(simple["quality"], "best");
        assert_eq!(simple["rating"], "safe");
        assert_eq!(simple["custom_note"], "keep");
        assert_eq!(simple["age"], 16);
        assert_eq!(simple["score"], 0.9);
        assert_eq!(&keys(&simple)[..9], &FLAT_KEYS_ORDER);
    }

    const FLAT_KEYS_ORDER: [&str; 9] = [
        "quality",
        "series",
        "artist",
        "character",
        "count",
        "appearance",
        "tags",
        "environment",
        "nl",
    ];

    #[test]
    fn non_object_reply_goes_into_nl() {
        for (reply, nl) in [
            (json!("plain description"), "plain description"),
            (json!(["1girl", "solo"]), r#"["1girl","solo"]"#),
            (json!(42), "42"),
            (json!(null), ""),
        ] {
            let full = normalize_tag_json(reply.clone(), false);
            assert_eq!(full["ai_output"]["nl"], nl);
            let mut expected: Value = serde_json::from_str(EMPTY_FULL).unwrap();
            expected["ai_output"]["nl"] = json!(nl);
            assert_eq!(full, expected);

            let simple = normalize_tag_json(reply, true);
            let mut expected: Value = serde_json::from_str(EMPTY_SIMPLE).unwrap();
            expected["nl"] = json!(nl);
            assert_eq!(simple, expected);
        }
        assert_eq!(
            serde_json::to_string(&normalize_tag_json(json!({}), false)).unwrap(),
            EMPTY_FULL
        );
    }

    /// 段名的值类型不对时丢弃，骨架不会被冲掉
    #[test]
    fn malformed_sections_do_not_break_the_skeleton() {
        let out = normalize_tag_json(
            json!({"fixed": "masterpiece", "ai_output": ["x"], "from_path": null, "tags": "a"}),
            false,
        );
        assert!(out["fixed"].is_object() && out["ai_output"].is_object());
        assert_eq!(out["ai_output"]["tags"], json!(["a"]));
        assert_eq!(keys(&out), ["fixed", "character", "from_path", "ai_output"]);
    }

    #[test]
    fn normalizing_twice_changes_nothing() {
        let reply = json!({
            "fixed": {"artist": "a, @b", "x": 1},
            "character": "miku",
            "ai_output": {"tags": "smile, ", "nl": " text "},
            "extra": [1, 2]
        });
        for simplified in [false, true] {
            let once = normalize_tag_json(reply.clone(), simplified);
            assert_eq!(normalize_tag_json(once.clone(), simplified), once);
        }
    }
}

#[cfg(test)]
mod save_tests {
    use super::*;
    use crate::commands::test_support::TempDir;
    use serde_json::json;

    fn image(root: &Path, name: &str) -> String {
        let path = root.join(name);
        std::fs::write(&path, b"image").unwrap();
        path.to_string_lossy().into_owned()
    }

    /// 只有写成功的条目算 saved，失败项带请求里的图片路径和原因
    #[test]
    fn save_all_reports_each_failure_with_its_image_path() {
        let root = TempDir::new("save_all_tags");
        let ok = image(&root, "ok.png");
        let blocked = image(&root, "blocked.png");
        // 标签文件的位置被同名目录占住，写入必然失败
        std::fs::create_dir(root.join("blocked.txt")).unwrap();
        let missing = root.join("missing.png").to_string_lossy().into_owned();
        let item = |path: &str| SaveTagItem {
            path: path.to_string(),
            tags: vec![" solo".into(), "".into(), "1girl ".into(), "  ".into()],
        };
        let result = save_all_tag_files(vec![item(&ok), item(&blocked), item(&missing)]);

        assert_eq!(result.saved, 1);
        assert_eq!(
            std::fs::read_to_string(root.join("ok.txt")).unwrap(),
            "solo, 1girl"
        );
        let failed: Vec<&str> = result.failed.iter().map(|f| f.path.as_str()).collect();
        assert_eq!(failed, [blocked.as_str(), missing.as_str()]);
        assert!(
            result.failed[0].error.starts_with("写入失败"),
            "{:?}",
            result.failed
        );
        assert_eq!(result.failed[1].error, format!("图片不存在: {}", missing));
        assert!(!root.join("missing.txt").exists());

        let value = serde_json::to_value(&result).unwrap();
        assert_eq!(value["saved"], 1);
        assert_eq!(value["failed"][0]["path"], json!(blocked));
        assert!(value["failed"][0]["error"].is_string());
    }

    #[test]
    fn single_tag_save_trims_and_drops_blank_tags() {
        let root = TempDir::new("save_single_tags");
        let img = image(&root, "a.png");
        save_single_tag_file(img, vec![" solo".into(), " ".into(), "long hair ".into()]).unwrap();
        assert_eq!(
            std::fs::read_to_string(root.join("a.txt")).unwrap(),
            "solo, long hair"
        );
    }

    #[test]
    fn caption_and_json_batches_return_results() {
        let root = TempDir::new("save_all_other");
        let img = image(&root, "a.png");
        let missing = root.join("gone.png").to_string_lossy().into_owned();

        let result = save_all_caption_files(vec![
            SaveCaptionItem {
                path: img.clone(),
                content: " A girl. ".into(),
            },
            SaveCaptionItem {
                path: missing.clone(),
                content: "x".into(),
            },
        ]);
        assert_eq!(result.saved, 1);
        assert_eq!(result.failed.len(), 1);
        assert_eq!(result.failed[0].path, missing);
        // 自然语言描述原样写入
        assert_eq!(
            std::fs::read_to_string(root.join("a.txt")).unwrap(),
            " A girl. "
        );

        // 前端发来的 JSON：标签数组里的空白和空项在读入时就清掉
        let items: Vec<SaveJsonItem> = serde_json::from_value(json!([
            {"path": img, "data": {"ai_output": {"tags": [" smile ", "", "solo"]}}},
            {"path": missing, "data": {}}
        ]))
        .unwrap();
        let result = save_all_json_files(items, true);
        assert_eq!(result.saved, 1);
        assert_eq!(result.failed[0].path, missing);
        let written: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(root.join("a.json")).unwrap()).unwrap();
        assert_eq!(written["tags"], json!(["smile", "solo"]));
    }
}

#[cfg(test)]
mod json_extra_tests {
    use super::*;

    #[test]
    fn full_json_requires_structured_fields() {
        for key in ["ai_output", "fixed", "from_path", "character"] {
            assert!(!is_full_json(&serde_json::json!({key: "text"})));
            assert!(!is_full_json(&serde_json::json!({key: null})));
            assert!(!is_full_json(&serde_json::json!({key: []})));
            assert!(is_full_json(&serde_json::json!({key: {}})));
        }
    }

    /// 角色名、版本或整段写成 null 时按空值读，整份文件照常加载
    #[test]
    fn null_character_name_and_variant_load_as_empty() {
        let root = crate::commands::test_support::TempDir::new("json_null_character");
        std::fs::write(root.join("a.png"), b"image").unwrap();
        std::fs::write(
            root.join("a.json"),
            r#"{"fixed": {"quality": null}, "character": {"name": null, "variant": null},
                "ai_output": {"tags": ["smile"]}}"#,
        )
        .unwrap();
        let dataset = load_json_dataset(root.to_string_lossy().into_owned(), None).unwrap();
        let item = &dataset.images[0];
        assert!(item.has_json && !item.parse_failed);
        assert_eq!(dataset.detected_format, "full");
        assert_eq!(item.data.character.name, "");
        assert_eq!(item.data.character.variant, "");
        assert_eq!(item.data.ai_output.tags, ["smile"]);
        let out = serde_json::to_value(&item.data).unwrap();
        assert_eq!(
            out["character"],
            serde_json::json!({"name": "", "variant": ""})
        );

        let data: JsonTagData = serde_json::from_str(
            r#"{"fixed": null, "character": null, "from_path": null, "ai_output": {"nl": "x"}}"#,
        )
        .unwrap();
        assert_eq!(data.ai_output.nl.as_deref(), Some("x"));
        assert_eq!(
            serde_json::to_string(&data).unwrap(),
            serde_json::to_string(&JsonTagData {
                ai_output: JsonAiOutput {
                    nl: Some("x".into()),
                    ..Default::default()
                },
                ..Default::default()
            })
            .unwrap()
        );
    }

    /// 标签数组字段：数组元素去空白、丢空项和非字符串项；null 为空；数字等类型仍判解析失败
    #[test]
    fn tag_list_fields_are_read_like_other_tag_fields() {
        let data: JsonTagData = serde_json::from_str(
            r#"{"ai_output": {"appearance": [" long hair ", "", 3, "blue eyes"],
                              "tags": "smile，solo", "environment": null}}"#,
        )
        .unwrap();
        assert_eq!(data.ai_output.appearance, ["long hair", "blue eyes"]);
        assert_eq!(data.ai_output.tags, ["smile", "solo"]);
        assert!(data.ai_output.environment.is_empty());
        assert!(serde_json::from_str::<JsonTagData>(r#"{"ai_output": {"tags": 5}}"#).is_err());

        let simplified = parse_simplified_format(&serde_json::json!({
            "appearance": [" twintails ", ""], "tags": "a, b"
        }))
        .unwrap();
        assert_eq!(simplified.ai_output.appearance, ["twintails"]);
        assert_eq!(simplified.ai_output.tags, ["a", "b"]);
    }

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
