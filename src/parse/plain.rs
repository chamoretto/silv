//! Basic parser for stdout-like lines: `timestamp | LEVEL | message key=value ...`.

use std::str::FromStr;

use serde_json::Map;
use serde_json::Value;

use crate::model::level::LogLevel;
use crate::model::record::FieldAliases;
use crate::model::record::LogRecord;
use crate::model::timestamp::Timestamp;
use crate::parse::LineResult;
use crate::parse::ParseIssue;

/// Layout knobs for the plain format.
#[derive(Clone, Debug)]
pub struct PlainOptions {
    pub separator: String,
}

impl Default for PlainOptions {
    fn default() -> Self {
        Self {
            separator: " | ".to_owned(),
        }
    }
}

pub fn parse_line(
    source: u32,
    number: u64,
    offset: usize,
    bytes: &[u8],
    aliases: &FieldAliases,
    options: &PlainOptions,
) -> LineResult {
    let raw = String::from_utf8_lossy(bytes).into_owned();
    let text = raw.trim_end_matches(['\r', '\n']);
    if text.trim().is_empty() {
        return Err(ParseIssue {
            source,
            line: number,
            offset,
            reason: "blank line".to_owned(),
            raw,
        });
    }

    let parts: Vec<&str> = if options.separator.is_empty() {
        vec![text]
    } else {
        text.split(options.separator.as_str()).map(str::trim).collect()
    };

    let mut object = Map::new();
    let mut message_parts: Vec<String> = Vec::new();
    let mut index = 0usize;

    // The leading columns are only honoured when the first segment really parses as a timestamp.
    if parts.len() > 1 && Timestamp::parse(parts[0]).is_some() {
        object.insert("timestamp".to_owned(), Value::String(parts[0].to_owned()));
        index = 1;
        if matches!(LogLevel::from_str(parts[1]), Ok(LogLevel::Known(_))) {
            object.insert("level".to_owned(), Value::String(parts[1].to_owned()));
            index = 2;
        }
    }

    for part in &parts[index..] {
        message_parts.push(collect_part(part, &mut object));
    }

    let message = message_parts
        .iter()
        .map(String::as_str)
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(options.separator.as_str());
    if message.is_empty() {
        object.insert("message".to_owned(), Value::String(text.to_owned()));
    } else {
        object.insert("message".to_owned(), Value::String(message));
    }

    let mut record = LogRecord::from_object(source, number, offset, object, aliases);
    record.raw = Some(raw.into());
    Ok(record)
}

/// Splits a segment into message text and `key=value` pairs.
fn collect_part(part: &str, object: &mut Map<String, Value>) -> String {
    let mut message: Vec<String> = Vec::new();
    let chars: Vec<char> = part.chars().collect();
    let mut cursor = 0usize;
    while cursor < chars.len() {
        skip_spaces(&chars, &mut cursor);
        if cursor >= chars.len() {
            break;
        }
        let (raw, plain) = read_token(&chars, &mut cursor);
        match split_assignment(&raw) {
            Some((key, value)) => insert_nested(object, key, parse_scalar(&value)),
            None => message.push(plain),
        }
    }
    message.join(" ")
}

fn skip_spaces(chars: &[char], cursor: &mut usize) {
    while *cursor < chars.len() && chars[*cursor].is_ascii_whitespace() {
        *cursor += 1;
    }
}

/// Reads one token, returning the raw slice (with quotes) and the unquoted text.
fn read_token(chars: &[char], cursor: &mut usize) -> (String, String) {
    let mut raw = String::new();
    let mut plain = String::new();
    let mut in_quotes = false;
    while *cursor < chars.len() {
        let ch = chars[*cursor];
        if !in_quotes && ch.is_ascii_whitespace() {
            break;
        }
        if ch == '"' {
            in_quotes = !in_quotes;
            raw.push(ch);
            *cursor += 1;
            continue;
        }
        raw.push(ch);
        plain.push(ch);
        *cursor += 1;
    }
    (raw, plain)
}

/// `key=value`, where the key is a plain identifier that may contain dots.
fn split_assignment(token: &str) -> Option<(&str, String)> {
    let equals = token.find('=')?;
    let key = &token[..equals];
    if key.is_empty() || !is_key(key) {
        return None;
    }
    let value = &token[equals + 1..];
    let value = value.trim_matches('"').to_owned();
    Some((key, value))
}

