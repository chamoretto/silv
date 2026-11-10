//! Application state, view rebuilding and all user actions.

use ratatui::crossterm::event::KeyCode;
use ratatui::crossterm::event::KeyModifiers;
use ratatui::crossterm::event::MouseEventKind;
use ratatui::layout::Rect;

use crate::app::input::Editor;
use crate::cli::Config;
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

/// Live store, kept in RAM by design.
pub type Store = LogStore;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Focus {
    Search,
    FilterChips,
    FilterInput,
    Header,
    Rows,
}

impl Focus {
    pub const ORDER: [Self; 5] = [
        Self::Search,
        Self::FilterChips,
        Self::FilterInput,
        Self::Header,
        Self::Rows,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Search => "search",
            Self::FilterChips => "filters",
            Self::FilterInput => "filter",
            Self::Header => "fields",
            Self::Rows => "rows",
        }
    }
}

/// Screen areas recorded during drawing, used for mouse hit testing.
#[derive(Clone, Copy, Debug, Default)]
pub struct Areas {
    pub search: Rect,
    pub chips: Rect,
    pub filter: Rect,
    pub rows: Rect,
    pub status: Rect,
}

/// One field shown in the record detail.
#[derive(Clone, Debug)]
pub struct DetailField {
    pub label: String,
    pub value: String,
    pub style: Sg,
    pub path: Option<FieldPath>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MenuAction {
    ToggleColumn,
    FilterEq,
    FilterNeq,
    FilterContains,
    Exists,
    NotExists,
}

impl MenuAction {
    pub const ALL: [Self; 6] = [
        Self::ToggleColumn,
        Self::FilterEq,
        Self::FilterNeq,
        Self::FilterContains,
        Self::Exists,
        Self::NotExists,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Self::ToggleColumn => "add or remove column",
            Self::FilterEq => "filter: field = value",
            Self::FilterNeq => "filter: field != value",
            Self::FilterContains => "filter: field contains value",
            Self::Exists => "filter: field exists",
            Self::NotExists => "filter: field missing",
        }
    }
}

#[derive(Clone, Debug)]
pub struct DetailState {
    pub row: usize,
    pub fields: Vec<DetailField>,
    pub cursor: usize,
    pub scroll: usize,
    pub json: bool,
    pub json_text: Vec<String>,
    pub menu: Option<usize>,
}

#[derive(Clone, Debug)]
pub struct PickerState {
    pub fields: Vec<PickField>,
    pub matches: Vec<usize>,
    pub cursor: usize,
    pub scroll: usize,
    pub input: Editor,
}

#[derive(Clone, Debug)]
pub struct PickField {
    pub path: FieldPath,
    pub label: String,
    pub count: u64,
    pub types: String,
    pub selected: bool,
}

/// The whole interactive session state.
pub struct AppState {
    pub store: Store,
    pub config: Config,
    pub view: View,
    pub columns: Vec<Column>,
    pub sorts: Vec<SortSpec>,
    pub filters: Vec<Predicate>,
    pub search: SearchSpec,
    pub search_input: Editor,
    pub filter_input: Editor,
    pub focus: Focus,
    pub cursor: usize,
    pub header_cursor: usize,
    pub filter_cursor: usize,
    pub column_scroll: usize,
    pub rows_scroll: usize,
    pub detail: Option<DetailState>,
    pub picker: Option<PickerState>,
    pub help: bool,
    pub message: Option<String>,
    pub status: String,
    pub areas: Areas,
    pub header_cells: Vec<(u16, u16, usize)>,
    pub chip_cells: Vec<(u16, u16, usize)>,
    pub regexes: Vec<regex::Regex>,
    pub dirty: bool,
    pub quit: bool,
}

/// Live rebuilds are skipped above this many records; Enter forces one.
const LIVE_REBUILD_LIMIT: usize = 30_000;

impl AppState {
    pub fn new(store: Store, config: Config) -> Self {
        let columns = resolve_columns(&config, &store);
        let sorts = config.sorts.clone();
        let filters = config.filters.clone();
        let search = config.search.clone();
        let search_input = Editor::with_text(&search.text());
        let mut state = Self {
            store,
            view: View::default(),
            columns,
            sorts,
            filters,
            search,
            search_input,
            filter_input: Editor::new(),
            focus: Focus::Rows,
            cursor: 0,
            header_cursor: 0,
            filter_cursor: 0,
            column_scroll: 0,
            rows_scroll: 0,
            detail: None,
            picker: None,
            help: false,
            message: None,
            status: String::new(),
            areas: Areas::default(),
            header_cells: Vec::new(),
            chip_cells: Vec::new(),
            regexes: Vec::new(),
            dirty: true,
            quit: false,
            config,
        };
        state.rebuild();
        state
    }

