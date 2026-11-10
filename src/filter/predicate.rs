//! Filtering: matchers, predicate tree, evaluation against records.

use std::fmt::Display;
use std::fmt::Formatter;

use miette::Result;

use crate::error::SilvError;
use crate::model::field::FieldPath;
use crate::model::record::LogRecord;
use crate::model::value::text_of;
use crate::model::value::visit_map_strings;
use crate::model::value::visit_strings;

/// Max depth explored when scanning scalars for contains/search matching.
const SCALAR_DEPTH: usize = 8;

/// How a pattern is matched against field text.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MatchKind {
    /// Anchored glob (`*`, `?`, `[a-z]`, `{a,b}`).
    Glob,
    /// Substring, case sensitivity as configured.
    Contains,
    /// Regular expression.
    Regex,
}

/// Compiled pattern plus the text it came from.
#[derive(Clone, Debug)]
pub struct Matcher {
    pub pattern: String,
    kind: MatchKind,
    compiled: Compiled,
}

#[derive(Clone, Debug)]
enum Compiled {
    Glob(globset::GlobMatcher),
    Regex(regex::Regex),
}

impl Matcher {
    pub fn compile(pattern: &str, kind: MatchKind, case_insensitive: bool, glob_anywhere: bool) -> Result<Self> {
        let compiled = match kind {
            MatchKind::Glob => {
                let effective = if glob_anywhere && !pattern.starts_with('*') {
                    format!("*{pattern}*")
                } else {
                    pattern.to_owned()
                };
                let glob = globset::GlobBuilder::new(&effective)
                    .case_insensitive(case_insensitive)
                    .literal_separator(false)
                    .build()
                    .map_err(|cause| SilvError::Glob {
                        pattern: pattern.to_owned(),
                        cause,
                    })?;
                Compiled::Glob(glob.compile_matcher())
            },
            MatchKind::Contains => Compiled::Regex(
                regex::RegexBuilder::new(&regex::escape(pattern))
                    .case_insensitive(case_insensitive)
                    .build()
                    .map_err(|cause| SilvError::Regex {
                        pattern: pattern.to_owned(),
                        cause,
                    })?,
            ),
            MatchKind::Regex => Compiled::Regex(
                regex::RegexBuilder::new(pattern)
                    .case_insensitive(case_insensitive)
                    .build()
                    .map_err(|cause| SilvError::Regex {
                        pattern: pattern.to_owned(),
                        cause,
                    })?,
            ),
        };
        Ok(Self {
            pattern: pattern.to_owned(),
            kind,
            compiled,
        })
    }

    pub const fn kind(&self) -> MatchKind {
        self.kind
    }

    pub fn is_match(&self, text: &str) -> bool {
        match &self.compiled {
            Compiled::Glob(glob) => glob.is_match(text),
            Compiled::Regex(regex) => regex.is_match(text),
        }
    }

    /// Byte ranges of matches, used for highlighting; globs report their whole-line match.
    pub fn ranges(&self, text: &str) -> Vec<(usize, usize)> {
        match &self.compiled {
            Compiled::Regex(regex) => regex.find_iter(text).map(|m| (m.start(), m.end())).collect(),
            Compiled::Glob(_) if self.is_match(text) => vec![(0, text.len())],
            Compiled::Glob(_) => Vec::new(),
        }
    }
}

/// Comparison applied at a leaf.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LeafOp {
    Eq,
    Neq,
    Exists,
    NotExists,
    Contains,
    NotContains,
}

impl LeafOp {
    pub const fn token(self) -> &'static str {
        match self {
            Self::Eq => "=",
            Self::Neq => "!=",
            Self::Exists => "^",
            Self::NotExists => "!^",
            Self::Contains => "~",
            Self::NotContains => "!~",
        }
    }

    pub const fn takes_pattern(self) -> bool {
        !matches!(self, Self::Exists | Self::NotExists)
    }

    pub const fn is_negative(self) -> bool {
        matches!(self, Self::Neq | Self::NotExists | Self::NotContains)
    }
}

impl Display for LeafOp {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.token())
    }
}

impl std::str::FromStr for LeafOp {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "=" | "==" => Ok(Self::Eq),
            "!=" => Ok(Self::Neq),
            "^" => Ok(Self::Exists),
            "!^" => Ok(Self::NotExists),
            "~" => Ok(Self::Contains),
            "!~" => Ok(Self::NotContains),
            other => Err(format!("`{other}` is not a filter operator")),
        }
    }
}

