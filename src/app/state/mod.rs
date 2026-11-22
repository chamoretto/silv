//! Application state, view rebuilding and all user actions.

use ratatui::crossterm::event::KeyCode;
use ratatui::crossterm::event::KeyModifiers;
use ratatui::crossterm::event::MouseEventKind;
use ratatui::layout::Rect;
use ratatui::text::Line;
use strum::EnumCount;
use strum::EnumIter;
use strum::IntoEnumIterator;
use strum::IntoStaticStr;

use crate::app::input::Editor;
use crate::app::keymap::KeyMap;
use crate::app::keymap::KeyPress;
use crate::cli::Config;
use crate::cli::KeyMapMode;
use crate::cli::RenderChoice;
use crate::cli::SortDir;
use crate::cli::SortSpec;
use crate::data::LogStore;
use crate::data::view::View;
use crate::data::view::build_view;
use crate::filter::LeafOp;
use crate::filter::Predicate;
use crate::filter::SearchScope;
use crate::filter::SearchSpec;
use crate::filter::build_leaf;
use crate::filter::dsl::FilterContext;
use crate::filter::parse_expression;
use crate::model::field::FieldPath;
use crate::render::color::Sg;
use crate::render::fields::Column;
use crate::render::fields::cell_text;
use crate::render::fields::leaf_values;
use crate::render::fields::resolve_columns;
use crate::render::record_render::Cell;
use crate::update::PerformOutcome;
use crate::update::ReleaseNote;
use crate::update::RepoRef;
use crate::update::UpdateEvent;
use crate::update::UpdateOutcome;
use crate::update::UpdateTarget;

/// Live store, kept in RAM by design.
pub type Store = LogStore;

mod geometry;
mod overlays;
mod query;
mod update;

pub use geometry::Areas;
pub use overlays::DetailField;
pub use overlays::DetailState;
pub use overlays::MenuAction;
pub use overlays::PickField;
pub use overlays::PickerState;
pub use overlays::detail_fields;
pub use overlays::detail_json_lines;
pub use query::FieldCandidate;
pub use query::FilterWizard;
pub use query::WIZARD_OPS;
pub use query::WizardOp;
pub use query::WizardPhase;
pub(super) use query::glob_escape;
pub use update::ChangelogState;
pub use update::RepoPrompt;

/// Clamps a cursor into a (possibly empty) collection.
pub(super) fn clamp_cursor(cursor: &mut usize, len: usize) {
    *cursor = (*cursor).min(len.saturating_sub(1));
}

/// Moves a cursor by `delta`, clamped at both ends (`len` must be positive).
pub(super) fn clamp_move(cursor: &mut usize, len: usize, delta: isize) {
    let last = isize::try_from(len - 1).unwrap_or(isize::MAX);
    let current = isize::try_from(*cursor).unwrap_or(0).clamp(0, last);
    *cursor = usize::try_from((current + delta).clamp(0, last)).unwrap_or(0);
}

/// Overlay scroll keys: arrows step a line, Page keys step a page.
pub(super) fn page_scroll(key: KeyCode, page: isize) -> Option<isize> {
    match key {
        KeyCode::Down => Some(1),
        KeyCode::Up => Some(-1),
        KeyCode::PageDown => Some(page),
        KeyCode::PageUp => Some(-page),
        _ => None,
    }
}

/// Rows and changelog scroll keys: the page keys plus the vim set.
pub(super) fn vim_scroll(key: KeyCode, page: isize) -> Option<isize> {
    match key {
        KeyCode::Char('j') => Some(1),
        KeyCode::Char('k') => Some(-1),
        KeyCode::Char('f') => Some(page),
        KeyCode::Char('b') => Some(-page),
        _ => page_scroll(key, page),
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, EnumIter, IntoStaticStr)]
pub enum Focus {
    #[strum(to_string = "search")]
    Search,
    #[strum(to_string = "filters")]
    FilterChips,
    #[strum(to_string = "filter")]
    FilterInput,
    #[strum(to_string = "fields")]
    Header,
    #[strum(to_string = "rows")]
    Rows,
    /// The bottom update line; in the Tab cycle only while the line is drawn.
    #[strum(to_string = "update")]
    UpdateLine,
}

/// Header focus sub-mode: navigate between columns, or reorder the focused one.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum HeaderMode {
    Navigate,
    Drag,
}

/// Row focus sub-mode: plain navigation, or picking a cell value to filter by.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RowMode {
    Normal,
    Pick,
}

impl Focus {
    pub fn label(self) -> &'static str {
        (&self).into()
    }
}

/// Row, header and chip navigation: focus, modes, cursors, scrolls.
pub struct Navigator {
    pub focus: Focus,
    pub header_mode: HeaderMode,
    pub row_mode: RowMode,
    pub cursor: usize,
    pub header_cursor: usize,
    pub filter_cursor: usize,
    pub pick_cursor: usize,
    pub column_scroll: usize,
    pub rows_scroll: usize,
}

/// The query pipeline: filters with undo history, sorts, search and the input lines.
pub struct QueryState {
    pub filters: Vec<Predicate>,
    /// Filter lists for Ctrl+Z: each entry is the list as it was before a change.
    filter_undo: Vec<Vec<Predicate>>,
    /// Filter lists for Ctrl+Shift+Z: each entry is a list taken over by undo.
    filter_redo: Vec<Vec<Predicate>>,
    pub sorts: Vec<SortSpec>,
    pub sort_marks: Vec<(String, SortDir)>,
    pub search: SearchSpec,
    pub search_input: Editor,
    pub filter_input: Editor,
    pub regexes: Vec<regex::Regex>,
}

/// Open pop-ups: record detail, column picker, filter wizard, help.
pub struct Overlays {
    pub detail: Option<DetailState>,
    pub picker: Option<PickerState>,
    pub wizard: Option<FilterWizard>,
    pub help: bool,
}

/// The resolved keymap, the mode it was resolved for, and the remaining layout probes.
pub struct KeymapState {
    /// Layout character to QWERTY position, resolved from `config.key_map`.
    pub map: KeyMap,
    /// Mode the current map was resolved for, so a mode change re-resolves it.
    mode: KeyMapMode,
    /// Rebuilds left: a layout switch mid-session is probed a few times, then left alone.
    rebuilds: u8,
}

/// Screen areas and hit cells recorded while drawing, for mouse hit testing.
#[derive(Clone, Debug, Default)]
pub struct Geometry {
    pub areas: Areas,
    pub header_cells: Vec<(u16, u16, usize)>,
    pub chip_cells: Vec<(u16, u16, usize, u16)>,
}

/// The interactive self-update: worker channel, repository, release notes, overlays, buttons.
pub struct UpdateState {
    /// Release repository of the interactive update check (default repo, overridable).
    pub repo: RepoRef,
    /// Newest releases newer than the running version, newest first; the button appears
    /// on the update line when this is non-empty.
    pub available: Option<Vec<ReleaseNote>>,
    /// Bumped on every spawned check; events from an older generation are stale
    /// (a repository url was submitted while the old check was in flight) and get dropped.
    pub(crate) check_generation: u64,
    /// The update download/replace is in flight.
    pub updating: bool,
    /// A quit was refused once because an update is in flight; the second press quits.
    /// Quitting mid-replace can truncate the binary (Windows replaces the file in place).
    pub(crate) quit_confirm_pending: bool,
    /// Open changelog overlay (delta between the running and the newest version).
    pub changelog: Option<ChangelogState>,
    /// Open pop-up asking for a repository url (unreachable repo, forks).
    pub repo_prompt: Option<RepoPrompt>,
    /// Hit rects of the update line buttons, recorded while drawing.
    pub btn_rect: Rect,
    pub changelog_btn: Rect,
    /// While the update line is focused: 0 = the update button, 1 = the changelog button.
    pub focus_index: usize,
    /// The binary is a `cargo install` artifact (it sits in the cargo bin dir): the update
    /// line tells the user to update through cargo, since a self-update desyncs cargo's list.
    pub cargo_managed: bool,
    /// Update worker events; `run_loop` drains them via `poll_update_events`.
    pub events: (std::sync::mpsc::Sender<UpdateEvent>, std::sync::mpsc::Receiver<UpdateEvent>),
}

/// The whole interactive session state.
pub struct AppState {
    pub store: Store,
    pub config: Config,
    pub view: View,
    pub columns: Vec<Column>,
    pub message: Option<String>,
    pub status: String,
    pub dirty: bool,
    pub quit: bool,
    /// Focus, modes, cursors, scrolls.
    pub nav: Navigator,
    /// Filters, sorts, search and their inputs.
    pub query: QueryState,
    /// Open pop-ups.
    pub overlays: Overlays,
    /// Resolved keymap and its probe budget.
    pub keymap: KeymapState,
    /// Areas and hit cells recorded while drawing.
    pub geometry: Geometry,
    /// The interactive self-update.
    pub update: UpdateState,
}

/// Live rebuilds are skipped above this many records; Enter forces one.
const LIVE_REBUILD_LIMIT: usize = 30_000;

/// Layout probes per session after an unmapped non-ASCII key: covers a few layout switches.
const KEYMAP_REBUILDS: u8 = 3;

/// Filter changes kept for Ctrl+Z: a few mistakes deep, cheap (the lists are short).
pub(super) const FILTER_HISTORY_LIMIT: usize = 32;

