//! Drawing of the sticky Kibana-like layout and all overlays.

use ratatui::Frame;
use ratatui::layout::Constraint;
use ratatui::layout::Flex;
use ratatui::layout::Layout;
use ratatui::layout::Rect;
use ratatui::style::Color;
use ratatui::style::Modifier;
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::text::Span;
use ratatui::widgets::Block;
use ratatui::widgets::Cell as TuiCell;
use ratatui::widgets::Clear;
use ratatui::widgets::HighlightSpacing;
use ratatui::widgets::List;
use ratatui::widgets::ListItem;
use ratatui::widgets::ListState;
use ratatui::widgets::Paragraph;
use ratatui::widgets::Row;
use ratatui::widgets::Table;
use ratatui::widgets::TableState;

use crate::app::detail;
use crate::app::help;
use crate::app::picker;
use crate::app::state::AppState;
use crate::app::state::Focus;
use crate::app::state::RowMode;
use crate::app::state::WIZARD_OPS;
use crate::app::state::WizardPhase;
use crate::cli::RenderChoice;
use crate::filter::Predicate;
use crate::render::fields::Column;
use crate::render::record_render::Cell;
use crate::render::record_render::highlight;
use crate::render::record_render::render_record;
use crate::render::record_render::resolve_render;
use crate::render::record_render::table_cells;
use crate::render::table as table_layout;

const WIDTH_CAP: usize = 60;
const GAP: &str = "  ";
const CHIPS_HINT: &str = "F to filter by field (field=pattern, field!=pattern, field^, not a and (b or c))";
const CHIP_PROMPTS: &str = "filters: d/Backspace remove, Enter edit, Left/Right select, x clear all, F add, Esc rows";

/// Renders the whole screen.
pub fn draw(frame: &mut Frame, state: &mut AppState) {
    let area = frame.area();
    let chips_height: u16 = match (state.filters.is_empty(), state.focus == Focus::FilterChips) {
        (true, _) => 1,
        (false, true) => 3,
        (false, false) => 2,
    };
    let rects = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(chips_height),
        Constraint::Length(1),
        Constraint::Min(3),
        Constraint::Length(1),
        Constraint::Length(1),
    ])
    .split(area);

    state.areas.search = rects[0];
    state.areas.chips = rects[1];
    state.areas.filter = rects[2];
    state.areas.rows = rects[3];
    state.areas.prompts = rects[4];
    state.areas.status = rects[5];

    draw_search(frame, state, rects[0]);
    draw_chips(frame, state, rects[1]);
    draw_filter_input(frame, state, rects[2]);
    draw_rows(frame, state, rects[3]);
    draw_prompts(frame, state, rects[4]);
    draw_status(frame, state, rects[5]);
    draw_wizard_dropdown(frame, state, area);

    if let Some(picker) = state.picker.as_ref() {
        let area = centered(area, 72, 60);
        frame.render_widget(Clear, area);
        picker::draw(frame, picker, area, state.config.color_enabled);
    }
    if state.detail.is_some() {
        frame.render_widget(Clear, area);
        detail::draw(frame, state, area);
    }
    if state.help {
        let area = centered(area, 84, 90);
        frame.render_widget(Clear, area);
        help::draw(frame, area);
    }
}

fn draw_search(frame: &mut Frame, state: &AppState, area: Rect) {
    let focused = state.focus == Focus::Search;
    let flags = format!(
        " {}{}{} ",
        if state.search.case_insensitive { "A" } else { "a" },
        if state.search.regex { "R" } else { "r" },
        if state.search.scope_is_all() { "F" } else { "f" },
    );
    let mut spans = vec![Span::styled(
        "search: ",
        Style::default().fg(ratatui::style::Color::DarkGray),
    )];
    let text = state.search_input.text();
    if focused {
        spans.extend(cursor_spans(&text, state.search_input.cursor(), true));
    } else if text.is_empty() {
        spans.push(Span::styled("[/] to search", Style::default().fg(ratatui::style::Color::DarkGray)));
    } else {
        spans.push(Span::raw(text));
    }
    spans.push(Span::styled(flags, Style::default().fg(ratatui::style::Color::DarkGray)));
    spans
        .push(Span::styled(format!(" {} hit", state.view.len()), Style::default().fg(ratatui::style::Color::DarkGray)));
    frame.render_widget(ratatui::widgets::Paragraph::new(Line::from(spans)), area);
}

