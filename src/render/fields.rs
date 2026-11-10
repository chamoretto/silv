//! Column model: pseudo columns, field columns, width and ordering rules.

use std::cmp::Ordering;

use serde_json::Value;

use crate::cli::Config;
use crate::cli::SortDir;
use crate::cli::SortSpec;
use crate::data::LogStore;
use crate::filter::predicate::MatchKind;
use crate::model::field::FieldPath;
use crate::model::field::Segment;
use crate::model::level::Level;
use crate::model::level::LogLevel;
use crate::model::record::LogRecord;
use crate::model::value::compare;
use crate::model::value::text_of;
use crate::render::color::Sg;

/// Pseudo column names understood in `--fields-order`, `--sort` and filters.
pub const PSEUDO_TIMESTAMP: [&str; 3] = ["@timestamp", "@ts", "@time"];
pub const PSEUDO_LEVEL: [&str; 3] = ["@level", "@severity", "@lvl"];
pub const PSEUDO_MESSAGE: [&str; 2] = ["@message", "@msg"];
pub const PSEUDO_SOURCE: [&str; 2] = ["@source", "@file"];
pub const PSEUDO_LINE: [&str; 2] = ["@line", "@n"];

/// A rendered column: a derived column, or a value addressed by a field path.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Column {
    Timestamp,
    Level,
    Message,
    Source,
    Line,
    Field(FieldPath),
}

impl Column {
    pub fn from_path(path: &FieldPath) -> Self {
        let label = path.to_string().to_ascii_lowercase();
        let name = label.as_str();
        if PSEUDO_TIMESTAMP.contains(&name) {
            Self::Timestamp
        } else if PSEUDO_LEVEL.contains(&name) {
            Self::Level
        } else if PSEUDO_MESSAGE.contains(&name) {
            Self::Message
        } else if PSEUDO_SOURCE.contains(&name) {
            Self::Source
        } else if PSEUDO_LINE.contains(&name) {
            Self::Line
        } else {
            Self::Field(path.clone())
        }
    }

    pub fn label(&self) -> String {
        match self {
            Self::Timestamp => "@timestamp".to_owned(),
            Self::Level => "@level".to_owned(),
            Self::Message => "@message".to_owned(),
            Self::Source => "@source".to_owned(),
            Self::Line => "@line".to_owned(),
            Self::Field(path) => path.to_string(),
        }
    }

    pub const fn is_numeric(&self) -> bool {
        matches!(self, Self::Line)
    }

    pub const fn is_temporal(&self) -> bool {
        matches!(self, Self::Timestamp)
    }
}

/// Resolves the columns to display from the selection and the discovered schema.
pub fn resolve_columns(config: &Config, store: &LogStore) -> Vec<Column> {
    let mut columns: Vec<Column> = Vec::new();
    for path in &config.columns.ordered {
        push_column(&mut columns, Column::from_path(path), &config.columns.excluded);
    }
    let wants_all = config.columns.all || config.columns.rest_last;
    if wants_all {
        let mut discovered: Vec<&crate::data::FieldInfo> = store.fields().iter().collect();
        if config.columns.rest_last {
            discovered = store.fields_by_frequency();
        }
        for info in discovered {
            if info.path.is_wildcard() {
                continue;
            }
            push_column(&mut columns, Column::Field(info.path.clone()), &config.columns.excluded);
        }
    }
    columns
}

fn push_column(columns: &mut Vec<Column>, column: Column, excluded: &[FieldPath]) {
    if let Column::Field(path) = &column
        && excluded.iter().any(|e| e == path || path.starts_with(e))
    {
        return;
    }
    if !columns.contains(&column) {
        columns.push(column);
    }
}

/// Text of one column for one record, with the style for its value kind.
pub fn cell_text(record: &LogRecord, column: &Column, source_label: &str, config: &Config) -> (String, Sg) {
    match column {
        Column::Timestamp => (record.stamp(config.time_format), Sg::Muted),
        Column::Level => (
            record
                .level
                .as_ref()
                .map(|level| level.padded())
                .unwrap_or_else(|| " ".repeat(Level::width())),
            Sg::for_level(record.level.as_ref().and_then(LogLevel::known).as_ref()),
        ),
        Column::Message => (record.message.clone().unwrap_or_default(), Sg::String),
        Column::Source => (source_label.to_owned(), Sg::Muted),
        Column::Line => (record.line.to_string(), Sg::Muted),
        Column::Field(path) => value_text(record, path),
    }
}

