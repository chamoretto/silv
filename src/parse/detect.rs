//! Format sniffing from the first non-empty line of a source.

use crate::error::SilvError;
use crate::parse::LogFormat;

/// Detects JSON Lines by shape, the stdout-like layout by its separators.
pub fn detect(bytes: &[u8], label: &str) -> Result<LogFormat, SilvError> {
    let Some(sample) = first_line(bytes) else {
        return Err(SilvError::InputEmpty);
    };
    let text = String::from_utf8_lossy(sample).trim().to_owned();
    if text.is_empty() {
        return Err(SilvError::InputEmpty);
    }
    if text.starts_with('{') && text.ends_with('}') && serde_json::from_str::<serde_json::Value>(&text).is_ok() {
        return Ok(LogFormat::Jsonl);
    }
    if looks_plain(&text) {
        return Ok(LogFormat::Plain);
    }
    Err(SilvError::FormatUndetected {
        name: label.to_owned(),
        sample: text.chars().take(80).collect(),
    })
}

fn looks_plain(text: &str) -> bool {
    let parts: Vec<&str> = text.split('|').collect();
    parts.len() >= 3 && crate::model::timestamp::Timestamp::parse(parts[0].trim()).is_some()
}

fn first_line(bytes: &[u8]) -> Option<&[u8]> {
    bytes
        .split_inclusive(|b| *b == b'\n')
        .map(|line| line.trim_ascii())
        .find(|line| !line.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_jsonl() {
        assert_eq!(detect(b"\n{\"a\":1}\n", "x.jsonl").unwrap(), LogFormat::Jsonl);
    }

    #[test]
    fn detects_plain() {
        assert_eq!(
            detect(b"2023-11-14 22:13:20.123 | INFO | started host=web1\n", "app.log").unwrap(),
            LogFormat::Plain
        );
    }

    #[test]
    fn reports_undetectable_and_empty() {
        let error = detect(b"not a log at all\n", "app.bin").expect_err("undetectable");
        assert!(matches!(error, SilvError::FormatUndetected { .. }));
        assert!(matches!(detect(b"", "empty").expect_err("empty"), SilvError::InputEmpty));
    }
}
