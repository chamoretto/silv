//! Screen geometry recorded while drawing, and mouse hit testing over it.

use super::*;

/// Screen areas recorded during drawing, used for mouse hit testing.
#[derive(Clone, Copy, Debug, Default)]
pub struct Areas {
    pub search: Rect,
    pub chips: Rect,
    pub filter: Rect,
    pub rows: Rect,
    pub prompts: Rect,
    pub status: Rect,
    pub dropdown: Rect,
    pub update: Rect,
}

impl AppState {
    /// Mouse: header cell sorts, rows open detail, inputs focus.
    pub fn handle_mouse(&mut self, kind: MouseEventKind, column: u16, row: u16) {
        match kind {
            MouseEventKind::Down(ratatui::crossterm::event::MouseButton::Left) => self.handle_click(column, row),
            MouseEventKind::ScrollUp => self.wheel(-1),
            MouseEventKind::ScrollDown => self.wheel(1),
            _ => {},
        }
    }

    fn handle_click(&mut self, column: u16, row: u16) {
        if self.overlays.help {
            self.overlays.help = false;
            return;
        }
        if self.update.repo_prompt.is_some() {
            // The pop-up is a text input: clicks elsewhere are inert, Esc closes it.
            return;
        }
        if self.update.changelog.is_some() {
            return;
        }
        // The value picker and the record detail own the screen: clicks behind them,
        // including on the update line row, are inert.
        if self.overlays.picker.is_some() || self.overlays.detail.is_some() {
            return;
        }
        if self.update_line_visible() && contains(self.geometry.areas.update, row) {
            if self
                .update
                .btn_rect
                .contains(ratatui::layout::Position::new(column, row))
            {
                self.nav.focus = Focus::UpdateLine;
                self.update.focus_index = 0;
                self.start_update();
            } else if self
                .update
                .changelog_btn
                .contains(ratatui::layout::Position::new(column, row))
            {
                self.nav.focus = Focus::UpdateLine;
                self.update.focus_index = 1;
                self.open_changelog();
            }
            return;
        }
        let areas = self.geometry.areas;
        if contains(areas.search, row) {
            self.nav.focus = Focus::Search;
            self.query.search_input.move_to(usize::from(column.saturating_sub(2)));
            return;
        }
        if self.overlays.wizard.is_some()
            && areas.dropdown.height > 2
            && row > areas.dropdown.y
            && row < areas.dropdown.y + areas.dropdown.height - 1
        {
            let index = usize::from(row - areas.dropdown.y - 1);
            if let Some(wizard) = self.overlays.wizard.as_mut() {
                wizard.cursor = index;
            }
            self.wizard_enter();
            return;
        }
        if contains(areas.filter, row) {
            self.open_filter_wizard();
            return;
        }
        if contains(areas.chips, row) {
            self.nav.focus = Focus::FilterChips;
            let row_offset = row.saturating_sub(areas.chips.y);
            if let Some((_, _, index, _)) = self
                .geometry
                .chip_cells
                .iter()
                .find(|(start, end, _, cell_row)| *cell_row == row_offset && *start <= column && column <= *end)
            {
                self.nav.filter_cursor = *index;
            }
            return;
        }
        if contains(areas.rows, row) {
            if row == areas.rows.top() {
                self.nav.focus = Focus::Header;
                if let Some((_, _, index)) = self
                    .geometry
                    .header_cells
                    .iter()
                    .find(|(start, end, _)| *start <= column && column <= *end)
                {
                    self.nav.header_cursor = *index;
                    match self.nav.header_mode {
                        // First click picks the column and enters drag; no sort change.
                        HeaderMode::Navigate => self.enter_column_drag(),
                        HeaderMode::Drag => {
                            if let Some(column) = self.focused_column().cloned() {
                                self.cycle_sort(&column);
                            }
                            self.leave_column_drag();
                        },
                    }
                }
                return;
            }
            let offset = usize::from(row - areas.rows.top() - 1);
            self.nav.cursor = (self.nav.rows_scroll + offset).min(self.view.len().saturating_sub(1));
            self.nav.focus = Focus::Rows;
            self.open_detail();
        }
    }
}

fn contains(area: Rect, row: u16) -> bool {
    area.height > 0 && row >= area.top() && row < area.top() + area.height
}
