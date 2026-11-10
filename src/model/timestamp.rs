//! Timestamp parsing and formatting for the common log conventions.

use std::fmt::Display;
use std::fmt::Formatter;
use std::str::FromStr;

use chrono::DateTime;
use chrono::Datelike;
use chrono::Local;
use chrono::LocalResult;
use chrono::NaiveDateTime;
use chrono::TimeZone;
use chrono::Timelike;
use chrono::Utc;
use serde_json::Value;

const NS_PER_SEC: i128 = 1_000_000_000;

/// Naive formats, interpreted as UTC.
const NAIVE_FORMATS: [&str; 9] = [
    "%Y-%m-%dT%H:%M:%S%.f",
    "%Y-%m-%d %H:%M:%S%.f",
    "%Y-%m-%d %H:%M:%S%.f UTC",
    "%Y/%m/%d %H:%M:%S%.f",
    "%d/%m/%Y %H:%M:%S%.f",
    "%d-%m-%Y %H:%M:%S%.f",
    "%d-%b-%Y %H:%M:%S%.f",
    "%b %d %Y %H:%M:%S%.f",
    "%Y%m%dT%H%M%S",
];

/// Formats carrying an explicit offset.
const OFFSET_FORMATS: [&str; 4] = [
    "%Y-%m-%dT%H:%M:%S%.f%:z",
    "%Y-%m-%d %H:%M:%S%.f %z",
    "%d/%b/%Y:%H:%M:%S %z",
    "%a, %d %b %Y %H:%M:%S %z",
];

/// How to render a timestamp value.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum TimeFormat {
    #[default]
    UtcDateTime,
    LocalDateTime,
    TimeUtc,
    TimeLocal,
    Rfc3339,
    Epoch,
    Raw,
    None,
}

impl TimeFormat {
    pub const ACCEPTED: [&'static str; 8] = [
        "utc",
        "local",
        "time-utc",
        "time-local",
        "rfc3339",
        "epoch",
        "raw",
        "none",
    ];

    pub const fn is_none(self) -> bool {
        matches!(self, Self::None)
    }
}

impl Display for TimeFormat {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::UtcDateTime => "utc",
            Self::LocalDateTime => "local",
            Self::TimeUtc => "time-utc",
            Self::TimeLocal => "time-local",
            Self::Rfc3339 => "rfc3339",
            Self::Epoch => "epoch",
            Self::Raw => "raw",
            Self::None => "none",
        })
    }
}

impl FromStr for TimeFormat {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "utc" | "utc-date" | "utc-date-time" | "date-utc" => Ok(Self::UtcDateTime),
            "local" | "local-date" | "local-date-time" | "date-local" => Ok(Self::LocalDateTime),
            "time-utc" | "utc-time" | "time" => Ok(Self::TimeUtc),
            "time-local" | "local-time" => Ok(Self::TimeLocal),
            "rfc3339" | "rfc-3339" | "iso" | "iso8601" => Ok(Self::Rfc3339),
            "epoch" | "unix" | "timestamp" => Ok(Self::Epoch),
            "raw" | "as-is" | "source" => Ok(Self::Raw),
            "none" | "off" | "no" | "skip" => Ok(Self::None),
            other => Err(format!("`{other}` is not a known time format")),
        }
    }
}

/// A point in time plus the text it was parsed from.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Timestamp {
    nanos: i128,
    raw: Box<str>,
}

impl Timestamp {
    pub fn from_nanos(nanos: i128, raw: &str) -> Self {
        Self { nanos, raw: raw.into() }
    }

    pub const fn nanos(&self) -> i128 {
        self.nanos
    }

    pub fn raw(&self) -> &str {
        &self.raw
    }

    pub fn to_datetime(&self) -> Option<DateTime<Utc>> {
        let secs = self.nanos.div_euclid(NS_PER_SEC);
        let nsecs = self.nanos.rem_euclid(NS_PER_SEC) as u32;
        i64::try_from(secs)
            .ok()
            .and_then(|secs| DateTime::from_timestamp(secs, nsecs))
    }

