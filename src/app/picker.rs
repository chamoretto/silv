//! Field picker overlay: discovered fields with counts, toggling columns.

use ratatui::Frame;
use ratatui::layout::Constraint;
use ratatui::layout::Layout;
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::text::Span;
use ratatui::widgets::Block;
use ratatui::widgets::Cell as TuiCell;
use ratatui::widgets::Paragraph;
use ratatui::widgets::Row;
use ratatui::widgets::Table;
use ratatui::widgets::TableState;

use crate::app::state::PickerState;
use crate::render::text::truncate;

/// Draws the picker inside `area` (already cleared by the caller).
pub fn draw(frame: &mut Frame, picker: &PickerState, area: Rect, _enabled: bool) {
    let block = Block::bordered()
        .title(" fields ")
        .title_bottom(" Enter toggles the column, type to filter, Esc closes ")
        .border_style(Style::default().fg(ratatui::style::Color::DarkGray));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let rects = Layout::vertical([Constraint::Length(1), Constraint::Min(1)]).split(inner);
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("filter: ", Style::default().fg(ratatui::style::Color::DarkGray)),
            Span::raw(picker.input.text()),
        ])),
        rects[0],
    );

    let visible = usize::from(rects[1].height).max(1);
    let scroll = if picker.cursor >= visible {
        picker.cursor - visible + 1
    } else {
        0
    };
    let rows: Vec<Row> = picker
        .matches
        .iter()
        .enumerate()
        .skip(scroll)
        .take(visible)
        .map(|(position, index)| {
            let field = &picker.fields[*index];
            let selected = position == picker.cursor;
            let marker = if field.selected { "[x] " } else { "[ ] " };
            let label = truncate(&field.label, 34);
            Row::new(vec![
                TuiCell::from(format!("{marker}{label}")).style(Style::default().add_modifier(if selected {
                    Modifier::REVERSED
                } else {
                    Modifier::empty()
                })),
                TuiCell::from(field.count.to_string()).style(Style::default().fg(ratatui::style::Color::DarkGray)),
                TuiCell::from(field.types.clone()).style(Style::default().fg(ratatui::style::Color::DarkGray)),
            ])
        })
        .collect();

    let mut table_state = TableState::default();
    table_state.select(if picker.matches.is_empty() {
        None
    } else {
        Some(picker.cursor.saturating_sub(scroll))
    });
    frame.render_stateful_widget(
        Table::new(rows, [Constraint::Fill(1), Constraint::Length(8), Constraint::Length(18)])
            .header(
                Row::new(vec![
                    TuiCell::from("field").style(Style::default().fg(ratatui::style::Color::Cyan)),
                    TuiCell::from("records").style(Style::default().fg(ratatui::style::Color::Cyan)),
                    TuiCell::from("types").style(Style::default().fg(ratatui::style::Color::Cyan)),
                ])
                .bottom_margin(0),
            )
            .column_spacing(2)
            .highlight_symbol("")
            .row_highlight_style(Style::default().add_modifier(Modifier::REVERSED)),
        rects[1],
        &mut table_state,
    );
}
