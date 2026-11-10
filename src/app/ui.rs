//! Drawing of the sticky Kibana-like layout and all overlays.

use ratatui::Frame;
use ratatui::layout::Constraint;
use ratatui::layout::Flex;
use ratatui::layout::Layout;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::text::Span;
use ratatui::widgets::Cell as TuiCell;
use ratatui::widgets::Clear;
use ratatui::widgets::HighlightSpacing;
use ratatui::widgets::Row;
use ratatui::widgets::Table;
use ratatui::widgets::TableState;

use crate::app::detail;
use crate::app::help;
use crate::app::picker;
use crate::app::state::AppState;
use crate::app::state::Focus;
use crate::cli::RenderChoice;
use crate::render::fields::Column;
use crate::render::record_render::Cell;
use crate::render::record_render::highlight;
use crate::render::record_render::render_record;
use crate::render::record_render::resolve_render;
use crate::render::record_render::table_cells;
use crate::render::table as table_layout;

const WIDTH_CAP: usize = 60;
const GAP: &str = "  ";

/// Renders the whole screen.
pub fn draw(frame: &mut Frame, state: &mut AppState) {
    let area = frame.area();
    let rects = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Min(3),
        Constraint::Length(1),
    ])
    .split(area);

    state.areas.search = rects[0];
    state.areas.chips = rects[1];
    state.areas.filter = rects[2];
    state.areas.rows = rects[3];
    state.areas.status = rects[4];

    draw_search(frame, state, rects[0]);
    draw_chips(frame, state, rects[1]);
    draw_filter_input(frame, state, rects[2]);
    draw_rows(frame, state, rects[3]);
    draw_status(frame, state, rects[4]);

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
    spans.extend(cursor_spans(&state.search_input.text(), state.search_input.cursor(), focused));
    spans.push(Span::styled(flags, Style::default().fg(ratatui::style::Color::DarkGray)));
    spans.push(Span::styled(format!("{} hit", state.view.len()), Style::default().fg(ratatui::style::Color::DarkGray)));
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
    let mut x = 0u16;
    let mut spans: Vec<Span<'static>> = vec![Span::styled(
        "filter: ",
        Style::default().fg(ratatui::style::Color::DarkGray),
    )];
    x += 8;
    state.chip_cells.clear();
    for (index, predicate) in state.filters.iter().enumerate() {
        let label = predicate.to_string();
        let focused = state.focus == Focus::FilterChips && state.filter_cursor == index;
        let text = format!("[{label}] ");
        let start = x;
        let length = u16::try_from(text.chars().count()).unwrap_or(u16::MAX);
        let style = if focused {
            Style::default().add_modifier(ratatui::style::Modifier::REVERSED)
        } else if predicate.has_glob() {
            Style::default()
        } else {
            Style::default().fg(ratatui::style::Color::Gray)
        };
        spans.push(Span::styled(text, style));
        state
            .chip_cells
            .push((start, start.saturating_add(length.saturating_sub(1)), index));
        x = x.saturating_add(length);
    }
    if state.filters.is_empty() {
        spans.push(Span::styled(
            "F to add a filter (field=pattern, field!=pattern, field^, field!^)",
            Style::default().fg(ratatui::style::Color::DarkGray),
        ));
    }
    frame.render_widget(ratatui::widgets::Paragraph::new(Line::from(spans)), area);
}

fn draw_filter_input(frame: &mut Frame, state: &AppState, area: Rect) {
    let focused = state.focus == Focus::FilterInput;
    let mut spans = vec![Span::styled(
        "add: ",
        Style::default().fg(ratatui::style::Color::DarkGray),
    )];
    spans.extend(cursor_spans(&state.filter_input.text(), state.filter_input.cursor(), focused));
    if focused {
        spans.push(Span::styled("  Enter applies, Esc closes", Style::default().fg(ratatui::style::Color::DarkGray)));
    }
    frame.render_widget(ratatui::widgets::Paragraph::new(Line::from(spans)), area);
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

    let rows: Vec<Row> = rows_cells
        .iter()
        .map(|cells| {
            let highlighted = highlight(cells.clone(), &state.regexes);
            Row::new(highlighted.iter().map(to_tui_cell))
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
    let message = state.message.clone().unwrap_or_default();
    let left = state.status.clone();
    let text = if message.is_empty() {
        left
    } else {
        let available = usize::from(area.width);
        let taken = crate::render::text::width(&left) + 3;
        format!("{left} | {}", crate::render::text::truncate(&message, available.saturating_sub(taken)))
    };
    frame.render_widget(
        ratatui::widgets::Paragraph::new(Line::from(Span::styled(
            pad(&text, usize::from(area.width)),
            Style::default().add_modifier(ratatui::style::Modifier::REVERSED),
        ))),
        area,
    );
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
