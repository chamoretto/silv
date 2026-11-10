//! Column width computation and row writing for the aligned table output.

use crate::render::record_render::Cell;
use crate::render::text::pad_left;
use crate::render::text::pad_right;
use crate::render::text::truncate;
use crate::render::text::width;

pub const COLUMN_GAP: usize = 2;
pub const MIN_COLUMN: usize = 6;

/// Widest content per column, capped, with headers counted.
pub fn natural_widths(headers: &[String], rows: &[Vec<Cell>], cap: usize) -> Vec<usize> {
    let columns = headers.len();
    let mut widths = vec![0usize; columns];
    for (index, header) in headers.iter().enumerate() {
        widths[index] = width(header);
    }
    for row in rows {
        for (index, cell) in row.iter().enumerate().take(columns) {
            widths[index] = widths[index].max(width(&cell.text));
        }
    }
    if cap > 0 {
        for column in &mut widths {
            *column = (*column).min(cap);
        }
    }
    widths
}

pub fn total_width(widths: &[usize]) -> usize {
    let gaps = if widths.is_empty() {
        0
    } else {
        (widths.len() - 1) * COLUMN_GAP
    };
    let sum: usize = widths.iter().sum();
    sum + gaps
}

/// Shrinks the widest columns until the table fits `available` cells.
/// Each step lowers the widest column toward the second widest, so wide
/// columns keep as much room as possible (water level), instead of one
/// column collapsing to the minimum first.
pub fn fit_widths(widths: &mut [usize], available: usize) {
    if available == 0 {
        return;
    }
    let mut guard = 0usize;
    while total_width(widths) > available && guard < 10_000 {
        guard += 1;
        let excess = total_width(widths) - available;
        let mut widest: Option<usize> = None;
        let mut second = MIN_COLUMN;
        for (index, width) in widths.iter().enumerate() {
            match widest {
                None => widest = Some(index),
                Some(best) => {
                    if *width > widths[best] {
                        second = widths[best].max(second);
                        widest = Some(index);
                    } else {
                        second = second.max(*width);
                    }
                },
            }
        }
        let Some(index) = widest.filter(|i| widths[*i] > MIN_COLUMN) else {
            return;
        };
        let target = second.max(MIN_COLUMN);
        let shrink = (widths[index] - target)
            .max(1)
            .min(excess)
            .min(widths[index] - MIN_COLUMN);
        widths[index] -= shrink;
    }
}

/// Writes one padded row of cells.
pub fn write_row(
    out: &mut impl std::io::Write,
    cells: &[Cell],
    widths: &[usize],
    enabled: bool,
    numeric_right: &[bool],
    gap: &str,
) -> std::io::Result<()> {
    for (index, cell) in cells.iter().enumerate() {
        let column = widths.get(index).copied().unwrap_or(0);
        let text = truncate(&cell.text, column);
        let padded = if numeric_right.get(index).copied().unwrap_or(false) {
            pad_left(&text, column)
        } else {
            pad_right(&text, column)
        };
        if index > 0 {
            out.write_all(gap.as_bytes())?;
        }
        crate::render::color::write_styled(out, enabled, cell.style, &padded)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::color::Sg;

    fn cells(texts: &[&str]) -> Vec<Cell> {
        texts.iter().map(|text| Cell::new(*text, Sg::String)).collect()
    }

    fn headers(texts: &[&str]) -> Vec<String> {
        texts.iter().map(|text| (*text).to_owned()).collect()
    }

    #[test]
    fn natural_widths_follow_the_widest_content() {
        let widths = natural_widths(&headers(&["a", "bb"]), &[cells(&["xxx", "y"])], 0);
        assert_eq!(widths, vec![3, 2]);
    }

    #[test]
    fn cap_limits_widths() {
        let widths = natural_widths(&headers(&["a"]), &[cells(&["0123456789"])], 4);
        assert_eq!(widths, vec![4]);
    }

    #[test]
    fn fit_shrinks_the_widest_columns() {
        let mut widths = vec![40, 10, 6];
        fit_widths(&mut widths, 40);
        assert!(total_width(&widths) <= 40, "{widths:?}");
        assert!(widths.iter().all(|w| *w >= MIN_COLUMN), "{widths:?}");
    }

    #[test]
    fn fit_stops_when_everything_is_minimal() {
        let mut widths = vec![MIN_COLUMN, MIN_COLUMN];
        fit_widths(&mut widths, 4);
        assert_eq!(widths, vec![MIN_COLUMN, MIN_COLUMN]);
    }

    #[test]
    fn row_is_padded_to_widths() {
        let mut out = Vec::new();
        write_row(&mut out, &cells(&["ab", "cd"]), &[6, 4], false, &[false, true], "  ").expect("writes");
        assert_eq!(String::from_utf8(out).expect("utf8"), "ab        cd");
    }

    #[test]
    fn styled_row_emits_codes() {
        let mut out = Vec::new();
        let row = vec![Cell::new("WARN", Sg::Warn)];
        write_row(&mut out, &row, &[6], true, &[false], "  ").expect("writes");
        let text = String::from_utf8(out).expect("utf8");
        assert!(text.starts_with("\x1b[38;5;172m"), "{text}");
        assert!(text.contains("WARN  "));
    }
}
