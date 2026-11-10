//! In-RAM record store: records, discovered fields, per-source metadata.

pub mod view;

use std::collections::HashMap;
use std::fmt::Display;
use std::sync::Arc;

use miette::Result;
use rayon::prelude::*;
use serde_json::Value;

use crate::cli::Config;
use crate::cli::FormatSelection;
use crate::error::SilvError;
use crate::model::field::FieldPath;
use crate::model::field::Segment;
use crate::model::record::LogRecord;
use crate::model::value::kind_rank;
use crate::parse::LogFormat;
use crate::parse::ParseIssue;
use crate::parse::detect;
use crate::parse::input::InputBundle;
use crate::parse::jsonl;
use crate::parse::plain;
use crate::parse::split_lines;

/// Upper bound of discovered fields, protecting RAM on wide schemas.
pub const MAX_DISCOVERED_FIELDS: usize = 4096;
const MAX_FIELD_DEPTH: usize = 12;

/// Bit set of value kinds seen under a field path.
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub struct ValueTypes(u8);

impl ValueTypes {
    pub const NULL: u8 = 1 << 0;
    pub const BOOL: u8 = 1 << 1;
    pub const NUMBER: u8 = 1 << 2;
    pub const STRING: u8 = 1 << 3;
    pub const ARRAY: u8 = 1 << 4;
    pub const OBJECT: u8 = 1 << 5;

    pub const fn from_rank(rank: u8) -> Self {
        Self(1u8 << rank)
    }

    pub fn set(&mut self, rank: u8) {
        self.0 |= 1u8 << rank.min(5);
    }

    pub const fn contains(self, mask: u8) -> bool {
        self.0 & mask != 0
    }

    pub const fn bits(self) -> u8 {
        self.0
    }

    pub const fn is_scalar(self) -> bool {
        self.0 & (Self::ARRAY | Self::OBJECT) == 0
    }
}

impl Display for ValueTypes {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut names: Vec<&str> = Vec::with_capacity(6);
        for (mask, name) in [
            (Self::NULL, "null"),
            (Self::BOOL, "bool"),
            (Self::NUMBER, "number"),
            (Self::STRING, "string"),
            (Self::ARRAY, "array"),
            (Self::OBJECT, "object"),
        ] {
            if self.0 & mask != 0 {
                names.push(name);
            }
        }
        f.write_str(&names.join(","))
    }
}

/// A field path seen in at least one record.
#[derive(Clone, Debug)]
pub struct FieldInfo {
    pub path: FieldPath,
    pub label: String,
    pub count: u64,
    pub types: ValueTypes,
    pub first_index: usize,
}

/// Per-source bookkeeping.
#[derive(Clone, Debug)]
pub struct SourceMeta {
    pub label: String,
    pub kind: LogFormat,
    pub lines: u64,
}

#[derive(Clone, Debug)]
struct Node {
    parent: usize,
    segment: Option<Segment>,
    children: Vec<(Segment, usize)>,
    field: Option<usize>,
}

/// Prefix tree of field paths, used during discovery without per-record string keys.
pub struct FieldTree {
    nodes: Vec<Node>,
    capped: bool,
}

impl FieldTree {
    pub fn new() -> Self {
        Self {
            nodes: vec![Node {
                parent: 0,
                segment: None,
                children: Vec::new(),
                field: None,
            }],
            capped: false,
        }
    }

    pub const fn is_capped(&self) -> bool {
        self.capped
    }

    fn child(&mut self, parent: usize, segment: &Segment) -> usize {
        if let Some((_, id)) = self.nodes[parent].children.iter().find(|(s, _)| s == segment) {
            return *id;
        }
        let id = self.nodes.len();
        self.nodes.push(Node {
            parent,
            segment: Some(segment.clone()),
            children: Vec::new(),
            field: None,
        });
        self.nodes[parent].children.push((segment.clone(), id));
        id
    }

    fn path_of(&self, node: usize) -> FieldPath {
        let mut segments = Vec::new();
        let mut current = node;
        while current != 0 {
            if let Some(segment) = &self.nodes[current].segment {
                segments.push(segment.clone());
            }
            current = self.nodes[current].parent;
        }
        segments.reverse();
        FieldPath::from_segments(segments)
    }