impl AppState {
    pub fn new(store: Store, config: Config) -> Self {
        let columns = resolve_columns(&config, &store);
        let sorts = config.sorts.clone();
        let filters = config.filters.clone();
        let search = config.search.clone();
        let search_input = Editor::with_text(&search.text());
        let keymap = KeyMap::resolve(&config.key_map);
        let keymap_mode = config.key_map.clone();
        let mut state = Self {
            store,
            view: View::default(),
            columns,
            message: None,
            status: String::new(),
            dirty: true,
            quit: false,
            nav: Navigator {
                focus: Focus::Rows,
                header_mode: HeaderMode::Navigate,
                row_mode: RowMode::Normal,
                cursor: 0,
                header_cursor: 0,
                filter_cursor: 0,
                pick_cursor: 0,
                column_scroll: 0,
                rows_scroll: 0,
            },
            query: QueryState {
                filters,
                filter_undo: Vec::new(),
                filter_redo: Vec::new(),
                sorts,
                sort_marks: Vec::new(),
                search,
                search_input,
                filter_input: Editor::new(),
                regexes: Vec::new(),
            },
            overlays: Overlays {
                detail: None,
                picker: None,
                wizard: None,
                help: false,
            },
            keymap: KeymapState {
                map: keymap,
                mode: keymap_mode,
                rebuilds: KEYMAP_REBUILDS,
            },
            geometry: Geometry::default(),
            update: UpdateState {
                repo: config.update_repo.clone().unwrap_or_else(RepoRef::default_repo),
                available: None,
                check_generation: 0,
                updating: false,
                quit_confirm_pending: false,
                changelog: None,
                repo_prompt: None,
                btn_rect: Rect::default(),
                changelog_btn: Rect::default(),
                focus_index: 0,
                cargo_managed: crate::update::cargo_managed_install(),
                events: std::sync::mpsc::channel(),
            },
            config,
        };
        state.rebuild();
        state
    }

    /// Recomputes the visible rows after any filter, search or sort change.
    pub fn rebuild(&mut self) {
        let search_predicate = if self.query.search.is_empty() {
            None
        } else {
            match self.query.search.compile() {
                Ok(predicate) => Some(predicate),
                Err(error) => {
                    self.message = Some(format!("search: {}", error));
                    None
                },
            }
        };
        self.view = build_view(
            &self.store,
            &self.query.filters,
            search_predicate.as_ref(),
            &self.query.sorts,
            self.config.offset,
            self.config.limit,
            self.config.tail,
        );
        self.query.regexes = self.query.search.highlight_regexes();
        clamp_cursor(&mut self.nav.cursor, self.view.len());
        self.nav.rows_scroll = self.nav.rows_scroll.min(self.view.len());
        clamp_cursor(&mut self.nav.header_cursor, self.columns.len());
        clamp_cursor(&mut self.nav.filter_cursor, self.query.filters.len());
        self.update_status();
    }

    fn update_status(&mut self) {
        let shown = self.view.len();
        let total = self.store.len();
        self.query.sort_marks = self
            .query
            .sorts
            .iter()
            .map(|spec| (spec.field.to_string(), spec.dir))
            .collect();
        let focus_label = if self.nav.focus == Focus::Header && self.nav.header_mode == HeaderMode::Drag {
            "fields (drag)"
        } else {
            self.nav.focus.label()
        };
        self.status = format!(
            "rows {shown}/{} of {total} | fields {} | focus {focus_label} | {}",
            self.view.matched,
            self.columns.len(),
            self.config.render,
        );
    }

    /// Shows a status-line message and requests a redraw.
    pub fn say(&mut self, message: impl Into<String>) {
        self.message = Some(message.into());
        self.dirty = true;
    }

    pub const fn record_count(&self) -> usize {
        self.store.len()
    }

    pub fn record_at(&self, position: usize) -> Option<&crate::model::record::LogRecord> {
        self.view.record(&self.store, position)
    }

    pub fn current_record(&self) -> Option<&crate::model::record::LogRecord> {
        self.record_at(self.nav.cursor)
    }

    pub fn source_label(&self, record: &crate::model::record::LogRecord) -> &str {
        self.store.source_label_of(record)
    }

    pub fn toggle_column(&mut self, path: FieldPath) {
        let column = Column::from_path(&path);
        if let Some(index) = self.columns.iter().position(|c| *c == column) {
            self.columns.remove(index);
            self.message = Some(format!("column removed: {}", column.label()));
        } else {
            let label = column.label();
            self.columns.push(column);
            self.message = Some(format!("column added: {label}"));
        }
        clamp_cursor(&mut self.nav.header_cursor, self.columns.len());
        self.update_status();
        self.dirty = true;
    }

    pub fn columns_contain(&self, path: &FieldPath) -> bool {
        let column = Column::from_path(path);
        self.columns.contains(&column)
    }

    /// Ascending, then descending, then unsorted.
    pub fn cycle_sort(&mut self, column: &Column) {
        let label = column.label();
        let path = FieldPath::parse(&label).unwrap_or_else(|_| FieldPath::from_key(&label));
        let position = self
            .query
            .sorts
            .iter()
            .position(|spec| Column::from_path(&spec.field) == *column);
        match position {
            Some(index) => match self.query.sorts[index].dir {
                SortDir::Asc => self.query.sorts[index].dir = SortDir::Desc,
                SortDir::Desc => {
                    self.query.sorts.remove(index);
                    self.message = Some(format!("sort cleared: {label}"));
                },
            },
            None => {
                self.query.sorts.push(SortSpec {
                    field: path,
                    dir: SortDir::Asc,
                });
                self.message = Some(format!("sorted by: {label}"));
            },
        }
        self.rebuild();
    }

    pub fn sort_of(&self, column: &Column) -> Option<SortDir> {
        self.query
            .sorts
            .iter()
            .find(|spec| Column::from_path(&spec.field) == *column)
            .map(|spec| spec.dir)
    }

    pub fn enter_column_drag(&mut self) {
        if self.nav.header_mode == HeaderMode::Drag {
            return;
        }
        self.nav.header_mode = HeaderMode::Drag;
        self.update_status();
        self.say("column drag: Left/Right move column, Enter/click sorts, Space/Esc leaves");
    }

    pub fn leave_column_drag(&mut self) {
        if self.nav.header_mode == HeaderMode::Navigate {
            return;
        }
        self.nav.header_mode = HeaderMode::Navigate;
        self.update_status();
        self.dirty = true;
    }

    /// Reorders the focused column by `delta` positions; the cursor follows the column.
    pub fn move_column(&mut self, delta: isize) {
        if self.columns.is_empty() {
            return;
        }
        let from = self.nav.header_cursor;
        let to = (from as isize + delta).clamp(0, self.columns.len() as isize - 1) as usize;
        if to == from {
            return;
        }
        let label = self.columns[from].label();
        self.columns.swap(from, to);
        self.nav.header_cursor = to;
        self.say(format!("column moved: {label} -> {}", to + 1));
    }

    pub fn move_cursor(&mut self, delta: isize) {
        if self.view.is_empty() {
            self.nav.cursor = 0;
            return;
        }
        clamp_move(&mut self.nav.cursor, self.view.len(), delta);
    }

    /// Wheel scrolling, overlay-aware: three lines per notch in the active layer.
    /// The repository pop-up owns the screen: wheel there is inert.
    pub fn wheel(&mut self, notches: isize) {
        if self.update.repo_prompt.is_some() {
            return;
        }
        let delta = notches.saturating_mul(3);
        if self.update.changelog.is_some() {
            self.changelog_move(delta);
        } else if self.overlays.detail.is_some() {
            self.detail_move(delta);
        } else if self.overlays.picker.is_some() {
            self.picker_move(delta);
        } else {
            self.move_cursor(delta);
        }
    }

    pub fn go_first(&mut self) {
        self.nav.cursor = 0;
    }

    pub fn go_last(&mut self) {
        self.nav.cursor = self.view.len().saturating_sub(1);
    }

    pub fn page(&mut self, direction: isize, page: usize) {
        self.move_cursor(direction * isize::try_from(page).unwrap_or(isize::MAX));
    }

    pub fn cycle_render(&mut self) {
        self.config.render = match self.config.render {
            RenderChoice::Auto => RenderChoice::Table,
            RenderChoice::Table => RenderChoice::Kv,
            RenderChoice::Kv => RenderChoice::Line,
            RenderChoice::Line => RenderChoice::Json,
            RenderChoice::Json => RenderChoice::Jsonl,
            RenderChoice::Jsonl => RenderChoice::Auto,
        };
        self.update_status();
        self.say(format!("render: {:?}", self.config.render));
    }

    pub fn focused_column(&self) -> Option<&Column> {
        self.columns.get(self.nav.header_cursor)
    }

    pub fn cycle_focus(&mut self, direction: isize) {
        // The update line joins the cycle only while it is drawn; when it hides,
        // a focus on it falls back to the rows.
        let order: Vec<Focus> = Focus::iter()
            .filter(|focus| *focus != Focus::UpdateLine || self.update_line_visible())
            .collect();
        if order.iter().all(|focus| *focus != self.nav.focus) {
            self.nav.focus = Focus::Rows;
        }
        let len = isize::try_from(order.len()).unwrap_or(isize::MAX);
        let current = order.iter().position(|f| *f == self.nav.focus).unwrap_or(0);
        let current = isize::try_from(current).unwrap_or(0);
        let next = (current + direction).rem_euclid(len);
        self.nav.focus = order[usize::try_from(next).unwrap_or(0)];
        self.update_status();
    }

