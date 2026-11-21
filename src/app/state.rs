//! Application state, view rebuilding and all user actions.

use ratatui::crossterm::event::KeyCode;
use ratatui::crossterm::event::KeyModifiers;
use ratatui::crossterm::event::MouseEventKind;
use ratatui::layout::Rect;
use ratatui::text::Line;

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

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Focus {
    Search,
    FilterChips,
    FilterInput,
    Header,
    Rows,
    /// The bottom update line; in the Tab cycle only while the line is drawn.
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
    /// Tab cycle order; `UpdateLine` joins the cycle only while the update line is drawn.
    pub const ORDER: [Self; 6] = [
        Self::Search,
        Self::FilterChips,
        Self::FilterInput,
        Self::Header,
        Self::Rows,
        Self::UpdateLine,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Search => "search",
            Self::FilterChips => "filters",
            Self::FilterInput => "filter",
            Self::Header => "fields",
            Self::Rows => "rows",
            Self::UpdateLine => "update",
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
    pub prompts: Rect,
    pub status: Rect,
    pub dropdown: Rect,
    pub update: Rect,
}

/// Step of the guided filter builder.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum WizardPhase {
    Field,
    Operator,
    Pattern,
}

/// A field name offered by the autocomplete dropdown.
#[derive(Clone, Debug)]
pub struct FieldCandidate {
    pub path: FieldPath,
    pub label: String,
}

#[derive(Clone, Debug)]
pub struct FilterWizard {
    pub phase: WizardPhase,
    pub field: Option<FieldPath>,
    pub op: Option<LeafOp>,
    pub matches: Vec<FieldCandidate>,
    pub cursor: usize,
    /// Value pattern typed into the operator step (e.g. from the row value picker).
    pub prefill: Option<String>,
}

/// One operator option of the builder's second step.
pub struct WizardOp {
    pub op: LeafOp,
    pub label: &'static str,
    pub shortcut: char,
}