/// Filter expression tree.
#[derive(Clone, Debug)]
pub enum Predicate {
    All(Vec<Predicate>),
    Any(Vec<Predicate>),
    Not(Box<Predicate>),
    Leaf {
        field: FieldPath,
        op: LeafOp,
        matcher: Option<Matcher>,
    },
}

impl Predicate {
    pub fn leaf(field: FieldPath, op: LeafOp, matcher: Option<Matcher>) -> Self {
        Self::Leaf { field, op, matcher }
    }

    pub fn eq(field: FieldPath, matcher: Matcher) -> Self {
        Self::leaf(field, LeafOp::Eq, Some(matcher))
    }

    pub fn neq(field: FieldPath, matcher: Matcher) -> Self {
        Self::leaf(field, LeafOp::Neq, Some(matcher))
    }

    pub fn exists(field: FieldPath) -> Self {
        Self::leaf(field, LeafOp::Exists, None)
    }

    pub fn not_exists(field: FieldPath) -> Self {
        Self::leaf(field, LeafOp::NotExists, None)
    }

    pub fn contains(field: FieldPath, matcher: Matcher) -> Self {
        Self::leaf(field, LeafOp::Contains, Some(matcher))
    }

    /// AND of several predicates, flattening nested `All` children.
    pub fn all(children: Vec<Predicate>) -> Self {
        let mut out = Vec::with_capacity(children.len());
        for child in children {
            match child {
                Self::All(inner) => out.extend(inner),
                other => out.push(other),
            }
        }
        if out.len() == 1 {
            return out.into_iter().next().unwrap_or(Self::All(Vec::new()));
        }
        Self::All(out)
    }

    /// OR of several predicates, flattening nested `Any` children.
    pub fn any(children: Vec<Predicate>) -> Self {
        let mut out = Vec::with_capacity(children.len());
        for child in children {
            match child {
                Self::Any(inner) => out.extend(inner),
                other => out.push(other),
            }
        }
        if out.len() == 1 {
            return out.into_iter().next().unwrap_or(Self::Any(Vec::new()));
        }
        Self::Any(out)
    }

    pub fn eval(&self, record: &LogRecord) -> bool {
        match self {
            Self::All(children) => children.iter().all(|child| child.eval(record)),
            Self::Any(children) => children.iter().any(|child| child.eval(record)),
            Self::Not(child) => !child.eval(record),
            Self::Leaf { field, op, matcher } => eval_leaf(record, field, *op, matcher.as_ref()),
        }
    }

    /// True when the leaf uses a glob pattern, so highlighting is skipped.
    pub fn has_glob(&self) -> bool {
        match self {
            Self::Leaf { matcher, .. } => matcher.as_ref().is_some_and(|m| m.kind() == MatchKind::Glob),
            Self::All(children) | Self::Any(children) => children.iter().any(Self::has_glob),
            Self::Not(child) => child.has_glob(),
        }
    }
}

fn eval_leaf(record: &LogRecord, field: &FieldPath, op: LeafOp, matcher: Option<&Matcher>) -> bool {
    match op {
        LeafOp::Exists => field_present(record, field),
        LeafOp::NotExists => !field_present(record, field),
        LeafOp::Eq => matcher.is_some_and(|m| any_value_matches(record, field, m)),
        LeafOp::Neq => !matcher.is_some_and(|m| any_value_matches(record, field, m)),
        LeafOp::Contains => matcher.is_some_and(|m| any_scalar_matches(record, field, m)),
        LeafOp::NotContains => !matcher.is_some_and(|m| any_scalar_matches(record, field, m)),
    }
}

fn field_present(record: &LogRecord, field: &FieldPath) -> bool {
    if field.is_empty() {
        return true;
    }
    record.value_at(field).is_some()
}

/// Matches the canonical text of each value addressed by `field` (full glob semantics).
fn any_value_matches(record: &LogRecord, field: &FieldPath, matcher: &Matcher) -> bool {
    if field.is_empty() {
        return false;
    }
    let mut values = Vec::new();
    field.collect(&record.object, &mut values);
    let mut buf = String::new();
    values
        .iter()
        .any(|value| matcher.is_match(text_of(value, &mut buf).as_str()))
}

/// Descends into containers under `field`; the root path covers every scalar of the record.
fn any_scalar_matches(record: &LogRecord, field: &FieldPath, matcher: &Matcher) -> bool {
    let mut found = false;
    if field.is_empty() {
        visit_map_strings(&record.object, SCALAR_DEPTH, &mut |text| {
            let hit = matcher.is_match(text);
            found |= hit;
            hit
        });
        return found;
    }
    if let Some(value) = record.value_at(field) {
        visit_strings(value, SCALAR_DEPTH, &mut |text| {
            let hit = matcher.is_match(text);
            found |= hit;
            hit
        });
    }
    found
}