    pub fn quit(&mut self) {
        if self.update.updating && !self.update.quit_confirm_pending {
            // The self-replace is in flight: killing the process now can truncate the
            // binary (Windows renames the running file, then writes the new one).
            self.update.quit_confirm_pending = true;
            self.say("update: an update is in progress, press q again to quit anyway");
            return;
        }
        self.quit = true;
    }

    /// Resolves the key map, re-probing when the mode changed or the layout switched.
    fn keymap_press(&mut self, code: KeyCode, modifiers: KeyModifiers) -> KeyPress {
        if self.keymap.mode != self.config.key_map {
            self.keymap.mode = self.config.key_map.clone();
            self.keymap.map = KeyMap::resolve(&self.config.key_map);
        }
        let mut key = KeyPress::new(code, modifiers, &self.keymap.map);
        if key.needs_rebuild() && self.keymap.rebuilds > 0 {
            self.keymap.rebuilds -= 1;
            self.keymap.map = KeyMap::resolve(&self.config.key_map);
            key = KeyPress::new(code, modifiers, &self.keymap.map);
        }
        key
    }

    /// Handles a key press for the current focus and overlays.
    pub fn handle_key(&mut self, code: KeyCode, modifiers: KeyModifiers) {
        let key = self.keymap_press(code, modifiers);
        if key.modifiers.contains(KeyModifiers::CONTROL) && matches!(key.bind, KeyCode::Char('c') | KeyCode::Char('d'))
        {
            self.quit();
            return;
        }
        if key.modifiers.contains(KeyModifiers::CONTROL) {
            match key.bind {
                KeyCode::Char('z') => {
                    self.undo_filters();
                    self.dirty = true;
                    return;
                },
                KeyCode::Char('y') => {
                    self.redo_filters();
                    self.dirty = true;
                    return;
                },
                _ => {},
            }
        }
        if self.overlays.help {
            match key.bind {
                KeyCode::Esc | KeyCode::Char('?') | KeyCode::Char('q') => self.overlays.help = false,
                _ => {},
            }
            self.dirty = true;
            return;
        }
        if self.update.repo_prompt.is_some() {
            self.handle_repo_prompt_key(key);
            self.dirty = true;
            return;
        }
        if self.update.changelog.is_some() {
            self.handle_changelog_key(key);
            self.dirty = true;
            return;
        }
        if self.overlays.picker.is_some() {
            self.handle_picker_key(key);
            self.dirty = true;
            return;
        }
        if self.overlays.detail.is_some() {
            self.handle_detail_key(key);
            self.dirty = true;
            return;
        }
        match key.bind {
            KeyCode::Char('?') => {
                self.overlays.help = true;
                return;
            },
            KeyCode::Char('c') if !self.focus_is_text() && self.nav.focus != Focus::UpdateLine => {
                self.open_picker();
                return;
            },
            _ => {},
        }
        if !self.focus_is_wizard() {
            // Tab walks the focus ring from every plain focus; only the wizard
            // binds it to its own lists.
            match key.bind {
                KeyCode::Tab => {
                    self.cycle_focus(1);
                    self.dirty = true;
                    return;
                },
                KeyCode::BackTab => {
                    self.cycle_focus(-1);
                    self.dirty = true;
                    return;
                },
                _ => {},
            }
        }
        match self.nav.focus {
            Focus::Search => self.handle_search_key(key),
            Focus::FilterInput => self.handle_filter_key(key),
            Focus::FilterChips => self.handle_chips_key(key),
            Focus::Header => self.handle_header_key(key),
            Focus::Rows => self.handle_rows_key(key),
            Focus::UpdateLine => self.handle_update_line_key(key),
        }
        self.dirty = true;
    }

    const fn focus_is_text(&self) -> bool {
        matches!(self.nav.focus, Focus::Search | Focus::FilterInput)
    }

    /// True while the filter wizard owns the keys of the filter input.
    const fn focus_is_wizard(&self) -> bool {
        matches!(self.nav.focus, Focus::FilterInput) && self.overlays.wizard.is_some()
    }

    fn handle_header_key(&mut self, key: KeyPress) {
        let dragging = self.nav.header_mode == HeaderMode::Drag;
        match key.bind {
            KeyCode::Left | KeyCode::Char('h') => {
                if dragging {
                    self.move_column(-1);
                } else {
                    clamp_move(&mut self.nav.header_cursor, self.columns.len(), -1);
                }
            },
            KeyCode::Right | KeyCode::Char('l') => {
                if dragging {
                    self.move_column(1);
                } else {
                    clamp_move(&mut self.nav.header_cursor, self.columns.len(), 1);
                }
            },
            KeyCode::Enter | KeyCode::Char('s') => {
                if let Some(column) = self.focused_column().cloned() {
                    self.cycle_sort(&column);
                }
                self.leave_column_drag();
            },
            KeyCode::Char(' ') => {
                if dragging {
                    self.leave_column_drag();
                } else {
                    self.enter_column_drag();
                }
            },
            KeyCode::Char('d') => {
                if let Some(column) = self.focused_column().cloned() {
                    self.columns.remove(self.nav.header_cursor);
                    self.message = Some(format!("column removed: {}", column.label()));
                    clamp_cursor(&mut self.nav.header_cursor, self.columns.len());
                    self.update_status();
                }
            },
            KeyCode::Esc => {
                if dragging {
                    self.leave_column_drag();
                } else {
                    self.nav.focus = Focus::Rows;
                }
            },
            KeyCode::Down | KeyCode::Char('j') => self.nav.focus = Focus::Rows,
            _ => {},
        }
    }

    fn handle_rows_key(&mut self, key: KeyPress) {
        let picking = self.nav.row_mode == RowMode::Pick;
        match key.bind {
            KeyCode::Char('q') => self.quit(),
            KeyCode::Char('g') => self.go_first(),
            KeyCode::Char('G') => self.go_last(),
            KeyCode::Enter => {
                if picking {
                    self.pick_apply();
                } else {
                    self.open_detail();
                }
            },
            KeyCode::Char(' ') => self.toggle_row_pick(),
            KeyCode::Char('s') => {
                if let Some(column) = self.focused_column().cloned() {
                    self.cycle_sort(&column);
                }
            },
            KeyCode::Char('S') => self.nav.focus = Focus::Header,
            KeyCode::Char('t') => {
                self.toggle_column(FieldPath::parse("@timestamp").unwrap_or_else(|_| FieldPath::from_key("@timestamp")))
            },
            KeyCode::Char('m') => {
                self.toggle_column(FieldPath::parse("@message").unwrap_or_else(|_| FieldPath::from_key("@message")))
            },
            KeyCode::Char('L') => self.toggle_case(),
            KeyCode::Char('r') => self.toggle_regex(),
            KeyCode::Char('A') => self.toggle_scope(),
            KeyCode::Char('v') => self.cycle_render(),
            KeyCode::Char('x') => self.clear_search(),
            KeyCode::Char('d') => self.remove_filter(self.nav.filter_cursor),
            KeyCode::Char('X') => self.clear_filters(),
            KeyCode::Left => {
                if picking {
                    clamp_move(&mut self.nav.pick_cursor, self.columns.len(), -1);
                } else {
                    self.nav.column_scroll = self.nav.column_scroll.saturating_sub(1);
                }
            },
            KeyCode::Right => {
                if picking {
                    clamp_move(&mut self.nav.pick_cursor, self.columns.len(), 1);
                } else {
                    self.nav.column_scroll = (self.nav.column_scroll + 1).min(self.columns.len().saturating_sub(1));
                }
            },
            KeyCode::Esc => {
                if picking {
                    self.nav.row_mode = RowMode::Normal;
                } else if !self.query.search.patterns.is_empty() {
                    self.clear_search();
                } else if !self.query.filters.is_empty() {
                    self.nav.focus = Focus::FilterChips;
                }
            },
            KeyCode::Char('/') => self.nav.focus = Focus::Search,
            KeyCode::Char('F') => self.open_filter_wizard(),
            KeyCode::Char('u') if !self.config.no_update => self.open_repo_prompt(),
            _ => {
                let page = isize::try_from(self.page_height()).unwrap_or(1);
                if let Some(delta) = vim_scroll(key.bind, page) {
                    self.move_cursor(delta);
                }
            },
        }
    }

    /// Space on rows: toggle the value picker; Enter on a cell starts a filter on that value.
    pub fn toggle_row_pick(&mut self) {
        if self.nav.row_mode == RowMode::Pick {
            self.nav.row_mode = RowMode::Normal;
        } else if self.columns.is_empty() {
            self.message = Some("pick value needs columns: press c to pick fields".to_owned());
        } else {
            self.nav.row_mode = RowMode::Pick;
            self.nav.pick_cursor = self.nav.header_cursor.min(self.columns.len() - 1);
        }
        self.dirty = true;
    }

