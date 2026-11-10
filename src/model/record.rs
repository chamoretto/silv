//! Log record representation and field alias resolution.

use std::str::FromStr;

use serde_json::Map;
use serde_json::Value;

use crate::model::field::FieldPath;
use crate::model::level::LogLevel;
use crate::model::timestamp::TimeFormat;
use crate::model::timestamp::Timestamp;
use crate::model::value::text_of;

/// Sentinel for a timestamp that exists as text but has no parseable instant.
pub const UNPARSED_NANOS: i128 = i128::MIN;

/// Keys probed for the timestamp column, in priority order.
pub const DEFAULT_TIMESTAMP_KEYS: [&str; 16] = [
    "@timestamp",
    "timestamp",
    "system_create_datetime",
    "time_iso8601",
    "time",
    "@time",
    "ts",
    "datetime",
    "date",
    "event.timestamp",
    "log.time",
    "timeMillis",
    "unixTime",
    "asctime",
    "created",
    "t",
];

/// Keys probed for the severity column, in priority order.
pub const DEFAULT_LEVEL_KEYS: [&str; 12] = [
    "level",
    "severity",
    "log.level",
    "severity_text",
    "levelname",
    "level_name",
    "lvl",
    "lv",
    "ll",
    "levelNo",
    "syslog.severity",
    "severityNumber",
];

/// Keys probed for the message column, in priority order.
pub const DEFAULT_MESSAGE_KEYS: [&str; 9] = [
    "message",
    "msg",
    "log.message",
    "body",
    "text",
    "logline",
    "event",
    "log",
    "line",
];

/// Which paths provide the timestamp, severity and message of a record.
#[derive(Clone, Debug)]
pub struct FieldAliases {
    pub timestamp: Vec<FieldPath>,
    pub level: Vec<FieldPath>,
    pub message: Vec<FieldPath>,
}

impl Default for FieldAliases {
    /// The default alias lists (without user overrides).
    fn default() -> Self {
        Self::with_defaults(&[], &[], &[]).unwrap_or(Self {
            timestamp: Vec::new(),
            level: Vec::new(),
            message: Vec::new(),
        })
    }
}

impl FieldAliases {
    /// Builds aliases from user overrides (highest priority) followed by defaults.
    pub fn with_defaults(timestamp: &[String], level: &[String], message: &[String]) -> Result<Self, (String, String)> {
        Ok(Self {
            timestamp: merge_paths(timestamp, &DEFAULT_TIMESTAMP_KEYS)?,
            level: merge_paths(level, &DEFAULT_LEVEL_KEYS)?,
            message: merge_paths(message, &DEFAULT_MESSAGE_KEYS)?,
        })
    }

    fn first_value<'a>(&self, list: &[FieldPath], object: &'a Map<String, Value>) -> Option<&'a Value> {
        list.iter().find_map(|path| path.get(object))
    }

    /// The alias path that actually supplied a value, used to avoid repeating it in kv output.
    pub fn first_path(&self, list: &[FieldPath], object: &Map<String, Value>) -> Option<FieldPath> {
        list.iter()
            .filter(|path| !path.is_empty())
            .find(|path| path.get(object).is_some())
            .cloned()
    }

    pub fn timestamp_value<'a>(&self, object: &'a Map<String, Value>) -> Option<&'a Value> {
        self.first_value(&self.timestamp, object)
    }

    pub fn level_value<'a>(&self, object: &'a Map<String, Value>) -> Option<&'a Value> {
        self.first_value(&self.level, object)
    }

    pub fn message_value<'a>(&self, object: &'a Map<String, Value>) -> Option<&'a Value> {
        self.first_value(&self.message, object)
    }
}

fn merge_paths(custom: &[String], defaults: &[&str]) -> Result<Vec<FieldPath>, (String, String)> {
    let mut out = Vec::with_capacity(custom.len() + defaults.len());
    for raw in custom.iter().map(String::as_str).chain(defaults.iter().copied()) {
        let path = FieldPath::parse(raw).map_err(|reason| (raw.to_owned(), reason))?;
        if !out.contains(&path) {
            out.push(path);
        }
    }
    Ok(out)
}

/// A parsed log entry.
#[derive(Clone, Debug)]
pub struct LogRecord {
    pub source: u32,
    pub line: u64,
    pub offset: usize,
    pub timestamp: Option<Timestamp>,
    pub level: Option<LogLevel>,
    pub message: Option<String>,
    pub object: Map<String, Value>,
    pub raw: Option<Box<str>>,
}

impl LogRecord {
    /// Extracts the well-known columns; every other key stays addressable by field path.
    pub fn from_object(
        source: u32,
        line: u64,
        offset: usize,
        object: Map<String, Value>,
        aliases: &FieldAliases,
    ) -> Self {
        let timestamp = aliases
            .timestamp_value(&object)
            .and_then(Timestamp::from_value)
            .or_else(|| aliases.timestamp_value(&object).and_then(unparsed_stamp));
        let level = aliases.level_value(&object).and_then(|value| {
            let mut buf = String::new();
            LogLevel::from_str(text_of(value, &mut buf).as_str()).ok()
        });
        let message = aliases.message_value(&object).map(|value| {
            let mut buf = String::new();
            text_of(value, &mut buf).as_str().to_owned()
        });
        Self {
            source,
            line,
            offset,
            timestamp,
            level,
            message,
            object,
            raw: None,
        }
    }