    /// Recomputes the visible rows after any filter, search or sort change.
    pub fn rebuild(&mut self) {
        let search_predicate = if self.search.is_empty() {
            None
        } else {
            match self.search.compile() {
                Ok(predicate) => Some(predicate),
                Err(error) => {
                    self.message = Some(format!("search: {}", error));
                    None
                },
            }
        };
        self.view = build_view(
            &self.store,
            &self.filters,
            search_predicate.as_ref(),
            &self.sorts,
            self.config.offset,
            self.config.limit,
            self.config.tail,
        );
        self.regexes = self.search.highlight_regexes();
        self.cursor = self.cursor.min(self.view.len().saturating_sub(1));
        self.rows_scroll = self.rows_scroll.min(self.view.len());
        self.header_cursor = self.header_cursor.min(self.columns.len().saturating_sub(1));
        self.filter_cursor = self.filter_cursor.min(self.filters.len().saturating_sub(1));
        self.update_status();
    }

    fn update_status(&mut self) {
        let shown = self.view.len();
        let total = self.store.len();
        let marks = self
            .sorts
            .iter()
            .map(|spec| format!("{}{}", spec.field, spec.dir.marker()))
            .collect::<Vec<_>>()
            .join(",");
        self.status = format!(
            "rows {shown}/{} of {total} | fields {} | focus {} | {}{}",
            self.view.matched,
            self.columns.len(),
            self.focus.label(),
            self.config.render,
            if marks.is_empty() {
                String::new()
            } else {
                format!(" | sort {marks}")
            }
        );
    }

    pub const fn record_count(&self) -> usize {
        self.store.len()
    }

    pub fn record_at(&self, position: usize) -> Option<&crate::model::record::LogRecord> {
        self.view.record(&self.store, position)
    }

    pub fn current_record(&self) -> Option<&crate::model::record::LogRecord> {
        self.record_at(self.cursor)
    }

    pub fn source_label(&self, record: &crate::model::record::LogRecord) -> &str {
        self.store
            .sources()
            .get(usize::try_from(record.source).unwrap_or(usize::MAX))
            .map(|source| source.label.as_str())
            .unwrap_or("<unknown>")
    }

    pub fn set_search_text(&mut self, text: &str) {
        self.search.patterns = text.split_whitespace().map(str::to_owned).collect();
        self.rebuild();
    }

    pub fn commit_search(&mut self) {
        let text = self.search_input.commit();
        self.search.patterns = text.split_whitespace().map(str::to_owned).collect();
        self.focus = Focus::Rows;
        self.rebuild();
    }

    pub fn add_filter(&mut self, predicate: Predicate) {
        self.filters.push(predicate);
        self.filter_cursor = self.filters.len() - 1;
        self.focus = Focus::FilterChips;
        self.rebuild();
    }

    /// Parses the filter input line, reporting the reason on failure.
    pub fn submit_filter_input(&mut self) {
        let text = self.filter_input.text();
        if text.trim().is_empty() {
            self.focus = Focus::Rows;
            return;
        }
        let ctx = self.filter_context();
        match parse_expression(&text, &ctx) {
            Ok(predicate) => {
                self.filter_input.commit();
                self.filter_input.clear();
                self.add_filter(predicate);
                self.message = None;
            },
            Err(error) => {
                self.message = Some(format!("filter: {}", error));
            },
        }
    }

    pub fn add_filter_from_op(&mut self, field: FieldPath, op: LeafOp, value: Option<String>) {
        let ctx = self.filter_context();
        let label = field.to_string();
        match build_leaf(&label, op, value.as_deref(), &ctx) {
            Ok(predicate) => {
                self.add_filter(predicate);
                self.message = Some(format!("filter added: {label}{}", op.token()));
            },
            Err(error) => {
                self.message = Some(format!("filter: {error}"));
            },
        }
    }

    pub fn remove_filter(&mut self, index: usize) {
        if index < self.filters.len() {
            self.filters.remove(index);
            self.filter_cursor = self.filter_cursor.min(self.filters.len().saturating_sub(1));
            self.rebuild();
        }
    }

    pub fn clear_filters(&mut self) {
        self.filters.clear();
        self.filter_cursor = 0;
        self.rebuild();
    }

    pub fn clear_search(&mut self) {
        self.search.patterns.clear();
        self.search_input.clear();
        self.rebuild();
    }

    pub fn filter_context(&self) -> FilterContext {
        FilterContext {
            case_insensitive: self.search.case_insensitive,
            glob_anywhere: self.config.glob_anywhere,
            regex: self.config.filter_regex,
        }
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
        self.header_cursor = self.header_cursor.min(self.columns.len().saturating_sub(1));
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
            .sorts
            .iter()
            .position(|spec| Column::from_path(&spec.field) == *column);
        match position {
            Some(index) => match self.sorts[index].dir {
                SortDir::Asc => self.sorts[index].dir = SortDir::Desc,
                SortDir::Desc => {
                    self.sorts.remove(index);
                    self.message = Some(format!("sort cleared: {label}"));
                },
            },
            None => {
                self.sorts.push(SortSpec {
                    field: path,
                    dir: SortDir::Asc,
                });
                self.message = Some(format!("sorted by: {label}"));
            },
        }
        self.rebuild();
    }