/// Text of a focused line editor, with the character under the cursor reversed.
fn cursor_spans(text: &str, cursor: usize, focused: bool) -> Vec<Span<'static>> {
    if !focused {
        return vec![Span::raw(text.to_owned())];
    }
    let chars: Vec<char> = text.chars().collect();
    if cursor >= chars.len() {
        return vec![
            Span::raw(text.to_owned()),
            Span::styled(" ", Style::default().add_modifier(ratatui::style::Modifier::REVERSED)),
        ];
    }
    let mut spans = Vec::with_capacity(chars.len());
    for (index, ch) in chars.iter().enumerate() {
        if index == cursor {
            spans.push(Span::styled(ch.to_string(), Style::default().add_modifier(ratatui::style::Modifier::REVERSED)));
        } else {
            spans.push(Span::raw(ch.to_string()));
        }
    }
    spans
}

fn draw_chips(frame: &mut Frame, state: &mut AppState, area: Rect) {
    state.chip_cells.clear();
    let mut lines = vec![Line::from(vec![
        Span::styled("filters: ", Style::default().fg(Color::DarkGray)),
        Span::styled(CHIPS_HINT, Style::default().fg(Color::DarkGray)),
    ])];
    if state.filters.is_empty() {
        frame.render_widget(Paragraph::new(lines), area);
        return;
    }
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut x = 0u16;
    for (index, predicate) in state.filters.iter().enumerate() {
        let focused = state.focus == Focus::FilterChips && state.filter_cursor == index;
        let base = if focused {
            Style::default().add_modifier(Modifier::REVERSED)
        } else {
            Style::default()
        };
        let brackets = base.fg(Color::DarkGray);
        let body = predicate_spans(predicate, base);
        let body_width: usize = body.iter().map(span_width).sum();
        let length = u16::try_from(body_width + 3).unwrap_or(u16::MAX);
        spans.push(Span::styled("[", brackets));
        spans.extend(body);
        spans.push(Span::styled("] ", brackets));
        let start = x;
        state
            .chip_cells
            .push((start, start.saturating_add(length.saturating_sub(1)), index, 1));
        x = x.saturating_add(length);
    }
    lines.push(Line::from(spans));
    if state.focus == Focus::FilterChips {
        lines.push(Line::from(Span::styled(CHIP_PROMPTS, Style::default().fg(Color::DarkGray))));
    }
    frame.render_widget(Paragraph::new(lines), area);
}

/// The mode-specific control keys, on the line above the status bar.
fn draw_prompts(frame: &mut Frame, state: &AppState, area: Rect) {
    let text = state.prompts_text();
    if text.is_empty() {
        return;
    }
    frame.render_widget(Paragraph::new(Line::from(Span::styled(text, Style::default().fg(Color::DarkGray)))), area);
}

/// Colored spans for one predicate: gray field, green/soft-red operator, plain pattern.
fn predicate_spans(predicate: &Predicate, base: Style) -> Vec<Span<'static>> {
    let muted = base.fg(Color::DarkGray);
    match predicate {
        Predicate::Leaf { field, op, matcher } => {
            let mut spans = vec![
                Span::styled(field.to_string(), base.fg(Color::Gray)),
                Span::styled(
                    if matcher.is_some() {
                        format!(" {} ", op.token())
                    } else {
                        format!(" {}", op.token())
                    },
                    base.patch(op.style().ratatui()),
                ),
            ];
            if let Some(matcher) = matcher {
                let style = if predicate.has_glob() {
                    base
                } else {
                    base.fg(Color::Gray)
                };
                spans.push(Span::styled(matcher.pattern.clone(), style));
            }
            spans
        },
        Predicate::All(children) => join_predicate_spans(children, "and", base),
        Predicate::Any(children) => join_predicate_spans(children, "or", base),
        Predicate::Not(child) => {
            let mut spans = vec![Span::styled("not ", muted)];
            let needs_paren = matches!(child.as_ref(), Predicate::Any(_));
            if needs_paren {
                spans.push(Span::styled("(", muted));
            }
            spans.extend(predicate_spans(child, base));
            if needs_paren {
                spans.push(Span::styled(")", muted));
            }
            spans
        },
    }
}