    /// Parses ISO 8601, common date-time layouts, `HH:MM:SS`, and epoch values.
    pub fn parse(text: &str) -> Option<Self> {
        let trimmed = text.trim();
        if trimmed.is_empty() {
            return None;
        }
        if let Some(nanos) = parse_epoch(trimmed) {
            return Some(Self::from_nanos(nanos, trimmed));
        }
        if let Ok(parsed) = DateTime::parse_from_rfc3339(trimmed) {
            return Some(Self::from_nanos(parsed.timestamp_nanos_loose(), trimmed));
        }
        for format in OFFSET_FORMATS {
            if let Ok(parsed) = DateTime::parse_from_str(trimmed, format) {
                return Some(Self::from_nanos(parsed.timestamp_nanos_loose(), trimmed));
            }
        }
        for format in NAIVE_FORMATS {
            if let Ok(parsed) = NaiveDateTime::parse_from_str(trimmed, format) {
                let nanos = normalize_naive(parsed).and_then(naive_to_nanos)?;
                return Some(Self::from_nanos(nanos, trimmed));
            }
        }
        if let Some(nanos) = parse_syslog_no_year(trimmed) {
            return Some(Self::from_nanos(nanos, trimmed));
        }
        parse_time_only(trimmed).map(|nanos| Self::from_nanos(nanos, trimmed))
    }

    /// Reads a timestamp from a JSON value (string, epoch number, or object with a `$date` key).
    pub fn from_value(value: &Value) -> Option<Self> {
        match value {
            Value::String(text) => Self::parse(text),
            Value::Number(number) => {
                let text = number.to_string();
                parse_epoch(&text).map(|nanos| Self::from_nanos(nanos, &text))
            },
            Value::Object(map) => {
                for key in ["$date", "$time", "$timestamp"] {
                    if let Some(found) = map.get(key) {
                        return Self::from_value(found);
                    }
                }
                None
            },
            _ => None,
        }
    }

    pub fn format(&self, format: TimeFormat) -> String {
        match format {
            TimeFormat::None => String::new(),
            TimeFormat::Raw => self.raw.to_string(),
            TimeFormat::Epoch => format_epoch(self.nanos),
            TimeFormat::Rfc3339 => self
                .to_datetime()
                .map(|dt| dt.to_rfc3339_opts(chrono::SecondsFormat::Millis, true))
                .unwrap_or_else(|| self.raw.to_string()),
            TimeFormat::UtcDateTime => self.render_utc("%Y-%m-%d %H:%M:%S%.3f"),
            TimeFormat::TimeUtc => self.render_utc("%H:%M:%S%.3f"),
            TimeFormat::LocalDateTime => self.render_local("%Y-%m-%d %H:%M:%S%.3f"),
            TimeFormat::TimeLocal => self.render_local("%H:%M:%S%.3f"),
        }
    }

    fn render_utc(&self, format: &str) -> String {
        self.to_datetime()
            .map(|dt| dt.format(format).to_string())
            .unwrap_or_else(|| self.raw.to_string())
    }

    fn render_local(&self, format: &str) -> String {
        let Some(secs) = self.to_datetime().map(|dt| dt.timestamp()) else {
            return self.raw.to_string();
        };
        let nsecs = self.nanos.rem_euclid(NS_PER_SEC) as u32;
        match Local.timestamp_opt(secs, nsecs) {
            LocalResult::Single(dt) => dt.format(format).to_string(),
            _ => self.raw.to_string(),
        }
    }
}

impl PartialOrd for Timestamp {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Timestamp {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.nanos.cmp(&other.nanos)
    }
}

trait NanoSeconds {
    fn timestamp_nanos_loose(&self) -> i128;
}

impl<Tz: chrono::TimeZone> NanoSeconds for DateTime<Tz> {
    fn timestamp_nanos_loose(&self) -> i128 {
        i128::from(self.timestamp()) * NS_PER_SEC + i128::from(self.timestamp_subsec_nanos())
    }
}

