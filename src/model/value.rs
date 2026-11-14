//! Value helpers: canonical text form, ordering, and scalar traversal.

use std::cmp::Ordering;
use std::fmt::Write as _;

use ijson::INumber;
use ijson::IValue;

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
pub fn text_of<'a>(value: &'a IValue, buf: &'a mut String) -> TextRef<'a> {
    if let Some(text) = value.as_string() {
        return TextRef::Borrowed(text.as_str());
    }
    if value.is_null() {
        return TextRef::Borrowed("null");
    }
    if value.is_bool() {
        return TextRef::Borrowed(if value.is_true() { "true" } else { "false" });
    }
    if let Some(number) = value.as_number() {
        return TextRef::Formatted(number_text(number, buf));
    }
    TextRef::Formatted(compact_json(value, buf))
}

fn number_text<'a>(number: &'a INumber, buf: &'a mut String) -> &'a str {
    buf.clear();
    if let Some(int) = number.to_i64() {
        let _ = write!(buf, "{int}");
    } else if let Some(unsigned) = number.to_u64() {
        let _ = write!(buf, "{unsigned}");
    } else {
        let _ = write!(buf, "{}", number.to_f64_lossy());
    }
    buf.as_str()
}

fn compact_json<'a>(value: &'a IValue, buf: &'a mut String) -> &'a str {
    buf.clear();
    match serde_json::to_string(value) {
        Ok(text) => buf.push_str(&text),
        Err(_) => buf.push_str("<unserializable>"),
    }
    buf.as_str()
}

pub fn kind_rank(value: &IValue) -> u8 {
    if value.is_null() {
        0
    } else if value.is_bool() {
        1
    } else if value.is_number() {
        2
    } else if value.is_string() {
        3
    } else if value.is_array() {
        4
    } else {
        5
    }
}

pub fn is_scalar(value: &IValue) -> bool {
    !(value.is_array() || value.is_object())
}

pub fn is_container(value: &IValue) -> bool {
    value.is_array() || value.is_object()
}

/// Total ordering across value kinds, used for sorting.
pub fn compare(a: &IValue, b: &IValue) -> Ordering {
    if a.is_null() && b.is_null() {
        return Ordering::Equal;
    }
    if let (Some(x), Some(y)) = (a.to_bool(), b.to_bool()) {
        return x.cmp(&y);
    }
    if let (Some(x), Some(y)) = (a.as_number(), b.as_number()) {
        return compare_numbers(x, y);
    }
    if let (Some(x), Some(y)) = (a.as_string(), b.as_string()) {
        return x.as_str().cmp(y.as_str());
    }
    if let (Some(x), Some(y)) = (a.as_array(), b.as_array()) {
        return compare_arrays(x, y);
    }
    if let (Some(x), Some(y)) = (a.as_object(), b.as_object()) {
        return compare_objects(x, y);
    }
    kind_rank(a).cmp(&kind_rank(b))
}

fn compare_arrays<'a>(x: &'a ijson::IArray, y: &'a ijson::IArray) -> Ordering {
    let mut left = x.iter();
    let mut right = y.iter();
    loop {
        match (left.next(), right.next()) {
            (Some(a), Some(b)) => {
                let ord = compare(a, b);
                if ord != Ordering::Equal {
                    return ord;
                }
            }
            _ => return x.len().cmp(&y.len()),
        }
    }
}

fn compare_objects(x: &ijson::IObject, y: &ijson::IObject) -> Ordering {
    let left = serde_json::to_string(x).unwrap_or_default();
    let right = serde_json::to_string(y).unwrap_or_default();
    left.cmp(&right)
}

fn compare_numbers(x: &INumber, y: &INumber) -> Ordering {
    if let (Some(x), Some(y)) = (x.to_i64(), y.to_i64()) {
        return x.cmp(&y);
    }
    if let (Some(x), Some(y)) = (x.to_u64(), y.to_u64()) {
        return x.cmp(&y);
    }
    x.to_f64_lossy().total_cmp(&y.to_f64_lossy())
}

/// Visits every scalar text under `value`, depth limited; stops early when `f` returns true.
pub fn visit_strings<F: FnMut(&str) -> bool>(value: &IValue, max_depth: usize, f: &mut F) -> bool {
    visit_inner(value, max_depth, f, &mut String::new())
}

/// Same as [`visit_strings`] for a record's root object value, without descending into it as one node.
pub fn visit_map_strings<F: FnMut(&str) -> bool>(object: &IValue, max_depth: usize, f: &mut F) -> bool {
    let Some(map) = object.as_object() else {
        return false;
    };
    if max_depth == 0 {
        return false;
    }
    let mut buf = String::new();
    map.values().any(|item| visit_inner(item, max_depth - 1, f, &mut buf))
}

fn visit_inner<F: FnMut(&str) -> bool>(value: &IValue, depth: usize, f: &mut F, buf: &mut String) -> bool {
    if let Some(items) = value.as_array() {
        return depth > 0 && items.iter().any(|item| visit_inner(item, depth - 1, f, buf));
    }
    if let Some(map) = value.as_object() {
        return depth > 0 && map.values().any(|item| visit_inner(item, depth - 1, f, buf));
    }
    f(text_of(value, buf).as_str())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn value(json: serde_json::Value) -> IValue {
        ijson::to_value(json).expect("fixture value")
    }

    #[test]
    fn text_of_borrows_strings() {
        let value = value(json!("hello"));
        let mut buf = String::new();
        assert!(matches!(text_of(&value, &mut buf), TextRef::Borrowed("hello")));
    }

    #[test]
    fn text_of_formats_numbers_and_structures() {
        let mut buf = String::new();
        assert_eq!(text_of(&value(json!(12)), &mut buf).as_str(), "12");
        assert_eq!(text_of(&value(json!(1.5)), &mut buf).as_str(), "1.5");
        assert_eq!(text_of(&value(json!({"a": 1})), &mut buf).as_str(), r#"{"a":1}"#);
        assert_eq!(text_of(&value(json!([1, "x"])), &mut buf).as_str(), r#"[1,"x"]"#);
    }

    #[test]
    fn ordering_is_by_kind_then_value() {
        assert_eq!(compare(&value(json!(null)), &value(json!(false))), Ordering::Less);
        assert_eq!(compare(&value(json!(2)), &value(json!(10))), Ordering::Less);
        assert_eq!(compare(&value(json!(-1.5)), &value(json!(-2))), Ordering::Greater);
        assert_eq!(compare(&value(json!("b")), &value(json!("a"))), Ordering::Greater);
        assert_eq!(compare(&value(json!(1)), &value(json!("1"))), Ordering::Less);
        assert_eq!(compare(&value(json!([1, 2])), &value(json!([1, 3]))), Ordering::Less);
    }

    #[test]
    fn visits_scalars_and_stops_early() {
        let value = value(json!({"a": "x", "b": [1, {"c": "y"}]}));
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
        let value = value(json!({"a": {"b": "deep"}}));
        let mut seen = Vec::new();
        visit_strings(&value, 1, &mut |s| {
            seen.push(s.to_owned());
            false
        });
        assert!(seen.is_empty());
    }
}
