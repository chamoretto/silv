//! Colored JSON serialization, used by the `json` and `jsonl` render modes.

use serde_json::Value;

use crate::render::color::Sg;
use crate::render::record_render::Cell;

/// Emits JSON as styled tokens; `pretty` adds newlines and indentation.
pub fn spans(value: &Value, pretty: bool, out: &mut Vec<Cell>) {
    emit(value, pretty, 0, out);
}

fn push(out: &mut Vec<Cell>, text: &str, style: Sg) {
    if !text.is_empty() {
        out.push(Cell::new(text, style));
    }
}

fn emit(value: &Value, pretty: bool, depth: usize, out: &mut Vec<Cell>) {
    match value {
        Value::Object(map) => {
            if map.is_empty() {
                push(out, "{}", Sg::Punct);
                return;
            }
            push(out, "{", Sg::Punct);
            for (index, (key, child)) in map.iter().enumerate() {
                if index > 0 {
                    push(out, ",", Sg::Punct);
                }
                newline(out, pretty, depth + 1);
                push(out, &quoted(key), Sg::Key);
                push(out, ": ", Sg::Punct);
                emit(child, pretty, depth + 1, out);
            }
            newline(out, pretty, depth);
            push(out, "}", Sg::Punct);
        },
        Value::Array(items) => {
            if items.is_empty() {
                push(out, "[]", Sg::Punct);
                return;
            }
            push(out, "[", Sg::Punct);
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    push(out, ",", Sg::Punct);
                }
                newline(out, pretty, depth + 1);
                emit(item, pretty, depth + 1, out);
            }
            newline(out, pretty, depth);
            push(out, "]", Sg::Punct);
        },
        Value::String(text) => push(out, &quoted(text), Sg::String),
        Value::Number(number) => push(out, &number.to_string(), Sg::Number),
        Value::Bool(value) => push(out, if *value { "true" } else { "false" }, Sg::Bool),
        Value::Null => push(out, "null", Sg::Null),
    }
}

fn newline(out: &mut Vec<Cell>, pretty: bool, depth: usize) {
    if pretty {
        let mut indent = String::from("\n");
        indent.push_str(&"  ".repeat(depth));
        push(out, &indent, Sg::Default);
    }
}

fn quoted(text: &str) -> String {
    serde_json::to_string(text).unwrap_or_else(|_| format!("\"{text}\""))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn text(value: &Value, pretty: bool) -> String {
        let mut out = Vec::new();
        spans(value, pretty, &mut out);
        out.iter().map(|cell| cell.text.as_str()).collect()
    }

    #[test]
    fn compact_json_roundtrip() {
        assert_eq!(text(&json!({"a": 1, "b": [true, null]}), false), r#"{"a": 1,"b": [true,null]}"#);
        assert_eq!(text(&json!(12), false), "12");
        assert_eq!(text(&json!("a\"b"), false), r#""a\"b""#);
    }

    #[test]
    fn empty_containers_are_compact() {
        assert_eq!(text(&json!({}), false), "{}");
        assert_eq!(text(&json!([]), false), "[]");
    }

    #[test]
    fn pretty_json_has_newlines() {
        let rendered = text(&json!({"a": {"b": 1}}), true);
        assert!(rendered.contains("\n  \"a\": "), "{rendered}");
        assert!(rendered.ends_with("\n}"));
    }

    #[test]
    fn styles_follow_value_kinds() {
        let mut out = Vec::new();
        spans(&json!({"k": 1, "s": "x", "n": null}), false, &mut out);
        let styles: Vec<Sg> = out.iter().map(|cell| cell.style).collect();
        assert!(styles.contains(&Sg::Key));
        assert!(styles.contains(&Sg::Number));
        assert!(styles.contains(&Sg::Null));
    }
}
