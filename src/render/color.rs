//! Semantic styles, shared by the stream writer (ANSI) and the TUI (ratatui).

use std::io::Write;

use crate::model::level::Level;

/// Semantic style roles used across output surfaces.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Sg {
    Default,
    Muted,
    Key,
    Punct,
    Heading,
    Number,
    String,
    Bool,
    Null,
    Container,
    Search,
    Selected,
    Accent,
    Trace,
    Debug,
    Info,
    Warn,
    Error,
    Fatal,
    Invalid,
}

impl Sg {
    /// Maps a severity to its style.
    pub const fn for_level(level: Option<&Level>) -> Self {
        match level {
            Some(Level::Trace) => Self::Trace,
            Some(Level::Debug) => Self::Debug,
            Some(Level::Info) => Self::Info,
            Some(Level::Warn) => Self::Warn,
            Some(Level::Error) => Self::Error,
            Some(Level::Fatal) => Self::Fatal,
            None => Self::Muted,
        }
    }

    /// SGR parameter list for this style, empty for the default role.
    pub const fn sgr(self) -> &'static str {
        match self {
            Self::Default => "",
            Self::Muted => "90",
            Self::Key => "90",
            Self::Punct => "90",
            Self::Heading => "1;4;38;5;81",
            Self::Number => "38;5;37",
            Self::String => "",
            Self::Bool => "38;5;179",
            Self::Null => "2;90",
            Self::Container => "38;5;139",
            Self::Search => "7",
            Self::Selected => "7",
            Self::Accent => "38;5;81",
            Self::Trace => "2;36",
            Self::Debug => "38;5;33",
            Self::Info => "38;5;28",
            Self::Warn => "38;5;172",
            Self::Error => "1;38;5;196",
            Self::Fatal => "1;38;5;207",
            Self::Invalid => "1;38;5;204",
        }
    }

    /// Ratatui style for the same role, used by the TUI.
    pub fn ratatui(self) -> ratatui::style::Style {
        use ratatui::style::Color;
        use ratatui::style::Modifier;
        use ratatui::style::Style;

        let style = Style::new();
        match self {
            Self::Default => style,
            Self::Muted | Self::Key | Self::Punct => style.fg(Color::DarkGray),
            Self::Heading => style
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
            Self::Number => style.fg(Color::Cyan),
            Self::String => style,
            Self::Bool => style.fg(Color::Yellow),
            Self::Null => style.fg(Color::DarkGray).add_modifier(Modifier::DIM),
            Self::Container => style.fg(Color::Magenta),
            Self::Search => style.add_modifier(Modifier::REVERSED),
            Self::Selected => style.add_modifier(Modifier::REVERSED),
            Self::Accent => style.fg(Color::LightCyan),
            Self::Trace => style.fg(Color::DarkGray).add_modifier(Modifier::DIM),
            Self::Debug => style.fg(Color::Blue),
            Self::Info => style.fg(Color::Green),
            Self::Warn => style.fg(Color::Yellow),
            Self::Error => style.fg(Color::Red).add_modifier(Modifier::BOLD),
            Self::Fatal => style.fg(Color::Magenta).add_modifier(Modifier::BOLD),
            Self::Invalid => style.fg(Color::LightRed).add_modifier(Modifier::BOLD),
        }
    }
}

/// Writes `text` with the style applied when colors are enabled.
pub fn write_styled(out: &mut impl Write, enabled: bool, style: Sg, text: &str) -> std::io::Result<()> {
    if !enabled || text.is_empty() {
        return out.write_all(text.as_bytes());
    }
    let code = style.sgr();
    if code.is_empty() {
        return out.write_all(text.as_bytes());
    }
    out.write_all(b"\x1b[")?;
    out.write_all(code.as_bytes())?;
    out.write_all(b"m")?;
    out.write_all(text.as_bytes())?;
    out.write_all(b"\x1b[0m")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn level_styles_are_distinct() {
        assert_eq!(Sg::for_level(Some(&Level::Warn)), Sg::Warn);
        assert_eq!(Sg::for_level(None), Sg::Muted);
        assert_ne!(Sg::Error.sgr(), Sg::Fatal.sgr());
    }

    #[test]
    fn styled_write_emits_codes_only_when_enabled() {
        let mut enabled = Vec::new();
        write_styled(&mut enabled, true, Sg::Warn, "WARN").expect("writes");
        assert_eq!(String::from_utf8(enabled).expect("utf8"), "\x1b[38;5;172mWARN\x1b[0m");

        let mut plain = Vec::new();
        write_styled(&mut plain, false, Sg::Warn, "WARN").expect("writes");
        assert_eq!(String::from_utf8(plain).expect("utf8"), "WARN");

        let mut unstyled = Vec::new();
        write_styled(&mut unstyled, true, Sg::String, "text").expect("writes");
        assert_eq!(String::from_utf8(unstyled).expect("utf8"), "text");
    }

    #[test]
    fn ratatui_styles_build() {
        use ratatui::style::Modifier;
        assert!(Sg::Fatal.ratatui().add_modifier.contains(Modifier::BOLD));
        assert!(Sg::Info.ratatui().fg.is_some());
        assert!(Sg::Default.ratatui().fg.is_none());
    }
}