    pub fn sort_of(&self, column: &Column) -> Option<SortDir> {
        self.sorts
            .iter()
            .find(|spec| Column::from_path(&spec.field) == *column)
            .map(|spec| spec.dir)
    }

    pub fn move_cursor(&mut self, delta: isize) {
        if self.view.is_empty() {
            self.cursor = 0;
            return;
        }
        let len = isize::try_from(self.view.len()).unwrap_or(isize::MAX);
        let current = isize::try_from(self.cursor).unwrap_or(0);
        let next = (current + delta).clamp(0, len - 1);
        self.cursor = usize::try_from(next).unwrap_or(0);
    }

    pub fn go_first(&mut self) {
        self.cursor = 0;
    }

    pub fn go_last(&mut self) {
        self.cursor = self.view.len().saturating_sub(1);
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
        self.message = Some(format!("render: {:?}", self.config.render));
        self.dirty = true;
    }

    pub fn toggle_case(&mut self) {
        self.search.case_insensitive = !self.search.case_insensitive;
        self.rebuild();
        self.message = Some(if self.search.case_insensitive {
            "search: case insensitive".to_owned()
        } else {
            "search: case sensitive".to_owned()
        });
    }

    pub fn toggle_regex(&mut self) {
        self.search.regex = !self.search.regex;
        self.rebuild();
        self.message = Some(format!("search regex: {}", self.search.regex));
    }

    pub fn toggle_scope(&mut self) {
        self.search.scope = match self.search.scope {
            SearchScope::All => SearchScope::Fields(vec![FieldPath::from_key("message")]),
            SearchScope::Fields(_) => SearchScope::All,
        };
        self.rebuild();
        self.message = Some(match self.search.scope {
            SearchScope::All => "search scope: all fields".to_owned(),
            SearchScope::Fields(_) => "search scope: message only".to_owned(),
        });
    }

    pub fn focused_column(&self) -> Option<&Column> {
        self.columns.get(self.header_cursor)
    }

    pub fn cycle_focus(&mut self, direction: isize) {
        let len = isize::try_from(Focus::ORDER.len()).unwrap_or(isize::MAX);
        let current = Focus::ORDER.iter().position(|f| *f == self.focus).unwrap_or(0);
        let current = isize::try_from(current).unwrap_or(0);
        let next = (current + direction).rem_euclid(len);
        self.focus = Focus::ORDER[usize::try_from(next).unwrap_or(0)];
        self.update_status();
    }

    /// Opens the record detail overlay for the focused row.
    pub fn open_detail(&mut self) {
        let Some(record) = self.current_record().cloned() else {
            return;
        };
        let fields = detail_fields(&record, &self.config);
        let json_text = detail_json_lines(&record);
        self.detail = Some(DetailState {
            row: self.cursor,
            fields,
            cursor: 0,
            scroll: 0,
            json: false,
            json_text,
            menu: None,
        });
        self.dirty = true;
    }

    pub fn close_detail(&mut self) {
        self.detail = None;
        self.dirty = true;
    }

    pub fn detail_move(&mut self, delta: isize) {
        let Some(detail) = self.detail.as_mut() else {
            return;
        };
        if let Some(menu) = detail.menu.as_mut() {
            let len = MenuAction::ALL.len();
            let current = isize::try_from(*menu).unwrap_or(0);
            let next = (current + delta).clamp(0, isize::try_from(len - 1).unwrap_or(0));
            *menu = usize::try_from(next).unwrap_or(0);
            return;
        }
        if detail.json {
            return;
        }
        let len = detail.fields.len();
        if len == 0 {
            return;
        }
        let current = isize::try_from(detail.cursor).unwrap_or(0);
        let next = (current + delta).clamp(0, isize::try_from(len - 1).unwrap_or(0));
        detail.cursor = usize::try_from(next).unwrap_or(0);
    }

    pub fn detail_toggle_json(&mut self) {
        if let Some(detail) = self.detail.as_mut() {
            detail.json = !detail.json;
            self.dirty = true;
        }
    }

    pub fn detail_open_menu(&mut self) {
        if let Some(detail) = self.detail.as_mut()
            && !detail.json
        {
            detail.menu = Some(0);
            self.dirty = true;
        }
    }

    pub fn detail_selected_field(&self) -> Option<&DetailField> {
        let detail = self.detail.as_ref()?;
        detail.fields.get(detail.cursor)
    }

