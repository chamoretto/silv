//! Free-text search compiled into the same predicate tree as filters.

use miette::Result;

use crate::filter::predicate::LeafOp;
use crate::filter::predicate::MatchKind;
use crate::filter::predicate::Matcher;
use crate::filter::predicate::Predicate;
use crate::model::field::FieldPath;

/// Where search patterns are looked for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SearchScope {
    /// Every scalar of the record.
    All,
    /// Only the listed field paths (containers are descended into).
    Fields(Vec<FieldPath>),
}

/// Search bar state of a run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SearchSpec {
    pub patterns: Vec<String>,
    pub scope: SearchScope,
    pub regex: bool,
    pub case_insensitive: bool,
}

impl Default for SearchSpec {
    fn default() -> Self {
        Self {
            patterns: Vec::new(),
            scope: SearchScope::All,
            regex: false,
            case_insensitive: true,
        }
    }
}

impl SearchSpec {
    pub fn is_empty(&self) -> bool {
        self.patterns.is_empty()
    }

    pub fn text(&self) -> String {
        self.patterns.join(" ")
    }

    pub const fn scope_is_all(&self) -> bool {
        matches!(self.scope, SearchScope::All)
    }

    fn kind_for(&self, pattern: &str) -> MatchKind {
        if self.regex {
            return MatchKind::Regex;
        }
        if pattern.contains(['*', '?', '[', '{']) {
            return MatchKind::Glob;
        }
        MatchKind::Contains
    }

    /// Every pattern must match; patterns scoped to several fields are ORed.
    pub fn compile(&self) -> Result<Predicate> {
        let mut children = Vec::with_capacity(self.patterns.len());
        for pattern in &self.patterns {
            let kind = self.kind_for(pattern);
            let matcher = Matcher::compile(pattern, kind, self.case_insensitive, false)?;
            let leaves = match &self.scope {
                SearchScope::All => vec![Predicate::leaf(
                    FieldPath::root(),
                    LeafOp::Contains,
                    Some(matcher.clone()),
                )],
                SearchScope::Fields(paths) => paths
                    .iter()
                    .map(|path| Predicate::leaf(path.clone(), LeafOp::Contains, Some(matcher.clone())))
                    .collect(),
            };
            children.push(Predicate::any(leaves));
        }
        Ok(Predicate::all(children))
    }

    /// Regexes used to highlight search hits; literals and globs are converted to regexes.
    pub fn highlight_regexes(&self) -> Vec<regex::Regex> {
        self.patterns
            .iter()
            .filter_map(|pattern| {
                let source = match self.kind_for(pattern) {
                    MatchKind::Regex => pattern.clone(),
                    MatchKind::Contains => regex::escape(pattern),
                    MatchKind::Glob => glob_to_regex_source(pattern),
                };
                regex::RegexBuilder::new(&source)
                    .case_insensitive(self.case_insensitive)
                    .build()
                    .ok()
            })
            .collect()
    }
}

fn glob_to_regex_source(pattern: &str) -> String {
    let mut out = String::with_capacity(pattern.len() + 8);
    for ch in pattern.chars() {
        match ch {
            '*' => out.push_str(".*"),
            '?' => out.push('.'),
            c if "+()^$.|[]{}\\".contains(c) => {
                out.push('\\');
                out.push(c);
            },
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::record::FieldAliases;
    use std::str::FromStr;

    fn record(text: &str) -> crate::model::record::LogRecord {
        let object = serde_json::from_str(text).expect("fixture object");
        crate::model::record::LogRecord::from_object(
            0,
            1,
            0,
            object,
            &FieldAliases::with_defaults(&[], &[], &[]).expect("aliases"),
        )
    }

    #[test]
    fn substring_search_descends_and_is_case_insensitive() {
        let spec = SearchSpec {
            patterns: vec!["disk Full".to_owned()],
            ..SearchSpec::default()
        };
        let predicate = spec.compile().expect("compiles");
        assert!(predicate.eval(&record(r#"{"message":"Disk FULL at 03:00"}"#)));
        assert!(!predicate.eval(&record(r#"{"message":"ok"}"#)));
    }

    #[test]
    fn glob_metachars_switch_to_glob() {
        let spec = SearchSpec {
            patterns: vec!["aiokafka.*".to_owned()],
            ..SearchSpec::default()
        };
        let predicate = spec.compile().expect("compiles");
        assert!(predicate.eval(&record(r#"{"logger":"aiokafka.consumer"}"#)));
        assert!(!predicate.eval(&record(r#"{"logger":"prefix aiokafka.consumer"}"#)));
    }

    #[test]
    fn scope_limits_the_search() {
        let spec = SearchSpec {
            patterns: vec!["x".to_owned()],
            scope: SearchScope::Fields(vec![FieldPath::from_str("message").unwrap()]),
            ..SearchSpec::default()
        };
        let predicate = spec.compile().expect("compiles");
        assert!(predicate.eval(&record(r#"{"message":"x here"}"#)));
        assert!(!predicate.eval(&record(r#"{"other":"x here"}"#)));
    }

    #[test]
    fn all_patterns_must_match() {
        let spec = SearchSpec {
            patterns: vec!["disk".to_owned(), "full".to_owned()],
            ..SearchSpec::default()
        };
        let predicate = spec.compile().expect("compiles");
        assert!(predicate.eval(&record(r#"{"message":"disk full"}"#)));
        assert!(!predicate.eval(&record(r#"{"message":"disk"}"#)));
    }

    #[test]
    fn highlight_regexes_cover_literals_and_regexes() {
        let spec = SearchSpec {
            patterns: vec!["a.b".to_owned(), "x*y".to_owned()],
            ..SearchSpec::default()
        };
        let regexes = spec.highlight_regexes();
        assert_eq!(regexes.len(), 2);
        assert!(regexes[0].is_match("zz a.b zz"));

        let spec = SearchSpec {
            patterns: vec!["\\d+ms".to_owned()],
            regex: true,
            ..SearchSpec::default()
        };
        assert!(spec.highlight_regexes()[0].is_match("took 12ms"));
    }
}
