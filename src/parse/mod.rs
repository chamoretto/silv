//! Input loading, format detection and line parsers.

pub mod detect;
pub mod input;
pub mod jsonl;
pub mod plain;

use std::fmt::Display;
use std::fmt::Formatter;
use std::str::FromStr;

use crate::model::record::LogRecord;

/// Log layout on disk.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LogFormat {
    Jsonl,
    Plain,
}

impl Display for LogFormat {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Jsonl => "jsonl",
            Self::Plain => "plain",
        })
    }
}

impl FromStr for LogFormat {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "jsonl" | "json-lines" | "ndjson" | "json" => Ok(Self::Jsonl),
            "plain" | "text" | "stdout" | "logfmt" => Ok(Self::Plain),
            other => Err(format!("`{other}` is not a known format, use `jsonl` or `plain`")),
        }
    }
}

/// A line that could not be parsed.
#[derive(Clone, Debug)]
pub struct ParseIssue {
    pub source: u32,
    pub line: u64,
    pub offset: usize,
    pub reason: String,
    pub raw: String,
}

/// One line of input, parsed into a record.
pub type LineResult = Result<LogRecord, ParseIssue>;

/// Splits input into `(line_number, byte_offset, bytes)` triples without copying.
pub fn split_lines(bytes: &[u8]) -> Vec<(u64, usize, &[u8])> {
    let mut out = Vec::with_capacity(bytes.len() / 128 + 1);
    let mut offset = 0usize;
    for (index, slice) in bytes.split_inclusive(|b| *b == b'\n').enumerate() {
        let number = u64::try_from(index + 1).unwrap_or(u64::MAX);
        let trimmed_end = slice.len() - slice.iter().rev().take_while(|b| **b == b'\n' || **b == b'\r').count();
        let body = &slice[..trimmed_end];
        if !body.iter().all(|b| b.is_ascii_whitespace()) {
            out.push((number, offset, body));
        }
        offset += slice.len();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_and_numbers_lines() {
        let lines = split_lines(b"a\n\nb\r\n");
        assert_eq!(lines.iter().map(|(n, o, _)| (*n, *o)).collect::<Vec<_>>(), vec![(1, 0), (3, 3)]);
        assert_eq!(lines[1].2, b"b");
    }

    #[test]
    fn handles_input_without_trailing_newline() {
        let lines = split_lines(b"only");
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].2, b"only");
    }

    #[test]
    fn format_tokens_roundtrip() {
        assert_eq!(LogFormat::from_str("ndjson").unwrap(), LogFormat::Jsonl);
        assert_eq!(LogFormat::from_str("logfmt").unwrap(), LogFormat::Plain);
        assert_eq!(LogFormat::Jsonl.to_string(), "jsonl");
        assert!(LogFormat::from_str("yaml").is_err());
    }
}