    /// Applies the focused menu action to the focused field.
    pub fn detail_apply_menu(&mut self) {
        let (Some(detail), Some(menu)) = (self.detail.clone(), self.detail.as_ref().and_then(|d| d.menu)) else {
            return;
        };
        let action = MenuAction::ALL[menu.min(MenuAction::ALL.len() - 1)];
        let Some(field) = detail.fields.get(detail.cursor) else {
            return;
        };
        let label = field.label.clone();
        let value = field.value.clone();
        let path = field.path.clone().or_else(|| FieldPath::parse(&label).ok());
        let Some(path) = path else {
            self.message = Some(format!("cannot use `{label}` as a field path"));
            return;
        };
        if let Some(detail) = self.detail.as_mut() {
            detail.menu = None;
        }
        match action {
            MenuAction::ToggleColumn => self.toggle_column(path),
            MenuAction::FilterEq => {
                let probe = truncate_probe(&value);
                self.add_filter_from_op(path, LeafOp::Eq, Some(probe));
            },
            MenuAction::FilterNeq => {
                let probe = truncate_probe(&value);
                self.add_filter_from_op(path, LeafOp::Neq, Some(probe));
            },
            MenuAction::FilterContains => {
                let probe = truncate_probe(&value);
                self.add_filter_from_op(path, LeafOp::Contains, Some(probe));
            },
            MenuAction::Exists => self.add_filter_from_op(path, LeafOp::Exists, None),
            MenuAction::NotExists => self.add_filter_from_op(path, LeafOp::NotExists, None),
        }
        self.dirty = true;
    }

    /// Opens the field picker overlay.
    pub fn open_picker(&mut self) {
        let fields: Vec<PickField> = self
            .store
            .fields_by_frequency()
            .into_iter()
            .map(|info| PickField {
                path: info.path.clone(),
                label: info.label.clone(),
                count: info.count,
                types: info.types.to_string(),
                selected: self.columns_contain(&info.path),
            })
            .collect();
        let matches: Vec<usize> = (0..u32::try_from(fields.len()).unwrap_or(u32::MAX))
            .filter_map(|i| usize::try_from(i).ok())
            .collect();
        self.picker = Some(PickerState {
            fields,
            matches,
            cursor: 0,
            scroll: 0,
            input: Editor::new(),
        });
        self.dirty = true;
    }

    pub fn close_picker(&mut self) {
        self.picker = None;
        self.dirty = true;
    }

    pub fn picker_filter(&mut self) {
        let needle = self
            .picker
            .as_ref()
            .map(|picker| picker.input.text().to_lowercase())
            .unwrap_or_default();
        if let Some(picker) = self.picker.as_mut() {
            picker.matches = (0..picker.fields.len())
                .filter(|index| picker.fields[*index].label.to_lowercase().contains(&needle))
                .collect();
            picker.cursor = picker.cursor.min(picker.matches.len().saturating_sub(1));
            picker.scroll = 0;
        }
        self.dirty = true;
    }

    pub fn picker_toggle(&mut self) {
        let index = match self.picker.as_ref().and_then(|p| p.matches.get(p.cursor)) {
            Some(index) => *index,
            None => return,
        };
        let path = match self
            .picker
            .as_ref()
            .and_then(|p| p.fields.get(index))
            .map(|field| field.path.clone())
        {
            Some(path) => path,
            None => return,
        };
        self.toggle_column(path.clone());
        let contains = self.columns_contain(&path);
        if let Some(picker) = self.picker.as_mut()
            && let Some(field) = picker.fields.get_mut(index)
        {
            field.selected = contains;
        }
    }

    pub fn picker_move(&mut self, delta: isize) {
        let Some(picker) = self.picker.as_mut() else {
            return;
        };
        if picker.matches.is_empty() {
            return;
        }
        let len = isize::try_from(picker.matches.len()).unwrap_or(isize::MAX);
        let current = isize::try_from(picker.cursor).unwrap_or(0);
        picker.cursor = usize::try_from((current + delta).clamp(0, len - 1)).unwrap_or(0);
        self.dirty = true;
    }

    pub fn quit(&mut self) {
        self.quit = true;
    }

    /// Handles a key press for the current focus and overlays.
    pub fn handle_key(&mut self, code: KeyCode, modifiers: KeyModifiers) {
        if modifiers.contains(KeyModifiers::CONTROL) && matches!(code, KeyCode::Char('c') | KeyCode::Char('d')) {
            self.quit();
            return;
        }
        if self.help {
            match code {
                KeyCode::Esc | KeyCode::Char('?') | KeyCode::Char('q') => self.help = false,
                _ => {},
            }
            self.dirty = true;
            return;
        }
        if self.picker.is_some() {
            self.handle_picker_key(code, modifiers);
            self.dirty = true;
            return;
        }
        if self.detail.is_some() {
            self.handle_detail_key(code);
            self.dirty = true;
            return;
        }
        match code {
            KeyCode::Char('?') => {
                self.help = true;
                return;
            },
            KeyCode::Char('c') if !self.focus_is_text() => {
                self.open_picker();
                return;
            },
            _ => {},
        }
        match self.focus {
            Focus::Search => self.handle_search_key(code, modifiers),
            Focus::FilterInput => self.handle_filter_key(code, modifiers),
            Focus::FilterChips => self.handle_chips_key(code),
            Focus::Header => self.handle_header_key(code),
            Focus::Rows => self.handle_rows_key(code),
        }
        self.dirty = true;
    }

