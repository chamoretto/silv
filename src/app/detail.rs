//! Record detail overlay: every field, action menu, JSON view.

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
use ratatui::widgets::Clear;
use ratatui::widgets::List;
use ratatui::widgets::ListItem;
use ratatui::widgets::ListState;
use ratatui::widgets::Paragraph;
use ratatui::widgets::Row;
use ratatui::widgets::Table;
use ratatui::widgets::TableState;

use crate::app::state::AppState;
use crate::app::state::DetailState;
use crate::app::state::MenuAction;
use crate::render::text::truncate;
use crate::render::text::width;

const FOOTER: &str = "a column, f eq, F neq, ~ contains, e exists, E missing, v json, Enter menu, Esc close";

/// Draws the full screen detail of the selected record.
pub fn draw(frame: &mut Frame, state: &AppState, area: Rect) {
    let Some(detail) = state.detail.as_ref() else {
        return;
    };
    let record_line = state
        .record_at(detail.row)
        .map(|record| format!("{}:{}", state.source_label(record), record.line))
        .unwrap_or_else(|| "record".to_owned());
    let title = format!(" record {record_line} of {} ", state.view.len());

    let block = Block::bordered()
        .title(Line::from(Span::styled(title, Style::default().add_modifier(Modifier::BOLD))))
        .title_bottom(Line::from(format!(
            " {}/{} ",
            detail.cursor + 1,
            if detail.json {
                detail.json_text.len()
            } else {
                detail.fields.len()
            }
        )))
        .border_style(Style::default().fg(ratatui::style::Color::DarkGray));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    // Control keys get their own line at the top of the content: the window header is too narrow.
    let rects = Layout::vertical([Constraint::Length(1), Constraint::Min(1)]).split(inner);
    let footer = if detail.json { "v fields, Esc close" } else { FOOTER };
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            format!(" {footer}"),
            Style::default().fg(ratatui::style::Color::DarkGray),
        ))),
        rects[0],
    );

    if detail.json {
        draw_json(frame, detail, rects[1]);
    } else {
        draw_fields(frame, state, detail, rects[1]);
    }

    if detail.menu.is_some() {
        draw_menu(frame, detail, area);
    }
}

fn draw_json(frame: &mut Frame, detail: &DetailState, area: Rect) {
    let visible = usize::from(area.height);
    let scroll = (detail.cursor / visible.max(1)).min(detail.json_text.len());
    let lines: Vec<Line> = detail
        .json_text
        .iter()
        .skip(scroll * visible)
        .take(visible)
        .map(|line| Line::from(Span::raw(line.clone())))
        .collect();
    frame.render_widget(Paragraph::new(lines), area);
}

fn draw_fields(frame: &mut Frame, state: &AppState, detail: &DetailState, area: Rect) {
    let label_width = detail
        .fields
        .iter()
        .map(|field| width(&field.label))
        .max()
        .unwrap_or(8)
        .clamp(10, 28);

    let rows: Vec<Row> = detail
        .fields
        .iter()
        .enumerate()
        .map(|(index, field)| {
            let value = truncate(&field.value, usize::from(area.width).saturating_sub(label_width + 2));
            let selected = index == detail.cursor;
            let label_style = Style::default()
                .fg(ratatui::style::Color::DarkGray)
                .add_modifier(if selected {
                    Modifier::REVERSED
                } else {
                    Modifier::empty()
                });
            Row::new(vec![
                TuiCell::from(truncate(&field.label, label_width)).style(label_style),
                TuiCell::from(value).style(field.style.ratatui()),
            ])
        })
        .collect();

    let mut table_state = TableState::default();
    let visible = usize::from(area.height).max(1);
    let selected = detail.cursor.min(detail.fields.len().saturating_sub(1));
    table_state.select(Some(selected));
    let mut scroll = 0usize;
    if selected >= visible {
        scroll = selected - visible + 1;
    }
    *table_state.offset_mut() = scroll;
    let _ = state;

    frame.render_stateful_widget(
        Table::new(
            rows,
            [
                Constraint::Length(u16::try_from(label_width).unwrap_or(u16::MAX)),
                Constraint::Fill(1),
            ],
        )
        .column_spacing(2)
        .highlight_symbol("")
        .row_highlight_style(Style::default().add_modifier(Modifier::REVERSED)),
        area,
        &mut table_state,
    );
}

fn draw_menu(frame: &mut Frame, detail: &DetailState, area: Rect) {
    let width = 34u16;
    let height = u16::try_from(MenuAction::ALL.len() + 2).unwrap_or(u16::MAX);
    let menu_area = Rect {
        x: area.x + area.width.saturating_sub(width) / 2,
        y: area.y + area.height.saturating_sub(height) / 2,
        width: width.min(area.width),
        height: height.min(area.height),
    };
    frame.render_widget(Clear, menu_area);
    let block = Block::bordered().title(" field action ");
    let inner = block.inner(menu_area);
    frame.render_widget(block, menu_area);

    let cursor = detail.menu.unwrap_or(0);
    let items: Vec<ListItem> = MenuAction::ALL
        .iter()
        .map(|action| ListItem::new(Line::from(Span::styled(action.label().to_owned(), action.style().ratatui()))))
        .collect();
    let mut list_state = ListState::default();
    list_state.select(Some(cursor.min(items.len().saturating_sub(1))));
    frame.render_stateful_widget(
        List::new(items)
            .highlight_symbol("> ")
            .highlight_style(Style::default().add_modifier(Modifier::REVERSED)),
        inner,
        &mut list_state,
    );
}