fn join_predicate_spans(children: &[Predicate], separator: &str, base: Style) -> Vec<Span<'static>> {
    let muted = base.fg(Color::DarkGray);
    let mut spans = Vec::new();
    for (index, child) in children.iter().enumerate() {
        if index > 0 {
            spans.push(Span::styled(format!(" {separator} "), muted));
        }
        let needs_paren = matches!(child, Predicate::All(_) | Predicate::Any(_));
        if needs_paren {
            spans.push(Span::styled("(", muted));
        }
        spans.extend(predicate_spans(child, base));
        if needs_paren {
            spans.push(Span::styled(")", muted));
        }
    }
    spans
}

fn span_width(span: &Span<'_>) -> usize {
    span.content.chars().count()
}

/// Label split into spans with every case-insensitive `needle` run highlighted by background.
pub fn highlight_label(label: &str, needle: &str) -> Vec<Span<'static>> {
    let needle_chars: Vec<char> = needle.chars().collect();
    if needle_chars.is_empty() {
        return vec![Span::raw(label.to_owned())];
    }
    let chars: Vec<char> = label.chars().collect();
    let mut spans = Vec::new();
    let mut plain = String::new();
    let mut index = 0usize;
    while index < chars.len() {
        let hit = index + needle_chars.len() <= chars.len()
            && chars[index..index + needle_chars.len()]
                .iter()
                .zip(needle_chars.iter())
                .all(|(a, b)| a.eq_ignore_ascii_case(b));
        if hit {
            if !plain.is_empty() {
                spans.push(Span::raw(std::mem::take(&mut plain)));
            }
            let text: String = chars[index..index + needle_chars.len()].iter().collect();
            spans.push(Span::styled(text, Style::default().bg(Color::DarkGray)));
            index += needle_chars.len();
        } else {
            plain.push(chars[index]);
            index += 1;
        }
    }
    if !plain.is_empty() {
        spans.push(Span::raw(plain));
    }
    spans
}

fn draw_filter_input(frame: &mut Frame, state: &AppState, area: Rect) {
    let focused = state.focus == Focus::FilterInput;
    let text = state.filter_input.text();
    let wizard_suffix = state.wizard.as_ref().map(|wizard| match wizard.phase {
        WizardPhase::Field => "  1/3 field  Up/Down selects, Enter picks, Esc exits",
        WizardPhase::Operator => "  2/3 operator  Up/Down, Enter, or type = ! ~ n e E",
        WizardPhase::Pattern => "  3/3 pattern  Enter applies, Backspace goes back",
    });
    if !focused && text.is_empty() && wizard_suffix.is_none() {
        return;
    }
    let mut spans = vec![Span::styled(
        "filter: ",
        Style::default().fg(ratatui::style::Color::DarkGray),
    )];
    spans.extend(cursor_spans(&text, state.filter_input.cursor(), focused));
    if let Some(suffix) = wizard_suffix {
        spans.push(Span::styled(suffix, Style::default().fg(ratatui::style::Color::DarkGray)));
    } else if focused {
        spans.push(Span::styled("  Enter applies, Esc closes", Style::default().fg(ratatui::style::Color::DarkGray)));
    }
    frame.render_widget(ratatui::widgets::Paragraph::new(Line::from(spans)), area);
}