    const fn focus_is_text(&self) -> bool {
        matches!(self.focus, Focus::Search | Focus::FilterInput)
    }

    fn handle_search_key(&mut self, code: KeyCode, _modifiers: KeyModifiers) {
        match code {
            KeyCode::Char(ch) => {
                self.search_input.insert(ch);
                self.sync_search_from_input();
            },
            KeyCode::Backspace => {
                self.search_input.backspace();
                self.sync_search_from_input();
            },
            KeyCode::Delete => {
                self.search_input.delete();
                self.sync_search_from_input();
            },
            KeyCode::Left => self.search_input.left(),
            KeyCode::Right => self.search_input.right(),
            KeyCode::Home => self.search_input.home(),
            KeyCode::End => self.search_input.end(),
            KeyCode::Up => {
                self.search_input.history_previous();
                self.sync_search_from_input();
            },
            KeyCode::Down => {
                self.search_input.history_next();
                self.sync_search_from_input();
            },
            KeyCode::Enter => self.commit_search(),
            KeyCode::Esc => {
                if !self.search_input.is_empty() {
                    self.search_input.clear();
                    self.clear_search();
                }
                self.focus = Focus::Rows;
            },
            KeyCode::Tab => self.cycle_focus(1),
            KeyCode::BackTab => self.cycle_focus(-1),
            _ => {},
        }
    }

    fn sync_search_from_input(&mut self) {
        self.search.patterns = self.search_input.text().split_whitespace().map(str::to_owned).collect();
        if self.store.len() <= LIVE_REBUILD_LIMIT {
            self.rebuild();
        } else {
            self.message = Some("press Enter to apply the search".to_owned());
        }
    }

    fn handle_filter_key(&mut self, code: KeyCode, _modifiers: KeyModifiers) {
        match code {
            KeyCode::Char(ch) => self.filter_input.insert(ch),
            KeyCode::Backspace => {
                if !self.filter_input.backspace() && self.focus == Focus::FilterInput {
                    self.focus = Focus::FilterChips;
                }
            },
            KeyCode::Delete => self.filter_input.delete(),
            KeyCode::Left => self.filter_input.left(),
            KeyCode::Right => self.filter_input.right(),
            KeyCode::Home => self.filter_input.home(),
            KeyCode::End => self.filter_input.end(),
            KeyCode::Up => {
                self.filter_input.history_previous();
            },
            KeyCode::Down => {
                self.filter_input.history_next();
            },
            KeyCode::Enter => self.submit_filter_input(),
            KeyCode::Esc => {
                self.filter_input.clear();
                self.focus = Focus::Rows;
            },
            KeyCode::Tab => self.cycle_focus(1),
            KeyCode::BackTab => self.cycle_focus(-1),
            _ => {},
        }
    }

    fn handle_chips_key(&mut self, code: KeyCode) {
        match code {
            KeyCode::Left | KeyCode::Char('h') => self.filter_cursor = self.filter_cursor.saturating_sub(1),
            KeyCode::Right | KeyCode::Char('l') => {
                self.filter_cursor = (self.filter_cursor + 1).min(self.filters.len().saturating_sub(1));
            },
            KeyCode::Char('d') | KeyCode::Delete | KeyCode::Backspace => self.remove_filter(self.filter_cursor),
            KeyCode::Enter => {
                if let Some(predicate) = self.filters.get(self.filter_cursor) {
                    let text = predicate.to_string();
                    self.filter_input.set(&text);
                    self.focus = Focus::FilterInput;
                }
            },
            KeyCode::Char('i') | KeyCode::Char('a') => self.focus = Focus::FilterInput,
            KeyCode::Char('x') => self.clear_filters(),
            KeyCode::Esc => self.focus = Focus::Rows,
            KeyCode::Tab => self.cycle_focus(1),
            KeyCode::BackTab => self.cycle_focus(-1),
            _ => {},
        }
    }

    fn handle_header_key(&mut self, code: KeyCode) {
        match code {
            KeyCode::Left | KeyCode::Char('h') => self.header_cursor = self.header_cursor.saturating_sub(1),
            KeyCode::Right | KeyCode::Char('l') => {
                self.header_cursor = (self.header_cursor + 1).min(self.columns.len().saturating_sub(1));
            },
            KeyCode::Enter | KeyCode::Char('s') => {
                if let Some(column) = self.focused_column().cloned() {
                    self.cycle_sort(&column);
                }
            },
            KeyCode::Char('d') => {
                if let Some(column) = self.focused_column().cloned() {
                    self.columns.remove(self.header_cursor);
                    self.message = Some(format!("column removed: {}", column.label()));
                    self.header_cursor = self.header_cursor.min(self.columns.len().saturating_sub(1));
                    self.update_status();
                }
            },
            KeyCode::Esc => self.focus = Focus::Rows,
            KeyCode::Tab => self.cycle_focus(1),
            KeyCode::BackTab => self.cycle_focus(-1),
            KeyCode::Down | KeyCode::Char('j') => self.focus = Focus::Rows,
            _ => {},
        }
    }