pub const WIZARD_OPS: [WizardOp; 6] = [
    WizardOp {
        op: LeafOp::Eq,
        label: "=   equals (glob pattern)",
        shortcut: '=',
    },
    WizardOp {
        op: LeafOp::Neq,
        label: "!=  not equals (glob pattern)",
        shortcut: '!',
    },
    WizardOp {
        op: LeafOp::Contains,
        label: "~   contains text",
        shortcut: '~',
    },
    WizardOp {
        op: LeafOp::NotContains,
        label: "!~  not contains text",
        shortcut: 'n',
    },
    WizardOp {
        op: LeafOp::Exists,
        label: "^   field exists",
        shortcut: 'e',
    },
    WizardOp {
        op: LeafOp::NotExists,
        label: "!^  field missing",
        shortcut: 'E',
    },
];

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

    pub const fn style(self) -> crate::render::color::Sg {
        match self {
            Self::ToggleColumn => crate::render::color::Sg::Default,
            Self::FilterEq | Self::FilterContains | Self::Exists => crate::render::color::Sg::OpPositive,
            Self::FilterNeq | Self::NotExists => crate::render::color::Sg::OpNegative,
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
    pub header_mode: HeaderMode,
    pub row_mode: RowMode,
    pub pick_cursor: usize,
    pub cursor: usize,
    pub header_cursor: usize,
    pub filter_cursor: usize,
    pub column_scroll: usize,
    pub rows_scroll: usize,
    pub detail: Option<DetailState>,
    pub picker: Option<PickerState>,
    pub wizard: Option<FilterWizard>,
    pub help: bool,
    pub message: Option<String>,
    pub status: String,
    pub sort_marks: Vec<(String, SortDir)>,
    pub areas: Areas,
    pub header_cells: Vec<(u16, u16, usize)>,
    pub chip_cells: Vec<(u16, u16, usize, u16)>,
    pub regexes: Vec<regex::Regex>,
    /// Layout character to QWERTY position, resolved from `config.key_map`.
    pub keymap: KeyMap,
    /// Mode the current map was resolved for, so a mode change re-resolves it.
    keymap_mode: KeyMapMode,
    /// Rebuilds left: a layout switch mid-session is probed a few times, then left alone.
    keymap_rebuilds: u8,
    /// Filter lists for Ctrl+Z: each entry is the list as it was before a change.
    filter_undo: Vec<Vec<Predicate>>,
    /// Filter lists for Ctrl+Shift+Z: each entry is a list taken over by undo.
    filter_redo: Vec<Vec<Predicate>>,
    /// Release repository of the interactive update check (default repo, overridable).
    pub update_repo: RepoRef,
    /// Newest releases newer than the running version, newest first; the button appears
    /// on the update line when this is non-empty.
    pub update_available: Option<Vec<ReleaseNote>>,
    /// A background release check is in flight.
    pub update_checking: bool,
    /// Bumped on every spawned check; events from an older generation are stale
    /// (a repository url was submitted while the old check was in flight) and get dropped.
    update_check_generation: u64,
    /// The update download/replace is in flight.
    pub updating: bool,
    /// Open changelog overlay (delta between the running and the newest version).
    pub changelog: Option<ChangelogState>,
    /// Open pop-up asking for a repository url (unreachable repo, forks).
    pub repo_prompt: Option<RepoPrompt>,
    /// Hit rects of the update line buttons, recorded while drawing.
    pub update_btn: Rect,
    pub changelog_btn: Rect,
    /// While the update line is focused: 0 = the update button, 1 = the changelog button.
    pub update_focus_index: usize,
    /// The binary is a `cargo install` artifact (it sits in the cargo bin dir): the update
    /// line tells the user to update through cargo, since a self-update desyncs cargo's list.
    pub update_cargo_managed: bool,
    /// Update worker events; `run_loop` drains them via `poll_update_events`.
    pub update_events: (std::sync::mpsc::Sender<UpdateEvent>, std::sync::mpsc::Receiver<UpdateEvent>),
    pub dirty: bool,
    pub quit: bool,
}

/// The changelog overlay: release notes between the running and the newest version.
/// `content_height`/`page` are recorded while drawing so keys and wheel can clamp scroll.
#[derive(Clone, Debug, Default)]
pub struct ChangelogState {
    pub scroll: usize,
    pub content_height: usize,
    pub page: usize,
}

/// The repository url pop-up: shown when the release repo is unreachable, opened with `u`.
#[derive(Clone, Debug)]
pub struct RepoPrompt {
    pub message: String,
    pub editor: Editor,
}

/// Live rebuilds are skipped above this many records; Enter forces one.
const LIVE_REBUILD_LIMIT: usize = 30_000;

/// Layout probes per session after an unmapped non-ASCII key: covers a few layout switches.
const KEYMAP_REBUILDS: u8 = 3;

/// Filter changes kept for Ctrl+Z: a few mistakes deep, cheap (the lists are short).
const FILTER_HISTORY_LIMIT: usize = 32;

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
            sorts,
            filters,
            search,
            search_input,
            filter_input: Editor::new(),
            focus: Focus::Rows,
            header_mode: HeaderMode::Navigate,
            row_mode: RowMode::Normal,
            pick_cursor: 0,
            cursor: 0,
            header_cursor: 0,
            filter_cursor: 0,
            column_scroll: 0,
            rows_scroll: 0,
            detail: None,
            picker: None,
            wizard: None,
            help: false,
            message: None,
            status: String::new(),
            sort_marks: Vec::new(),
            areas: Areas::default(),
            header_cells: Vec::new(),
            chip_cells: Vec::new(),
            regexes: Vec::new(),
            keymap,
            keymap_mode,
            keymap_rebuilds: KEYMAP_REBUILDS,
            filter_undo: Vec::new(),
            filter_redo: Vec::new(),
            update_repo: config.update_repo.clone().unwrap_or_else(RepoRef::default_repo),
            update_available: None,
            update_checking: false,
            update_check_generation: 0,
            updating: false,
            changelog: None,
            repo_prompt: None,
            update_btn: Rect::default(),
            changelog_btn: Rect::default(),
            update_focus_index: 0,
            update_cargo_managed: crate::update::cargo_managed_install(),
            update_events: std::sync::mpsc::channel(),
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
        self.sort_marks = self
            .sorts
            .iter()
            .map(|spec| (spec.field.to_string(), spec.dir))
            .collect();
        let focus_label = if self.focus == Focus::Header && self.header_mode == HeaderMode::Drag {
            "fields (drag)"
        } else {
            self.focus.label()
        };
        self.status = format!(
            "rows {shown}/{} of {total} | fields {} | focus {focus_label} | {}",
            self.view.matched,
            self.columns.len(),
            self.config.render,
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
        self.push_filter_history();
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

    /// Opens the guided filter builder on the field step.
    pub fn open_filter_wizard(&mut self) {
        self.filter_input.clear();
        let matches = self.field_candidates("");
        self.wizard = Some(FilterWizard {
            phase: WizardPhase::Field,
            field: None,
            op: None,
            matches,
            cursor: 0,
            prefill: None,
        });
        self.focus = Focus::FilterInput;
        self.dirty = true;
    }

    pub fn close_filter_wizard(&mut self) {
        self.wizard = None;
        self.filter_input.clear();
        self.dirty = true;
    }

    fn field_candidates(&self, needle: &str) -> Vec<FieldCandidate> {
        let needle = needle.to_lowercase();
        self.store
            .fields_by_frequency()
            .into_iter()
            .filter(|info| needle.is_empty() || info.label.to_lowercase().contains(&needle))
            .map(|info| FieldCandidate {
                path: info.path.clone(),
                label: info.label.clone(),
            })
            .collect()
    }

    fn wizard_recompute(&mut self) {
        let in_field_phase = self
            .wizard
            .as_ref()
            .is_some_and(|wizard| wizard.phase == WizardPhase::Field);
        if !in_field_phase {
            return;
        }
        let needle = self.filter_input.text();
        let matches = self.field_candidates(&needle);
        if let Some(wizard) = self.wizard.as_mut() {
            wizard.matches = matches;
            wizard.cursor = 0;
        }
        self.dirty = true;
    }

    pub fn wizard_move(&mut self, delta: isize) {
        let Some(wizard) = self.wizard.as_mut() else {
            return;
        };
        let len = match wizard.phase {
            WizardPhase::Field => wizard.matches.len(),
            WizardPhase::Operator => WIZARD_OPS.len(),
            WizardPhase::Pattern => 0,
        };
        if len == 0 {
            return;
        }
        let current = isize::try_from(wizard.cursor).unwrap_or(0);
        let len = isize::try_from(len).unwrap_or(isize::MAX);
        wizard.cursor = usize::try_from((current + delta).clamp(0, len - 1)).unwrap_or(0);
        self.dirty = true;
    }

    pub fn wizard_pick_cursor(&mut self) {
        self.wizard_enter();
    }

    /// Enter on the builder: advances field -> operator -> value, or commits.
    pub fn wizard_enter(&mut self) {
        let Some(wizard) = self.wizard.clone() else {
            return;
        };
        match wizard.phase {
            WizardPhase::Field => self.wizard_pick_field(wizard),
            WizardPhase::Operator => self.wizard_pick_operator(wizard),
            WizardPhase::Pattern => self.wizard_commit(wizard),
        }
    }

    fn wizard_pick_field(&mut self, wizard: FilterWizard) {
        let chosen = wizard
            .matches
            .get(wizard.cursor)
            .map(|candidate| candidate.path.clone())
            .or_else(|| {
                let text = self.filter_input.text();
                FieldPath::parse(text.trim()).ok()
            });
        let Some(path) = chosen else {
            // Not a plain field name: try the text as a full DSL expression.
            self.submit_filter_input();
            if self.filter_input.text().trim().is_empty() {
                self.wizard = None;
            }
            self.dirty = true;
            return;
        };
        self.filter_input.set(&path.to_string());
        if let Some(wizard) = self.wizard.as_mut() {
            wizard.field = Some(path);
            wizard.phase = WizardPhase::Operator;
            wizard.cursor = 0;
        }
        self.dirty = true;
    }

    fn wizard_pick_operator(&mut self, wizard: FilterWizard) {
        let Some(op) = WIZARD_OPS.get(wizard.cursor).map(|option| option.op) else {
            return;
        };
        let Some(field) = wizard.field.clone() else {
            self.close_filter_wizard();
            return;
        };
        if !op.takes_pattern() {
            self.close_filter_wizard();
            self.add_filter_from_op(field, op, None);
            self.focus = Focus::Rows;
            self.dirty = true;
            return;
        }
        self.filter_input.clear();
        if let Some(prefill) = wizard.prefill.clone() {
            self.filter_input.set(&prefill);
        }
        if let Some(wizard) = self.wizard.as_mut() {
            wizard.op = Some(op);
            wizard.phase = WizardPhase::Pattern;
        }
        self.dirty = true;
    }

    fn wizard_commit(&mut self, wizard: FilterWizard) {
        let (Some(field), Some(op)) = (wizard.field.clone(), wizard.op) else {
            self.close_filter_wizard();
            return;
        };
        let text = self.filter_input.text();
        let value = if text.is_empty() { "*".to_owned() } else { text };
        self.close_filter_wizard();
        self.add_filter_from_op(field, op, Some(value));
        self.focus = Focus::Rows;
        self.dirty = true;
    }

    /// Esc / Backspace at the start of a step: back to the previous step.
    pub fn wizard_back(&mut self) {
        let Some(wizard) = self.wizard.clone() else {
            return;
        };
        match wizard.phase {
            WizardPhase::Pattern => {
                if let Some(field) = wizard.field.as_ref() {
                    self.filter_input.set(&field.to_string());
                }
                if let Some(wizard) = self.wizard.as_mut() {
                    wizard.phase = WizardPhase::Operator;
                }
            },
            WizardPhase::Operator => {
                if let Some(field) = wizard.field.as_ref() {
                    self.filter_input.set(&field.to_string());
                }
                self.wizard_recompute();
                if let Some(wizard) = self.wizard.as_mut() {
                    wizard.phase = WizardPhase::Field;
                    wizard.op = None;
                }
            },
            WizardPhase::Field => {
                self.close_filter_wizard();
                self.focus = Focus::FilterChips;
            },
        }
        self.dirty = true;
    }

    /// Typed operator shortcut on the operator step (e.g. `=`, `e`).
    fn wizard_op_shortcut(&mut self, ch: char) {
        let Some(index) = WIZARD_OPS.iter().position(|option| option.shortcut == ch) else {
            return;
        };
        if let Some(wizard) = self.wizard.as_mut() {
            wizard.cursor = index;
        }
        let Some(wizard) = self.wizard.clone() else {
            return;
        };
        self.wizard_pick_operator(wizard);
    }

    pub fn remove_filter(&mut self, index: usize) {
        if index < self.filters.len() {
            self.push_filter_history();
            self.filters.remove(index);
            self.filter_cursor = self.filter_cursor.min(self.filters.len().saturating_sub(1));
            self.rebuild();
        }
    }

    pub fn clear_filters(&mut self) {
        if self.filters.is_empty() {
            return;
        }
        self.push_filter_history();
        self.filters.clear();
        self.filter_cursor = 0;
        self.rebuild();
    }

    /// Ctrl+Z: back to the filters as they were before the last add, removal or clear.
    pub fn undo_filters(&mut self) {
        self.swap_filter_lists(true);
    }

    /// Ctrl+Shift+Z: forward to the filters that undo took away.
    pub fn redo_filters(&mut self) {
        self.swap_filter_lists(false);
    }

    /// Records the filter list for Ctrl+Z; a new change ends the redo branch.
    fn push_filter_history(&mut self) {
        self.filter_undo.push(self.filters.clone());
        if self.filter_undo.len() > FILTER_HISTORY_LIMIT {
            self.filter_undo.remove(0);
        }
        self.filter_redo.clear();
    }

    /// Undo and redo are the same swap: the current list goes to one stack, the other pops.
    fn swap_filter_lists(&mut self, backward: bool) {
        let (from, to) = if backward {
            (&mut self.filter_undo, &mut self.filter_redo)
        } else {
            (&mut self.filter_redo, &mut self.filter_undo)
        };
        let Some(list) = from.pop() else {
            let token = if backward { "undo" } else { "redo" };
            self.message = Some(format!("nothing to {token}"));
            return;
        };
        to.push(self.filters.clone());
        self.filters = list;
        self.filter_cursor = self.filter_cursor.min(self.filters.len().saturating_sub(1));
        self.rebuild();
        let token = if backward { "undone" } else { "redone" };
        self.message = Some(format!("filter change {token}"));
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

    pub fn enter_column_drag(&mut self) {
        if self.header_mode == HeaderMode::Drag {
            return;
        }
        self.header_mode = HeaderMode::Drag;
        self.message = Some("column drag: Left/Right move column, Enter/click sorts, Space/Esc leaves".to_owned());
        self.update_status();
        self.dirty = true;
    }

    pub fn leave_column_drag(&mut self) {
        if self.header_mode == HeaderMode::Navigate {
            return;
        }
        self.header_mode = HeaderMode::Navigate;
        self.update_status();
        self.dirty = true;
    }

    /// Reorders the focused column by `delta` positions; the cursor follows the column.
    pub fn move_column(&mut self, delta: isize) {
        if self.columns.is_empty() {
            return;
        }
        let from = self.header_cursor;
        let to = (from as isize + delta).clamp(0, self.columns.len() as isize - 1) as usize;
        if to == from {
            return;
        }
        let label = self.columns[from].label();
        self.columns.swap(from, to);
        self.header_cursor = to;
        self.message = Some(format!("column moved: {label} -> {}", to + 1));
        self.dirty = true;
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

    /// Wheel scrolling, overlay-aware: three lines per notch in the active layer.
    pub fn wheel(&mut self, notches: isize) {
        let delta = notches.saturating_mul(3);
        if self.changelog.is_some() {
            self.changelog_move(delta);
        } else if self.detail.is_some() {
            self.detail_move(delta);
        } else if self.picker.is_some() {
            self.picker_move(delta);
        } else {
            self.move_cursor(delta);
        }
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
        // The update line joins the cycle only while it is drawn; when it hides,
        // a focus on it falls back to the rows.
        let order: Vec<Focus> = Focus::ORDER
            .into_iter()
            .filter(|focus| *focus != Focus::UpdateLine || self.update_line_visible())
            .collect();
        if order.iter().all(|focus| *focus != self.focus) {
            self.focus = Focus::Rows;
        }
        let len = isize::try_from(order.len()).unwrap_or(isize::MAX);
        let current = order.iter().position(|f| *f == self.focus).unwrap_or(0);
        let current = isize::try_from(current).unwrap_or(0);
        let next = (current + direction).rem_euclid(len);
        self.focus = order[usize::try_from(next).unwrap_or(0)];
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

    /// Resolves the key map, re-probing when the mode changed or the layout switched.
    fn keymap_press(&mut self, code: KeyCode, modifiers: KeyModifiers) -> KeyPress {
        if self.keymap_mode != self.config.key_map {
            self.keymap_mode = self.config.key_map.clone();
            self.keymap = KeyMap::resolve(&self.config.key_map);
        }
        let mut key = KeyPress::new(code, modifiers, &self.keymap);
        if key.needs_rebuild() && self.keymap_rebuilds > 0 {
            self.keymap_rebuilds -= 1;
            self.keymap = KeyMap::resolve(&self.config.key_map);
            key = KeyPress::new(code, modifiers, &self.keymap);
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
        if self.help {
            match key.bind {
                KeyCode::Esc | KeyCode::Char('?') | KeyCode::Char('q') => self.help = false,
                _ => {},
            }
            self.dirty = true;
            return;
        }
        if self.repo_prompt.is_some() {
            self.handle_repo_prompt_key(key);
            self.dirty = true;
            return;
        }
        if self.changelog.is_some() {
            self.handle_changelog_key(key);
            self.dirty = true;
            return;
        }
        if self.picker.is_some() {
            self.handle_picker_key(key);
            self.dirty = true;
            return;
        }
        if self.detail.is_some() {
            self.handle_detail_key(key);
            self.dirty = true;
            return;
        }
        match key.bind {
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
        matches!(self.focus, Focus::Search | Focus::FilterInput)
    }

    fn handle_search_key(&mut self, key: KeyPress) {
        match key.bind {
            KeyCode::Char(_) => {
                if let Some(ch) = key.typed_char() {
                    self.search_input.insert(ch);
                }
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

    fn handle_filter_key(&mut self, key: KeyPress) {
        if self.wizard.is_some() {
            self.handle_wizard_key(key);
            return;
        }
        match key.bind {
            KeyCode::Char(_) => {
                if let Some(ch) = key.typed_char() {
                    self.filter_input.insert(ch);
                }
            },
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

    /// Keys on the guided builder: field autocomplete, operator list, value.
    fn handle_wizard_key(&mut self, key: KeyPress) {
        let phase = self.wizard.as_ref().map(|wizard| wizard.phase);
        match (phase, key.bind) {
            (Some(WizardPhase::Field), KeyCode::Char(_)) => {
                if let Some(ch) = key.typed_char() {
                    self.filter_input.insert(ch);
                }
                self.wizard_recompute();
            },
            (Some(WizardPhase::Field), KeyCode::Backspace) => {
                if self.filter_input.is_empty() {
                    self.wizard_back();
                } else {
                    self.filter_input.backspace();
                    self.wizard_recompute();
                }
            },
            (Some(WizardPhase::Field), KeyCode::Left) => self.filter_input.left(),
            (Some(WizardPhase::Field), KeyCode::Right) => self.filter_input.right(),
            (Some(WizardPhase::Field), KeyCode::Home) => self.filter_input.home(),
            (Some(WizardPhase::Field), KeyCode::End) => self.filter_input.end(),
            (Some(WizardPhase::Field), KeyCode::Up) => self.wizard_move(-1),
            (Some(WizardPhase::Field), KeyCode::Down) => self.wizard_move(1),
            (Some(WizardPhase::Field), KeyCode::Tab) => self.wizard_move(1),
            (Some(WizardPhase::Field), KeyCode::BackTab) => self.wizard_move(-1),
            (Some(WizardPhase::Operator), KeyCode::Char(_)) => {
                if let Some(ch) = key.bind_char() {
                    self.wizard_op_shortcut(ch);
                }
            },
            (Some(WizardPhase::Operator), KeyCode::Up) => self.wizard_move(-1),
            (Some(WizardPhase::Operator), KeyCode::Down) => self.wizard_move(1),
            (Some(WizardPhase::Operator), KeyCode::Tab) => self.wizard_move(1),
            (Some(WizardPhase::Operator), KeyCode::BackTab) => self.wizard_move(-1),
            (Some(WizardPhase::Operator), KeyCode::Backspace) => self.wizard_back(),
            (Some(WizardPhase::Pattern), KeyCode::Char(_)) => {
                if let Some(ch) = key.typed_char() {
                    self.filter_input.insert(ch);
                }
            },
            (Some(WizardPhase::Pattern), KeyCode::Backspace) => {
                if self.filter_input.is_empty() {
                    self.wizard_back();
                } else {
                    self.filter_input.backspace();
                }
            },
            (Some(WizardPhase::Pattern), KeyCode::Left) => self.filter_input.left(),
            (Some(WizardPhase::Pattern), KeyCode::Right) => self.filter_input.right(),
            (Some(WizardPhase::Pattern), KeyCode::Home) => self.filter_input.home(),
            (Some(WizardPhase::Pattern), KeyCode::End) => self.filter_input.end(),
            (Some(WizardPhase::Pattern), KeyCode::Delete) => self.filter_input.delete(),
            (Some(_), KeyCode::Enter) => self.wizard_enter(),
            (Some(_), KeyCode::Esc) => self.wizard_back(),
            _ => {},
        }
        self.dirty = true;
    }

    fn handle_chips_key(&mut self, key: KeyPress) {
        match key.bind {
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
            KeyCode::Char('i') | KeyCode::Char('a') => self.open_filter_wizard(),
            KeyCode::Char('x') => self.clear_filters(),
            KeyCode::Esc => self.focus = Focus::Rows,
            KeyCode::Tab => self.cycle_focus(1),
            KeyCode::BackTab => self.cycle_focus(-1),
            _ => {},
        }
    }

    fn handle_header_key(&mut self, key: KeyPress) {
        let dragging = self.header_mode == HeaderMode::Drag;
        match key.bind {
            KeyCode::Left | KeyCode::Char('h') => {
                if dragging {
                    self.move_column(-1);
                } else {
                    self.header_cursor = self.header_cursor.saturating_sub(1);
                }
            },
            KeyCode::Right | KeyCode::Char('l') => {
                if dragging {
                    self.move_column(1);
                } else {
                    self.header_cursor = (self.header_cursor + 1).min(self.columns.len().saturating_sub(1));
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
                    self.columns.remove(self.header_cursor);
                    self.message = Some(format!("column removed: {}", column.label()));
                    self.header_cursor = self.header_cursor.min(self.columns.len().saturating_sub(1));
                    self.update_status();
                }
            },
            KeyCode::Esc => {
                if dragging {
                    self.leave_column_drag();
                } else {
                    self.focus = Focus::Rows;
                }
            },
            KeyCode::Tab => self.cycle_focus(1),
            KeyCode::BackTab => self.cycle_focus(-1),
            KeyCode::Down | KeyCode::Char('j') => self.focus = Focus::Rows,
            _ => {},
        }
    }

    fn handle_rows_key(&mut self, key: KeyPress) {
        let picking = self.row_mode == RowMode::Pick;
        match key.bind {
            KeyCode::Char('q') => self.quit(),
            KeyCode::Char('j') | KeyCode::Down => self.move_cursor(1),
            KeyCode::Char('k') | KeyCode::Up => self.move_cursor(-1),
            KeyCode::PageDown => self.page(1, self.page_height()),
            KeyCode::PageUp => self.page(-1, self.page_height()),
            KeyCode::Char('f') => self.page(1, self.page_height()),
            KeyCode::Char('b') => self.page(-1, self.page_height()),
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
            KeyCode::Char('S') => self.focus = Focus::Header,
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
            KeyCode::Left => {
                if picking {
                    self.pick_cursor = self.pick_cursor.saturating_sub(1);
                } else {
                    self.column_scroll = self.column_scroll.saturating_sub(1);
                }
            },
            KeyCode::Right => {
                if picking {
                    self.pick_cursor = (self.pick_cursor + 1).min(self.columns.len().saturating_sub(1));
                } else {
                    self.column_scroll = (self.column_scroll + 1).min(self.columns.len().saturating_sub(1));
                }
            },
            KeyCode::Esc => {
                if picking {
                    self.row_mode = RowMode::Normal;
                } else if !self.search.patterns.is_empty() {
                    self.clear_search();
                } else if !self.filters.is_empty() {
                    self.focus = Focus::FilterChips;
                }
            },
            KeyCode::Tab => self.cycle_focus(1),
            KeyCode::BackTab => self.cycle_focus(-1),
            KeyCode::Char('/') => self.focus = Focus::Search,
            KeyCode::Char('F') => self.open_filter_wizard(),
            KeyCode::Char('u') if !self.config.no_update => self.open_repo_prompt(),
            _ => {},
        }
    }

    /// The focused bottom update line: Left/Right choose the button, Enter presses it.
    fn handle_update_line_key(&mut self, key: KeyPress) {
        if !self.update_line_visible() {
            self.focus = Focus::Rows;
            return;
        }
        match key.bind {
            KeyCode::Char('q') => self.quit(),
            KeyCode::Left | KeyCode::Char('h') => self.update_focus_index = 0,
            KeyCode::Right | KeyCode::Char('l') => self.update_focus_index = 1,
            KeyCode::Tab => self.cycle_focus(1),
            KeyCode::BackTab => self.cycle_focus(-1),
            KeyCode::Esc => self.focus = Focus::Rows,
            KeyCode::Enter => {
                if self.update_focus_index == 0 {
                    self.start_update();
                } else {
                    self.open_changelog();
                }
            },
            KeyCode::Char('u') if !self.config.no_update => self.open_repo_prompt(),
            _ => {},
        }
    }

    /// Space on rows: toggle the value picker; Enter on a cell starts a filter on that value.
    pub fn toggle_row_pick(&mut self) {
        if self.row_mode == RowMode::Pick {
            self.row_mode = RowMode::Normal;
        } else if self.columns.is_empty() {
            self.message = Some("pick value needs columns: press c to pick fields".to_owned());
        } else {
            self.row_mode = RowMode::Pick;
            self.pick_cursor = self.header_cursor.min(self.columns.len() - 1);
        }
        self.dirty = true;
    }

    /// Opens the filter wizard on the operator step, field and value taken from the picked cell.
    pub fn pick_apply(&mut self) {
        let Some(column) = self.columns.get(self.pick_cursor).cloned() else {
            return;
        };
        let Some(record) = self.current_record().cloned() else {
            return;
        };
        let source = self.source_label(&record).to_owned();
        let (value, _style) = cell_text(&record, &column, &source, &self.config);
        let path = column_path(&column);
        let probe = truncate_probe(&value);
        self.row_mode = RowMode::Normal;
        self.wizard = Some(FilterWizard {
            phase: WizardPhase::Operator,
            field: Some(path.clone()),
            op: None,
            matches: Vec::new(),
            cursor: 0,
            prefill: Some(probe.clone()),
        });
        self.filter_input.set(&path.to_string());
        self.focus = Focus::FilterInput;
        self.message = Some(format!("filter by value: {path} = {probe}, choose the operator"));
        self.dirty = true;
    }

    /// Mode-aware control keys shown on the prompt line; `[...]` marks key tokens.
    pub fn prompts_text(&self) -> &'static str {
        match self.focus {
            Focus::Header if self.header_mode == HeaderMode::Drag => {
                "column drag: [Left]/[Right] move column, [Enter]/[click] sort (asc/desc/off), [Space]/[Esc] leave drag"
            },
            Focus::Header => {
                "fields: [Left]/[Right] move, [Space] column drag, [Enter]/[s] sort (asc/desc/off), [d] remove column, [Esc] rows"
            },
            Focus::Rows if self.row_mode == RowMode::Pick => {
                "pick value: [Left]/[Right] choose field, [Enter] filter by this value, [Space]/[Esc] leave"
            },
            Focus::UpdateLine => {
                "update line: [Left]/[Right] choose update / changelog, [Enter] press it, [u] repository url, [Esc] rows"
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
        match self.focus {
            Focus::Rows if self.row_mode == RowMode::Normal => {
                Some("filters: [Ctrl+Z] undo last filter, [Ctrl+Y] redo last deleted filter")
            },
            _ => None,
        }
    }

    fn handle_detail_key(&mut self, key: KeyPress) {
        match key.bind {
            KeyCode::Esc => self.close_detail(),
            KeyCode::Char('q') => self.close_detail(),
            KeyCode::Char('j') | KeyCode::Down => self.detail_move(1),
            KeyCode::Char('k') | KeyCode::Up => self.detail_move(-1),
            KeyCode::PageDown => self.detail_move(isize::try_from(self.page_height()).unwrap_or(1)),
            KeyCode::PageUp => self.detail_move(-isize::try_from(self.page_height()).unwrap_or(1)),
            KeyCode::Enter => {
                if self.detail.as_ref().is_some_and(|detail| detail.menu.is_some()) {
                    self.detail_apply_menu();
                } else {
                    self.detail_open_menu();
                }
            },
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

    fn handle_picker_key(&mut self, key: KeyPress) {
        match key.bind {
            KeyCode::Esc => self.close_picker(),
            // Space toggles like Enter; it must be matched before the catch-all char arm.
            KeyCode::Char(' ') => self.picker_toggle(),
            KeyCode::Char(_) => {
                if let Some(ch) = key.typed_char() {
                    if let Some(picker) = self.picker.as_mut() {
                        picker.input.insert(ch);
                    }
                    self.picker_filter();
                }
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
            MouseEventKind::ScrollUp => self.wheel(-1),
            MouseEventKind::ScrollDown => self.wheel(1),
            _ => {},
        }
    }

    fn handle_click(&mut self, column: u16, row: u16) {
        if self.help {
            self.help = false;
            return;
        }
        if self.repo_prompt.is_some() {
            // The pop-up is a text input: clicks elsewhere are inert, Esc closes it.
            return;
        }
        if self.changelog.is_some() {
            return;
        }
        if self.update_line_visible() && contains(self.areas.update, row) {
            if self.update_btn.contains(ratatui::layout::Position::new(column, row)) {
                self.focus = Focus::UpdateLine;
                self.update_focus_index = 0;
                self.start_update();
            } else if self.changelog_btn.contains(ratatui::layout::Position::new(column, row)) {
                self.focus = Focus::UpdateLine;
                self.update_focus_index = 1;
                self.open_changelog();
            }
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
        if self.wizard.is_some()
            && areas.dropdown.height > 2
            && row > areas.dropdown.y
            && row < areas.dropdown.y + areas.dropdown.height - 1
        {
            let index = usize::from(row - areas.dropdown.y - 1);
            if let Some(wizard) = self.wizard.as_mut() {
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
            self.focus = Focus::FilterChips;
            let row_offset = row.saturating_sub(areas.chips.y);
            if let Some((_, _, index, _)) = self
                .chip_cells
                .iter()
                .find(|(start, end, _, cell_row)| *cell_row == row_offset && *start <= column && column <= *end)
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
                    match self.header_mode {
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
            self.cursor = (self.rows_scroll + offset).min(self.view.len().saturating_sub(1));
            self.focus = Focus::Rows;
            self.open_detail();
        }
    }

    pub fn page_height(&self) -> usize {
        usize::from(self.areas.rows.height.saturating_sub(1)).max(1)
    }

    /// True when the bottom update line (update + changelog buttons) is drawn.
    pub fn update_line_visible(&self) -> bool {
        !self.config.no_update && self.update_available.is_some() && !self.updating
    }

    /// The newest available version, when an update is available.
    pub fn update_latest(&self) -> Option<&str> {
        self.update_available
            .as_ref()
            .and_then(|notes| notes.first())
            .map(|note| note.version.as_str())
    }

    /// Spawns a background release check; the result arrives via `poll_update_events`.
    /// Each check gets a new generation: a check already in flight is superseded, and
    /// its result is dropped when it lands (the typed repository url wins).
    pub fn start_update_check(&mut self) {
        if self.config.no_update {
            return;
        }
        self.update_checking = true;
        self.update_check_generation = self.update_check_generation.wrapping_add(1);
        let generation = self.update_check_generation;
        let tx = self.update_events.0.clone();
        let target = UpdateTarget::new(self.update_repo.clone());
        std::thread::spawn(move || {
            let outcome = target.check();
            let _ = tx.send(UpdateEvent::Checked(generation, outcome));
        });
    }

    /// Spawns the download-and-replace worker, triggered by the update button.
    /// A cargo-managed install is not self-updated: cargo tracks its version, so the
    /// button prints the cargo command that keeps the bookkeeping in sync.
    pub fn start_update(&mut self) {
        if self.config.no_update || self.updating {
            return;
        }
        if self.update_cargo_managed {
            self.message = Some(format!(
                "update: this binary is a cargo install, update with `cargo install --git {} --force` \
                 (a self-update would desync cargo's install list)",
                self.update_repo.url()
            ));
            self.dirty = true;
            return;
        }
        self.updating = true;
        self.changelog = None;
        // The line hides while updating; a focus on it returns to the rows.
        if self.focus == Focus::UpdateLine {
            self.focus = Focus::Rows;
        }
        let latest = self.update_latest().unwrap_or("the latest release").to_owned();
        self.message = Some(format!("update: updating to {latest} ..."));
        let version = self.update_latest().map(str::to_owned);
        let tx = self.update_events.0.clone();
        let target = UpdateTarget::new(self.update_repo.clone());
        std::thread::spawn(move || {
            let outcome = target.perform(version.as_deref());
            let _ = tx.send(UpdateEvent::Performed(outcome));
        });
    }

    /// Applies update worker events that arrived since the last poll.
    /// Returns true when at least one event changed the state.
    pub fn poll_update_events(&mut self) -> bool {
        let mut applied = false;
        while let Ok(event) = self.update_events.1.try_recv() {
            self.apply_update_event(event);
            applied = true;
        }
        applied
    }

    pub fn apply_update_event(&mut self, event: UpdateEvent) {
        match event {
            UpdateEvent::Checked(generation, outcome) => {
                if generation != self.update_check_generation {
                    // A repository url was submitted while this check ran: its answer
                    // is about the old repository, drop it.
                    return;
                }
                self.update_checking = false;
                match outcome {
                    UpdateOutcome::Available { notes, .. } => {
                        // No status bar message: the update line itself is the prompt,
                        // the rows line stays clean.
                        self.update_available = Some(notes);
                    },
                    UpdateOutcome::UpToDate => {},
                    UpdateOutcome::Unreachable { repo, reason } => {
                        self.message = Some(format!("update: {repo} is unreachable: {reason}"));
                        self.open_repo_prompt();
                    },
                }
            },
            UpdateEvent::Performed(outcome) => {
                self.updating = false;
                match outcome {
                    PerformOutcome::Updated { version } => {
                        self.update_available = None;
                        self.changelog = None;
                        self.message = Some(format!("update: silv updated to {version}, restart silv to use it"));
                    },
                    PerformOutcome::UpToDate => {
                        self.update_available = None;
                        self.message = Some("update: already up to date".to_owned());
                    },
                    PerformOutcome::Failed {
                        reason,
                        suggest_repo_url,
                        suggest_sudo,
                    } => {
                        let mut message = format!("update: failed: {reason}");
                        if suggest_sudo {
                            message.push_str(" - the binary directory is not writable, try `sudo silv update`");
                        }
                        self.message = Some(message);
                        if suggest_repo_url {
                            self.open_repo_prompt();
                        }
                    },
                }
                if self.focus == Focus::UpdateLine && !self.update_line_visible() {
                    self.focus = Focus::Rows;
                }
            },
        }
    }

    /// Opens the repository url pop-up (also the answer to an unreachable repo).
    pub fn open_repo_prompt(&mut self) {
        if self.config.no_update {
            return;
        }
        let mut editor = Editor::with_text(&self.update_repo.url());
        if self.repo_prompt.is_none() {
            editor.clear();
        }
        self.repo_prompt = Some(RepoPrompt {
            message: "enter repository url: owner/repo or https://github.com/owner/repo \
                      (Esc closes, --no-update / SILV_NO_UPDATE=true disables checks)"
                .to_owned(),
            editor,
        });
    }

    pub fn close_repo_prompt(&mut self) {
        self.repo_prompt = None;
    }

    /// Applies the typed repository url and re-checks in the background.
    /// The submit supersedes a check that is still in flight: its answer is about
    /// the previous repository and is dropped when it lands.
    pub fn submit_repo_url(&mut self) {
        let text = self.repo_prompt.as_ref().map(|prompt| prompt.editor.text());
        let Some(text) = text else { return };
        match text.parse::<RepoRef>() {
            Ok(repo) => {
                self.message = Some(format!("update: checking {} ...", repo.url()));
                self.update_repo = repo;
                self.update_available = None;
                self.update_checking = false;
                self.update_check_generation = self.update_check_generation.wrapping_add(1);
                self.close_repo_prompt();
                self.start_update_check();
            },
            Err(reason) => {
                self.message = Some(format!("update: {reason}"));
            },
        }
    }

    pub fn open_changelog(&mut self) {
        if self.update_available.is_some() {
            self.changelog = Some(ChangelogState::default());
        }
    }

    pub fn close_changelog(&mut self) {
        self.changelog = None;
    }

    pub fn changelog_move(&mut self, delta: isize) {
        let Some(changelog) = self.changelog.as_mut() else {
            return;
        };
        let max = changelog.content_height.saturating_sub(changelog.page.max(1));
        let current = isize::try_from(changelog.scroll).unwrap_or(0);
        changelog.scroll = usize::try_from(current + delta)
            .unwrap_or(if delta < 0 { 0 } else { max })
            .min(max);
    }

    pub fn changelog_first(&mut self) {
        if let Some(changelog) = self.changelog.as_mut() {
            changelog.scroll = 0;
        }
    }

    pub fn changelog_last(&mut self) {
        let max = self
            .changelog
            .as_ref()
            .map(|c| c.content_height.saturating_sub(c.page.max(1)))
            .unwrap_or(0);
        if let Some(changelog) = self.changelog.as_mut() {
            changelog.scroll = max;
        }
    }

    fn handle_changelog_key(&mut self, key: KeyPress) {
        let page = self
            .changelog
            .as_ref()
            .map(|c| isize::try_from(c.page.max(1)).unwrap_or(1))
            .unwrap_or(1);
        match key.bind {
            KeyCode::Esc | KeyCode::Char('q') => self.close_changelog(),
            KeyCode::Char('j') | KeyCode::Down => self.changelog_move(1),
            KeyCode::Char('k') | KeyCode::Up => self.changelog_move(-1),
            KeyCode::Char('f') | KeyCode::PageDown => self.changelog_move(page),
            KeyCode::Char('b') | KeyCode::PageUp => self.changelog_move(-page),
            KeyCode::Char('g') => self.changelog_first(),
            KeyCode::Char('G') => self.changelog_last(),
            _ => {},
        }
    }

    fn handle_repo_prompt_key(&mut self, key: KeyPress) {
        let Some(prompt) = self.repo_prompt.as_mut() else {
            return;
        };
        match key.bind {
            KeyCode::Char(_) => {
                if let Some(ch) = key.typed_char() {
                    prompt.editor.insert(ch);
                }
            },
            KeyCode::Backspace => {
                prompt.editor.backspace();
            },
            KeyCode::Delete => {
                prompt.editor.delete();
            },
            KeyCode::Left => prompt.editor.left(),
            KeyCode::Right => prompt.editor.right(),
            KeyCode::Home => prompt.editor.home(),
            KeyCode::End => prompt.editor.end(),
            KeyCode::Enter => self.submit_repo_url(),
            KeyCode::Esc => self.close_repo_prompt(),
            _ => {},
        }
    }
}

fn contains(area: Rect, row: u16) -> bool {
    area.height > 0 && row >= area.top() && row < area.top() + area.height
}

/// The field path a column addresses; pseudo columns parse from their labels.
fn column_path(column: &Column) -> FieldPath {
    match column {
        Column::Field(path) => path.clone(),
        other => FieldPath::parse(&other.label()).unwrap_or_else(|_| FieldPath::from_key(&other.label())),
    }
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
        assert_eq!(state.focus, Focus::Rows);
        assert!(state.status.contains("rows 2/2 of 2"));
    }

    #[test]
    fn slash_focuses_search_and_typing_narrows_the_view() {
        let mut state = state_of(&["{\"message\":\"disk full\"}", "{\"message\":\"ok\"}"]);
        state.handle_key(KeyCode::Char('/'), KeyModifiers::NONE);
        assert_eq!(state.focus, Focus::Search);
        for ch in "disk".chars() {
            state.handle_key(KeyCode::Char(ch), KeyModifiers::NONE);
        }
        assert_eq!(state.search_input.text(), "disk");
        assert_eq!(state.view.len(), 1);
        state.handle_key(KeyCode::Enter, KeyModifiers::NONE);
        assert_eq!(state.focus, Focus::Rows);
        assert_eq!(state.view.len(), 1);
    }

    #[test]
    fn russian_layout_keys_drive_the_bindings() {
        let mut state = state_of(&["{\"a\":1}", "{\"a\":2}", "{\"a\":3}"]);
        // Cyrillic letters land on the QWERTY key that carries the binding.
        state.handle_key(KeyCode::Char('о'), KeyModifiers::NONE); // physical j
        assert_eq!(state.cursor, 1);
        state.handle_key(KeyCode::Char('к'), KeyModifiers::NONE); // physical r
        assert!(state.search.regex);
        state.handle_key(KeyCode::Char('.'), KeyModifiers::NONE); // physical / focuses search
        assert_eq!(state.focus, Focus::Search);
        state.handle_key(KeyCode::Char('й'), KeyModifiers::NONE); // physical q types text here
        assert_eq!(state.search_input.text(), "й");
        state.handle_key(KeyCode::Char('й'), KeyModifiers::SHIFT); // physical ? closes help
        state.focus = Focus::Rows;
        state.handle_key(KeyCode::Char(','), KeyModifiers::SHIFT); // physical / with Shift is ?
        assert!(state.help);
        state.handle_key(KeyCode::Char('й'), KeyModifiers::NONE); // physical q closes help
        assert!(!state.help);
        state.handle_key(KeyCode::Char('с'), KeyModifiers::CONTROL); // physical Ctrl-C quits
        assert!(state.quit);
    }

    #[test]
    fn shift_s_focuses_the_field_header_from_rows() {
        let mut state = state_of(&["{\"a\":1}", "{\"b\":2}"]);
        assert_eq!(state.focus, Focus::Rows);
        state.handle_key(KeyCode::Char('S'), KeyModifiers::SHIFT);
        assert_eq!(state.focus, Focus::Header);
        state.handle_key(KeyCode::Esc, KeyModifiers::NONE);
        assert_eq!(state.focus, Focus::Rows);
    }

    #[test]
    fn russian_layout_shift_s_focuses_the_field_header() {
        let mut state = state_of(&["{\"a\":1}"]);
        state.handle_key(KeyCode::Char('Ы'), KeyModifiers::SHIFT); // the physical S key
        assert_eq!(state.focus, Focus::Header);
    }

    #[test]
    fn russian_layout_text_entry_keeps_cyrillic() {
        let mut state = state_of(&["{\"message\":\"ошибка диска\"}", "{\"message\":\"ok\"}"]);
        state.handle_key(KeyCode::Char('.'), KeyModifiers::NONE); // physical / focuses search
        assert_eq!(state.focus, Focus::Search);
        for ch in "ошиб".chars() {
            state.handle_key(KeyCode::Char(ch), KeyModifiers::NONE);
        }
        assert_eq!(state.search_input.text(), "ошиб");
        assert_eq!(state.view.len(), 1);
    }

    #[test]
    fn raw_key_map_matches_the_typed_character() {
        let mut state = state_of(&["{\"a\":1}", "{\"a\":2}"]);
        state.config.key_map = KeyMapMode::Raw;
        state.handle_key(KeyCode::Char('о'), KeyModifiers::NONE); // physical j, unbound in raw
        assert_eq!(state.cursor, 0);
        state.handle_key(KeyCode::Char('j'), KeyModifiers::NONE);
        assert_eq!(state.cursor, 1);
    }

    #[test]
    fn wheel_scrolls_and_clamps() {
        let records: Vec<String> = (0..10).map(|i| format!("{{\"a\":{i}}}")).collect();
        let refs: Vec<&str> = records.iter().map(String::as_str).collect();
        let mut state = state_of(&refs);
        state.wheel(1);
        assert_eq!(state.cursor, 3);
        state.wheel(-5);
        assert_eq!(state.cursor, 0);
        state.wheel(-1);
        assert_eq!(state.cursor, 0);
        state.wheel(100);
        assert_eq!(state.cursor, 9);
    }

    #[test]
    fn wizard_walks_field_operator_pattern() {
        let mut state = state_of(&["{\"level\":\"INFO\"}", "{\"level\":\"WARN\"}"]);
        state.handle_key(KeyCode::Char('F'), KeyModifiers::NONE);
        assert!(state.wizard.is_some());
        for ch in "lev".chars() {
            state.handle_key(KeyCode::Char(ch), KeyModifiers::NONE);
        }
        assert!(
            state
                .wizard
                .as_ref()
                .is_some_and(|w| w.matches.iter().any(|c| c.label == "level"))
        );
        state.handle_key(KeyCode::Enter, KeyModifiers::NONE);
        assert!(state.wizard.as_ref().is_some_and(|w| w.phase == WizardPhase::Operator));
        state.handle_key(KeyCode::Enter, KeyModifiers::NONE);
        assert!(state.wizard.as_ref().is_some_and(|w| w.phase == WizardPhase::Pattern));
        for ch in "WARN".chars() {
            state.handle_key(KeyCode::Char(ch), KeyModifiers::NONE);
        }
        state.handle_key(KeyCode::Enter, KeyModifiers::NONE);
        assert!(state.wizard.is_none());
        assert_eq!(state.filters.len(), 1);
        assert!(state.filters[0].to_string().contains("level=WARN"));
        assert_eq!(state.view.len(), 1);
    }

    #[test]
    fn wizard_exists_completes_immediately() {
        let mut state = state_of(&["{\"level\":\"WARN\",\"trace.id\":\"t1\"}"]);
        state.open_filter_wizard();
        state.wizard_enter(); // field: first candidate
        state.wizard_move(4); // Exists
        state.wizard_enter();
        assert!(state.wizard.is_none());
        assert_eq!(state.filters.len(), 1);
        assert!(state.filters[0].to_string().contains('^'));
    }

    #[test]
    fn wizard_back_walks_out_one_phase_at_a_time() {
        let mut state = state_of(&["{\"level\":\"WARN\"}"]);
        state.open_filter_wizard();
        state.wizard_enter();
        state.wizard_enter();
        assert!(state.wizard.as_ref().is_some_and(|w| w.phase == WizardPhase::Pattern));
        state.wizard_back();
        assert!(state.wizard.as_ref().is_some_and(|w| w.phase == WizardPhase::Operator));
        state.wizard_back();
        assert!(state.wizard.as_ref().is_some_and(|w| w.phase == WizardPhase::Field));
    }

    #[test]
    fn detail_menu_enter_applies_the_action() {
        let mut state = state_of(&["{\"level\":\"WARN\",\"trace.id\":\"t1\"}"]);
        state.open_detail();
        state.detail_move(1); // focus @level (field list starts with @timestamp)
        state.handle_key(KeyCode::Enter, KeyModifiers::NONE); // open the action menu
        assert!(state.detail.as_ref().is_some_and(|d| d.menu == Some(0)));
        state.detail_move(1); // menu cursor: FilterEq
        state.handle_key(KeyCode::Enter, KeyModifiers::NONE); // apply
        assert!(state.detail.as_ref().is_some_and(|d| d.menu.is_none()));
        assert_eq!(state.filters.len(), 1);
        assert!(state.filters[0].to_string().contains("level=WARN"));
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
        assert_eq!(state.filters.len(), 2);
        state.handle_key(KeyCode::Char('z'), CONTROL);
        assert_eq!(state.filters.len(), 1);
        state.handle_key(KeyCode::Char('z'), CONTROL);
        assert_eq!(state.filters.len(), 0);
        assert_eq!(state.view.len(), 2);
        state.handle_key(KeyCode::Char('y'), CONTROL);
        assert_eq!(state.filters.len(), 1);
        state.handle_key(KeyCode::Char('y'), CONTROL);
        assert_eq!(state.filters.len(), 2);
        assert!(state.message.as_deref().unwrap_or_default().contains("redone"));
    }

    #[test]
    fn ctrl_z_restores_a_cleared_list_and_a_new_change_ends_the_redo() {
        let mut state = state_of(&["{\"level\":\"INFO\"}", "{\"level\":\"WARN\"}"]);
        state.add_filter(level_filter(&state, "INFO"));
        state.add_filter(level_filter(&state, "WARN"));
        state.clear_filters();
        assert!(state.filters.is_empty());
        state.handle_key(KeyCode::Char('z'), CONTROL);
        assert_eq!(state.filters.len(), 2);
        state.remove_filter(0);
        state.add_filter(level_filter(&state, "INFO"));
        state.handle_key(KeyCode::Char('y'), CONTROL);
        assert_eq!(state.filters.len(), 2);
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
        assert!(state.filters.is_empty());
        state.handle_key(KeyCode::Char('н'), CONTROL); // the physical y key
        assert_eq!(state.filters.len(), 1);
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

    fn columns_of(state: &AppState) -> Vec<String> {
        state.columns.iter().map(|column| column.label()).collect()
    }

    fn with_columns(state: &mut AppState, names: &[&str]) {
        state.columns.clear();
        for name in names {
            state.columns.push(Column::Field(FieldPath::parse(name).expect("path")));
        }
        state.header_cursor = 0;
    }

    #[test]
    fn move_column_swaps_and_follows_cursor() {
        let mut state = state_of(&["{\"a\":1}"]);
        with_columns(&mut state, &["a", "b", "c"]);
        state.move_column(1);
        assert_eq!(columns_of(&state), ["b", "a", "c"]);
        assert_eq!(state.header_cursor, 1);
        state.move_column(-1);
        assert_eq!(columns_of(&state), ["a", "b", "c"]);
        assert_eq!(state.header_cursor, 0);
        state.move_column(-1);
        assert_eq!(columns_of(&state), ["a", "b", "c"]);
    }

    #[test]
    fn header_drag_flow_moves_columns_then_sorts() {
        let mut state = state_of(&["{\"a\":2}", "{\"a\":1}"]);
        with_columns(&mut state, &["a", "b"]);
        state.focus = Focus::Header;
        state.handle_key(KeyCode::Char(' '), KeyModifiers::NONE);
        assert_eq!(state.header_mode, HeaderMode::Drag);
        assert!(state.status.contains("fields (drag)"));
        state.handle_key(KeyCode::Right, KeyModifiers::NONE);
        assert_eq!(columns_of(&state), ["b", "a"]);
        state.handle_key(KeyCode::Enter, KeyModifiers::NONE);
        assert_eq!(state.header_mode, HeaderMode::Navigate);
        assert_eq!(state.sorts.len(), 1);
        assert_eq!(state.sorts[0].field, FieldPath::parse("a").expect("path"));
    }

    #[test]
    fn header_click_enters_drag_and_second_click_sorts() {
        use ratatui::crossterm::event::MouseButton;
        let mut state = state_of(&["{\"a\":2}", "{\"a\":1}"]);
        with_columns(&mut state, &["a"]);
        state.areas.rows = Rect::new(0, 2, 40, 10);
        state.header_cells.push((0, 8, 0));
        let click = MouseEventKind::Down(MouseButton::Left);
        state.handle_mouse(click, 1, 2);
        assert_eq!(state.focus, Focus::Header);
        assert_eq!(state.header_mode, HeaderMode::Drag);
        assert!(state.sorts.is_empty());
        state.handle_mouse(click, 1, 2);
        assert_eq!(state.sorts.len(), 1);
        assert_eq!(state.header_mode, HeaderMode::Navigate);
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
        state.focus = Focus::Rows;
        state.filter_cursor = 0;
        state.areas.chips = Rect::new(0, 1, 40, 2);
        state.chip_cells.push((0, 10, 0, 1));
        state.chip_cells.push((11, 22, 1, 1));
        let click = MouseEventKind::Down(MouseButton::Left);
        state.handle_mouse(click, 15, 1);
        assert_eq!(state.filter_cursor, 0);
        state.handle_mouse(click, 15, 2);
        assert_eq!(state.filter_cursor, 1);
    }

    #[test]
    fn status_sort_marks_are_structured() {
        let mut state = state_of(&["{\"a\":2}"]);
        let column = Column::Field(FieldPath::parse("a").expect("path"));
        state.columns.push(column.clone());
        state.cycle_sort(&column);
        assert_eq!(state.sort_marks, vec![("a".to_owned(), SortDir::Asc)]);
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
        state.focus = Focus::Rows;
        state.handle_key(KeyCode::Char(' '), KeyModifiers::NONE);
        assert_eq!(state.row_mode, RowMode::Pick);
        assert!(state.prompts_text().contains("pick value"));
        state.handle_key(KeyCode::Right, KeyModifiers::NONE);
        assert_eq!(state.pick_cursor, 0);
        state.handle_key(KeyCode::Enter, KeyModifiers::NONE);
        assert_eq!(state.row_mode, RowMode::Normal);
        assert!(
            state
                .wizard
                .as_ref()
                .is_some_and(|w| w.phase == WizardPhase::Operator && w.prefill.is_some())
        );
        assert_eq!(state.focus, Focus::FilterInput);
        state.handle_key(KeyCode::Enter, KeyModifiers::NONE); // Eq: the pattern step is prefilled
        assert_eq!(state.filter_input.text(), "INFO");
        state.handle_key(KeyCode::Enter, KeyModifiers::NONE); // commit
        assert_eq!(state.filters.len(), 1);
        assert_eq!(state.filters[0].to_string(), "level=INFO");
        assert_eq!(state.view.len(), 1);
    }

    #[test]
    fn pick_value_esc_leaves_the_mode() {
        let mut state = state_of(&["{\"a\":1}"]);
        state.columns.clear();
        state.columns.push(Column::Field(FieldPath::parse("a").expect("path")));
        state.focus = Focus::Rows;
        state.handle_key(KeyCode::Char(' '), KeyModifiers::NONE);
        assert_eq!(state.row_mode, RowMode::Pick);
        state.handle_key(KeyCode::Esc, KeyModifiers::NONE);
        assert_eq!(state.row_mode, RowMode::Normal);
    }

    #[test]
    fn prompt_line_tracks_the_active_mode() {
        let mut state = state_of(&["{\"a\":1}"]);
        state.columns.clear();
        state.columns.push(Column::Field(FieldPath::parse("a").expect("path")));
        state.focus = Focus::Rows;
        assert!(state.prompts_text().contains("[Space] select value to filter by"));
        state.focus = Focus::Header;
        assert!(state.prompts_text().contains("[Space] column drag"));
        state.handle_key(KeyCode::Char(' '), KeyModifiers::NONE);
        assert!(state.prompts_text().contains("column drag"));
        state.focus = Focus::Search;
        assert!(state.prompts_text().is_empty());
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
    fn picker_space_toggles_like_enter() {
        let mut state = state_of(&["{\"alpha\":1,\"beta\":2}"]);
        state.open_picker();
        state.handle_key(KeyCode::Char(' '), KeyModifiers::NONE);
        let picker = state.picker.as_ref().expect("picker stays open");
        assert!(picker.input.text().is_empty());
        assert_eq!(picker.matches.len(), 2);
        assert!(picker.fields[picker.matches[0]].selected);
        state.handle_key(KeyCode::Char(' '), KeyModifiers::NONE);
        assert!(!state.picker.as_ref().expect("picker").fields.iter().any(|f| f.selected));
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
        state.updating = true;
        assert!(!state.update_line_visible());
        state.updating = false;
        state.config.no_update = true;
        assert!(!state.update_line_visible());
    }

    #[test]
    fn up_to_date_check_leaves_no_line_and_no_prompt() {
        let mut state = state_of(&["{\"a\":1}"]);
        state.apply_update_event(UpdateEvent::Checked(0, UpdateOutcome::UpToDate));
        assert!(!state.update_line_visible());
        assert!(state.repo_prompt.is_none());
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
        assert!(state.repo_prompt.is_some());
        assert!(state.message.as_deref().is_some_and(|m| m.contains("offline")));
        state.close_repo_prompt();
        state.handle_key(KeyCode::Char('u'), KeyModifiers::NONE);
        assert!(state.repo_prompt.is_some());
    }

    #[test]
    fn url_prompt_submit_saves_repo_and_keeps_focus_on_the_prompt_until_ok() {
        let mut state = state_of(&["{\"a\":1}"]);
        state.config.no_update = true; // suppresses the background check thread
        state.repo_prompt = Some(RepoPrompt {
            message: String::new(),
            editor: Editor::with_text("not a url!!"),
        });
        state.handle_key(KeyCode::Enter, KeyModifiers::NONE);
        assert!(state.repo_prompt.is_some(), "a bad url keeps the prompt open");
        assert!(
            state
                .message
                .as_deref()
                .is_some_and(|m| m.contains("not a repository url"))
        );
        if let Some(prompt) = state.repo_prompt.as_mut() {
            prompt.editor.set("you/fork");
        }
        state.handle_key(KeyCode::Enter, KeyModifiers::NONE);
        assert!(state.repo_prompt.is_none());
        assert_eq!(state.update_repo.url(), "https://github.com/you/fork");
        assert_eq!(state.update_repo.author, "you");
        assert_eq!(state.update_repo.repo, "fork");
        assert!(!state.update_checking, "no_update suppresses the check thread");
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
        state.areas.update = Rect {
            x: 0,
            y: 20,
            width: 80,
            height: 1,
        };
        state.changelog_btn = Rect {
            x: 70,
            y: 20,
            width: 10,
            height: 1,
        };
        state.handle_mouse(MouseEventKind::Down(MouseButton::Left), 72, 20);
        assert!(state.changelog.is_some());
        {
            let changelog = state.changelog.as_mut().expect("open");
            changelog.content_height = 100;
            changelog.page = 10;
        }
        state.handle_key(KeyCode::Char('j'), KeyModifiers::NONE);
        assert_eq!(state.changelog.as_ref().expect("open").scroll, 1);
        state.handle_key(KeyCode::Char('G'), KeyModifiers::NONE);
        assert_eq!(state.changelog.as_ref().expect("open").scroll, 90);
        state.handle_key(KeyCode::Char('g'), KeyModifiers::NONE);
        assert_eq!(state.changelog.as_ref().expect("open").scroll, 0);
        state.handle_mouse(MouseEventKind::ScrollDown, 40, 10);
        assert_eq!(state.changelog.as_ref().expect("open").scroll, 3);
        state.handle_key(KeyCode::Esc, KeyModifiers::NONE);
        assert!(state.changelog.is_none());
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
        state.updating = true;
        state.apply_update_event(UpdateEvent::Performed(PerformOutcome::Updated {
            version: "9.9.9".to_owned(),
        }));
        assert!(!state.updating);
        assert!(state.update_available.is_none());
        assert!(state.message.as_deref().is_some_and(|m| m.contains("restart")));
    }

    #[test]
    fn start_update_is_inert_when_updates_are_disabled() {
        let mut state = state_of(&["{\"a\":1}"]);
        state.config.no_update = true;
        state.updating = false;
        state.start_update();
        assert!(!state.updating, "no_update never spawns a worker");
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
        state.update_cargo_managed = true;
        // The click and Enter on the focused button both route here: they must not self-update,
        // that would replace a binary cargo tracks and desync its install list.
        state.start_update();
        assert!(!state.updating, "a cargo install is never self-updated");
        let message = state.message.as_deref().expect("a message explains the refusal");
        assert!(message.contains("cargo install --git"), "the cargo command is given: {message}");
        assert!(message.contains(&state.update_repo.url()), "the command names the repository: {message}");
    }

    #[test]
    fn a_repo_url_submitted_mid_check_supersedes_the_in_flight_one() {
        let mut state = state_of(&["{\"a\":1}"]);
        state.config.no_update = true; // suppresses the check thread
        // A startup check of the default repository is in flight (generation 1).
        state.update_checking = true;
        state.update_check_generation = 1;
        state.repo_prompt = Some(RepoPrompt {
            message: String::new(),
            editor: Editor::with_text("you/fork"),
        });
        state.handle_key(KeyCode::Enter, KeyModifiers::NONE);
        assert_eq!(state.update_repo.url(), "https://github.com/you/fork", "the url is applied at once");
        assert_eq!(state.update_check_generation, 2, "the submit starts a new generation");
        // no_update suppressed the spawned check; mark a new one in flight to test
        // that a stale answer cannot clear the flag of the running one.
        state.update_checking = true;
        // The old check answers late, about the default repository: it must not drive the ui.
        state.apply_update_event(UpdateEvent::Checked(
            1,
            UpdateOutcome::Unreachable {
                repo: "https://github.com/chamoretto/silv".to_owned(),
                reason: "late".to_owned(),
            },
        ));
        assert!(state.repo_prompt.is_none(), "the stale answer must not reopen the prompt");
        state.apply_update_event(UpdateEvent::Checked(
            1,
            UpdateOutcome::Available {
                latest: "9.9.9".to_owned(),
                notes: release_notes("9.9.9"),
            },
        ));
        assert!(
            state.update_available.is_none(),
            "the stale answer must not show the update line for the old repository"
        );
        assert!(state.update_checking, "a stale event must not clear the in-flight flag");
        // The new generation's answer applies.
        state.apply_update_event(UpdateEvent::Checked(
            2,
            UpdateOutcome::Available {
                latest: "9.9.9".to_owned(),
                notes: release_notes("9.9.9"),
            },
        ));
        assert!(state.update_available.is_some(), "the current generation drives the ui");
        assert!(!state.update_checking);
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
        assert_eq!(state.focus, Focus::Rows);
        state.handle_key(KeyCode::Tab, KeyModifiers::NONE);
        assert_eq!(state.focus, Focus::UpdateLine, "Tab reaches the update line from the rows");
        state.handle_key(KeyCode::Left, KeyModifiers::NONE);
        assert_eq!(state.update_focus_index, 0, "Left selects the update button");
        state.handle_key(KeyCode::Right, KeyModifiers::NONE);
        assert_eq!(state.update_focus_index, 1, "Right selects the changelog button");
        state.handle_key(KeyCode::Enter, KeyModifiers::NONE);
        assert!(state.changelog.is_some(), "Enter pressed the focused changelog button");
        assert!(!state.updating, "Enter on the changelog never starts an update");
        state.handle_key(KeyCode::Esc, KeyModifiers::NONE);
        assert!(state.changelog.is_none());
        state.handle_key(KeyCode::Esc, KeyModifiers::NONE);
        assert_eq!(state.focus, Focus::Rows, "Esc leaves the update line");
        state.handle_key(KeyCode::BackTab, KeyModifiers::NONE);
        assert_ne!(state.focus, Focus::UpdateLine, "BackTab cycles from the rows, not to the line");
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
        state.focus = Focus::UpdateLine;
        state.updating = true; // the line hides while updating
        state.apply_update_event(UpdateEvent::Performed(PerformOutcome::UpToDate));
        assert_eq!(state.focus, Focus::Rows, "the focus leaves a line that is no longer drawn");
    }

    #[test]
    fn the_tab_cycle_skips_the_update_line_while_it_is_hidden() {
        let mut state = state_of(&["{\"a\":1}"]);
        for _ in 0..8 {
            state.handle_key(KeyCode::Tab, KeyModifiers::NONE);
            assert_ne!(state.focus, Focus::UpdateLine, "no update line, no stop at it");
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
        state.updating = true;
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
        assert!(state.repo_prompt.is_none(), "the repository is not the problem here");
    }

    #[test]
    fn the_repo_prompt_is_inert_to_clicks_and_closes_on_esc() {
        let mut state = state_of(&["{\"a\":1}"]);
        state.open_repo_prompt();
        assert!(state.repo_prompt.is_some());
        state.handle_mouse(MouseEventKind::Down(ratatui::crossterm::event::MouseButton::Left), 40, 12);
        assert!(state.repo_prompt.is_some(), "a click outside must not close the text prompt");
        state.handle_key(KeyCode::Esc, KeyModifiers::NONE);
        assert!(state.repo_prompt.is_none(), "Esc closes it, like every other window");
    }
}