/// Autocomplete list under the filter input while the guided builder is open.
fn draw_wizard_dropdown(frame: &mut Frame, state: &mut AppState, screen: Rect) {
    state.areas.dropdown = Rect::default();
    let (phase, cursor, items): (WizardPhase, usize, Vec<ListItem>) = match state.wizard.as_ref() {
        Some(wizard) if wizard.phase == WizardPhase::Field => {
            let needle = state.filter_input.text().to_ascii_lowercase();
            (
                wizard.phase,
                wizard.cursor,
                wizard
                    .matches
                    .iter()
                    .map(|candidate| ListItem::new(Line::from(highlight_label(&candidate.label, &needle))))
                    .collect(),
            )
        },
        Some(wizard) if wizard.phase == WizardPhase::Operator => (
            wizard.phase,
            wizard.cursor,
            WIZARD_OPS
                .iter()
                .map(|option| {
                    // Color the leading operator token green (positive) or soft red (negation).
                    let (token, rest) = option.label.split_once(' ').unwrap_or((option.label, ""));
                    ListItem::new(Line::from(vec![
                        Span::styled(token.to_owned(), option.op.style().ratatui()),
                        Span::raw(rest.to_owned()),
                    ]))
                })
                .collect(),
        ),
        _ => return,
    };
    if items.is_empty() {
        return;
    }
    let y = state.areas.filter.y.saturating_add(1);
    if y >= screen.y.saturating_add(screen.height) {
        return;
    }
    let height = u16::try_from(items.len().min(12) + 2)
        .unwrap_or(u16::MAX)
        .min(screen.y.saturating_add(screen.height) - y);
    if height < 2 {
        return;
    }
    let rect = Rect {
        x: screen.x,
        y,
        width: screen.width.min(72),
        height,
    };
    state.areas.dropdown = rect;
    frame.render_widget(Clear, rect);
    let title = match phase {
        WizardPhase::Field => " fields (Enter) ",
        WizardPhase::Operator => " operators (Enter) ",
        WizardPhase::Pattern => "",
    };
    let block = Block::bordered()
        .title(Line::from(title))
        .border_style(Style::default().fg(Color::DarkGray));
    let inner = block.inner(rect);
    frame.render_widget(block, rect);
    if inner.height == 0 || inner.width == 0 {
        return;
    }
    let visible = usize::from(inner.height);
    let selected = cursor.min(items.len().saturating_sub(1));
    let mut list_state = ListState::default();
    list_state.select(Some(selected));
    if selected >= visible {
        *list_state.offset_mut() = selected - visible + 1;
    }
    let list = List::new(items)
        .highlight_symbol("> ")
        .highlight_style(Style::default().add_modifier(Modifier::REVERSED));
    frame.render_stateful_widget(list, inner, &mut list_state);
}

fn draw_rows(frame: &mut Frame, state: &mut AppState, area: Rect) {
    let height = usize::from(area.height);
    let body_height = height.saturating_sub(1).max(1);
    keep_cursor_visible(state, body_height);

    let mode = resolve_render(&state.config, state.store.format(), !state.columns.is_empty());
    if state.columns.is_empty() {
        draw_text_rows(frame, state, area, mode, body_height);
        return;
    }

    let start = state.rows_scroll;
    let visible: Vec<u32> = state.view.rows.iter().copied().skip(start).take(body_height).collect();
    let rows_cells: Vec<Vec<Cell>> = visible
        .iter()
        .map(|index| {
            state
                .store
                .record(usize::try_from(*index).unwrap_or(usize::MAX))
                .map(|record| table_cells(record, &state.columns, state.source_label(record), &state.config))
                .unwrap_or_default()
        })
        .collect();

    let headers = header_labels(state);
    let mut widths = table_layout::natural_widths(&headers, &rows_cells, WIDTH_CAP);
    table_layout::fit_widths(&mut widths, usize::from(area.width));
    record_header_cells(state, &widths, area);

    let header_row = build_header_row(state, &headers, &widths);
    let mut table_state = TableState::default();
    table_state.select(Some(state.cursor.saturating_sub(start)));

    let picking = state.focus == Focus::Rows && state.row_mode == RowMode::Pick;
    let rows: Vec<Row> = rows_cells
        .iter()
        .enumerate()
        .map(|(offset, cells)| {
            let highlighted = highlight(cells.clone(), &state.regexes);
            let mark_pick = picking && start + offset == state.cursor;
            let tui_cells: Vec<TuiCell<'static>> = highlighted
                .iter()
                .enumerate()
                .map(|(position, cell)| {
                    let mut style = cell.style.ratatui();
                    if mark_pick && position == state.pick_cursor {
                        style = style.bg(Color::DarkGray);
                    }
                    TuiCell::from(cell.text.clone()).style(style)
                })
                .collect();
            Row::new(tui_cells)
        })
        .collect();

    let table = Table::new(
        rows,
        widths
            .iter()
            .map(|w| Constraint::Length(u16::try_from(*w).unwrap_or(u16::MAX))),
    )
    .header(header_row)
    .column_spacing(2)
    .highlight_symbol("")
    .highlight_spacing(HighlightSpacing::Never)
    .row_highlight_style(Style::default().add_modifier(ratatui::style::Modifier::REVERSED))
    .flex(Flex::Legacy);
    frame.render_stateful_widget(table, area, &mut table_state);
}