    /// Record for a line that failed to parse, kept so the line is still visible.
    pub fn invalid(source: u32, line: u64, offset: usize, reason: &str, raw: &str) -> Self {
        let mut object = Map::new();
        object.insert("level".to_owned(), Value::String("INVALID".to_owned()));
        object.insert("message".to_owned(), Value::String(reason.to_owned()));
        object.insert("raw".to_owned(), Value::String(raw.to_owned()));
        Self {
            source,
            line,
            offset,
            timestamp: None,
            level: Some(LogLevel::Custom("INVALID".to_owned())),
            message: Some(reason.to_owned()),
            object,
            raw: Some(raw.into()),
        }
    }

    pub fn is_invalid(&self) -> bool {
        self.raw.is_some() && matches!(self.level, Some(LogLevel::Custom(_)))
    }

    pub fn value_at(&self, path: &FieldPath) -> Option<&Value> {
        path.get(&self.object)
    }

    pub fn text_at(&self, path: &FieldPath, buf: &mut String) -> Option<String> {
        self.value_at(path).map(|value| text_of(value, buf).as_str().to_owned())
    }

    pub fn raw_text(&self) -> &str {
        self.raw.as_deref().unwrap_or_default()
    }

    /// Timestamp rendered for display; empty when the record has no timestamp.
    pub fn stamp(&self, format: TimeFormat) -> String {
        self.timestamp
            .as_ref()
            .map(|stamp| stamp.format(format))
            .unwrap_or_default()
    }
}

fn unparsed_stamp(value: &Value) -> Option<Timestamp> {
    match value {
        Value::String(text) if !text.trim().is_empty() => Some(Timestamp::from_nanos(UNPARSED_NANOS, text)),
        Value::Number(number) => Some(Timestamp::from_nanos(UNPARSED_NANOS, &number.to_string())),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::level::Level;
    use serde_json::json;

    fn aliases() -> FieldAliases {
        FieldAliases::with_defaults(&[], &[], &[]).expect("defaults parse")
    }

    fn object(text: &str) -> Map<String, Value> {
        serde_json::from_str(text).expect("fixture is a JSON object")
    }

    #[test]
    fn extracts_columns_from_default_aliases() {
        let record = LogRecord::from_object(
            0,
            1,
            0,
            object(
                r#"{"@timestamp":"2023-11-14T22:13:20.123Z","level":"WARNING","message":"disk full","host":"web1"}"#,
            ),
            &aliases(),
        );
        assert_eq!(record.level, Some(LogLevel::Known(Level::Warn)));
        assert_eq!(record.message.as_deref(), Some("disk full"));
        assert_eq!(record.stamp(TimeFormat::UtcDateTime), "2023-11-14 22:13:20.123");
    }

    #[test]
    fn nested_alias_paths_and_custom_aliases_win() {
        let record = LogRecord::from_object(
            0,
            1,
            0,
            object(r#"{"log":{"level":30,"message":"started"},"time":"1700000000"}"#),
            &FieldAliases::with_defaults(&["time".to_owned()], &[], &[]).expect("aliases"),
        );
        assert_eq!(record.level, Some(LogLevel::Known(Level::Warn)));
        assert_eq!(record.message.as_deref(), Some("started"));
        assert_eq!(record.timestamp.as_ref().map(Timestamp::nanos), Some(1_700_000_000_000_000_000));
    }

    #[test]
    fn unparseable_timestamp_keeps_its_text() {
        let record = LogRecord::from_object(0, 1, 0, object(r#"{"timestamp":"just now"}"#), &aliases());
        assert_eq!(record.stamp(TimeFormat::Raw), "just now");
        assert_eq!(record.stamp(TimeFormat::UtcDateTime), "just now");
    }

    #[test]
    fn value_at_uses_field_paths() {
        let record = LogRecord::from_object(0, 1, 0, object(r#"{"a":{"b":[1,2]},"x.y":5}"#), &aliases());
        assert_eq!(record.value_at(&FieldPath::parse("a.b[1]").unwrap()), Some(&json!(2)));
        assert_eq!(record.value_at(&FieldPath::parse("\"x.y\"").unwrap()), Some(&json!(5)));
        assert_eq!(record.value_at(&FieldPath::parse("missing").unwrap()), None);
    }

    #[test]
    fn invalid_records_are_marked() {
        let record = LogRecord::invalid(0, 3, 0, "trailing characters", "{bad json");
        assert!(record.is_invalid());
        assert_eq!(record.raw_text(), "{bad json");
        assert_eq!(record.message.as_deref(), Some("trailing characters"));
    }

    #[test]
    fn bad_alias_is_reported() {
        let error = FieldAliases::with_defaults(&["a..b".to_owned()], &[], &[]).expect_err("invalid path");
        assert_eq!(error.0, "a..b");
    }
}