/// Canonical text of a field, joining wildcard matches with a comma.
pub fn value_text(record: &LogRecord, path: &FieldPath) -> (String, Sg) {
    let mut values = Vec::new();
    path.collect(&record.object, &mut values);
    let Some(first) = values.first() else {
        return (String::new(), Sg::Default);
    };
    let mut buf = String::new();
    let style = kind_style(first);
    let mut out = text_of(first, &mut buf).as_str().to_owned();
    for value in &values[1..] {
        let mut item = String::new();
        out.push_str(", ");
        out.push_str(text_of(value, &mut item).as_str());
    }
    (out, style)
}

pub const fn kind_style(value: &Value) -> Sg {
    match value {
        Value::Null => Sg::Null,
        Value::Bool(_) => Sg::Bool,
        Value::Number(_) => Sg::Number,
        Value::String(_) => Sg::String,
        Value::Array(_) | Value::Object(_) => Sg::Container,
    }
}

/// Leaf paths of a record used by kv rendering, in record order, with display labels.
pub fn leaf_values(record: &LogRecord, max_depth: usize) -> Vec<(String, &Value)> {
    let mut out = Vec::new();
    for (key, value) in record.object.iter() {
        walk_leaves(&mut out, vec![Segment::Key(key.as_str().into())], value, max_depth);
    }
    out
}

fn walk_leaves<'a>(out: &mut Vec<(String, &'a Value)>, segments: Vec<Segment>, value: &'a Value, depth: usize) {
    let path = FieldPath::from_segments(segments.clone());
    match value {
        Value::Object(map) if depth > 0 => {
            for (key, child) in map {
                let mut nested = segments.clone();
                nested.push(Segment::Key(key.as_str().into()));
                walk_leaves(out, nested, child, depth - 1);
            }
        },
        Value::Array(items) if depth > 0 => {
            for (index, child) in items.iter().enumerate() {
                let mut nested = segments.clone();
                nested.push(Segment::Index(index));
                walk_leaves(out, nested, child, depth - 1);
            }
        },
        _ => out.push((path.to_string(), value)),
    }
}

/// Sorts record indices by the requested columns, stable, with missing values last.
pub fn sort_indices(store: &LogStore, indices: &mut [u32], sorts: &[SortSpec]) {
    if sorts.is_empty() {
        return;
    }
    indices.sort_by(|a, b| {
        for spec in sorts {
            let ordering = compare_column(store, *a, *b, &Column::from_path(&spec.field));
            if ordering != Ordering::Equal {
                return match spec.dir {
                    SortDir::Asc => ordering,
                    SortDir::Desc => ordering.reverse(),
                };
            }
        }
        a.cmp(b)
    });
}

fn compare_column(store: &LogStore, a: u32, b: u32, column: &Column) -> Ordering {
    let Some(left) = store.record(usize::try_from(a).unwrap_or(usize::MAX)) else {
        return Ordering::Equal;
    };
    let Some(right) = store.record(usize::try_from(b).unwrap_or(usize::MAX)) else {
        return Ordering::Equal;
    };
    match column {
        Column::Timestamp => order_missing(left.timestamp.as_ref(), right.timestamp.as_ref()),
        Column::Level => {
            order_missing(left.level.as_ref().and_then(LogLevel::rank), right.level.as_ref().and_then(LogLevel::rank))
        },
        Column::Message => order_missing(left.message.as_deref(), right.message.as_deref()),
        Column::Source => left.source.cmp(&right.source),
        Column::Line => left.line.cmp(&right.line),
        Column::Field(path) => match (left.value_at(path), right.value_at(path)) {
            (Some(a), Some(b)) => compare(a, b),
            (Some(_), None) => Ordering::Less,
            (None, Some(_)) => Ordering::Greater,
            (None, None) => Ordering::Equal,
        },
    }
}

fn order_missing<T: Ord>(left: Option<T>, right: Option<T>) -> Ordering {
    match (left, right) {
        (Some(a), Some(b)) => a.cmp(&b),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => Ordering::Equal,
    }
}