    fn record(&mut self, node: usize, value: &Value, index: usize, fields: &mut Vec<FieldInfo>) {
        if let Some(id) = self.nodes[node].field {
            let info = &mut fields[id];
            info.count += 1;
            info.types.set(kind_rank(value));
            return;
        }
        if fields.len() >= MAX_DISCOVERED_FIELDS {
            self.capped = true;
            return;
        }
        let path = self.path_of(node);
        let label = path.to_string();
        fields.push(FieldInfo {
            path,
            label,
            count: 1,
            types: ValueTypes::from_rank(kind_rank(value)),
            first_index: index,
        });
        self.nodes[node].field = Some(fields.len() - 1);
    }

    fn visit(
        &mut self,
        parent: usize,
        segment: Segment,
        value: &Value,
        index: usize,
        fields: &mut Vec<FieldInfo>,
        depth: usize,
    ) {
        let child = self.child(parent, &segment);
        self.record(child, value, index, fields);
        if depth >= MAX_FIELD_DEPTH {
            return;
        }
        match value {
            Value::Object(map) => {
                for (key, child_value) in map {
                    self.visit(child, Segment::Key(Arc::from(key.as_str())), child_value, index, fields, depth + 1);
                }
            },
            Value::Array(items) => {
                for item in items {
                    self.visit(child, Segment::IndexWildcard, item, index, fields, depth + 1);
                }
            },
            _ => {},
        }
    }

    pub fn register_record(&mut self, record: &LogRecord, index: usize, fields: &mut Vec<FieldInfo>) {
        for (key, value) in &record.object {
            self.visit(0, Segment::Key(Arc::from(key.as_str())), value, index, fields, 0);
        }
    }
}

impl Default for FieldTree {
    fn default() -> Self {
        Self::new()
    }
}

/// All records of a run plus the schema discovered while parsing them.
pub struct LogStore {
    records: Vec<LogRecord>,
    fields: Vec<FieldInfo>,
    field_lookup: HashMap<String, usize>,
    sources: Vec<SourceMeta>,
    issues: Vec<ParseIssue>,
    total_lines: u64,
    truncated: bool,
    fields_capped: bool,
    format: LogFormat,
}

impl LogStore {
    /// Parses every source, in parallel per line, and discovers the field tree.
    pub fn build(bundle: &InputBundle, config: &Config) -> Result<Self> {
        let aliases = config.aliases();
        let plain_options = config.plain_options();
        let mut store = Self {
            records: Vec::new(),
            fields: Vec::new(),
            field_lookup: HashMap::new(),
            sources: Vec::new(),
            issues: Vec::new(),
            total_lines: 0,
            truncated: false,
            fields_capped: false,
            format: LogFormat::Jsonl,
        };
        let mut tree = FieldTree::new();

        for (index, source) in bundle.sources.iter().enumerate() {
            let source_index = u32::try_from(index).unwrap_or(u32::MAX);
            let kind = match config.format {
                FormatSelection::Auto => detect::detect(source.bytes(), &source.label)?,
                FormatSelection::Forced(format) => format,
            };
            store.format = kind;

            let lines = split_lines(source.bytes());
            let results: Vec<crate::parse::LineResult> = lines
                .par_iter()
                .map(|(number, offset, bytes)| match kind {
                    LogFormat::Jsonl => jsonl::parse_line(source_index, *number, *offset, bytes, aliases),
                    LogFormat::Plain => {
                        plain::parse_line(source_index, *number, *offset, bytes, aliases, &plain_options)
                    },
                })
                .collect();

            store.total_lines += u64::from(u32::try_from(lines.len()).unwrap_or(u32::MAX));
            store.sources.push(SourceMeta {
                label: source.label.clone(),
                kind,
                lines: u64::try_from(lines.len()).unwrap_or(u64::MAX),
            });

            let mut stop = false;
            for result in results {
                match result {
                    Ok(record) => {
                        if store.records.len() >= config.max_records {
                            store.truncated = true;
                            stop = true;
                            break;
                        }
                        let position = store.records.len();
                        tree.register_record(&record, position, &mut store.fields);
                        store.records.push(record);
                    },
                    Err(issue) => {
                        if config.strict {
                            return Err(diagnostic_for(bundle, &issue).into());
                        }
                        if config.keep_bad_lines && store.records.len() < config.max_records {
                            store.records.push(LogRecord::invalid(
                                issue.source,
                                issue.line,
                                issue.offset,
                                &issue.reason,
                                &issue.raw,
                            ));
                        }
                        store.issues.push(issue);
                    },
                }
            }
            if stop {
                break;
            }
        }

        store.fields_capped = tree.is_capped();
        store.field_lookup = store
            .fields
            .iter()
            .enumerate()
            .map(|(id, info)| (info.label.clone(), id))
            .collect();
        Ok(store)
    }