fn normalize_naive(parsed: NaiveDateTime) -> Option<NaiveDateTime> {
    if parsed.year() >= 1970 {
        return Some(parsed);
    }
    let year = Utc::now().year();
    let date = chrono::NaiveDate::from_ymd_opt(year, parsed.month(), parsed.day())?;
    Some(date.and_time(parsed.time()))
}

/// `Nov 14 22:13:20` style syslog stamps carry no year; assume the current one.
fn parse_syslog_no_year(text: &str) -> Option<i128> {
    let mut parts = text.split_whitespace();
    let month = month_number(parts.next()?)?;
    let day: u32 = parts.next()?.parse().ok()?;
    let time = parts.next()?;
    if parts.next().is_some() {
        return None;
    }
    let time = chrono::NaiveTime::parse_from_str(time, "%H:%M:%S%.f").ok()?;
    let date = chrono::NaiveDate::from_ymd_opt(Utc::now().year(), month, day)?;
    naive_to_nanos(date.and_time(time))
}

fn month_number(name: &str) -> Option<u32> {
    let token = name.get(0..3)?.to_ascii_lowercase();
    Some(match token.as_str() {
        "jan" => 1,
        "feb" => 2,
        "mar" => 3,
        "apr" => 4,
        "may" => 5,
        "jun" => 6,
        "jul" => 7,
        "aug" => 8,
        "sep" => 9,
        "oct" => 10,
        "nov" => 11,
        "dec" => 12,
        _ => return None,
    })
}

fn naive_to_nanos(parsed: NaiveDateTime) -> Option<i128> {
    Utc.from_utc_datetime(&parsed).timestamp_nanos_opt().map(i128::from)
}

fn parse_time_only(text: &str) -> Option<i128> {
    let parsed = NaiveDateTime::parse_from_str(&format!("1970-01-01 {text}"), "%Y-%m-%d %H:%M:%S%.f").ok()?;
    let seconds = parsed.num_seconds_from_midnight() as i128;
    let nanos = parsed.and_utc().timestamp_subsec_nanos() as i128;
    let today = Utc::now().date_naive();
    let days = i128::from(today.num_days_from_ce() - 719_162);
    Some((days * 86_400 + seconds) * NS_PER_SEC + nanos)
}

fn format_epoch(nanos: i128) -> String {
    let secs = nanos.div_euclid(NS_PER_SEC);
    let frac = nanos.rem_euclid(NS_PER_SEC);
    if frac == 0 {
        format!("{secs}")
    } else {
        format!("{secs}.{:09}", frac).trim_end_matches('0').to_owned()
    }
}

