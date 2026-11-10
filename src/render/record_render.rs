//! Record rendering into styled cells for every output mode.

use serde_json::Value;

use crate::cli::Config;
use crate::cli::RenderChoice;
use crate::model::field::FieldPath;
use crate::model::record::LogRecord;
use crate::parse::LogFormat;
use crate::render::color::Sg;
use crate::render::fields::Column;
use crate::render::fields::cell_text;
use crate::render::fields::leaf_values;
use crate::render::json;
use crate::render::text::pad_right;
use crate::render::text::truncate;

/// A run of text with a semantic style; the stream writer adds ANSI codes, the TUI adds ratatui styles.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cell {
    pub text: String,
    pub style: Sg,
}

impl Cell {
    pub fn new(text: impl Into<String>, style: Sg) -> Self {
        Self {
            text: text.into(),
            style,
        }
    }
}

/// Turns `auto` into a concrete mode.
pub fn resolve_render(config: &Config, format: LogFormat, has_columns: bool) -> RenderChoice {
    match config.render {
        RenderChoice::Auto => {
            if has_columns {
                RenderChoice::Table
            } else if format == LogFormat::Plain {
                RenderChoice::Line
            } else {
                RenderChoice::Kv
            }
        },
        other => other,
    }
}

pub fn render_record(
    record: &LogRecord,
    mode: RenderChoice,
    columns: &[Column],
    source_label: &str,
    config: &Config,
) -> Vec<Cell> {
    match mode {
        RenderChoice::Table => table_cells(record, columns, source_label, config),
        RenderChoice::Kv => kv_cells(record, columns, config),
        RenderChoice::Line => line_cells(record, config),
        RenderChoice::Json => json_cells(record, true),
        RenderChoice::Jsonl => json_cells(record, false),
        RenderChoice::Auto => kv_cells(record, columns, config),
    }
}

pub fn table_cells(record: &LogRecord, columns: &[Column], source_label: &str, config: &Config) -> Vec<Cell> {
    let cut = config.truncate_values.saturating_add(ELLIPSIS_LEN);
    columns
        .iter()
        .map(|column| {
            let (text, style) = cell_text(record, column, source_label, config);
            Cell::new(truncate(&text, cut), style)
        })
        .collect()
}

const ELLIPSIS_LEN: usize = 2;

pub fn kv_cells(record: &LogRecord, columns: &[Column], config: &Config) -> Vec<Cell> {
    let mut cells: Vec<Cell> = Vec::new();
    let stamp = record.stamp(config.time_format);
    if !stamp.is_empty() {
        cells.push(Cell::new(format!("{stamp} "), Sg::Muted));
    }
    if let Some(level) = &record.level {
        cells.push(Cell::new(
            format!("{} ", pad_right(&level.to_string(), crate::model::level::Level::width())),
            Sg::for_level(level.known().as_ref()),
        ));
    }
    if let Some(message) = &record.message {
        cells.push(Cell::new(format!("{} ", truncate(message, config.truncate_values)), Sg::String));
    }

    let shown = field_paths(columns);
    let mut rendered = shown;
    rendered.extend(resolved_alias_paths(record, config));

    for column in columns {
        if let Column::Field(path) = column {
            let (text, style) = cell_text(record, column, "", config);
            if text.is_empty() {
                continue;
            }
            cells.push(Cell::new(format!("{path}="), Sg::Key));
            cells.push(Cell::new(truncate(&text, config.truncate_values), style));
            cells.push(Cell::new(" ", Sg::Default));
        }
    }

    if columns.is_empty() || config.columns.rest_last {
        for (label, value) in leaf_values(record, LEAF_DEPTH) {
            let Ok(path) = FieldPath::parse(&label) else {
                continue;
            };
            if is_covered(&path, &rendered, &config.columns.excluded) {
                continue;
            }
            let mut buf = String::new();
            let text = crate::model::value::text_of(value, &mut buf).as_str().to_owned();
            cells.push(Cell::new(format!("{label}="), Sg::Key));
            cells.push(Cell::new(truncate(&text, config.truncate_values), kind_style_of(value)));
            cells.push(Cell::new(" ", Sg::Default));
        }
    }
    if cells.last().is_some_and(|cell| cell.text == " ") {
        cells.pop();
    }
    cells
}

