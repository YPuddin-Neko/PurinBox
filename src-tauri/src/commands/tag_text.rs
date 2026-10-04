//! 标签文本的拆分与读取：逗号分隔的标签串、JSON 标签字段（字符串或数组）。

use serde_json::Value;

/// 按中英文逗号拆分标签串：每项去掉首尾空白（含全角空格、换行），丢弃空项。
pub(crate) fn split_tags(text: &str) -> Vec<String> {
    text.split([',', '，'])
        .map(str::trim)
        .filter(|tag| !tag.is_empty())
        .map(str::to_owned)
        .collect()
}

/// 读一个 JSON 标签字段：字符串按 `split_tags` 拆分；数组取其中的字符串元素，
/// 同样去掉首尾空白、丢弃空项（元素内的逗号不再拆分）。null、数字、对象等视为没有标签。
pub(crate) fn field_tags(value: &Value) -> Vec<String> {
    match value {
        Value::String(text) => split_tags(text),
        Value::Array(items) => items
            .iter()
            .filter_map(Value::as_str)
            .map(str::trim)
            .filter(|tag| !tag.is_empty())
            .map(str::to_owned)
            .collect(),
        _ => Vec::new(),
    }
}

/// 把标签列表写成 txt 标签文件的内容：每项去掉首尾空白、丢弃空项，以 ", " 连接
pub(crate) fn join_tags<S: AsRef<str>>(tags: &[S]) -> String {
    tags.iter()
        .map(|tag| tag.as_ref().trim())
        .filter(|tag| !tag.is_empty())
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn split_tags_handles_both_commas_and_blanks() {
        assert_eq!(
            split_tags(" 1girl,solo ，  long hair,\n smile\t,\u{3000}blue eyes\u{3000}"),
            ["1girl", "solo", "long hair", "smile", "blue eyes"]
        );
        assert_eq!(
            split_tags("hatsune miku (racing)"),
            ["hatsune miku (racing)"]
        );
        assert!(split_tags("").is_empty());
        assert!(split_tags(" , ，,\n").is_empty());
    }

    #[test]
    fn field_tags_reads_strings_and_arrays() {
        assert_eq!(field_tags(&json!("a, b，c")), ["a", "b", "c"]);
        assert_eq!(
            field_tags(&json!([" a ", "", "  ", "b, c", 3, null, ["d"], {"e": 1}])),
            ["a", "b, c"]
        );
        for empty in [
            json!(null),
            json!(1),
            json!(true),
            json!({"tags": "a"}),
            json!([]),
        ] {
            assert!(field_tags(&empty).is_empty(), "{empty}");
        }
    }

    #[test]
    fn join_tags_trims_and_drops_blank_items() {
        assert_eq!(
            join_tags(&[" solo", "1girl ", "", "  ", "\u{3000}long hair\u{3000}"]),
            "solo, 1girl, long hair"
        );
        assert_eq!(join_tags(&[String::from("a b")]), "a b");
        assert_eq!(join_tags::<&str>(&[]), "");
        assert_eq!(join_tags(&[" ", ""]), "");
    }
}