    fn handle_rows_key(&mut self, code: KeyCode) {
        match code {
            KeyCode::Char('q') => self.quit(),
            KeyCode::Char('j') | KeyCode::Down => self.move_cursor(1),
            KeyCode::Char('k') | KeyCode::Up => self.move_cursor(-1),
            KeyCode::PageDown => self.page(1, self.page_height()),
            KeyCode::PageUp => self.page(-1, self.page_height()),
            KeyCode::Char('f') => self.page(1, self.page_height()),
            KeyCode::Char('b') => self.page(-1, self.page_height()),
            KeyCode::Char('g') => self.go_first(),
            KeyCode::Char('G') => self.go_last(),
            KeyCode::Enter => self.open_detail(),
            KeyCode::Char('s') => {
                if let Some(column) = self.focused_column().cloned() {
                    self.cycle_sort(&column);
                }
            },
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
            KeyCode::Char('d') => self.remove_filter(self.filter_cursor),
            KeyCode::Char('X') => self.clear_filters(),
            KeyCode::Left => self.column_scroll = self.column_scroll.saturating_sub(1),
            KeyCode::Right => self.column_scroll = (self.column_scroll + 1).min(self.columns.len().saturating_sub(1)),
            KeyCode::Esc => {
                if !self.search.patterns.is_empty() {
                    self.clear_search();
                } else if !self.filters.is_empty() {
                    self.focus = Focus::FilterChips;
                }
            },
            KeyCode::Tab => self.cycle_focus(1),
            KeyCode::BackTab => self.cycle_focus(-1),
            KeyCode::Char('/') => self.focus = Focus::Search,
            KeyCode::Char('F') => self.focus = Focus::FilterInput,
            _ => {},
        }
    }

    fn handle_detail_key(&mut self, code: KeyCode) {
        match code {
            KeyCode::Esc => self.close_detail(),
            KeyCode::Char('q') => self.close_detail(),
            KeyCode::Char('j') | KeyCode::Down => self.detail_move(1),
            KeyCode::Char('k') | KeyCode::Up => self.detail_move(-1),
            KeyCode::PageDown => self.detail_move(isize::try_from(self.page_height()).unwrap_or(1)),
            KeyCode::PageUp => self.detail_move(-isize::try_from(self.page_height()).unwrap_or(1)),
            KeyCode::Enter => self.detail_open_menu(),
            KeyCode::Char('a') => self.detail_apply_menu_action(MenuAction::ToggleColumn),
            KeyCode::Char('f') => self.detail_apply_menu_action(MenuAction::FilterEq),
            KeyCode::Char('F') => self.detail_apply_menu_action(MenuAction::FilterNeq),
            KeyCode::Char('~') => self.detail_apply_menu_action(MenuAction::FilterContains),
            KeyCode::Char('e') => self.detail_apply_menu_action(MenuAction::Exists),
            KeyCode::Char('E') => self.detail_apply_menu_action(MenuAction::NotExists),
            KeyCode::Char('v') => self.detail_toggle_json(),
            KeyCode::Tab => self.detail_move(1),
            KeyCode::BackTab => self.detail_move(-1),
            _ => {},
        }
    }

    fn detail_apply_menu_action(&mut self, action: MenuAction) {
        let target = self.detail.as_ref().map(|detail| detail.menu).unwrap_or(None);
        if target.is_some() {
            self.detail_apply_menu();
            return;
        }
        let Some(field) = self.detail_selected_field().cloned() else {
            return;
        };
        let label = field.label;
        let value = field.value;
        let Some(path) = field.path.or_else(|| FieldPath::parse(&label).ok()) else {
            self.message = Some(format!("cannot use `{label}` as a field path"));
            return;
        };
        match action {
            MenuAction::ToggleColumn => self.toggle_column(path),
            MenuAction::FilterEq => {
                let probe = truncate_probe(&value);
                self.add_filter_from_op(path, LeafOp::Eq, Some(probe));
            },
            MenuAction::FilterNeq => {
                let probe = truncate_probe(&value);
                self.add_filter_from_op(path, LeafOp::Neq, Some(probe));
            },
            MenuAction::FilterContains => {
                let probe = truncate_probe(&value);
                self.add_filter_from_op(path, LeafOp::Contains, Some(probe));
            },
            MenuAction::Exists => self.add_filter_from_op(path, LeafOp::Exists, None),
            MenuAction::NotExists => self.add_filter_from_op(path, LeafOp::NotExists, None),
        }
    }

