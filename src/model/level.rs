//! Severity levels with alias and numeric-scale support.

use std::fmt::Display;
use std::fmt::Formatter;
use std::str::FromStr;

/// Known severities, ordered from the least to the most severe.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub enum Level {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
    Fatal,
}

impl Level {
    pub const ALL: [Self; 6] = [
        Self::Trace,
        Self::Debug,
        Self::Info,
        Self::Warn,
        Self::Error,
        Self::Fatal,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Trace => "TRACE",
            Self::Debug => "DEBUG",
            Self::Info => "INFO",
            Self::Warn => "WARN",
            Self::Error => "ERROR",
            Self::Fatal => "FATAL",
        }
    }

    pub const fn width() -> usize {
        5
    }

    pub const fn rank(self) -> u8 {
        match self {
            Self::Trace => 1,
            Self::Debug => 2,
            Self::Info => 3,
            Self::Warn => 4,
            Self::Error => 5,
            Self::Fatal => 6,
        }
    }
}

impl Display for Level {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.label())
    }
}

impl FromStr for Level {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        level_from_token(s).ok_or_else(|| format!("`{s}` is not a known severity name or number"))
    }
}

/// Severity as found in a log record: a known level, or an unrecognised token kept verbatim.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum LogLevel {
    Known(Level),
    Custom(String),
}

impl LogLevel {
    pub const fn known(&self) -> Option<Level> {
        match self {
            Self::Known(level) => Some(*level),
            Self::Custom(_) => None,
        }
    }

    pub const fn rank(&self) -> Option<u8> {
        match self {
            Self::Known(level) => Some(level.rank()),
            Self::Custom(_) => None,
        }
    }

    /// Fixed width label for table rendering.
    pub fn padded(&self) -> String {
        match self {
            Self::Known(level) => format!("{:>width$}", level.label(), width = Level::width()),
            Self::Custom(raw) => raw.clone(),
        }
    }
}

impl From<Level> for LogLevel {
    fn from(level: Level) -> Self {
        Self::Known(level)
    }
}

impl Display for LogLevel {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Known(level) => level.fmt(f),
            Self::Custom(raw) => f.write_str(raw),
        }
    }
}

impl FromStr for LogLevel {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let trimmed = s.trim();
        if trimmed.is_empty() {
            return Err("empty severity token".to_owned());
        }
        match level_from_token(trimmed) {
            Some(level) => Ok(Self::Known(level)),
            None => Ok(Self::Custom(trimmed.to_ascii_uppercase())),
        }
    }
}

fn level_from_token(token: &str) -> Option<Level> {
    let token = token.trim();
    if token.is_empty() {
        return None;
    }
    if let Some(level) = level_from_name(&token.to_ascii_lowercase()) {
        return Some(level);
    }
    if token.len() <= 3 && token.chars().all(|c| c.is_ascii_digit()) {
        let value: u32 = token.parse().ok()?;
        return level_from_number(f64::from(value));
    }
    if token.contains('.') {
        let value: f64 = token.parse().ok()?;
        return level_from_number(value);
    }
    if token.chars().all(|c| c.is_ascii_digit()) {
        let value: f64 = token.parse().ok()?;
        return level_from_number(value);
    }
    None
}

fn level_from_name(name: &str) -> Option<Level> {
    let level = match name {
        "trace" | "tracemsg" | "tracing" | "finest" | "vtrace" | "silly" => Level::Trace,
        "debug" | "dbg" | "verbose" | "fine" | "finer" | "diagnostic" | "config" | "notice" => Level::Debug,
        "info" | "information" | "inf" | "log" | "note" | "default" => Level::Info,
        "warn" | "warning" | "warnmsg" | "caution" => Level::Warn,
        "error" | "err" | "severe" | "failure" | "fail" => Level::Error,
        "fatal" | "crit" | "critical" | "alert" | "emerg" | "emergency" | "panic" | "dpanic" => Level::Fatal,
        _ => return None,
    };
    Some(level)
}

/// Maps numeric levels of the common logging scales (syslog, OTel, log4j/pino/python/logback).
fn level_from_number(value: f64) -> Option<Level> {
    if !value.is_finite() {
        return None;
    }
    let scale = if value >= 1000.0 { value / 1000.0 } else { value };
    let level = if scale <= 7.0 {
        // syslog: 0 emerg, 1 alert, 2 crit, 3 err, 4 warn, 5 notice, 6 info, 7 debug
        match scale as i32 {
            0..=2 => Level::Fatal,
            3 => Level::Error,
            4 => Level::Warn,
            5..=6 => Level::Info,
            _ => Level::Debug,
        }
    } else if scale < 25.0 {
        // OTel numeric severities
        match scale as i32 {
            8..=10 => Level::Trace,
            11..=14 => Level::Debug,
            15..=18 => Level::Info,
            19..=22 => Level::Warn,
            _ => Level::Error,
        }
    } else if scale < 40.0 {
        // log4j/pino/python/logback tens: ~20 info, ~30 warn, ~40 error, ~50 fatal
        Level::Warn
    } else if scale < 50.0 {
        Level::Error
    } else {
        Level::Fatal
    };
    Some(level)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_and_aliases() {
        for token in ["warn", "WARNING", "Warning"] {
            assert_eq!(LogLevel::from_str(token).unwrap(), LogLevel::Known(Level::Warn));
        }
        assert_eq!(LogLevel::from_str("critical").unwrap(), LogLevel::Known(Level::Fatal));
        assert_eq!(LogLevel::from_str("information").unwrap(), LogLevel::Known(Level::Info));
    }

    #[test]
    fn numeric_scales() {
        assert_eq!(LogLevel::from_str("7").unwrap(), LogLevel::Known(Level::Debug));
        assert_eq!(LogLevel::from_str("3").unwrap(), LogLevel::Known(Level::Error));
        assert_eq!(LogLevel::from_str("30").unwrap(), LogLevel::Known(Level::Warn));
        assert_eq!(LogLevel::from_str("60").unwrap(), LogLevel::Known(Level::Fatal));
        assert_eq!(LogLevel::from_str("30000").unwrap(), LogLevel::Known(Level::Warn));
    }

    #[test]
    fn unknown_tokens_are_custom() {
        assert_eq!(LogLevel::from_str("spamed").unwrap(), LogLevel::Custom("SPAMED".to_owned()));
        assert_eq!(LogLevel::from_str("").unwrap_err(), "empty severity token");
    }

    #[test]
    fn ordering_matches_severity() {
        assert!(Level::Info < Level::Warn);
        assert!(Level::Warn < Level::Fatal);
        assert_eq!(Level::Info.label(), "INFO");
    }
}