fn draw_text_rows(frame: &mut Frame, state: &mut AppState, area: Rect, mode: RenderChoice, body_height: usize) {
    let start = state.rows_scroll;
    let rows: Vec<Row> = state
        .view
        .rows
        .iter()
        .copied()
        .skip(start)
        .take(body_height)
        .map(|index| {
            let cells = state
                .store
                .record(usize::try_from(index).unwrap_or(usize::MAX))
                .map(|record| {
                    let cells = render_record(record, mode, &state.columns, state.source_label(record), &state.config);
                    highlight(cells, &state.regexes)
                })
                .unwrap_or_default();
            Row::new(cells.iter().map(to_tui_cell))
        })
        .collect();

    let header = Row::new(vec![TuiCell::from(if state.columns.is_empty() {
        "(no fields selected - press c)".to_owned()
    } else {
        String::new()
    })]);
    let mut table_state = TableState::default();
    table_state.select(Some(state.cursor.saturating_sub(start)));
    let table = Table::new(rows, [Constraint::Fill(1)])
        .header(header)
        .highlight_symbol("")
        .highlight_spacing(HighlightSpacing::Never)
        .row_highlight_style(Style::default().add_modifier(ratatui::style::Modifier::REVERSED));
    state.header_cells.clear();
    frame.render_stateful_widget(table, area, &mut table_state);
}

fn keep_cursor_visible(state: &mut AppState, body_height: usize) {
    if body_height == 0 {
        return;
    }
    let body = u16::try_from(body_height).unwrap_or(u16::MAX);
    if state.cursor < state.rows_scroll {
        state.rows_scroll = state.cursor;
    } else if state.cursor >= state.rows_scroll + body_height {
        state.rows_scroll = state.cursor.saturating_sub(body_height - 1);
    }
    state.areas.rows.height = state.areas.rows.height.max(body);
}

fn header_labels(state: &AppState) -> Vec<String> {
    state
        .columns
        .iter()
        .map(|column| {
            let marker = state
                .sort_of(column)
                .map(|dir| format!(" {}", dir.marker()))
                .unwrap_or_default();
            format!("{}{marker}", column.label())
        })
        .collect()
}

fn build_header_row<'a>(state: &'a AppState, headers: &[String], widths: &[usize]) -> Row<'a> {
    let focused = state.focus == Focus::Header;
    let cells = headers.iter().enumerate().map(|(index, label)| {
        let column: Option<&Column> = state.columns.get(index);
        let is_key = column.map(|column| state.sort_of(column).is_some()).unwrap_or(false);
        let mut style = Style::default()
            .fg(ratatui::style::Color::Cyan)
            .add_modifier(ratatui::style::Modifier::BOLD);
        if is_key {
            style = style.add_modifier(ratatui::style::Modifier::UNDERLINED);
        }
        if focused && state.header_cursor == index {
            style = style.add_modifier(ratatui::style::Modifier::REVERSED);
        }
        let text = pad(label, widths.get(index).copied().unwrap_or(0));
        TuiCell::from(text).style(style)
    });
    Row::new(cells).bottom_margin(0)
}

fn record_header_cells(state: &mut AppState, widths: &[usize], area: Rect) {
    state.header_cells.clear();
    let mut x = 0u16;
    for (index, width) in widths.iter().enumerate() {
        let width_u16 = u16::try_from(*width).unwrap_or(u16::MAX);
        let start = x;
        let end = start.saturating_add(width_u16.saturating_sub(1));
        state.header_cells.push((start, end, index));
        x = x.saturating_add(width_u16).saturating_add(2);
    }
    let _ = area;
}

fn to_tui_cell(cell: &Cell) -> TuiCell<'static> {
    TuiCell::from(cell.text.clone()).style(cell.style.ratatui())
}

