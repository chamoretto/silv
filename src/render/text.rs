//! Display width, truncation and padding helpers that respect wide characters.

use unicode_width::UnicodeWidthChar;

pub const ELLIPSIS: &str = "..";

/// Terminal width of a string, control characters counted as zero.
pub fn width(text: &str) -> usize {
    text.chars()
        .map(|ch| if ch.is_control() { 0 } else { ch.width().unwrap_or(0) })
        .sum()
}

/// Cuts the text to at most `max` display cells, appending `..` when something was removed.
pub fn truncate(text: &str, max: usize) -> String {
    if max == 0 || width(text) <= max {
        return text.to_owned();
    }
    if max <= ELLIPSIS.len() {
        return ELLIPSIS[..max].to_owned();
    }
    let budget = max - ELLIPSIS.len();
    let mut used = 0usize;
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        let ch_width = if ch.is_control() { 0 } else { ch.width().unwrap_or(0) };
        if used + ch_width > budget {
            break;
        }
        used += ch_width;
        out.push(ch);
    }
    out.push_str(ELLIPSIS);
    out
}

/// Replaces control characters with visible escapes so cells stay on one line.
pub fn sanitize(text: &str) -> String {
    if !text.chars().any(char::is_control) {
        return text.to_owned();
    }
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("  "),
            c if c.is_control() => out.push('\u{fffd}'),
            c => out.push(c),
        }
    }
    out
}

pub fn pad_right(text: &str, column: usize) -> String {
    let missing = column.saturating_sub(width(text));
    let mut out = String::with_capacity(text.len() + missing);
    out.push_str(text);
    out.push_str(&" ".repeat(missing));
    out
}

pub fn pad_left(text: &str, column: usize) -> String {
    let missing = column.saturating_sub(width(text));
    let mut out = String::with_capacity(text.len() + missing);
    out.push_str(&" ".repeat(missing));
    out.push_str(text);
    out
}

/// Splits text into lines limited to `width` cells, wrapping on whitespace.
pub fn wrap(text: &str, limit: usize) -> Vec<String> {
    if limit == 0 {
        return vec![text.to_owned()];
    }
    let mut out = Vec::new();
    let mut line = String::new();
    let mut used = 0usize;
    for word in text.split_whitespace() {
        let word_width = width(word);
        if word_width > limit {
            if !line.is_empty() {
                out.push(std::mem::take(&mut line));
                used = 0;
            }
            let mut chunk = String::new();
            let mut chunk_used = 0usize;
            for ch in word.chars() {
                let ch_width = ch.width().unwrap_or(0);
                if chunk_used + ch_width > limit {
                    out.push(std::mem::take(&mut chunk));
                    chunk_used = 0;
                }
                chunk.push(ch);
                chunk_used += ch_width;
            }
            if !chunk.is_empty() {
                out.push(chunk);
            }
            continue;
        }
        let extra = if used == 0 { word_width } else { word_width + 1 };
        if used + extra > limit {
            out.push(std::mem::take(&mut line));
            used = 0;
        }
        if used > 0 {
            line.push(' ');
            used += 1;
        }
        line.push_str(word);
        used += word_width;
    }
    if !line.is_empty() {
        out.push(line);
    }
    if out.is_empty() {
        out.push(String::new());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn width_counts_wide_characters() {
        assert_eq!(width("abc"), 3);
        assert_eq!(width("日本語"), 6);
        assert_eq!(width("a\u{7}b"), 2);
    }

    #[test]
    fn truncate_marks_cut_text() {
        assert_eq!(truncate("abcdef", 6), "abcdef");
        assert_eq!(truncate("abcdef", 5), "abc..");
        assert_eq!(truncate("abcdef", 1), ".");
        assert_eq!(truncate("abcdef", 0), "abcdef");
        assert_eq!(width(truncate("日本語です", 5).as_str()), 4);
        assert_eq!(truncate("日本語です", 5), "日..");
    }

    #[test]
    fn sanitize_keeps_cells_single_line() {
        assert_eq!(sanitize("a\nb\tc"), "a\\nb  c");
        assert_eq!(sanitize("plain"), "plain");
    }

    #[test]
    fn padding_uses_display_width() {
        assert_eq!(pad_right("ab", 5), "ab   ");
        assert_eq!(pad_left("ab", 5), "   ab");
        assert_eq!(pad_right("日本語", 7).len(), "日本語".len() + 1);
    }

    #[test]
    fn wrap_splits_on_words_and_breaks_long_words() {
        assert_eq!(wrap("the quick brown fox", 9), vec!["the quick", "brown fox"]);
        assert_eq!(wrap("supercalifragilistic", 9), vec!["supercali", "fragilist", "ic"]);
        assert_eq!(wrap("", 10), vec![""]);
    }
}
