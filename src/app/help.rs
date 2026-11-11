//! Help overlay.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::text::Span;
use ratatui::widgets::Block;
use ratatui::widgets::Paragraph;

const LINES: [&str; 26] = [
    "SILV - simple interactive log viewer",
    "",
    "Layout: search bar, filter chips, field header, rows, prompt line, status. The top stays visible",
    "while scrolling. The prompt line shows the control keys of the mode you are in.",
    "Rows show columns when fields are selected, otherwise text in the format default (kv for JSON Lines,",
    "the source line for plain logs).",
    "",
    "Keys:",
    "  /            focus search            F            filter wizard: field, operator, value",
    "  Tab/BackTab  move focus              Enter        open record / apply input / sort focused field",
    "  j k f b g G  move row, page          s            sort focused field (asc, desc, off)",
    "  c            field picker            d            delete focused filter, or focused column in header",
    "  v            cycle render mode       t m          toggle timestamp / message column",
    "  L r A        case, regex, scope      x X          clear search, clear all filters",
    "  Ctrl+Z       undo last filter       Ctrl+Y        redo the undone filter change",
    "  Header       Space = drag columns    drag mode: Left/Right reorder, Enter/click = sort",
    "  Space (row)  pick value, Enter =    filter by that value (asks the operator); Esc leaves the mode",
    "  Mouse        click row = detail, click header = drag mode, wheel = scroll (PgUp/PgDn too)",
    "  ?            this help               q, Ctrl-C    quit",
    "  Layout       keys map to QWERTY positions (Russian works), text entry keeps your characters",
    "",
    "Filter grammar: field=pattern, field!=pattern, field~text, field!~text, field^ (exists), field!^ (missing),",
    "combined with not / and / or and parentheses. Patterns are anchored globs; --glob-anywhere wraps them.",
    "Pseudo fields: @timestamp, @level, @message, @source, @line.",
    "",
    "Esc closes overlays, then clears search, then focuses filters.",
];

/// Draws the help text inside `area` (already cleared by the caller).
pub fn draw(frame: &mut Frame, area: Rect) {
    let block = Block::bordered()
        .title(" help ")
        .title_bottom(" Esc closes ")
        .border_style(Style::default().fg(ratatui::style::Color::DarkGray));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let lines: Vec<Line> = LINES
        .iter()
        .enumerate()
        .map(|(index, text)| {
            let style = if index == 0 {
                Style::default().add_modifier(Modifier::BOLD)
            } else if text.starts_with("Keys:") || text.starts_with("Filter grammar") {
                Style::default().add_modifier(Modifier::UNDERLINED)
            } else {
                Style::default()
            };
            Line::from(Span::styled((*text).to_owned(), style))
        })
        .collect();
    frame.render_widget(Paragraph::new(lines), inner);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn help_mentions_the_main_keys() {
        let text = LINES.join("\n");
        for token in ["focus search", "field picker", "cycle render", "Esc closes"] {
            assert!(text.contains(token), "missing {token}");
        }
    }
}