fn pad(text: &str, column: usize) -> String {
    let mut out = crate::render::text::truncate(text, column);
    let missing = column.saturating_sub(crate::render::text::width(&out));
    out.push_str(&" ".repeat(missing));
    out
}

fn draw_status(frame: &mut Frame, state: &AppState, area: Rect) {
    let reversed = Style::default().add_modifier(Modifier::REVERSED);
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut used = 0usize;
    let push = |spans: &mut Vec<Span<'static>>, used: &mut usize, text: String, style: Style| {
        *used += crate::render::text::width(&text);
        spans.push(Span::styled(text, style));
    };
    push(&mut spans, &mut used, state.status.clone(), reversed);
    for (index, (label, dir)) in state.sort_marks.iter().enumerate() {
        let separator = if index == 0 { " | sort " } else { "," };
        push(&mut spans, &mut used, separator.to_owned(), reversed);
        push(&mut spans, &mut used, label.clone(), reversed);
        push(&mut spans, &mut used, " ".to_owned(), reversed);
        // The sort symbol gets its own background so it stands out in the bar.
        push(&mut spans, &mut used, dir.marker().to_owned(), Style::default().bg(Color::DarkGray));
    }
    if let Some(message) = state.message.as_ref() {
        push(&mut spans, &mut used, format!(" | {message}"), reversed);
    }
    let missing = usize::from(area.width).saturating_sub(used);
    push(&mut spans, &mut used, " ".repeat(missing), reversed);
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

/// Centers an area of the given percentage within `area`.
pub fn centered(area: Rect, width_percent: u16, height_percent: u16) -> Rect {
    let width = (area.width * width_percent / 100).clamp(20, area.width);
    let height = (area.height * height_percent / 100).clamp(6, area.height);
    Rect {
        x: area.x + (area.width.saturating_sub(width)) / 2,
        y: area.y + (area.height.saturating_sub(height)) / 2,
        width,
        height,
    }
}

pub const fn gap() -> &'static str {
    GAP
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::filter::MatchKind;
    use crate::filter::Matcher;
    use crate::model::field::FieldPath;

    fn spans_text(spans: &[Span<'_>]) -> String {
        spans.iter().map(|span| span.content.as_ref()).collect()
    }

    #[test]
    fn highlight_label_marks_needle_runs() {
        let spans = highlight_label("record.file.path", "file");
        assert_eq!(spans_text(&spans), "record.file.path");
        assert!(
            spans
                .iter()
                .any(|span| span.style.bg == Some(Color::DarkGray) && span.content.as_ref() == "file")
        );
    }

    #[test]
    fn highlight_label_is_case_insensitive_and_empty_needle_is_plain() {
        let spans = highlight_label("Message", "me");
        assert!(spans.iter().any(|span| span.content.as_ref() == "Me"));
        let plain = highlight_label("Message", "");
        assert_eq!(plain.len(), 1);
    }

    #[test]
    fn chip_spans_pad_the_operator_and_color_it() {
        let matcher = Matcher::compile("*aiokafka*", MatchKind::Glob, true, false).expect("glob");
        let predicate = Predicate::neq(FieldPath::parse("record.file.path").expect("path"), matcher);
        let spans = predicate_spans(&predicate, Style::default());
        assert_eq!(spans_text(&spans), "record.file.path != *aiokafka*");
        let op = spans
            .iter()
            .find(|span| span.content.as_ref() == " != ")
            .expect("operator span");
        assert_eq!(op.style.fg, Some(Color::Rgb(255, 135, 135)));
    }

    #[test]
    fn chip_spans_compose_composites() {
        let predicate = Predicate::all(vec![
            Predicate::exists(FieldPath::parse("a").expect("path")),
            Predicate::eq(
                FieldPath::parse("b").expect("path"),
                Matcher::compile("x", MatchKind::Glob, true, false).expect("glob"),
            ),
        ]);
        let spans = predicate_spans(&predicate, Style::default());
        assert_eq!(spans_text(&spans), "a ^ and b = x");
        let eq = spans
            .iter()
            .find(|span| span.content.as_ref() == " = ")
            .expect("operator span");
        assert_eq!(eq.style.fg, Some(Color::Green));
    }
}
