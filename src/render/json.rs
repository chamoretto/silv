//! Colored JSON serialization, used by the `json` and `jsonl` render modes.

use ijson::IValue;

use crate::model::value::text_of;
use crate::render::color::Sg;
use crate::render::record_render::Cell;

/// Emits JSON as styled tokens; `pretty` adds newlines and indentation.
pub fn spans(value: &IValue, pretty: bool, out: &mut Vec<Cell>) {
    emit(value, pretty, 0, out);
}

fn push(out: &mut Vec<Cell>, text: &str, style: Sg) {
    if !text.is_empty() {
        out.push(Cell::new(text, style));
    }
}

fn emit(value: &IValue, pretty: bool, depth: usize, out: &mut Vec<Cell>) {
    if let Some(map) = value.as_object() {
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
            push(out, &quoted(key.as_str()), Sg::Key);
            push(out, ": ", Sg::Punct);
            emit(child, pretty, depth + 1, out);
        }
        newline(out, pretty, depth);
        push(out, "}", Sg::Punct);
        return;
    }
    if let Some(items) = value.as_array() {
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
        return;
    }
    if let Some(text) = value.as_string() {
        push(out, &quoted(text.as_str()), Sg::String);
        return;
    }
    if value.is_number() {
        let mut buf = String::new();
        push(out, text_of(value, &mut buf).as_str(), Sg::Number);
        return;
    }
    if value.is_bool() {
        push(out, if value.is_true() { "true" } else { "false" }, Sg::Bool);
        return;
    }
    push(out, "null", Sg::Null);
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

    use crate::fixtures::ivalue;

    fn text(value: &IValue, pretty: bool) -> String {
        let mut out = Vec::new();
        spans(value, pretty, &mut out);
        out.iter().map(|cell| cell.text.as_str()).collect()
    }

    #[test]
    fn compact_json_roundtrip() {
        assert_eq!(text(&ivalue(json!({"a": 1, "b": [true, null]})), false), r#"{"a": 1,"b": [true,null]}"#);
        assert_eq!(text(&ivalue(json!(12)), false), "12");
        assert_eq!(text(&ivalue(json!("a\"b")), false), r#""a\"b""#);
    }

    #[test]
    fn empty_containers_are_compact() {
        assert_eq!(text(&ivalue(json!({})), false), "{}");
        assert_eq!(text(&ivalue(json!([])), false), "[]");
    }

    #[test]
    fn pretty_json_has_newlines() {
        let rendered = text(&ivalue(json!({"a": {"b": 1}})), true);
        assert!(rendered.contains("\n  \"a\": "), "{rendered}");
        assert!(rendered.ends_with("\n}"));
    }

    #[test]
    fn styles_follow_value_kinds() {
        let mut out = Vec::new();
        spans(&ivalue(json!({"k": 1, "s": "x", "n": null})), false, &mut out);
        let styles: Vec<Sg> = out.iter().map(|cell| cell.style).collect();
        assert!(styles.contains(&Sg::Key));
        assert!(styles.contains(&Sg::Number));
        assert!(styles.contains(&Sg::Null));
    }
}