    /// Opens the filter wizard on the operator step, field and value taken from the picked cell.
    pub fn pick_apply(&mut self) {
        let Some(column) = self.columns.get(self.nav.pick_cursor).cloned() else {
            return;
        };
        let Some(record) = self.current_record().cloned() else {
            return;
        };
        let source = self.source_label(&record).to_owned();
        let (value, _style) = cell_text(&record, &column, &source, &self.config);
        let path = column_path(&column);
        let probe = truncate_probe(&value);
        self.nav.row_mode = RowMode::Normal;
        self.overlays.wizard = Some(FilterWizard {
            phase: WizardPhase::Operator,
            field: Some(path.clone()),
            op: None,
            matches: Vec::new(),
            cursor: 0,
            prefill: Some(probe.clone()),
        });
        self.query.filter_input.set(&path.to_string());
        self.nav.focus = Focus::FilterInput;
        self.say(format!("filter by value: {path} = {probe}, choose the operator"));
    }

    /// Mode-aware control keys shown on the prompt line; `[...]` marks key tokens.
    pub fn prompts_text(&self) -> &'static str {
        match self.nav.focus {
            Focus::Header if self.nav.header_mode == HeaderMode::Drag => {
                "column drag: [Left]/[Right] move column, [Enter]/[click] sort (asc/desc/off), [Space]/[Esc] leave drag"
            },
            Focus::Header => {
                "fields: [Left]/[Right] move, [Space] column drag, [Enter]/[s] sort (asc/desc/off), [d] remove column, [Esc] rows"
            },
            Focus::Rows if self.nav.row_mode == RowMode::Pick => {
                "pick value: [Left]/[Right] choose field, [Enter] filter by this value, [Space]/[Esc] leave"
            },
            Focus::UpdateLine => {
                "update line: [Left]/[Right] (h/l) choose update / changelog, [Enter] press it, [u] repository url, [q] quit, [Esc] rows"
            },
            Focus::Rows => concat!(
                "rows: [Enter] detail view, [Space] select value to filter by, [j]/[k] move, [f]/[b] page, ",
                "[g]/[G] first/last, [/] search, [F] add filter, [c] select fields to display as columns, ",
                "[s] sort by first column, [S] select column to sort at headers"
            ),
            _ => "",
        }
    }

    /// Filter history keys, on a second prompt line in the plain rows view.
    pub fn prompts_history(&self) -> Option<&'static str> {
        match self.nav.focus {
            Focus::Rows if self.nav.row_mode == RowMode::Normal => {
                Some("filters: [Ctrl+Z] undo last filter, [Ctrl+Y] redo last deleted filter")
            },
            _ => None,
        }
    }

    pub fn page_height(&self) -> usize {
        usize::from(self.geometry.areas.rows.height.saturating_sub(1)).max(1)
    }
}

/// The field path a column addresses; pseudo columns parse from their labels.
pub(super) fn column_path(column: &Column) -> FieldPath {
    match column {
        Column::Field(path) => path.clone(),
        other => FieldPath::parse(&other.label()).unwrap_or_else(|_| FieldPath::from_key(&other.label())),
    }
}

