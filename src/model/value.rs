//! Value helpers: canonical text form, ordering, and scalar traversal.

use std::cmp::Ordering;
use std::fmt::Write as _;

use serde_json::Map;
use serde_json::Number;
use serde_json::Value;

/// Text form of a value, borrowed when the value is already a string.
#[derive(Debug, Clone, Copy)]
pub enum TextRef<'a> {
    Borrowed(&'a str),
    Formatted(&'a str),
}

impl TextRef<'_> {
    pub const fn as_str(&self) -> &str {
        match self {
            Self::Borrowed(s) | Self::Formatted(s) => s,
        }
    }
}

/// Canonical string used for rendering, filters and search.
pub fn text_of<'a>(value: &'a Value, buf: &'a mut String) -> TextRef<'a> {
    match value {
        Value::String(s) => TextRef::Borrowed(s.as_str()),
        Value::Null => TextRef::Borrowed("null"),
        Value::Bool(true) => TextRef::Borrowed("true"),
        Value::Bool(false) => TextRef::Borrowed("false"),
        Value::Number(number) => TextRef::Formatted(number_text(number, buf)),
        Value::Array(_) | Value::Object(_) => TextRef::Formatted(compact_json(value, buf)),
    }
}

fn number_text<'a>(number: &'a Number, buf: &'a mut String) -> &'a str {
    buf.clear();
    if let Some(int) = number.as_i64() {
        let _ = write!(buf, "{int}");
    } else if let Some(unsigned) = number.as_u64() {
        let _ = write!(buf, "{unsigned}");
    } else if let Some(float) = number.as_f64() {
        let _ = write!(buf, "{float}");
    } else {
        let _ = write!(buf, "{number}");
    }
    buf.as_str()
}

fn compact_json<'a>(value: &'a Value, buf: &'a mut String) -> &'a str {
    buf.clear();
    match serde_json::to_string(value) {
        Ok(text) => buf.push_str(&text),
        Err(_) => buf.push_str("<unserializable>"),
    }
    buf.as_str()
}

pub const fn kind_rank(value: &Value) -> u8 {
    match value {
        Value::Null => 0,
        Value::Bool(_) => 1,
        Value::Number(_) => 2,
        Value::String(_) => 3,
        Value::Array(_) => 4,
        Value::Object(_) => 5,
    }
}

pub const fn is_scalar(value: &Value) -> bool {
    !matches!(value, Value::Array(_) | Value::Object(_))
}

pub const fn is_container(value: &Value) -> bool {
    matches!(value, Value::Array(_) | Value::Object(_))
}

/// Total ordering across value kinds, used for sorting.
pub fn compare(a: &Value, b: &Value) -> Ordering {
    match (a, b) {
        (Value::Null, Value::Null) => Ordering::Equal,
        (Value::Bool(x), Value::Bool(y)) => x.cmp(y),
        (Value::Number(x), Value::Number(y)) => compare_numbers(x, y),
        (Value::String(x), Value::String(y)) => x.as_str().cmp(y.as_str()),
        (Value::Array(x), Value::Array(y)) => compare_arrays(x, y),
        (Value::Object(x), Value::Object(y)) => compare_maps(x, y),
        _ => kind_rank(a).cmp(&kind_rank(b)),
    }
}

fn compare_arrays(x: &[Value], y: &[Value]) -> Ordering {
    for (a, b) in x.iter().zip(y) {
        let ord = compare(a, b);
        if ord != Ordering::Equal {
            return ord;
        }
    }
    x.len().cmp(&y.len())
}

fn compare_maps(x: &Map<String, Value>, y: &Map<String, Value>) -> Ordering {
    let left = serde_json::to_string(x).unwrap_or_default();
    let right = serde_json::to_string(y).unwrap_or_default();
    left.cmp(&right)
}

fn compare_numbers(x: &Number, y: &Number) -> Ordering {
    if let (Some(x), Some(y)) = (x.as_i64(), y.as_i64()) {
        return x.cmp(&y);
    }
    if let (Some(x), Some(y)) = (x.as_u64(), y.as_u64()) {
        return x.cmp(&y);
    }
    match (x.as_f64(), y.as_f64()) {
        (Some(x), Some(y)) => x.total_cmp(&y),
        _ => x.to_string().cmp(&y.to_string()),
    }
}

/// Visits every scalar text under `value`, depth limited; stops early when `f` returns true.
pub fn visit_strings<F: FnMut(&str) -> bool>(value: &Value, max_depth: usize, f: &mut F) -> bool {
    visit_inner(value, max_depth, f, &mut String::new())
}

/// Same as [`visit_strings`] for a record's root object, without building a temporary value.
pub fn visit_map_strings<F: FnMut(&str) -> bool>(map: &Map<String, Value>, max_depth: usize, f: &mut F) -> bool {
    if max_depth == 0 {
        return false;
    }
    let mut buf = String::new();
    map.values().any(|item| visit_inner(item, max_depth - 1, f, &mut buf))
}

fn visit_inner<F: FnMut(&str) -> bool>(value: &Value, depth: usize, f: &mut F, buf: &mut String) -> bool {
    match value {
        Value::Array(items) => depth > 0 && items.iter().any(|item| visit_inner(item, depth - 1, f, buf)),
        Value::Object(map) => depth > 0 && map.values().any(|item| visit_inner(item, depth - 1, f, buf)),
        other => f(text_of(other, buf).as_str()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn text_of_borrows_strings() {
        let value = json!("hello");
        let mut buf = String::new();
        assert!(matches!(text_of(&value, &mut buf), TextRef::Borrowed("hello")));
    }

    #[test]
    fn text_of_formats_numbers_and_structures() {
        let mut buf = String::new();
        assert_eq!(text_of(&json!(12), &mut buf).as_str(), "12");
        assert_eq!(text_of(&json!(1.5), &mut buf).as_str(), "1.5");
        assert_eq!(text_of(&json!({"a": 1}), &mut buf).as_str(), r#"{"a":1}"#);
        assert_eq!(text_of(&json!([1, "x"]), &mut buf).as_str(), r#"[1,"x"]"#);
    }

    #[test]
    fn ordering_is_by_kind_then_value() {
        assert_eq!(compare(&json!(null), &json!(false)), Ordering::Less);
        assert_eq!(compare(&json!(2), &json!(10)), Ordering::Less);
        assert_eq!(compare(&json!(-1.5), &json!(-2)), Ordering::Greater);
        assert_eq!(compare(&json!("b"), &json!("a")), Ordering::Greater);
        assert_eq!(compare(&json!(1), &json!("1")), Ordering::Less);
        assert_eq!(compare(&json!([1, 2]), &json!([1, 3])), Ordering::Less);
    }

    #[test]
    fn visits_scalars_and_stops_early() {
        let value = json!({"a": "x", "b": [1, {"c": "y"}]});
        let mut seen = Vec::new();
        visit_strings(&value, 8, &mut |s| {
            seen.push(s.to_owned());
            false
        });
        assert_eq!(seen, vec!["x", "1", "y"]);

        let mut hits = 0;
        visit_strings(&value, 8, &mut |_s| {
            hits += 1;
            true
        });
        assert_eq!(hits, 1);
    }

    #[test]
    fn depth_limit_stops_descending() {
        let value = json!({"a": {"b": "deep"}});
        let mut seen = Vec::new();
        visit_strings(&value, 1, &mut |s| {
            seen.push(s.to_owned());
            false
        });
        assert!(seen.is_empty());
    }
}