/// Heuristic epoch decoding: digit count selects the unit (s, ms, us, ns).
fn parse_epoch(text: &str) -> Option<i128> {
    let digits = text.trim_start_matches(['-', '+']);
    let is_integer = !digits.contains(['.', 'e', 'E']) && digits.chars().all(|c| c.is_ascii_digit());
    if is_integer {
        let value: i128 = text.parse().ok()?;
        let magnitude = value.abs();
        let nanos = if magnitude >= 100_000_000_000_000_000 {
            value
        } else if magnitude >= 100_000_000_000_000 {
            value * 1_000
        } else if magnitude >= 100_000_000_000 {
            value * 1_000_000
        } else {
            value * NS_PER_SEC
        };
        return Some(nanos);
    }
    let value: f64 = text.parse().ok()?;
    if !value.is_finite() {
        return None;
    }
    let magnitude = value.abs();
    let nanos = if magnitude >= 1e17 {
        value as i128
    } else if magnitude >= 1e14 {
        (value * 1e3) as i128
    } else if magnitude >= 1e11 {
        (value * 1e6) as i128
    } else {
        (value * 1e9) as i128
    };
    Some(nanos)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn epoch_units_are_detected_from_magnitude() {
        assert_eq!(Timestamp::parse("1700000000").unwrap().nanos(), 1_700_000_000_000_000_000);
        assert_eq!(Timestamp::parse("1700000000123").unwrap().nanos(), 1_700_000_000_123_000_000);
        assert_eq!(Timestamp::parse("1700000000123456").unwrap().nanos(), 1_700_000_000_123_456_000);
        assert_eq!(Timestamp::parse("1700000000123456789").unwrap().nanos(), 1_700_000_000_123_456_789);
        assert_eq!(Timestamp::parse("1700000000.5").unwrap().nanos(), 1_700_000_000_500_000_000);
    }

    #[test]
    fn text_formats_parse() {
        let cases = [
            ("2023-11-14T22:13:20.123Z", "2023-11-14 22:13:20.123"),
            ("2023-11-14T23:13:20.123+01:00", "2023-11-14 22:13:20.123"),
            ("2023-11-14 22:13:20.123", "2023-11-14 22:13:20.123"),
            ("14/Nov/2023:22:13:20 +0000", "2023-11-14 22:13:20.000"),
            ("14-11-2023 22:13:20.123", "2023-11-14 22:13:20.123"),
            ("Tue, 14 Nov 2023 22:13:20 +0000", "2023-11-14 22:13:20.000"),
        ];
        for (case, expected) in cases {
            let parsed = Timestamp::parse(case).unwrap_or_else(|| panic!("should parse {case}"));
            assert_eq!(parsed.format(TimeFormat::UtcDateTime), expected, "{case}");
        }
        assert_eq!(
            Timestamp::parse("2023-11-14 22:13:20")
                .unwrap()
                .format(TimeFormat::UtcDateTime),
            "2023-11-14 22:13:20.000"
        );
        assert_eq!(
            Timestamp::parse("2023-11-14T22:13:20.123Z")
                .unwrap()
                .format(TimeFormat::Rfc3339),
            "2023-11-14T22:13:20.123Z"
        );
    }

    #[test]
    fn syslog_time_without_year_gets_the_current_date() {
        let parsed = Timestamp::parse("Nov 14 22:13:20").expect("parses");
        assert_eq!(parsed.format(TimeFormat::TimeUtc), "22:13:20.000");
        assert_eq!(Timestamp::parse("22:13:20.123").unwrap().format(TimeFormat::TimeUtc), "22:13:20.123");
    }

    #[test]
    fn formats_render_as_configured() {
        let ts = Timestamp::parse("2023-11-14T22:13:20.123Z").unwrap();
        assert_eq!(ts.format(TimeFormat::TimeUtc), "22:13:20.123");
        assert_eq!(ts.format(TimeFormat::Epoch), "1700000000.123");
        assert_eq!(ts.format(TimeFormat::Raw), "2023-11-14T22:13:20.123Z");
        assert_eq!(ts.format(TimeFormat::None), "");
    }

    #[test]
    fn out_of_range_values_do_not_panic() {
        let ts = Timestamp::from_nanos(i128::MAX, "unparseable-stamp");
        assert_eq!(ts.format(TimeFormat::UtcDateTime), "unparseable-stamp");
        assert_eq!(ts.format(TimeFormat::Rfc3339), "unparseable-stamp");
        assert_eq!(ts.format(TimeFormat::TimeLocal), "unparseable-stamp");
        let far = Timestamp::from_nanos(i128::from(i64::MAX), "far");
        assert_eq!(far.format(TimeFormat::Epoch), "9223372036.854775807");
    }

    #[test]
    fn json_values_are_accepted() {
        assert_eq!(Timestamp::from_value(&serde_json::json!(1700000000)).unwrap().nanos(), 1_700_000_000_000_000_000);
        assert_eq!(
            Timestamp::from_value(&serde_json::json!({ "$date": "2023-11-14T22:13:20.123Z" }))
                .unwrap()
                .format(TimeFormat::UtcDateTime),
            "2023-11-14 22:13:20.123"
        );
        assert!(Timestamp::parse("not a time").is_none());
        assert!(Timestamp::from_value(&serde_json::json!(true)).is_none());
    }

    #[test]
    fn time_format_tokens_roundtrip() {
        for token in TimeFormat::ACCEPTED {
            let parsed: TimeFormat = token.parse().expect("accepted token");
            assert_eq!(parsed.to_string(), token);
        }
        assert_eq!(TimeFormat::from_str("iso").unwrap(), TimeFormat::Rfc3339);
        assert!(TimeFormat::from_str("nope").is_err());
    }
}
