//! Dotted field paths used for columns, filters, sorting and search scope.

use std::cmp::Ordering;
use std::fmt::Display;
use std::fmt::Formatter;
use std::str::FromStr;
use std::sync::Arc;

use serde_json::Map;
use serde_json::Value;

/// One element of a [`FieldPath`].
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub enum Segment {
    Key(Arc<str>),
    Index(usize),
    KeyWildcard,
    IndexWildcard,
}

impl Display for Segment {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Key(key) => write_quoted(f, key),
            Self::Index(index) => write!(f, "[{index}]"),
            Self::KeyWildcard => f.write_str("*"),
            Self::IndexWildcard => f.write_str("[*]"),
        }
    }
}

fn write_quoted(f: &mut Formatter<'_>, key: &str) -> std::fmt::Result {
    if needs_quotes(key) {
        let mut out = String::with_capacity(key.len() + 2);
        out.push('"');
        for ch in key.chars() {
            if ch == '"' || ch == '\\' {
                out.push('\\');
            }
            out.push(ch);
        }
        out.push('"');
        return f.write_str(&out);
    }
    f.write_str(key)
}

fn needs_quotes(key: &str) -> bool {
    key.is_empty()
        || key == "*"
        || key.chars().any(|c| {
            matches!(
                c,
                '.' | '['
                    | ']'
                    | '*'
                    | '"'
                    | '\\'
                    | '!'
                    | '~'
                    | '='
                    | '('
                    | ')'
                    | ' '
                    | '\t'
                    | '\n'
                    | '{'
                    | '}'
                    | ','
                    | ':'
            )
        })
}

/// Path to a value inside a record: `a.b[2]."x.y"`.
#[derive(Clone, PartialEq, Eq, Hash, Debug, Default)]
pub struct FieldPath {
    segments: Vec<Segment>,
}

impl FieldPath {
    pub fn root() -> Self {
        Self::default()
    }

    pub fn from_key(key: &str) -> Self {
        Self {
            segments: vec![Segment::Key(Arc::from(key))],
        }
    }

    pub fn from_segments(segments: Vec<Segment>) -> Self {
        Self { segments }
    }

    pub fn segments(&self) -> &[Segment] {
        &self.segments
    }

    pub fn len(&self) -> usize {
        self.segments.len()
    }

    pub const fn is_empty(&self) -> bool {
        self.segments.is_empty()
    }

    pub fn is_wildcard(&self) -> bool {
        self.segments
            .iter()
            .any(|s| matches!(s, Segment::KeyWildcard | Segment::IndexWildcard))
    }

    /// Path of the first segment, used as a column label prefix.
    pub fn first_key(&self) -> Option<&str> {
        match self.segments.first() {
            Some(Segment::Key(key)) => Some(key),
            _ => None,
        }
    }

    pub fn starts_with(&self, prefix: &FieldPath) -> bool {
        self.segments.starts_with(&prefix.segments)
    }

    /// Parses user supplied path syntax.
    pub fn parse(input: &str) -> Result<Self, String> {
        let chars: Vec<char> = input.chars().collect();
        let mut segments = Vec::new();
        let mut i = 0usize;
        let mut expect_key = true;

        while i < chars.len() {
            match chars[i] {
                '.' => {
                    if expect_key {
                        return Err(format!("empty path segment at position {}", i + 1));
                    }
                    expect_key = true;
                    i += 1;
                    if i < chars.len() && chars[i] == '[' {
                        expect_key = false;
                    }
                },
                '[' => {
                    let (segment, next) = parse_bracket(&chars, i)?;
                    segments.push(segment);
                    i = next;
                    expect_key = false;
                },
                _ => {
                    let (key, next) = parse_key(&chars, i)?;
                    if key == "*" {
                        segments.push(Segment::KeyWildcard);
                    } else {
                        segments.push(Segment::Key(Arc::from(key.as_str())));
                    }
                    i = next;
                    expect_key = false;
                },
            }
        }

        if segments.is_empty() {
            return Err("empty path".to_owned());
        }
        if expect_key {
            return Err("path ends with a separator".to_owned());
        }
        Ok(Self { segments })
    }

    /// Collects every value addressed by this path (more than one when wildcards are used).
    pub fn collect<'a>(&self, root: &'a Map<String, Value>, out: &mut Vec<&'a Value>) {
        let Some(first) = self.segments.first() else {
            return;
        };
        match first {
            Segment::Key(key) => {
                if let Some(value) = root.get(key.as_ref()) {
                    collect_value(&self.segments[1..], value, out);
                }
            },
            Segment::KeyWildcard => {
                for value in root.values() {
                    collect_value(&self.segments[1..], value, out);
                }
            },
            Segment::Index(_) | Segment::IndexWildcard => {},
        }
    }

    /// Convenience single-value lookup, first match wins.
    pub fn get<'a>(&self, root: &'a Map<String, Value>) -> Option<&'a Value> {
        let mut out = Vec::with_capacity(1);
        self.collect(root, &mut out);
        out.into_iter().next()
    }

    /// Same as [`Self::collect`] starting from any JSON value.
    pub fn collect_from_value<'a>(&self, root: &'a Value, out: &mut Vec<&'a Value>) {
        collect_value(&self.segments, root, out);
    }
}