/// Probe used to build a filter from a field value; long values become a contains pattern.
pub(super) fn truncate_probe(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.len() <= 64 {
        return glob_escape(trimmed);
    }
    trimmed.chars().take(48).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::KeyMapMode;
    use crate::model::field::FieldPath;

    fn state_of(records: &[&str]) -> AppState {
        let config = Config::stub();
        let bundle = crate::parse::input::InputBundle {
            sources: vec![crate::parse::input::RawSource::for_test(
                "t.jsonl",
                records.join("\n").into_bytes(),
            )],
            read_stdin: false,
        };
        let store = LogStore::build(&bundle, &config).expect("builds");
        AppState::new(store, config)
    }

    #[test]
    fn new_state_selects_everything() {
        let state = state_of(&["{\"a\":1}", "{\"b\":2}"]);
        assert_eq!(state.view.len(), 2);
        assert_eq!(state.nav.focus, Focus::Rows);
        assert!(state.status.contains("rows 2/2 of 2"));
    }

    #[test]
    fn slash_focuses_search_and_typing_narrows_the_view() {
        let mut state = state_of(&["{\"message\":\"disk full\"}", "{\"message\":\"ok\"}"]);
        state.handle_key(KeyCode::Char('/'), KeyModifiers::NONE);
        assert_eq!(state.nav.focus, Focus::Search);
        for ch in "disk".chars() {
            state.handle_key(KeyCode::Char(ch), KeyModifiers::NONE);
        }
        assert_eq!(state.query.search_input.text(), "disk");
        assert_eq!(state.view.len(), 1);
        state.handle_key(KeyCode::Enter, KeyModifiers::NONE);
        assert_eq!(state.nav.focus, Focus::Rows);
        assert_eq!(state.view.len(), 1);
    }

    #[test]
    fn russian_layout_keys_drive_the_bindings() {
        let mut state = state_of(&["{\"a\":1}", "{\"a\":2}", "{\"a\":3}"]);
        // Cyrillic letters land on the QWERTY key that carries the binding.
        state.handle_key(KeyCode::Char('о'), KeyModifiers::NONE); // physical j
        assert_eq!(state.nav.cursor, 1);
        state.handle_key(KeyCode::Char('к'), KeyModifiers::NONE); // physical r
        assert!(state.query.search.regex);
        state.handle_key(KeyCode::Char('.'), KeyModifiers::NONE); // physical / focuses search
        assert_eq!(state.nav.focus, Focus::Search);
        state.handle_key(KeyCode::Char('й'), KeyModifiers::NONE); // physical q types text here
        assert_eq!(state.query.search_input.text(), "й");
        state.handle_key(KeyCode::Char('й'), KeyModifiers::SHIFT); // physical ? closes help
        state.nav.focus = Focus::Rows;
        state.handle_key(KeyCode::Char(','), KeyModifiers::SHIFT); // physical / with Shift is ?
        assert!(state.overlays.help);
        state.handle_key(KeyCode::Char('й'), KeyModifiers::NONE); // physical q closes help
        assert!(!state.overlays.help);
        state.handle_key(KeyCode::Char('с'), KeyModifiers::CONTROL); // physical Ctrl-C quits
        assert!(state.quit);
    }

    #[test]
    fn shift_s_focuses_the_field_header_from_rows() {
        let mut state = state_of(&["{\"a\":1}", "{\"b\":2}"]);
        assert_eq!(state.nav.focus, Focus::Rows);
        state.handle_key(KeyCode::Char('S'), KeyModifiers::SHIFT);
        assert_eq!(state.nav.focus, Focus::Header);
        state.handle_key(KeyCode::Esc, KeyModifiers::NONE);
        assert_eq!(state.nav.focus, Focus::Rows);
    }

    #[test]
    fn russian_layout_shift_s_focuses_the_field_header() {
        let mut state = state_of(&["{\"a\":1}"]);
        state.handle_key(KeyCode::Char('Ы'), KeyModifiers::SHIFT); // the physical S key
        assert_eq!(state.nav.focus, Focus::Header);
    }

    #[test]
    fn russian_layout_text_entry_keeps_cyrillic() {
        let mut state = state_of(&["{\"message\":\"ошибка диска\"}", "{\"message\":\"ok\"}"]);
        state.handle_key(KeyCode::Char('.'), KeyModifiers::NONE); // physical / focuses search
        assert_eq!(state.nav.focus, Focus::Search);
        for ch in "ошиб".chars() {
            state.handle_key(KeyCode::Char(ch), KeyModifiers::NONE);
        }
        assert_eq!(state.query.search_input.text(), "ошиб");
        assert_eq!(state.view.len(), 1);
    }

    #[test]
    fn raw_key_map_matches_the_typed_character() {
        let mut state = state_of(&["{\"a\":1}", "{\"a\":2}"]);
        state.config.key_map = KeyMapMode::Raw;
        state.handle_key(KeyCode::Char('о'), KeyModifiers::NONE); // physical j, unbound in raw
        assert_eq!(state.nav.cursor, 0);
        state.handle_key(KeyCode::Char('j'), KeyModifiers::NONE);
        assert_eq!(state.nav.cursor, 1);
    }

    #[test]
    fn wheel_scrolls_and_clamps() {
        let records: Vec<String> = (0..10).map(|i| format!("{{\"a\":{i}}}")).collect();
        let refs: Vec<&str> = records.iter().map(String::as_str).collect();
        let mut state = state_of(&refs);
        state.wheel(1);
        assert_eq!(state.nav.cursor, 3);
        state.wheel(-5);
        assert_eq!(state.nav.cursor, 0);
        state.wheel(-1);
        assert_eq!(state.nav.cursor, 0);
        state.wheel(100);
        assert_eq!(state.nav.cursor, 9);
    }

    #[test]
    fn wizard_walks_field_operator_pattern() {
        let mut state = state_of(&["{\"level\":\"INFO\"}", "{\"level\":\"WARN\"}"]);
        state.handle_key(KeyCode::Char('F'), KeyModifiers::NONE);
        assert!(state.overlays.wizard.is_some());
        for ch in "lev".chars() {
            state.handle_key(KeyCode::Char(ch), KeyModifiers::NONE);
        }
        assert!(
            state
                .overlays
                .wizard
                .as_ref()
                .is_some_and(|w| w.matches.iter().any(|c| c.label == "level"))
        );
        state.handle_key(KeyCode::Enter, KeyModifiers::NONE);
        assert!(
            state
                .overlays
                .wizard
                .as_ref()
                .is_some_and(|w| w.phase == WizardPhase::Operator)
        );
        state.handle_key(KeyCode::Enter, KeyModifiers::NONE);
        assert!(
            state
                .overlays
                .wizard
                .as_ref()
                .is_some_and(|w| w.phase == WizardPhase::Pattern)
        );
        for ch in "WARN".chars() {
            state.handle_key(KeyCode::Char(ch), KeyModifiers::NONE);
        }
        state.handle_key(KeyCode::Enter, KeyModifiers::NONE);
        assert!(state.overlays.wizard.is_none());
        assert_eq!(state.query.filters.len(), 1);
        assert!(state.query.filters[0].to_string().contains("level=WARN"));
        assert_eq!(state.view.len(), 1);
    }

    #[test]
    fn wizard_exists_completes_immediately() {
        let mut state = state_of(&["{\"level\":\"WARN\",\"trace.id\":\"t1\"}"]);
        state.open_filter_wizard();
        state.wizard_enter(); // field: first candidate
        state.wizard_move(4); // Exists
        state.wizard_enter();
        assert!(state.overlays.wizard.is_none());
        assert_eq!(state.query.filters.len(), 1);
        assert!(state.query.filters[0].to_string().contains('^'));
    }

    #[test]
    fn wizard_back_walks_out_one_phase_at_a_time() {
        let mut state = state_of(&["{\"level\":\"WARN\"}"]);
        state.open_filter_wizard();
        state.wizard_enter();
        state.wizard_enter();
        assert!(
            state
                .overlays
                .wizard
                .as_ref()
                .is_some_and(|w| w.phase == WizardPhase::Pattern)
        );
        state.wizard_back();
        assert!(
            state
                .overlays
                .wizard
                .as_ref()
                .is_some_and(|w| w.phase == WizardPhase::Operator)
        );
        state.wizard_back();
        assert!(
            state
                .overlays
                .wizard
                .as_ref()
                .is_some_and(|w| w.phase == WizardPhase::Field)
        );
    }

    #[test]
    fn detail_menu_enter_applies_the_action() {
        let mut state = state_of(&["{\"level\":\"WARN\",\"trace.id\":\"t1\"}"]);
        state.open_detail();
        state.detail_move(1); // focus @level (field list starts with @timestamp)
        state.handle_key(KeyCode::Enter, KeyModifiers::NONE); // open the action menu
        assert!(state.overlays.detail.as_ref().is_some_and(|d| d.menu == Some(0)));
        state.detail_move(1); // menu cursor: FilterEq
        state.handle_key(KeyCode::Enter, KeyModifiers::NONE); // apply
        assert!(state.overlays.detail.as_ref().is_some_and(|d| d.menu.is_none()));
        assert_eq!(state.query.filters.len(), 1);
        assert!(state.query.filters[0].to_string().contains("level=WARN"));
    }

    #[test]
    fn filters_narrow_and_can_be_removed() {
        let mut state = state_of(&["{\"level\":\"INFO\"}", "{\"level\":\"WARN\"}"]);
        let state_config = state.filter_context();
        let predicate = crate::filter::build_leaf("level", LeafOp::Eq, Some("WARN"), &state_config).expect("leaf");
        state.add_filter(predicate);
        assert_eq!(state.view.len(), 1);
        assert_eq!(state.query.filters.len(), 1);
        state.remove_filter(0);
        assert_eq!(state.view.len(), 2);
    }

    /// Leaf filter on `level`, for the history tests.
    fn level_filter(state: &AppState, value: &str) -> Predicate {
        crate::filter::build_leaf("level", LeafOp::Eq, Some(value), &state.filter_context()).expect("leaf")
    }

    const CONTROL: KeyModifiers = KeyModifiers::CONTROL;

    #[test]
    fn ctrl_z_undoes_filter_changes_and_redo_restores_them() {
        let mut state = state_of(&["{\"level\":\"INFO\"}", "{\"level\":\"WARN\"}"]);
        state.add_filter(level_filter(&state, "INFO"));
        state.add_filter(level_filter(&state, "WARN"));
        assert_eq!(state.query.filters.len(), 2);
        state.handle_key(KeyCode::Char('z'), CONTROL);
        assert_eq!(state.query.filters.len(), 1);
        state.handle_key(KeyCode::Char('z'), CONTROL);
        assert_eq!(state.query.filters.len(), 0);
        assert_eq!(state.view.len(), 2);
        state.handle_key(KeyCode::Char('y'), CONTROL);
        assert_eq!(state.query.filters.len(), 1);
        state.handle_key(KeyCode::Char('y'), CONTROL);
        assert_eq!(state.query.filters.len(), 2);
        assert!(state.message.as_deref().unwrap_or_default().contains("redone"));
    }

    #[test]
    fn ctrl_z_restores_a_cleared_list_and_a_new_change_ends_the_redo() {
        let mut state = state_of(&["{\"level\":\"INFO\"}", "{\"level\":\"WARN\"}"]);
        state.add_filter(level_filter(&state, "INFO"));
        state.add_filter(level_filter(&state, "WARN"));
        state.clear_filters();
        assert!(state.query.filters.is_empty());
        state.handle_key(KeyCode::Char('z'), CONTROL);
        assert_eq!(state.query.filters.len(), 2);
        state.remove_filter(0);
        state.add_filter(level_filter(&state, "INFO"));
        state.handle_key(KeyCode::Char('y'), CONTROL);
        assert_eq!(state.query.filters.len(), 2);
        assert!(state.message.as_deref().unwrap_or_default().contains("nothing to redo"));
    }

    #[test]
    fn undo_without_history_says_so() {
        let mut state = state_of(&["{\"level\":\"INFO\"}"]);
        state.handle_key(KeyCode::Char('z'), CONTROL);
        assert!(state.message.as_deref().unwrap_or_default().contains("nothing to undo"));
        assert!(!state.quit);
    }

    #[test]
    fn russian_layout_reaches_undo_and_redo() {
        let mut state = state_of(&["{\"level\":\"INFO\"}", "{\"level\":\"WARN\"}"]);
        state.add_filter(level_filter(&state, "INFO"));
        state.handle_key(KeyCode::Char('я'), CONTROL); // the physical z key
        assert!(state.query.filters.is_empty());
        state.handle_key(KeyCode::Char('н'), CONTROL); // the physical y key
        assert_eq!(state.query.filters.len(), 1);
    }

    #[test]
    fn filter_input_parses_and_reports_errors() {
        let mut state = state_of(&["{\"level\":\"WARN\",\"msg\":\"disk full\"}"]);
        state.query.filter_input.set("level=WARN");
        state.submit_filter_input();
        assert_eq!(state.query.filters.len(), 1);
        state.query.filter_input.set("level");
        state.submit_filter_input();
        assert!(state.message.as_deref().unwrap_or_default().contains("filter:"));
        assert_eq!(state.query.filters.len(), 1);
    }

    #[test]
    fn search_updates_the_view() {
        let mut state = state_of(&["{\"message\":\"disk full\"}", "{\"message\":\"ok\"}"]);
        state.set_search_text("disk");
        assert_eq!(state.view.len(), 1);
        state.clear_search();
        assert_eq!(state.view.len(), 2);
    }

    #[test]
    fn sort_cycles_through_three_states() {
        let mut state = state_of(&["{\"a\":2}", "{\"a\":1}"]);
        let column = Column::Field(FieldPath::parse("a").unwrap());
        state.columns.push(column.clone());
        state.cycle_sort(&column);
        assert_eq!(state.view.rows, vec![1, 0]);
        state.cycle_sort(&column);
        assert_eq!(state.view.rows, vec![0, 1]);
        state.cycle_sort(&column);
        assert!(state.query.sorts.is_empty());
    }

    #[test]
    fn column_toggle_adds_and_removes() {
        let mut state = state_of(&["{\"a\":1}"]);
        let path = FieldPath::parse("a").unwrap();
        state.toggle_column(path.clone());
        assert!(state.columns_contain(&path));
        state.toggle_column(path.clone());
        assert!(!state.columns_contain(&path));
    }

    fn columns_of(state: &AppState) -> Vec<String> {
        state.columns.iter().map(|column| column.label()).collect()
    }

    fn with_columns(state: &mut AppState, names: &[&str]) {
        state.columns.clear();
        for name in names {
            state.columns.push(Column::Field(FieldPath::parse(name).expect("path")));
        }
        state.nav.header_cursor = 0;
    }

    #[test]
    fn move_column_swaps_and_follows_cursor() {
        let mut state = state_of(&["{\"a\":1}"]);
        with_columns(&mut state, &["a", "b", "c"]);
        state.move_column(1);
        assert_eq!(columns_of(&state), ["b", "a", "c"]);
        assert_eq!(state.nav.header_cursor, 1);
        state.move_column(-1);
        assert_eq!(columns_of(&state), ["a", "b", "c"]);
        assert_eq!(state.nav.header_cursor, 0);
        state.move_column(-1);
        assert_eq!(columns_of(&state), ["a", "b", "c"]);
    }

    #[test]
    fn header_drag_flow_moves_columns_then_sorts() {
        let mut state = state_of(&["{\"a\":2}", "{\"a\":1}"]);
        with_columns(&mut state, &["a", "b"]);
        state.nav.focus = Focus::Header;
        state.handle_key(KeyCode::Char(' '), KeyModifiers::NONE);
        assert_eq!(state.nav.header_mode, HeaderMode::Drag);
        assert!(state.status.contains("fields (drag)"));
        state.handle_key(KeyCode::Right, KeyModifiers::NONE);
        assert_eq!(columns_of(&state), ["b", "a"]);
        state.handle_key(KeyCode::Enter, KeyModifiers::NONE);
        assert_eq!(state.nav.header_mode, HeaderMode::Navigate);
        assert_eq!(state.query.sorts.len(), 1);
        assert_eq!(state.query.sorts[0].field, FieldPath::parse("a").expect("path"));
    }

    #[test]
    fn header_click_enters_drag_and_second_click_sorts() {
        use ratatui::crossterm::event::MouseButton;
        let mut state = state_of(&["{\"a\":2}", "{\"a\":1}"]);
        with_columns(&mut state, &["a"]);
        state.geometry.areas.rows = Rect::new(0, 2, 40, 10);
        state.geometry.header_cells.push((0, 8, 0));
        let click = MouseEventKind::Down(MouseButton::Left);
        state.handle_mouse(click, 1, 2);
        assert_eq!(state.nav.focus, Focus::Header);
        assert_eq!(state.nav.header_mode, HeaderMode::Drag);
        assert!(state.query.sorts.is_empty());
        state.handle_mouse(click, 1, 2);
        assert_eq!(state.query.sorts.len(), 1);
        assert_eq!(state.nav.header_mode, HeaderMode::Navigate);
    }

    #[test]
    fn chip_click_maps_row_offset_to_filter_index() {
        use ratatui::crossterm::event::MouseButton;
        let mut state = state_of(&["{\"level\":\"INFO\"}", "{\"level\":\"WARN\"}"]);
        let first =
            crate::filter::build_leaf("level", LeafOp::Eq, Some("INFO"), &state.filter_context()).expect("leaf");
        let second =
            crate::filter::build_leaf("level", LeafOp::Neq, Some("INFO"), &state.filter_context()).expect("leaf");
        state.add_filter(first);
        state.add_filter(second);
        state.nav.focus = Focus::Rows;
        state.nav.filter_cursor = 0;
        state.geometry.areas.chips = Rect::new(0, 1, 40, 2);
        state.geometry.chip_cells.push((0, 10, 0, 1));
        state.geometry.chip_cells.push((11, 22, 1, 1));
        let click = MouseEventKind::Down(MouseButton::Left);
        state.handle_mouse(click, 15, 1);
        assert_eq!(state.nav.filter_cursor, 0);
        state.handle_mouse(click, 15, 2);
        assert_eq!(state.nav.filter_cursor, 1);
    }

    #[test]
    fn status_sort_marks_are_structured() {
        let mut state = state_of(&["{\"a\":2}"]);
        let column = Column::Field(FieldPath::parse("a").expect("path"));
        state.columns.push(column.clone());
        state.cycle_sort(&column);
        assert_eq!(state.query.sort_marks, vec![("a".to_owned(), SortDir::Asc)]);
        assert!(!state.status.contains("sort"));
        assert!(state.status.contains("rows 1/1 of 1"));
    }

    #[test]
    fn pick_value_walks_into_operator_step_with_prefill() {
        let mut state = state_of(&["{\"level\":\"INFO\"}", "{\"level\":\"WARN\"}"]);
        state.columns.clear();
        state
            .columns
            .push(Column::Field(FieldPath::parse("level").expect("path")));
        state.nav.focus = Focus::Rows;
        state.handle_key(KeyCode::Char(' '), KeyModifiers::NONE);
        assert_eq!(state.nav.row_mode, RowMode::Pick);
        assert!(state.prompts_text().contains("pick value"));
        state.handle_key(KeyCode::Right, KeyModifiers::NONE);
        assert_eq!(state.nav.pick_cursor, 0);
        state.handle_key(KeyCode::Enter, KeyModifiers::NONE);
        assert_eq!(state.nav.row_mode, RowMode::Normal);
        assert!(
            state
                .overlays
                .wizard
                .as_ref()
                .is_some_and(|w| w.phase == WizardPhase::Operator && w.prefill.is_some())
        );
        assert_eq!(state.nav.focus, Focus::FilterInput);
        state.handle_key(KeyCode::Enter, KeyModifiers::NONE); // Eq: the pattern step is prefilled
        assert_eq!(state.query.filter_input.text(), "INFO");
        state.handle_key(KeyCode::Enter, KeyModifiers::NONE); // commit
        assert_eq!(state.query.filters.len(), 1);
        assert_eq!(state.query.filters[0].to_string(), "level=INFO");
        assert_eq!(state.view.len(), 1);
    }

    #[test]
    fn pick_value_esc_leaves_the_mode() {
        let mut state = state_of(&["{\"a\":1}"]);
        state.columns.clear();
        state.columns.push(Column::Field(FieldPath::parse("a").expect("path")));
        state.nav.focus = Focus::Rows;
        state.handle_key(KeyCode::Char(' '), KeyModifiers::NONE);
        assert_eq!(state.nav.row_mode, RowMode::Pick);
        state.handle_key(KeyCode::Esc, KeyModifiers::NONE);
        assert_eq!(state.nav.row_mode, RowMode::Normal);
    }

    #[test]
    fn prompt_line_tracks_the_active_mode() {
        let mut state = state_of(&["{\"a\":1}"]);
        state.columns.clear();
        state.columns.push(Column::Field(FieldPath::parse("a").expect("path")));
        state.nav.focus = Focus::Rows;
        assert!(state.prompts_text().contains("[Space] select value to filter by"));
        state.nav.focus = Focus::Header;
        assert!(state.prompts_text().contains("[Space] column drag"));
        state.handle_key(KeyCode::Char(' '), KeyModifiers::NONE);
        assert!(state.prompts_text().contains("column drag"));
        state.nav.focus = Focus::Search;
        assert!(state.prompts_text().is_empty());
    }

    #[test]
    fn detail_open_and_apply_filter_from_field() {
        let mut state = state_of(&["{\"level\":\"ERROR\",\"message\":\"boom\"}"]);
        state.set_search_text("");
        state.open_detail();
        assert!(state.overlays.detail.is_some());
        let index = state
            .overlays
            .detail
            .as_ref()
            .map(|d| d.fields.iter().position(|f| f.label == "level").unwrap_or(usize::MAX))
            .unwrap_or(usize::MAX);
        state.overlays.detail.as_mut().expect("detail").cursor = index;
        state.detail_apply_menu_action(MenuAction::FilterEq);
        assert_eq!(state.query.filters.len(), 1);
        assert_eq!(state.query.filters[0].to_string(), "level=ERROR");
    }

    #[test]
    fn picker_lists_and_toggles_fields() {
        let mut state = state_of(&["{\"a\":1,\"b\":2}"]);
        state.open_picker();
        assert_eq!(state.overlays.picker.as_ref().map(|p| p.fields.len()), Some(2));
        state.picker_toggle();
        let picker = state.overlays.picker.as_ref().expect("picker");
        assert!(picker.fields[picker.matches[0]].selected);
        state.close_picker();
        assert_eq!(state.columns.len(), 1);
    }

    #[test]
    fn picker_filter_narrows_matches() {
        let mut state = state_of(&["{\"alpha\":1,\"beta\":2}"]);
        state.open_picker();
        if let Some(picker) = state.overlays.picker.as_mut() {
            picker.input.set("al");
        }
        state.picker_filter();
        assert_eq!(state.overlays.picker.as_ref().map(|p| p.matches.len()), Some(1));
    }

    #[test]
    fn picker_space_toggles_like_enter() {
        let mut state = state_of(&["{\"alpha\":1,\"beta\":2}"]);
        state.open_picker();
        state.handle_key(KeyCode::Char(' '), KeyModifiers::NONE);
        let picker = state.overlays.picker.as_ref().expect("picker stays open");
        assert!(picker.input.text().is_empty());
        assert_eq!(picker.matches.len(), 2);
        assert!(picker.fields[picker.matches[0]].selected);
        state.handle_key(KeyCode::Char(' '), KeyModifiers::NONE);
        assert!(
            !state
                .overlays
                .picker
                .as_ref()
                .expect("picker")
                .fields
                .iter()
                .any(|f| f.selected)
        );
    }

    #[test]
    fn keys_drive_rows_and_focus() {
        let mut state = state_of(&["{\"a\":1}", "{\"a\":2}", "{\"a\":3}"]);
        state.handle_key(KeyCode::Char('j'), KeyModifiers::NONE);
        assert_eq!(state.nav.cursor, 1);
        state.handle_key(KeyCode::Char('G'), KeyModifiers::NONE);
        assert_eq!(state.nav.cursor, 2);
        state.handle_key(KeyCode::Char('g'), KeyModifiers::NONE);
        assert_eq!(state.nav.cursor, 0);
        state.handle_key(KeyCode::Tab, KeyModifiers::NONE);
        assert_eq!(state.nav.focus, Focus::Search);
        state.handle_key(KeyCode::Esc, KeyModifiers::NONE);
        assert_eq!(state.nav.focus, Focus::Rows);
        state.handle_key(KeyCode::Char('?'), KeyModifiers::NONE);
        assert!(state.overlays.help);
        state.handle_key(KeyCode::Esc, KeyModifiers::NONE);
        assert!(!state.overlays.help);
    }

    #[test]
    fn typing_in_search_focus_edits_the_pattern() {
        let mut state = state_of(&["{\"message\":\"disk full\"}"]);
        state.nav.focus = Focus::Search;
        for ch in "disk".chars() {
            state.handle_key(KeyCode::Char(ch), KeyModifiers::NONE);
        }
        assert_eq!(state.query.search.patterns, vec!["disk"]);
        assert_eq!(state.view.len(), 1);
        state.handle_key(KeyCode::Enter, KeyModifiers::NONE);
        assert_eq!(state.nav.focus, Focus::Rows);
    }

    #[test]
    fn render_mode_cycles() {
        let mut state = state_of(&["{\"a\":1}"]);
        state.cycle_render();
        assert_eq!(state.config.render, RenderChoice::Table);
        state.cycle_render();
        assert_eq!(state.config.render, RenderChoice::Kv);
    }

    #[test]
    fn glob_escape_protects_literals() {
        assert_eq!(glob_escape("a*b"), "a\\*b");
        assert_eq!(truncate_probe("plain"), "plain");
    }

    #[test]
    fn detail_json_lines_render() {
        let state = state_of(&["{\"a\":1}"]);
        let record = state.current_record().cloned().expect("record");
        let lines = detail_json_lines(&record);
        assert!(lines.iter().any(|line| line.contains("\"a\"")));
    }

    fn release_notes(latest: &str) -> Vec<ReleaseNote> {
        vec![ReleaseNote {
            version: latest.to_owned(),
            published: "2026-11-20".to_owned(),
            body: "- fixed everything".to_owned(),
        }]
    }

    #[test]
    fn update_line_shows_only_when_available_enabled_and_idle() {
        let mut state = state_of(&["{\"a\":1}"]);
        assert!(!state.update_line_visible());
        state.apply_update_event(UpdateEvent::Checked(
            0,
            UpdateOutcome::Available {
                latest: "9.9.9".to_owned(),
                notes: release_notes("9.9.9"),
            },
        ));
        assert!(state.update_line_visible());
        state.update.updating = true;
        assert!(!state.update_line_visible());
        state.update.updating = false;
        state.config.no_update = true;
        assert!(!state.update_line_visible());
    }

    #[test]
    fn up_to_date_check_leaves_no_line_and_no_prompt() {
        let mut state = state_of(&["{\"a\":1}"]);
        state.apply_update_event(UpdateEvent::Checked(0, UpdateOutcome::UpToDate));
        assert!(!state.update_line_visible());
        assert!(state.update.repo_prompt.is_none());
    }

    #[test]
    fn unreachable_check_opens_the_url_prompt_and_u_reopens_it() {
        let mut state = state_of(&["{\"a\":1}"]);
        state.apply_update_event(UpdateEvent::Checked(
            0,
            UpdateOutcome::Unreachable {
                repo: "https://github.com/chamoretto/silv".to_owned(),
                reason: "offline".to_owned(),
            },
        ));
        assert!(state.update.repo_prompt.is_some());
        assert!(state.message.as_deref().is_some_and(|m| m.contains("offline")));
        state.close_repo_prompt();
        state.handle_key(KeyCode::Char('u'), KeyModifiers::NONE);
        assert!(state.update.repo_prompt.is_some());
    }

    #[test]
    fn url_prompt_submit_saves_repo_and_keeps_focus_on_the_prompt_until_ok() {
        let mut state = state_of(&["{\"a\":1}"]);
        state.config.no_update = true; // suppresses the background check thread
        state.update.repo_prompt = Some(RepoPrompt {
            message: String::new(),
            editor: Editor::with_text("not a url!!"),
        });
        state.handle_key(KeyCode::Enter, KeyModifiers::NONE);
        assert!(state.update.repo_prompt.is_some(), "a bad url keeps the prompt open");
        assert!(
            state
                .message
                .as_deref()
                .is_some_and(|m| m.contains("not a repository url"))
        );
        if let Some(prompt) = state.update.repo_prompt.as_mut() {
            prompt.editor.set("you/fork");
        }
        state.handle_key(KeyCode::Enter, KeyModifiers::NONE);
        assert!(state.update.repo_prompt.is_none());
        assert_eq!(state.update.repo.url(), "https://github.com/you/fork");
        assert_eq!(state.update.repo.author, "you");
        assert_eq!(state.update.repo.repo, "fork");
        assert_eq!(state.update.check_generation, 1, "the submit bumps the generation, no_update spawns no thread");
    }

    #[test]
    fn changelog_opens_from_the_button_rect_and_scrolls() {
        use ratatui::crossterm::event::MouseButton;
        let mut state = state_of(&["{\"a\":1}"]);
        state.apply_update_event(UpdateEvent::Checked(
            0,
            UpdateOutcome::Available {
                latest: "9.9.9".to_owned(),
                notes: release_notes("9.9.9"),
            },
        ));
        state.geometry.areas.update = Rect {
            x: 0,
            y: 20,
            width: 80,
            height: 1,
        };
        state.update.changelog_btn = Rect {
            x: 70,
            y: 20,
            width: 10,
            height: 1,
        };
        state.handle_mouse(MouseEventKind::Down(MouseButton::Left), 72, 20);
        assert!(state.update.changelog.is_some());
        {
            let changelog = state.update.changelog.as_mut().expect("open");
            changelog.content_height = 100;
            changelog.page = 10;
        }
        state.handle_key(KeyCode::Char('j'), KeyModifiers::NONE);
        assert_eq!(state.update.changelog.as_ref().expect("open").scroll, 1);
        state.handle_key(KeyCode::Char('G'), KeyModifiers::NONE);
        assert_eq!(state.update.changelog.as_ref().expect("open").scroll, 90);
        state.handle_key(KeyCode::Char('g'), KeyModifiers::NONE);
        assert_eq!(state.update.changelog.as_ref().expect("open").scroll, 0);
        state.handle_mouse(MouseEventKind::ScrollDown, 40, 10);
        assert_eq!(state.update.changelog.as_ref().expect("open").scroll, 3);
        state.handle_key(KeyCode::Esc, KeyModifiers::NONE);
        assert!(state.update.changelog.is_none());
    }

    #[test]
    fn performed_events_clear_the_update_line() {
        let mut state = state_of(&["{\"a\":1}"]);
        state.apply_update_event(UpdateEvent::Checked(
            0,
            UpdateOutcome::Available {
                latest: "9.9.9".to_owned(),
                notes: release_notes("9.9.9"),
            },
        ));
        state.update.updating = true;
        state.apply_update_event(UpdateEvent::Performed(PerformOutcome::Updated {
            version: "9.9.9".to_owned(),
        }));
        assert!(!state.update.updating);
        assert!(state.update.available.is_none());
        assert!(state.message.as_deref().is_some_and(|m| m.contains("restart")));
    }

    #[test]
    fn start_update_is_inert_when_updates_are_disabled() {
        let mut state = state_of(&["{\"a\":1}"]);
        state.config.no_update = true;
        state.update.updating = false;
        state.start_update();
        assert!(!state.update.updating, "no_update never spawns a worker");
    }

    #[test]
    fn a_cargo_managed_install_is_never_self_updated() {
        let mut state = state_of(&["{\"a\":1}"]);
        state.apply_update_event(UpdateEvent::Checked(
            0,
            UpdateOutcome::Available {
                latest: "9.9.9".to_owned(),
                notes: release_notes("9.9.9"),
            },
        ));
        state.update.cargo_managed = true;
        // The click and Enter on the focused button both route here: they must not self-update,
        // that would replace a binary cargo tracks and desync its install list.
        state.start_update();
        assert!(!state.update.updating, "a cargo install is never self-updated");
        let message = state.message.as_deref().expect("a message explains the refusal");
        assert!(message.contains("cargo install --git"), "the cargo command is given: {message}");
        assert!(message.contains(&state.update.repo.url()), "the command names the repository: {message}");
    }

    #[test]
    fn a_repo_url_submitted_mid_check_supersedes_the_in_flight_one() {
        let mut state = state_of(&["{\"a\":1}"]);
        state.config.no_update = true; // suppresses the check thread
        // A startup check of the default repository is in flight (generation 1).
        state.update.check_generation = 1;
        state.update.repo_prompt = Some(RepoPrompt {
            message: String::new(),
            editor: Editor::with_text("you/fork"),
        });
        state.handle_key(KeyCode::Enter, KeyModifiers::NONE);
        assert_eq!(state.update.repo.url(), "https://github.com/you/fork", "the url is applied at once");
        assert_eq!(state.update.check_generation, 2, "the submit starts a new generation");
        // The old check answers late, about the default repository: it must not drive the ui.
        state.apply_update_event(UpdateEvent::Checked(
            1,
            UpdateOutcome::Unreachable {
                repo: "https://github.com/chamoretto/silv".to_owned(),
                reason: "late".to_owned(),
            },
        ));
        assert!(state.update.repo_prompt.is_none(), "the stale answer must not reopen the prompt");
        state.apply_update_event(UpdateEvent::Checked(
            1,
            UpdateOutcome::Available {
                latest: "9.9.9".to_owned(),
                notes: release_notes("9.9.9"),
            },
        ));
        assert!(
            state.update.available.is_none(),
            "the stale answer must not show the update line for the old repository"
        );
        // The new generation's answer applies.
        state.apply_update_event(UpdateEvent::Checked(
            2,
            UpdateOutcome::Available {
                latest: "9.9.9".to_owned(),
                notes: release_notes("9.9.9"),
            },
        ));
        assert!(state.update.available.is_some(), "the current generation drives the ui");
    }

    #[test]
    fn an_uptodate_check_clears_a_stale_update_line() {
        let mut state = state_of(&["{\"a\":1}"]);
        state.apply_update_event(UpdateEvent::Checked(
            0,
            UpdateOutcome::Available {
                latest: "9.9.9".to_owned(),
                notes: release_notes("9.9.9"),
            },
        ));
        state.nav.focus = Focus::UpdateLine;
        state.update.focus_index = 1;
        assert!(state.update_line_visible());
        // The repository's releases moved (a rollback): the re-check says up to date.
        state.apply_update_event(UpdateEvent::Checked(0, UpdateOutcome::UpToDate));
        assert!(state.update.available.is_none(), "the stale line is cleared");
        assert!(!state.update_line_visible());
        assert_eq!(state.nav.focus, Focus::Rows, "a focus parked on a hidden line falls back to the rows");
    }

    #[test]
    fn quitting_during_an_update_needs_a_second_press() {
        let mut state = state_of(&["{\"a\":1}"]);
        state.update.updating = true;
        state.quit();
        assert!(!state.quit, "the first press is refused while the self-replace runs");
        assert!(state.message.as_deref().is_some_and(|m| m.contains("press q again")));
        state.quit();
        assert!(state.quit, "the second press quits");
    }

    #[test]
    fn a_finished_update_clears_the_quit_guard() {
        let mut state = state_of(&["{\"a\":1}"]);
        state.update.updating = true;
        state.quit();
        assert!(!state.quit);
        state.apply_update_event(UpdateEvent::Performed(PerformOutcome::Updated {
            version: "9.9.9".to_owned(),
        }));
        state.quit();
        assert!(state.quit, "after the update finished a single press quits");
    }

    #[test]
    fn a_submitted_repo_url_returns_a_parked_update_line_focus_to_the_rows() {
        let mut state = state_of(&["{\"a\":1}"]);
        state.apply_update_event(UpdateEvent::Checked(
            0,
            UpdateOutcome::Available {
                latest: "9.9.9".to_owned(),
                notes: release_notes("9.9.9"),
            },
        ));
        state.nav.focus = Focus::UpdateLine;
        state.open_repo_prompt();
        if let Some(prompt) = state.update.repo_prompt.as_mut() {
            // A refused localhost port: the spawned check's Unreachable answer lands in
            // the channel unpolled, so it cannot race the assertions below.
            prompt.editor.set("http://127.0.0.1:9/you/fork");
        }
        state.submit_repo_url();
        assert!(state.update.repo_prompt.is_none());
        assert_eq!(state.nav.focus, Focus::Rows, "the hidden line drops its focus at once");
    }

    #[test]
    fn a_modal_click_does_not_press_the_update_button_behind_it() {
        // The record detail owns the screen: a click on the update line row behind it
        // must not press the update button.
        use ratatui::crossterm::event::MouseButton;
        let mut state = state_of(&["{\"a\":1}"]);
        state.apply_update_event(UpdateEvent::Checked(
            0,
            UpdateOutcome::Available {
                latest: "9.9.9".to_owned(),
                notes: release_notes("9.9.9"),
            },
        ));
        state.geometry.areas.update = Rect {
            x: 0,
            y: 20,
            width: 80,
            height: 1,
        };
        state.update.btn_rect = Rect {
            x: 2,
            y: 20,
            width: 40,
            height: 1,
        };
        state.open_detail();
        assert!(state.overlays.detail.is_some(), "the detail overlay is open");
        state.handle_mouse(MouseEventKind::Down(MouseButton::Left), 5, 20);
        assert!(!state.update.updating, "a click behind the overlay is inert");
        assert_eq!(state.nav.focus, Focus::Rows, "the update line was not pressed or focused");
    }

    #[test]
    fn wheel_is_inert_while_the_repository_popup_is_open() {
        let mut state = state_of(&["{\"a\":1}", "{\"a\":2}", "{\"a\":3}", "{\"a\":4}", "{\"a\":5}"]);
        state.open_repo_prompt();
        state.wheel(1);
        assert_eq!(state.nav.cursor, 0, "the rows do not scroll behind the pop-up");
        state.close_repo_prompt();
        state.wheel(1);
        assert_eq!(state.nav.cursor, 3, "three lines per notch with no overlay");
    }

    #[test]
    fn the_column_picker_stays_closed_on_the_update_line_focus() {
        // Global `c` opens the value picker from rows/header focus; on the update line
        // it must not (the picker has no meaning there, `u` is the line's key).
        let mut state = state_of(&["{\"a\":1}"]);
        state.apply_update_event(UpdateEvent::Checked(
            0,
            UpdateOutcome::Available {
                latest: "9.9.9".to_owned(),
                notes: release_notes("9.9.9"),
            },
        ));
        state.nav.focus = Focus::UpdateLine;
        state.handle_key(KeyCode::Char('c'), KeyModifiers::NONE);
        assert!(state.overlays.picker.is_none(), "c on the update line never opens the value picker");
        assert!(state.update.repo_prompt.is_none(), "c on the update line is inert, u opens the pop-up");
        state.handle_key(KeyCode::Char('u'), KeyModifiers::NONE);
        assert!(state.update.repo_prompt.is_some(), "u on the update line opens the repository pop-up");
    }

    #[test]
    fn the_update_line_is_tab_focusable_and_enter_presses_the_changelog() {
        let mut state = state_of(&["{\"a\":1}"]);
        state.apply_update_event(UpdateEvent::Checked(
            0,
            UpdateOutcome::Available {
                latest: "9.9.9".to_owned(),
                notes: release_notes("9.9.9"),
            },
        ));
        assert_eq!(state.nav.focus, Focus::Rows);
        state.handle_key(KeyCode::Tab, KeyModifiers::NONE);
        assert_eq!(state.nav.focus, Focus::UpdateLine, "Tab reaches the update line from the rows");
        state.handle_key(KeyCode::Left, KeyModifiers::NONE);
        assert_eq!(state.update.focus_index, 0, "Left selects the update button");
        state.handle_key(KeyCode::Right, KeyModifiers::NONE);
        assert_eq!(state.update.focus_index, 1, "Right selects the changelog button");
        state.handle_key(KeyCode::Enter, KeyModifiers::NONE);
        assert!(state.update.changelog.is_some(), "Enter pressed the focused changelog button");
        assert!(!state.update.updating, "Enter on the changelog never starts an update");
        state.handle_key(KeyCode::Esc, KeyModifiers::NONE);
        assert!(state.update.changelog.is_none());
        state.handle_key(KeyCode::Esc, KeyModifiers::NONE);
        assert_eq!(state.nav.focus, Focus::Rows, "Esc leaves the update line");
        state.handle_key(KeyCode::BackTab, KeyModifiers::NONE);
        assert_ne!(state.nav.focus, Focus::UpdateLine, "BackTab cycles from the rows, not to the line");
    }

    #[test]
    fn a_focus_on_the_update_line_falls_back_when_the_line_hides() {
        let mut state = state_of(&["{\"a\":1}"]);
        state.apply_update_event(UpdateEvent::Checked(
            0,
            UpdateOutcome::Available {
                latest: "9.9.9".to_owned(),
                notes: release_notes("9.9.9"),
            },
        ));
        state.nav.focus = Focus::UpdateLine;
        state.update.updating = true; // the line hides while updating
        state.apply_update_event(UpdateEvent::Performed(PerformOutcome::UpToDate));
        assert_eq!(state.nav.focus, Focus::Rows, "the focus leaves a line that is no longer drawn");
    }

    #[test]
    fn the_tab_cycle_skips_the_update_line_while_it_is_hidden() {
        let mut state = state_of(&["{\"a\":1}"]);
        for _ in 0..8 {
            state.handle_key(KeyCode::Tab, KeyModifiers::NONE);
            assert_ne!(state.nav.focus, Focus::UpdateLine, "no update line, no stop at it");
        }
    }

    #[test]
    fn a_permission_failure_suggests_sudo_not_a_repo_url() {
        let mut state = state_of(&["{\"a\":1}"]);
        state.apply_update_event(UpdateEvent::Checked(
            0,
            UpdateOutcome::Available {
                latest: "9.9.9".to_owned(),
                notes: release_notes("9.9.9"),
            },
        ));
        state.update.updating = true;
        state.apply_update_event(UpdateEvent::Performed(PerformOutcome::Failed {
            reason: "Permission Denied: read-only directory".to_owned(),
            suggest_repo_url: false,
            suggest_sudo: true,
        }));
        assert!(
            state.message.as_deref().is_some_and(|m| m.contains("sudo silv update")),
            "the status bar names the sudo remedy: {:?}",
            state.message
        );
        assert!(state.update.repo_prompt.is_none(), "the repository is not the problem here");
    }

    #[test]
    fn the_repo_prompt_is_inert_to_clicks_and_closes_on_esc() {
        let mut state = state_of(&["{\"a\":1}"]);
        state.open_repo_prompt();
        assert!(state.update.repo_prompt.is_some());
        state.handle_mouse(MouseEventKind::Down(ratatui::crossterm::event::MouseButton::Left), 40, 12);
        assert!(state.update.repo_prompt.is_some(), "a click outside must not close the text prompt");
        state.handle_key(KeyCode::Esc, KeyModifiers::NONE);
        assert!(state.update.repo_prompt.is_none(), "Esc closes it, like every other window");
    }
}