    pub fn records(&self) -> &[LogRecord] {
        &self.records
    }

    pub fn record(&self, index: usize) -> Option<&LogRecord> {
        self.records.get(index)
    }

    pub const fn len(&self) -> usize {
        self.records.len()
    }

    pub const fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    pub fn fields(&self) -> &[FieldInfo] {
        &self.fields
    }

    pub fn sources(&self) -> &[SourceMeta] {
        &self.sources
    }

    pub fn issues(&self) -> &[ParseIssue] {
        &self.issues
    }

    pub const fn total_lines(&self) -> u64 {
        self.total_lines
    }

    pub const fn truncated(&self) -> bool {
        self.truncated
    }

    pub const fn fields_capped(&self) -> bool {
        self.fields_capped
    }

    pub const fn format(&self) -> LogFormat {
        self.format
    }

    pub fn field(&self, label: &str) -> Option<&FieldInfo> {
        self.field_lookup.get(label).map(|id| &self.fields[*id])
    }

    pub fn field_by_path(&self, path: &FieldPath) -> Option<&FieldInfo> {
        self.field(&path.to_string())
    }

    /// Discovered fields ordered by how often they occur, most frequent first.
    pub fn fields_by_frequency(&self) -> Vec<&FieldInfo> {
        let mut ordered: Vec<&FieldInfo> = self.fields.iter().collect();
        ordered.sort_by(|a, b| {
            b.count
                .cmp(&a.count)
                .then_with(|| a.first_index.cmp(&b.first_index))
                .then_with(|| a.label.cmp(&b.label))
        });
        ordered
    }

    pub fn all_paths(&self) -> Vec<FieldPath> {
        self.fields.iter().map(|info| info.path.clone()).collect()
    }
}

fn diagnostic_for(bundle: &InputBundle, issue: &ParseIssue) -> SilvError {
    let Some(source) = bundle.sources.get(usize::try_from(issue.source).unwrap_or(usize::MAX)) else {
        return SilvError::Parse(Box::new(crate::error::ParseDetail {
            line: issue.line,
            reason: issue.reason.clone(),
            code: miette::NamedSource::new("input", issue.raw.clone()),
            span: miette::SourceSpan::from(0usize),
            help: issue.reason.clone(),
        }));
    };
    let text = String::from_utf8_lossy(source.bytes()).into_owned();
    SilvError::parse(&source.label, &text, issue.line, issue.offset, issue.offset, &issue.reason)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::Config;
    use crate::parse::input::InputBundle;

    fn bundle(text: &[u8]) -> InputBundle {
        InputBundle {
            sources: vec![crate::parse::input::RawSource::for_test("t.jsonl", text.to_vec())],
            read_stdin: false,
        }
    }

    #[test]
    fn discovers_nested_fields() {
        let config = Config::stub();
        let store = LogStore::build(&bundle(br#"{"a":{"b":1},"c":["x","y"]}"#), &config).expect("builds");
        let labels: Vec<&str> = store.fields().iter().map(|f| f.label.as_str()).collect();
        assert_eq!(labels, vec!["a", "a.b", "c", "c[*]"]);
        assert_eq!(store.field("a.b").map(|f| f.count), Some(1));
        assert_eq!(store.field("c[*]").map(|f| f.types.to_string()), Some("string".to_owned()));
    }

    #[test]
    fn keeps_bad_lines_by_default() {
        let config = Config::stub();
        let store = LogStore::build(&bundle(b"{\"a\":1}\nnot json\n"), &config).expect("builds");
        assert_eq!(store.records().len(), 2);
        assert_eq!(store.issues().len(), 1);
        assert!(store.records()[1].is_invalid());
    }

    #[test]
    fn strict_mode_reports_the_offending_line() {
        let mut config = Config::stub();
        config.strict = true;
        let result = LogStore::build(&bundle(b"{\"a\":1}\nbroken\n"), &config);
        let error = match result {
            Err(error) => error,
            Ok(_) => panic!("strict mode must fail"),
        };
        assert!(format!("{error}").contains("line 2"), "{error}");
    }

    #[test]
    fn max_records_truncates_explicitly() {
        let mut config = Config::stub();
        config.max_records = 1;
        let store = LogStore::build(&bundle(b"{\"a\":1}\n{\"a\":2}\n"), &config).expect("builds");
        assert_eq!(store.records().len(), 1);
        assert!(store.truncated());
    }
}