const LEAF_DEPTH: usize = 4;

/// Paths that already feed the timestamp, severity and message prefix, so kv does not repeat them.
fn resolved_alias_paths(record: &LogRecord, config: &Config) -> Vec<FieldPath> {
    let aliases = config.aliases();
    let mut out = Vec::with_capacity(3);
    for paths in [&aliases.timestamp, &aliases.level, &aliases.message] {
        if let Some(path) = aliases.first_path(paths, &record.object) {
            out.push(path);
        }
    }
    out
}

pub fn line_cells(record: &LogRecord, config: &Config) -> Vec<Cell> {
    let raw = record.raw_text();
    if !raw.is_empty() {
        return plain_line_cells(raw, config);
    }
    json_cells(record, false)
}

fn plain_line_cells(raw: &str, config: &Config) -> Vec<Cell> {
    let separator = config.plain_separator.as_str();
    let parts: Vec<&str> = raw.split(separator).collect();
    let mut cells = Vec::new();
    if parts.len() >= 2 {
        cells.push(Cell::new(parts[0], Sg::Muted));
        cells.push(Cell::new(separator, Sg::Punct));
        let mut start = 1usize;
        if let Some(level) = parts[1].trim().parse::<crate::model::level::LogLevel>().ok()
            && level.known().is_some()
        {
            cells.push(Cell::new(parts[1], Sg::for_level(level.known().as_ref())));
            cells.push(Cell::new(separator, Sg::Punct));
            start = 2;
        }
        for (index, part) in parts[start..].iter().enumerate() {
            cells.extend(kv_spans(part));
            if index + 1 < parts[start..].len() {
                cells.push(Cell::new(separator, Sg::Punct));
            }
        }
        return cells;
    }
    kv_spans(raw)
}

/// Colors `key=value` pairs inside a free text segment.
pub fn kv_spans(text: &str) -> Vec<Cell> {
    let mut cells = Vec::new();
    for (index, token) in text.split_whitespace().enumerate() {
        if index > 0 {
            cells.push(Cell::new(" ", Sg::Default));
        }
        match token.split_once('=') {
            Some((key, value)) if !key.is_empty() && key.chars().all(|c| !c.is_whitespace()) => {
                cells.push(Cell::new(key, Sg::Key));
                cells.push(Cell::new("=", Sg::Punct));
                cells.push(Cell::new(value, Sg::String));
            },
            _ => cells.push(Cell::new(token, Sg::String)),
        }
    }
    cells
}

pub fn json_cells(record: &LogRecord, pretty: bool) -> Vec<Cell> {
    let mut cells = Vec::new();
    json::spans(&Value::Object(record.object.clone()), pretty, &mut cells);
    cells
}

fn field_paths(columns: &[Column]) -> Vec<FieldPath> {
    columns
        .iter()
        .filter_map(|column| {
            if let Column::Field(path) = column {
                Some(path.clone())
            } else {
                None
            }
        })
        .collect()
}

fn is_covered(path: &FieldPath, shown: &[FieldPath], excluded: &[FieldPath]) -> bool {
    excluded.iter().any(|e| e == path || path.starts_with(e))
        || shown.iter().any(|shown| shown == path || path.starts_with(shown))
}

fn kind_style_of(value: &Value) -> Sg {
    crate::render::fields::kind_style(value)
}