fn is_key(key: &str) -> bool {
    if key.starts_with('.') || key.ends_with('.') {
        return false;
    }
    let mut first = true;
    key.chars().all(|ch| {
        if first {
            first = false;
            ch.is_ascii_alphabetic() || ch == '_' || ch == '@'
        } else {
            ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.' | '@' | ':')
        }
    })
}

/// Turns a token into a typed value, keeping the text when it is not a JSON scalar.
fn parse_scalar(text: &str) -> Value {
    match text {
        "true" => return Value::Bool(true),
        "false" => return Value::Bool(false),
        "null" => return Value::Null,
        _ => {},
    }
    if let Ok(int) = text.parse::<i64>() {
        return Value::from(int);
    }
    if let Ok(float) = text.parse::<f64>()
        && float.is_finite()
    {
        return Value::from(float);
    }
    if ((text.starts_with('[') && text.ends_with(']')) || (text.starts_with('{') && text.ends_with('}')))
        && let Ok(parsed) = serde_json::from_str::<Value>(text)
    {
        return parsed;
    }
    Value::String(text.to_owned())
}

/// Stores dotted keys as nested objects, falling back to overwriting on type conflicts.
fn insert_nested(object: &mut Map<String, Value>, key: &str, value: Value) {
    if !key.contains('.') {
        object.insert(key.to_owned(), value);
        return;
    }
    let mut current = value;
    for segment in key.split('.').rev() {
        let mut map = Map::new();
        map.insert(segment.to_owned(), current);
        current = Value::Object(map);
    }
    merge_into(object, current);
}

fn merge_into(target: &mut Map<String, Value>, source: Value) {
    let Value::Object(source) = source else {
        return;
    };
    for (key, value) in source {
        if value.is_object()
            && let Some(Value::Object(child)) = target.get_mut(&key)
        {
            merge_into(child, value);
            continue;
        }
        target.insert(key, value);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::level::Level;
    use serde_json::json;

    fn aliases() -> FieldAliases {
        FieldAliases::with_defaults(&[], &[], &[]).expect("defaults")
    }

    fn parse(text: &str) -> LogRecord {
        parse_line(0, 1, 0, text.as_bytes(), &aliases(), &PlainOptions::default()).expect("parses")
    }

    #[test]
    fn parses_full_stdout_line() {
        let record = parse("2023-11-14 22:13:20.123 | INFO | worker started host=web1 pid=42 topic=\"orders in\"");
        assert_eq!(record.level, Some(Level::Info.into()));
        assert_eq!(record.message.as_deref(), Some("worker started"));
        assert_eq!(record.object.get("host"), Some(&json!("web1")));
        assert_eq!(record.object.get("pid"), Some(&json!(42)));
        assert_eq!(record.object.get("topic"), Some(&json!("orders in")));
        assert_eq!(record.object.get("timestamp"), Some(&json!("2023-11-14 22:13:20.123")));
        assert!(record.raw_text().starts_with("2023-11-14 22:13:20.123 | INFO |"));
    }

    #[test]
    fn nests_dotted_keys() {
        let record = parse("2023-11-14 22:13:20.123 | WARN | slow trace.id=abc-1 trace.ms=12.5");
        assert_eq!(record.object["trace"]["id"], json!("abc-1"));
        assert_eq!(record.object["trace"]["ms"], json!(12.5));
        assert_eq!(record.level, Some(Level::Warn.into()));
    }

    #[test]
    fn keeps_message_only_lines() {
        let record = parse("plain text without separators");
        assert_eq!(record.message.as_deref(), Some("plain text without separators"));
        assert_eq!(record.level, None);
    }

    #[test]
    fn level_is_optional() {
        let record = parse("2023-11-14 22:13:20.123 | no severity keyword here");
        assert_eq!(record.message.as_deref(), Some("no severity keyword here"));
        assert_eq!(record.level, None);
    }

    #[test]
    fn typed_scalars() {
        let record =
            parse("2023-11-14 22:13:20.123 | ERROR | boom ok=true bad=false missing=null ratio=0.25 list=[1,2]");
        assert_eq!(record.object["ok"], json!(true));
        assert_eq!(record.object["bad"], json!(false));
        assert_eq!(record.object["missing"], json!(null));
        assert_eq!(record.object["ratio"], json!(0.25));
        assert_eq!(record.object["list"], json!([1, 2]));
    }

    #[test]
    fn scalar_key_conflict_does_not_break_parsing() {
        let record = parse("2023-11-14 22:13:20.123 | INFO | a=1 a.b=2");
        assert_eq!(record.object["a"]["b"], json!(2));
    }
}
