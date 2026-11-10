//! Diagnostic codes and error types shown to the user.

use std::fmt::Display;

use miette::Diagnostic;
use miette::LabeledSpan;
use miette::NamedSource;
use miette::SourceSpan;
use thiserror::Error;

/// Every failure SILV can report, with stable machine readable codes.
#[derive(Debug, Error, Diagnostic)]
pub enum SilvError {
    #[error("cannot read `{path}`")]
    #[diagnostic(
        code(silv::io::read),
        help("check that the path exists and is readable, and that glob patterns match at least one file")
    )]
    InputRead {
        path: String,
        #[source]
        cause: std::io::Error,
    },

    #[error("cannot write output")]
    #[diagnostic(code(silv::io::write))]
    OutputWrite {
        #[source]
        cause: std::io::Error,
    },

    #[error("input is empty")]
    #[diagnostic(
        code(silv::input::empty),
        help("pass a file argument, pipe logs into stdin, or use `--include` with a matching glob")
    )]
    InputEmpty,

    #[error("cannot detect log format of `{name}`: first line is `{sample}`")]
    #[diagnostic(
        code(silv::input::format_undetected),
        help("use `--format jsonl` or `--format plain` to force a format")
    )]
    FormatUndetected { name: String, sample: String },

    #[error(transparent)]
    #[diagnostic(transparent)]
    Parse(#[from] Box<ParseDetail>),

    #[error("cannot parse field path `{input}`: {reason}")]
    #[diagnostic(
        code(silv::field::path_invalid),
        help(
            "use dots between keys (`a.b`), quotes for keys containing dots (`a.\"b.c\"`), and `[n]` or `[*]` for array elements"
        )
    )]
    FieldPath { input: String, reason: String },

    #[error("cannot parse filter `{input}`: {reason}")]
    #[diagnostic(
        code(silv::filter::expression_invalid),
        help(
            "filter grammar: `field=pattern`, `field!=pattern`, `field~pattern`, `field!~pattern`, `field^` (exists), \
              `field!^` (missing), combined with `not`, `and`, `or`, parentheses; quote values that contain spaces"
        )
    )]
    FilterParse { input: String, reason: String },

    #[error("cannot compile glob `{pattern}`")]
    #[diagnostic(code(silv::filter::glob_invalid))]
    Glob {
        pattern: String,
        #[source]
        cause: globset::Error,
    },

    #[error("cannot compile regex `{pattern}`")]
    #[diagnostic(code(silv::filter::regex_invalid))]
    Regex {
        pattern: String,
        #[source]
        cause: regex::Error,
    },

    #[error("unknown time format `{input}`")]
    #[diagnostic(
        code(silv::time::format_invalid),
        help("accepted: utc, local, time-utc, time-local, rfc3339, epoch, raw, none")
    )]
    TimeFormat { input: String },

    #[error("interactive mode requires a terminal on stdin and stdout")]
    #[diagnostic(
        code(silv::app::not_a_terminal),
        help("use `--print` for stream output, or run SILV from an interactive terminal")
    )]
    NotATerminal,

    #[error("`{flag}` requires a value: {reason}")]
    #[diagnostic(code(silv::cli::argument_invalid))]
    Argument { flag: String, reason: String },

    #[error("cannot read the key map `{path}`")]
    #[diagnostic(
        code(silv::keymap::read),
        help("a key map is a text file of `from<TAB>to` character pairs, one per line, `#` starts a comment")
    )]
    KeyMapRead {
        path: String,
        #[source]
        cause: std::io::Error,
    },

    #[error("key map `{path}`: {reason}")]
    #[diagnostic(
        code(silv::keymap::line_invalid),
        help("each line maps one layout character to one QWERTY character, separated by a tab")
    )]
    KeyMapLine { path: String, reason: String },
}

impl SilvError {
    /// Builds a parse diagnostic that points into the offending source line.
    pub fn parse(source_name: &str, text: &str, line: u64, offset: usize, line_start: usize, reason: &str) -> Self {
        let span = span_for_line(text, line_start);
        let _ = offset;
        Self::Parse(Box::new(ParseDetail {
            line,
            reason: reason.to_owned(),
            code: NamedSource::new(source_name, text.to_owned()),
            span: SourceSpan::new(span.offset().into(), span.len()),
            help: parse_help(reason),
        }))
    }

    /// Warning variant used by `--stats` reporting, never aborts a run.
    pub const fn severity_is_warning(&self) -> bool {
        false
    }
}

/// Payload of the parse diagnostic, boxed to keep [`SilvError`] small.
#[derive(Debug, Error, Diagnostic)]
#[error("line {line}: {reason}")]
#[diagnostic(code(silv::parse::invalid_line))]
pub struct ParseDetail {
    pub line: u64,
    pub reason: String,
    #[source_code]
    pub code: NamedSource<String>,
    #[label("here")]
    pub span: SourceSpan,
    #[help]
    pub help: String,
}

/// Marks a diagnostic that should not abort the run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IssueSeverity {
    Warning,
}

impl Display for IssueSeverity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Warning => f.write_str("warning"),
        }
    }
}

fn span_for_line(text: &str, line_start: usize) -> LabeledSpan {
    let end = text[line_start..]
        .find('\n')
        .map(|idx| line_start + idx)
        .unwrap_or(text.len());
    LabeledSpan::new(None, line_start, end - line_start)
}

fn parse_help(reason: &str) -> String {
    if reason.contains("expected value") || reason.contains("trailing characters") {
        return "each line of a JSON Lines file must contain exactly one JSON object".to_owned();
    }
    if reason.contains("key must be a string") {
        return "JSON object keys must be quoted strings".to_owned();
    }
    "check the reported line; use `--no-keep-bad-lines` to stop on the first bad line".to_owned()
}
