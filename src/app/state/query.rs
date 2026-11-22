//! Query pipeline state: search, filters, the filter wizard and their key handling.

use super::*;

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

impl AppState {
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
        clamp_move(&mut wizard.cursor, len, delta);
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
            clamp_cursor(&mut self.filter_cursor, self.filters.len());
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
        clamp_cursor(&mut self.filter_cursor, self.filters.len());
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
    pub(super) fn handle_search_key(&mut self, key: KeyPress) {
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

    pub(super) fn handle_filter_key(&mut self, key: KeyPress) {
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

    pub(super) fn handle_chips_key(&mut self, key: KeyPress) {
        match key.bind {
            KeyCode::Left | KeyCode::Char('h') => clamp_move(&mut self.filter_cursor, self.filters.len(), -1),
            KeyCode::Right | KeyCode::Char('l') => {
                clamp_move(&mut self.filter_cursor, self.filters.len(), 1);
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
}

/// Escapes glob metacharacters so a literal value matches itself.
pub fn glob_escape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        if matches!(ch, '*' | '?' | '[' | ']' | '{' | '}' | '\\') {
            out.push('\\');
        }
        out.push(ch);
    }
    out
}