    fn handle_picker_key(&mut self, code: KeyCode, _modifiers: KeyModifiers) {
        match code {
            KeyCode::Esc => self.close_picker(),
            KeyCode::Char(ch) => {
                if let Some(picker) = self.picker.as_mut() {
                    picker.input.insert(ch);
                }
                self.picker_filter();
            },
            KeyCode::Backspace => {
                if let Some(picker) = self.picker.as_mut() {
                    picker.input.backspace();
                }
                self.picker_filter();
            },
            KeyCode::Enter => self.picker_toggle(),
            KeyCode::Down => self.picker_move(1),
            KeyCode::Up => self.picker_move(-1),
            KeyCode::PageDown => self.picker_move(isize::try_from(self.page_height()).unwrap_or(1)),
            KeyCode::PageUp => self.picker_move(-isize::try_from(self.page_height()).unwrap_or(1)),
            KeyCode::Tab => self.cycle_focus(1),
            _ => {},
        }
    }

    /// Mouse: header cell sorts, rows open detail, inputs focus.
    pub fn handle_mouse(&mut self, kind: MouseEventKind, column: u16, row: u16) {
        match kind {
            MouseEventKind::Down(ratatui::crossterm::event::MouseButton::Left) => self.handle_click(column, row),
            MouseEventKind::ScrollUp => self.move_cursor(-3),
            MouseEventKind::ScrollDown => self.move_cursor(3),
            _ => {},
        }
    }

    fn handle_click(&mut self, column: u16, row: u16) {
        if self.help {
            self.help = false;
            return;
        }
        if self.picker.is_some() || self.detail.is_some() {
            return;
        }
        let areas = self.areas;
        if contains(areas.search, row) {
            self.focus = Focus::Search;
            self.search_input.move_to(usize::from(column.saturating_sub(2)));
            return;
        }
        if contains(areas.filter, row) {
            self.focus = Focus::FilterInput;
            self.filter_input.move_to(usize::from(column.saturating_sub(2)));
            return;
        }
        if contains(areas.chips, row) {
            self.focus = Focus::FilterChips;
            if let Some((_, _, index)) = self
                .chip_cells
                .iter()
                .find(|(start, end, _)| *start <= column && column <= *end)
            {
                self.filter_cursor = *index;
            }
            return;
        }
        if contains(areas.rows, row) {
            if row == areas.rows.top() {
                self.focus = Focus::Header;
                if let Some((_, _, index)) = self
                    .header_cells
                    .iter()
                    .find(|(start, end, _)| *start <= column && column <= *end)
                {
                    self.header_cursor = *index;
                    if let Some(column) = self.focused_column().cloned() {
                        self.cycle_sort(&column);
                    }
                }
                return;
            }
            let offset = usize::from(row - areas.rows.top() - 1);
            self.cursor = (self.rows_scroll + offset).min(self.view.len().saturating_sub(1));
            self.focus = Focus::Rows;
            self.open_detail();
        }
    }

    pub fn page_height(&self) -> usize {
        usize::from(self.areas.rows.height.saturating_sub(1)).max(1)
    }
}

fn contains(area: Rect, row: u16) -> bool {
    area.height > 0 && row >= area.top() && row < area.top() + area.height
}

/// Probe used to build a filter from a field value; long values become a contains pattern.
fn truncate_probe(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.len() <= 64 {
        return glob_escape(trimmed);
    }
    trimmed.chars().take(48).collect()
}

/// Escapes glob metacharacters so a literal value matches itself.
fn glob_escape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        if matches!(ch, '*' | '?' | '[' | ']' | '{' | '}' | '\\') {
            out.push('\\');
        }
        out.push(ch);
    }
    out
}

/// Field list of the detail view: source, line, pseudo columns, then record fields.
pub fn detail_fields(record: &crate::model::record::LogRecord, config: &Config) -> Vec<DetailField> {
    let mut fields = Vec::new();
    let config = config.clone();
    let (stamp, stamp_style) = cell_text(record, &Column::Timestamp, "", &config);
    fields.push(DetailField {
        label: "@timestamp".to_owned(),
        value: stamp,
        style: stamp_style,
        path: None,
    });
    let (level, level_style) = cell_text(record, &Column::Level, "", &config);
    fields.push(DetailField {
        label: "@level".to_owned(),
        value: level,
        style: level_style,
        path: None,
    });
    let (message, message_style) = cell_text(record, &Column::Message, "", &config);
    fields.push(DetailField {
        label: "@message".to_owned(),
        value: message,
        style: message_style,
        path: None,
    });
    let (line, line_style) = cell_text(record, &Column::Line, "", &config);
    fields.push(DetailField {
        label: "@line".to_owned(),
        value: line,
        style: line_style,
        path: None,
    });
    for (label, value) in leaf_values(record, 6) {
        let path = FieldPath::parse(&label).ok();
        let mut buf = String::new();
        let text = crate::model::value::text_of(value, &mut buf).as_str().to_owned();
        fields.push(DetailField {
            label,
            value: text,
            style: crate::render::fields::kind_style(value),
            path,
        });
    }
    fields
}