/// Splits cell text into runs, marking the parts matched by `regexes`.
pub fn highlight(cells: Vec<Cell>, regexes: &[regex::Regex]) -> Vec<Cell> {
    if regexes.is_empty() {
        return cells;
    }
    let mut out = Vec::with_capacity(cells.len());
    for cell in cells {
        let mut ranges: Vec<(usize, usize)> = Vec::new();
        for regex in regexes {
            for found in regex.find_iter(&cell.text) {
                ranges.push((found.start(), found.end()));
            }
        }
        if ranges.is_empty() {
            out.push(cell);
            continue;
        }
        ranges.sort_unstable();
        let mut position = 0usize;
        for (start, end) in ranges {
            if start < position || end <= start {
                continue;
            }
            if start > position {
                out.push(Cell::new(&cell.text[position..start], cell.style));
            }
            out.push(Cell::new(&cell.text[start..end], Sg::Search));
            position = end;
        }
        if position < cell.text.len() {
            out.push(Cell::new(&cell.text[position..], cell.style));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::record::FieldAliases;
    use crate::render::color::Sg;

    fn record(text: &str) -> LogRecord {
        let object = serde_json::from_str(text).expect("fixture object");
        LogRecord::from_object(0, 1, 0, object, &FieldAliases::with_defaults(&[], &[], &[]).expect("aliases"))
    }

    fn joined(cells: &[Cell]) -> String {
        cells.iter().map(|cell| cell.text.as_str()).collect()
    }

    #[test]
    fn auto_render_prefers_table_then_format_default() {
        let config = Config::stub();
        assert_eq!(resolve_render(&config, LogFormat::Jsonl, true), RenderChoice::Table);
        assert_eq!(resolve_render(&config, LogFormat::Jsonl, false), RenderChoice::Kv);
        assert_eq!(resolve_render(&config, LogFormat::Plain, false), RenderChoice::Line);
    }

    #[test]
    fn kv_mode_renders_key_value_pairs() {
        let config = Config::stub();
        let record = record(r#"{"@timestamp":"2023-11-14T22:13:20.123Z","level":"INFO","message":"up","host":"web1"}"#);
        let line = joined(&kv_cells(&record, &[], &config));
        assert_eq!(line, "2023-11-14 22:13:20.123 INFO  up host=web1");
    }

    #[test]
    fn kv_mode_skips_covered_and_excluded_fields() {
        let mut config = Config::stub();
        config.columns.ordered = vec![FieldPath::parse("host").unwrap()];
        config.columns.excluded = vec![FieldPath::parse("secret").unwrap()];
        config.columns.rest_last = true;
        let record = record(r#"{"message":"up","host":"web1","secret":"s","trace":{"id":"t"}}"#);
        let line = joined(&kv_cells(&record, &[Column::Field(FieldPath::parse("host").unwrap())], &config));
        assert_eq!(line, "up host=web1 trace.id=t");
    }

    #[test]
    fn jsonl_mode_emits_compact_json() {
        let record = record(r#"{"a":1,"b":"x"}"#);
        assert_eq!(joined(&json_cells(&record, false)), r#"{"a": 1,"b": "x"}"#);
    }

    #[test]
    fn plain_line_keeps_layout_and_colors_pairs() {
        let config = Config::stub();
        let record = crate::parse::plain::parse_line(
            0,
            1,
            0,
            b"2023-11-14 22:13:20.123 | WARN | slow trace.id=abc",
            &FieldAliases::with_defaults(&[], &[], &[]).expect("aliases"),
            &crate::parse::plain::PlainOptions::default(),
        )
        .expect("parses");
        let cells = line_cells(&record, &config);
        assert_eq!(joined(&cells), "2023-11-14 22:13:20.123 | WARN | slow trace.id=abc");
        assert_eq!(cells[2].style, Sg::Warn);
        assert_eq!(cells.last().map(|c| c.style), Some(Sg::String));
    }

    #[test]
    fn highlight_marks_matches_and_keeps_text() {
        let cells = vec![Cell::new("disk full at 03:00", Sg::String)];
        let regex = regex::Regex::new("full").expect("compiles");
        let highlighted = highlight(cells, &[regex]);
        assert_eq!(
            highlighted
                .iter()
                .map(|c| (c.text.as_str(), c.style))
                .collect::<Vec<_>>(),
            vec![("disk ", Sg::String), ("full", Sg::Search), (" at 03:00", Sg::String)]
        );
    }

    #[test]
    fn table_cells_truncate_long_values() {
        let mut config = Config::stub();
        config.truncate_values = 6;
        let record = record(r#"{"message":"a very long message"}"#);
        let cells = table_cells(&record, &[Column::Message], "src", &config);
        assert_eq!(cells[0].text, "a very..");
    }
}
