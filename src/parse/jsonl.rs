//! JSON Lines parsing, one JSON object per line.

use ijson::IValue;

use crate::model::record::FieldAliases;
use crate::model::record::LogRecord;
use crate::parse::LineResult;
use crate::parse::ParseIssue;

pub const IS_ARRAY: &str = "line is a JSON array, a record must be an object";
pub const NOT_AN_OBJECT: &str = "line is not a JSON object";

pub fn parse_line(source: u32, number: u64, offset: usize, bytes: &[u8], aliases: &FieldAliases) -> LineResult {
    match serde_json::from_slice::<IValue>(bytes) {
        Ok(object) if object.is_object() => Ok(LogRecord::from_object(source, number, offset, object, aliases)),
        Ok(_) => {
            let text = String::from_utf8_lossy(bytes).into_owned();
            let reason = if text.starts_with('[') { IS_ARRAY } else { NOT_AN_OBJECT }.to_owned();
            Err(ParseIssue {
                source,
                line: number,
                offset,
                reason,
                raw: text,
            })
        },
        Err(error) => {
            let text = String::from_utf8_lossy(bytes).into_owned();
            Err(ParseIssue {
                source,
                line: number,
                offset,
                reason: describe(&error, &text),
                raw: text,
            })
        },
    }
}

fn describe(error: &serde_json::Error, text: &str) -> String {
    if !text.starts_with('{') {
        if text.starts_with('[') {
            return IS_ARRAY.to_owned();
        }
        return NOT_AN_OBJECT.to_owned();
    }
    let reason = error.to_string();
    let stripped = strip_position(&reason);
    if stripped.is_empty() {
        "invalid JSON".to_owned()
    } else {
        stripped.to_owned()
    }
}

fn strip_position(reason: &str) -> &str {
    match reason.rfind(" at line ") {
        Some(index) => reason[..index].trim_end(),
        None => reason.trim_end(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::level::Level;
    use crate::model::timestamp::TimeFormat;
    use serde_json::json;

    use crate::fixtures::aliases;
    use crate::fixtures::ivalue;

    #[test]
    fn parses_object_line() {
        let record = parse_line(
            0,
            7,
            42,
            br#"{"@timestamp":"2023-11-14T22:13:20.123Z","level":"INFO","message":"ok","n":1.5,"nested":{"a":[1,2]}}"#,
            &aliases(),
        )
        .expect("parses");
        assert_eq!(record.line, 7);
        assert_eq!(record.level, Some(Level::Info.into()));
        assert_eq!(record.stamp(TimeFormat::UtcDateTime), "2023-11-14 22:13:20.123");
        assert_eq!(
            record
                .object
                .get("nested")
                .and_then(|v| v.get("a"))
                .and_then(|a| a.get(1)),
            Some(&ivalue(json!(2))),
        );
    }

    #[test]
    fn reports_non_object_lines() {
        let issue = parse_line(0, 1, 0, b"[1,2,3]", &aliases()).expect_err("array line");
        assert_eq!(issue.reason, "line is a JSON array, a record must be an object");
        assert_eq!(issue.line, 1);

        let issue = parse_line(0, 2, 0, b"plain text", &aliases()).expect_err("text line");
        assert_eq!(issue.reason, "line is not a JSON object");

        let issue = parse_line(0, 3, 0, br#"{"a":}"#, &aliases()).expect_err("broken json");
        assert!(issue.reason.starts_with("expected value"), "{}", issue.reason);
    }
}