/// Pretty JSON of a record, for the detail JSON view.
pub fn detail_json_lines(record: &crate::model::record::LogRecord) -> Vec<String> {
    let cells = crate::render::record_render::json_cells(record, true);
    let text: String = cells.iter().map(|cell: &Cell| cell.text.as_str()).collect();
    text.lines().map(str::to_owned).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
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
        assert_eq!(state.focus, Focus::Rows);
        assert!(state.status.contains("rows 2/2 of 2"));
    }

    #[test]
    fn filters_narrow_and_can_be_removed() {
        let mut state = state_of(&["{\"level\":\"INFO\"}", "{\"level\":\"WARN\"}"]);
        let state_config = state.filter_context();
        let predicate = crate::filter::build_leaf("level", LeafOp::Eq, Some("WARN"), &state_config).expect("leaf");
        state.add_filter(predicate);
        assert_eq!(state.view.len(), 1);
        assert_eq!(state.filters.len(), 1);
        state.remove_filter(0);
        assert_eq!(state.view.len(), 2);
    }

    #[test]
    fn filter_input_parses_and_reports_errors() {
        let mut state = state_of(&["{\"level\":\"WARN\",\"msg\":\"disk full\"}"]);
        state.filter_input.set("level=WARN");
        state.submit_filter_input();
        assert_eq!(state.filters.len(), 1);
        state.filter_input.set("level");
        state.submit_filter_input();
        assert!(state.message.as_deref().unwrap_or_default().contains("filter:"));
        assert_eq!(state.filters.len(), 1);
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
        assert!(state.sorts.is_empty());
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

    #[test]
    fn detail_open_and_apply_filter_from_field() {
        let mut state = state_of(&["{\"level\":\"ERROR\",\"message\":\"boom\"}"]);
        state.set_search_text("");
        state.open_detail();
        assert!(state.detail.is_some());
        let index = state
            .detail
            .as_ref()
            .map(|d| d.fields.iter().position(|f| f.label == "level").unwrap_or(usize::MAX))
            .unwrap_or(usize::MAX);
        state.detail.as_mut().expect("detail").cursor = index;
        state.detail_apply_menu_action(MenuAction::FilterEq);
        assert_eq!(state.filters.len(), 1);
        assert_eq!(state.filters[0].to_string(), "level=ERROR");
    }

    #[test]
    fn picker_lists_and_toggles_fields() {
        let mut state = state_of(&["{\"a\":1,\"b\":2}"]);
        state.open_picker();
        assert_eq!(state.picker.as_ref().map(|p| p.fields.len()), Some(2));
        state.picker_toggle();
        let picker = state.picker.as_ref().expect("picker");
        assert!(picker.fields[picker.matches[0]].selected);
        state.close_picker();
        assert_eq!(state.columns.len(), 1);
    }

    #[test]
    fn picker_filter_narrows_matches() {
        let mut state = state_of(&["{\"alpha\":1,\"beta\":2}"]);
        state.open_picker();
        if let Some(picker) = state.picker.as_mut() {
            picker.input.set("al");
        }
        state.picker_filter();
        assert_eq!(state.picker.as_ref().map(|p| p.matches.len()), Some(1));
    }

    #[test]
    fn keys_drive_rows_and_focus() {
        let mut state = state_of(&["{\"a\":1}", "{\"a\":2}", "{\"a\":3}"]);
        state.handle_key(KeyCode::Char('j'), KeyModifiers::NONE);
        assert_eq!(state.cursor, 1);
        state.handle_key(KeyCode::Char('G'), KeyModifiers::NONE);
        assert_eq!(state.cursor, 2);
        state.handle_key(KeyCode::Char('g'), KeyModifiers::NONE);
        assert_eq!(state.cursor, 0);
        state.handle_key(KeyCode::Tab, KeyModifiers::NONE);
        assert_eq!(state.focus, Focus::Search);
        state.handle_key(KeyCode::Esc, KeyModifiers::NONE);
        assert_eq!(state.focus, Focus::Rows);
        state.handle_key(KeyCode::Char('?'), KeyModifiers::NONE);
        assert!(state.help);
        state.handle_key(KeyCode::Esc, KeyModifiers::NONE);
        assert!(!state.help);
    }

    #[test]
    fn typing_in_search_focus_edits_the_pattern() {
        let mut state = state_of(&["{\"message\":\"disk full\"}"]);
        state.focus = Focus::Search;
        for ch in "disk".chars() {
            state.handle_key(KeyCode::Char(ch), KeyModifiers::NONE);
        }
        assert_eq!(state.search.patterns, vec!["disk"]);
        assert_eq!(state.view.len(), 1);
        state.handle_key(KeyCode::Enter, KeyModifiers::NONE);
        assert_eq!(state.focus, Focus::Rows);
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
}