fn collect_value<'a>(segs: &[Segment], value: &'a Value, out: &mut Vec<&'a Value>) {
    let Some(first) = segs.first() else {
        out.push(value);
        return;
    };
    match (first, value) {
        (Segment::Key(key), Value::Object(map)) => {
            if let Some(child) = map.get(key.as_ref()) {
                collect_value(&segs[1..], child, out);
            }
        },
        // Keys applied to arrays descend into elements, mirroring how nested arrays of objects are logged.
        (Segment::Key(key), Value::Array(items)) => {
            for item in items {
                if let Value::Object(map) = item
                    && let Some(child) = map.get(key.as_ref())
                {
                    collect_value(&segs[1..], child, out);
                }
            }
        },
        (Segment::KeyWildcard, Value::Object(map)) => {
            for child in map.values() {
                collect_value(&segs[1..], child, out);
            }
        },
        (Segment::KeyWildcard, Value::Array(items)) => {
            for item in items {
                collect_value(&segs[1..], item, out);
            }
        },
        (Segment::Index(index), Value::Array(items)) => {
            if let Some(child) = items.get(*index) {
                collect_value(&segs[1..], child, out);
            }
        },
        (Segment::IndexWildcard, Value::Array(items)) => {
            for item in items {
                collect_value(&segs[1..], item, out);
            }
        },
        _ => {},
    }
}

fn parse_key(chars: &[char], start: usize) -> Result<(String, usize), String> {
    let mut out = String::new();
    let mut i = start;
    if chars[i] == '"' {
        i += 1;
        while i < chars.len() {
            match chars[i] {
                '"' => {
                    i += 1;
                    return Ok((out, i));
                },
                '\\' if i + 1 < chars.len() => {
                    out.push(chars[i + 1]);
                    i += 2;
                },
                _ => {
                    out.push(chars[i]);
                    i += 1;
                },
            }
        }
        return Err("unterminated quoted segment".to_owned());
    }
    while i < chars.len() && !matches!(chars[i], '.' | '[') {
        out.push(chars[i]);
        i += 1;
    }
    if out.is_empty() {
        return Err(format!("empty path segment at position {}", start + 1));
    }
    Ok((out, i))
}

fn parse_bracket(chars: &[char], start: usize) -> Result<(Segment, usize), String> {
    let mut i = start + 1;
    if i >= chars.len() {
        return Err("unterminated `[`".to_owned());
    }
    if chars[i] == '*' {
        i += 1;
        if i < chars.len() && chars[i] == ']' {
            return Ok((Segment::IndexWildcard, i + 1));
        }
        return Err("expected `]` after `[*]`".to_owned());
    }
    let mut token = String::new();
    let quoted = chars[i] == '"';
    if quoted {
        i += 1;
    }
    while i < chars.len() && (chars[i] != ']' || quoted) {
        token.push(chars[i]);
        i += 1;
        if quoted && i < chars.len() && chars[i] == '"' {
            i += 1;
            break;
        }
    }
    if i >= chars.len() {
        return Err("unterminated `[`".to_owned());
    }
    i += 1;
    if quoted {
        return Ok((Segment::Key(Arc::from(token.as_str())), i));
    }
    if token == "*" {
        return Ok((Segment::IndexWildcard, i));
    }
    let index: usize = token.parse().map_err(|_| format!("`{token}` is not an array index"))?;
    Ok((Segment::Index(index), i))
}

impl Display for FieldPath {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        let mut first = true;
        for segment in &self.segments {
            match segment {
                Segment::Index(_) | Segment::IndexWildcard => {
                    write!(f, "{segment}")?;
                },
                _ => {
                    if !first {
                        f.write_str(".")?;
                    }
                    write!(f, "{segment}")?;
                },
            }
            first = false;
        }
        Ok(())
    }
}

impl FromStr for FieldPath {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s)
    }
}

impl Ord for FieldPath {
    fn cmp(&self, other: &Self) -> Ordering {
        self.segments.cmp(&other.segments)
    }
}

impl PartialOrd for FieldPath {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn map(text: &str) -> Map<String, Value> {
        serde_json::from_str(text).expect("test fixture is a JSON object")
    }

    #[test]
    fn parses_plain_and_quoted_paths() {
        let path = FieldPath::parse("a.\"b.c\"[2][*].d").expect("parses");
        assert_eq!(path.segments().len(), 5);
        assert_eq!(path.to_string(), "a.\"b.c\"[2][*].d");
        assert_eq!(FieldPath::parse("a..b").unwrap_err(), "empty path segment at position 3");
        assert_eq!(FieldPath::parse("a[1].").unwrap_err(), "path ends with a separator");
        assert_eq!(FieldPath::parse("a[1").unwrap_err(), "unterminated `[`");
    }

    #[test]
    fn quoted_keys_survive_display_roundtrip() {
        let path = FieldPath::from_segments(vec![Segment::Key(Arc::from("x.y")), Segment::Index(3)]);
        let text = path.to_string();
        assert_eq!(text, "\"x.y\"[3]");
        assert_eq!(FieldPath::parse(&text).expect("reparses"), path);
    }

    #[test]
    fn collects_values_with_wildcards() {
        let root = map(r#"{"a":{"b":[1,2]},"arr":[{"k":"v"},{"k":"w"}],"*":"star"}"#);
        let mut out = Vec::new();
        FieldPath::parse("a.b[*]").unwrap().collect(&root, &mut out);
        assert_eq!(out, vec![&json!(1), &json!(2)]);

        out.clear();
        FieldPath::parse("arr.k").unwrap().collect(&root, &mut out);
        assert_eq!(out, vec![&json!("v"), &json!("w")]);

        out.clear();
        FieldPath::parse("*").unwrap().collect(&root, &mut out);
        assert_eq!(out.len(), 3);
    }

    #[test]
    fn root_path_is_empty() {
        assert!(FieldPath::root().is_empty());
        assert!(!FieldPath::parse("a").unwrap().is_wildcard());
        assert!(FieldPath::parse("a[*]").unwrap().is_wildcard());
    }
}