impl Display for Predicate {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::All(children) => join(f, children, " and "),
            Self::Any(children) => join(f, children, " or "),
            Self::Not(child) => {
                if matches!(child.as_ref(), Self::Any(_)) {
                    write!(f, "not ({child})")
                } else {
                    write!(f, "not {child}")
                }
            },
            Self::Leaf { field, op, matcher } => {
                write!(f, "{field}{}", op.token())?;
                if let Some(matcher) = matcher {
                    let pattern = &matcher.pattern;
                    if pattern.contains(' ') {
                        write!(f, "\"{pattern}\"")
                    } else {
                        write!(f, "{pattern}")
                    }
                } else {
                    Ok(())
                }
            },
        }
    }
}

fn join(f: &mut Formatter<'_>, children: &[Predicate], sep: &str) -> std::fmt::Result {
    for (index, child) in children.iter().enumerate() {
        if index > 0 {
            f.write_str(sep)?;
        }
        let needs_paren = matches!(child, Predicate::All(_) | Predicate::Any(_));
        if needs_paren {
            f.write_str("(")?;
        }
        write!(f, "{child}")?;
        if needs_paren {
            f.write_str(")")?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::record::FieldAliases;
    use serde_json::json;

    fn record(text: &str) -> LogRecord {
        let object = serde_json::from_str(text).expect("fixture object");
        LogRecord::from_object(0, 1, 0, object, &FieldAliases::with_defaults(&[], &[], &[]).expect("aliases"))
    }

    fn glob(pattern: &str) -> Matcher {
        Matcher::compile(pattern, MatchKind::Glob, true, false).expect("compiles")
    }

    #[test]
    fn glob_matches_anchored() {
        let record = record(r#"{"message":{"logger":"aiokafka.consumer"},"level":"WARN"}"#);
        let logger = FieldPath::parse("message.logger").unwrap();
        let filter = Predicate::neq(logger.clone(), glob("aiokafka.*"));
        assert!(!filter.eval(&record));
        assert!(Predicate::neq(logger.clone(), glob("kafka.*")).eval(&record));
        assert!(Predicate::eq(logger, glob("aiokafka.*")).eval(&record));
    }

    #[test]
    fn neq_passes_records_without_the_field() {
        let record = record(r#"{"level":"INFO"}"#);
        let filter = Predicate::neq(FieldPath::parse("message.logger").unwrap(), glob("aiokafka.*"));
        assert!(filter.eval(&record));
        assert!(!Predicate::exists(FieldPath::parse("message.logger").unwrap()).eval(&record));
        assert!(Predicate::not_exists(FieldPath::parse("message.logger").unwrap()).eval(&record));
    }

    #[test]
    fn contains_descends_into_containers() {
        let record = record(r#"{"meta":{"tags":["a","b"]},"n":42}"#);
        assert!(Predicate::contains(FieldPath::root(), glob("b")).eval(&record));
        assert!(Predicate::contains(FieldPath::parse("meta.tags").unwrap(), glob("b")).eval(&record));
        assert!(Predicate::contains(FieldPath::root(), glob("42")).eval(&record));
        assert!(!Predicate::contains(FieldPath::root(), glob("zzz")).eval(&record));
    }

    #[test]
    fn composites_and_display() {
        let predicate = Predicate::all(vec![
            Predicate::exists(FieldPath::parse("a").unwrap()),
            Predicate::any(vec![
                Predicate::eq(FieldPath::parse("b").unwrap(), glob("x*")),
                Predicate::not_exists(FieldPath::parse("c").unwrap()),
            ]),
        ]);
        assert_eq!(predicate.to_string(), "a^ and (b=x* or c!^)");
        assert!(predicate.has_glob());
    }

    #[test]
    fn wildcard_fields_union_values() {
        let record = record(r#"{"m1":{"logger":"aiokafka.x"},"m2":{"logger":"other"}}"#);
        let pattern = FieldPath::parse("*.logger").unwrap();
        assert!(Predicate::eq(pattern.clone(), glob("other")).eval(&record));
        assert!(Predicate::eq(pattern, glob("aiokafka.*")).eval(&record));
    }

    #[test]
    fn json_value_fixture_helper() {
        assert_eq!(json!({"a": 1})["a"], json!(1));
    }
}