/// Reports which kind a matcher uses, kept here so the TUI and the writer agree.
pub const fn matcher_kind_name(kind: MatchKind) -> &'static str {
    match kind {
        MatchKind::Glob => "glob",
        MatchKind::Contains => "text",
        MatchKind::Regex => "regex",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::LogStore;
    use crate::model::level::Level;
    use crate::model::record::FieldAliases;

    fn record(text: &str) -> LogRecord {
        let object = serde_json::from_str(text).expect("fixture object");
        LogRecord::from_object(0, 1, 0, object, &FieldAliases::with_defaults(&[], &[], &[]).expect("aliases"))
    }

    #[test]
    fn pseudo_columns_are_recognised() {
        assert_eq!(Column::from_path(&FieldPath::parse("@ts").unwrap()), Column::Timestamp);
        assert_eq!(Column::from_path(&FieldPath::parse("@level").unwrap()), Column::Level);
        assert_eq!(
            Column::from_path(&FieldPath::parse("a.b").unwrap()),
            Column::Field(FieldPath::parse("a.b").unwrap())
        );
    }

    #[test]
    fn cell_texts_follow_value_kinds() {
        let record = record(r#"{"n":1,"s":"x","o":{"k":1},"l":[1,2]}"#);
        let config = Config::stub();
        assert_eq!(cell_text(&record, &Column::from_path(&FieldPath::parse("n").unwrap()), "", &config).1, Sg::Number);
        assert_eq!(
            cell_text(&record, &Column::from_path(&FieldPath::parse("o.k").unwrap()), "", &config),
            ("1".to_owned(), Sg::Number)
        );
        assert_eq!(
            cell_text(&record, &Column::from_path(&FieldPath::parse("missing").unwrap()), "", &config),
            (String::new(), Sg::Default)
        );
    }

    #[test]
    fn wildcard_columns_join_values() {
        let record = record(r#"{"a":{"v":1},"b":{"v":2}}"#);
        let config = Config::stub();
        let column = Column::from_path(&FieldPath::parse("*.v").unwrap());
        assert_eq!(cell_text(&record, &column, "", &config).0, "1, 2");
    }

    #[test]
    fn level_column_is_padded_and_styled() {
        let record = record(r#"{"level":"ERROR"}"#);
        let config = Config::stub();
        let (text, style) = cell_text(&record, &Column::Level, "", &config);
        assert_eq!(text, "ERROR");
        assert_eq!(style, Sg::for_level(Some(&Level::Error)));
    }

    #[test]
    fn leaf_values_lists_nested_paths() {
        let record = record(r#"{"b":1,"a":{"c":[1,2]}}"#);
        let leaves: Vec<String> = leaf_values(&record, 4).into_iter().map(|(label, _)| label).collect();
        assert_eq!(leaves, vec!["b", "a.c[0]", "a.c[1]"]);
    }

    #[test]
    fn resolve_columns_respects_selection_and_exclusions() {
        let mut config = Config::stub();
        config.columns.ordered = vec![FieldPath::parse("a").unwrap()];
        config.columns.excluded = vec![FieldPath::parse("c").unwrap()];
        config.columns.rest_last = true;
        let store = store_of(&["{\"a\":1,\"b\":2,\"c\":3}"]);
        let columns = resolve_columns(&config, &store);
        assert_eq!(columns.iter().map(Column::label).collect::<Vec<_>>(), vec!["a", "b"]);
    }

    fn store_of(records: &[&str]) -> LogStore {
        let config = Config::stub();
        let bundle = crate::parse::input::InputBundle {
            sources: vec![crate::parse::input::RawSource::for_test(
                "t.jsonl",
                records.join("\n").into_bytes(),
            )],
            read_stdin: false,
        };
        LogStore::build(&bundle, &config).expect("builds")
    }

    #[test]
    fn sorting_is_stable_and_missing_last() {
        let store = store_of(&["{\"a\":3}", "{\"a\":1}", "{\"b\":2}", "{\"a\":2}"]);
        let mut indices: Vec<u32> = (0..u32::try_from(store.len()).expect("fits")).collect();
        let sorts = vec![SortSpec {
            field: FieldPath::parse("a").unwrap(),
            dir: SortDir::Asc,
        }];
        sort_indices(&store, &mut indices, &sorts);
        assert_eq!(indices, vec![1, 3, 0, 2]);
    }

    #[test]
    fn descending_sort_reverses() {
        let store = store_of(&["{\"a\":3}", "{\"a\":1}", "{\"a\":2}"]);
        let mut indices: Vec<u32> = vec![0, 1, 2];
        sort_indices(
            &store,
            &mut indices,
            &[SortSpec {
                field: FieldPath::parse("a").unwrap(),
                dir: SortDir::Desc,
            }],
        );
        assert_eq!(indices, vec![0, 2, 1]);
    }

    #[test]
    fn sorting_by_level_and_message() {
        let store = store_of(&[
            "{\"level\":\"INFO\",\"message\":\"b\"}",
            "{\"level\":\"ERROR\",\"message\":\"a\"}",
            "{\"level\":\"WARN\",\"message\":\"a\"}",
        ]);
        let mut indices: Vec<u32> = vec![0, 1, 2];
        sort_indices(
            &store,
            &mut indices,
            &[
                SortSpec {
                    field: FieldPath::parse("@level").unwrap(),
                    dir: SortDir::Desc,
                },
                SortSpec {
                    field: FieldPath::parse("@message").unwrap(),
                    dir: SortDir::Asc,
                },
            ],
        );
        assert_eq!(indices, vec![1, 2, 0]);
    }
}
