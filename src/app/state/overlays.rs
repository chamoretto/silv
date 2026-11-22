//! Record detail overlay, column picker and their key handling.

use super::*;

/// One field shown in the record detail.
#[derive(Clone, Debug)]
pub struct DetailField {
    pub label: String,
    pub value: String,
    pub style: Sg,
    pub path: Option<FieldPath>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, EnumIter, EnumCount, IntoStaticStr)]
pub enum MenuAction {
    #[strum(to_string = "add or remove column")]
    ToggleColumn,
    #[strum(to_string = "filter: field = value")]
    FilterEq,
    #[strum(to_string = "filter: field != value")]
    FilterNeq,
    #[strum(to_string = "filter: field contains value")]
    FilterContains,
    #[strum(to_string = "filter: field exists")]
    Exists,
    #[strum(to_string = "filter: field missing")]
    NotExists,
}

impl MenuAction {
    pub fn label(self) -> &'static str {
        (&self).into()
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

impl AppState {
    /// Opens the record detail overlay for the focused row.
    pub fn open_detail(&mut self) {
        let Some(record) = self.current_record().cloned() else {
            return;
        };
        let fields = detail_fields(&record, &self.config);
        let json_text = detail_json_lines(&record);
        self.overlays.detail = Some(DetailState {
            row: self.nav.cursor,
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
        self.overlays.detail = None;
        self.dirty = true;
    }

    pub fn detail_move(&mut self, delta: isize) {
        let Some(detail) = self.overlays.detail.as_mut() else {
            return;
        };
        if let Some(menu) = detail.menu.as_mut() {
            clamp_move(menu, MenuAction::COUNT, delta);
            return;
        }
        if detail.json {
            return;
        }
        let len = detail.fields.len();
        if len == 0 {
            return;
        }
        clamp_move(&mut detail.cursor, len, delta);
    }

    pub fn detail_toggle_json(&mut self) {
        if let Some(detail) = self.overlays.detail.as_mut() {
            detail.json = !detail.json;
            self.dirty = true;
        }
    }

    pub fn detail_open_menu(&mut self) {
        if let Some(detail) = self.overlays.detail.as_mut()
            && !detail.json
        {
            detail.menu = Some(0);
            self.dirty = true;
        }
    }

    pub fn detail_selected_field(&self) -> Option<&DetailField> {
        let detail = self.overlays.detail.as_ref()?;
        detail.fields.get(detail.cursor)
    }

    /// Applies the focused menu action to the focused field.
    pub fn detail_apply_menu(&mut self) {
        let menu = self.overlays.detail.as_ref().and_then(|detail| detail.menu);
        let Some(action) = menu.and_then(|menu| MenuAction::iter().nth(menu.min(MenuAction::COUNT - 1)))
        else {
            return;
        };
        let Some((path, value)) = self.detail_field_target() else {
            return;
        };
        if let Some(detail) = self.overlays.detail.as_mut() {
            detail.menu = None;
        }
        self.detail_run_action(action, path, &value);
    }

    /// The focused field of the detail view as (path, value); explains why a
    /// label is unusable as a field path.
    fn detail_field_target(&mut self) -> Option<(FieldPath, String)> {
        let field = self.detail_selected_field().cloned()?;
        let label = field.label;
        let Some(path) = field.path.or_else(|| FieldPath::parse(&label).ok()) else {
            self.message = Some(format!("cannot use `{label}` as a field path"));
            return None;
        };
        Some((path, field.value))
    }

    fn detail_run_action(&mut self, action: MenuAction, path: FieldPath, value: &str) {
        match action {
            MenuAction::ToggleColumn => self.toggle_column(path),
            MenuAction::FilterEq => {
                let probe = truncate_probe(value);
                self.add_filter_from_op(path, LeafOp::Eq, Some(probe));
            },
            MenuAction::FilterNeq => {
                let probe = truncate_probe(value);
                self.add_filter_from_op(path, LeafOp::Neq, Some(probe));
            },
            MenuAction::FilterContains => {
                let probe = truncate_probe(value);
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
        let matches: Vec<usize> = (0..fields.len()).collect();
        self.overlays.picker = Some(PickerState {
            fields,
            matches,
            cursor: 0,
            scroll: 0,
            input: Editor::new(),
        });
        self.dirty = true;
    }

    pub fn close_picker(&mut self) {
        self.overlays.picker = None;
        self.dirty = true;
    }

    pub fn picker_filter(&mut self) {
        let needle = self
            .overlays
            .picker
            .as_ref()
            .map(|picker| picker.input.text().to_lowercase())
            .unwrap_or_default();
        if let Some(picker) = self.overlays.picker.as_mut() {
            picker.matches = (0..picker.fields.len())
                .filter(|index| picker.fields[*index].label.to_lowercase().contains(&needle))
                .collect();
            let matches_len = picker.matches.len();
            clamp_cursor(&mut picker.cursor, matches_len);
            picker.scroll = 0;
        }
        self.dirty = true;
    }

    pub fn picker_toggle(&mut self) {
        let index = match self.overlays.picker.as_ref().and_then(|p| p.matches.get(p.cursor)) {
            Some(index) => *index,
            None => return,
        };
        let path = match self
            .overlays
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
        if let Some(picker) = self.overlays.picker.as_mut()
            && let Some(field) = picker.fields.get_mut(index)
        {
            field.selected = contains;
        }
    }

    pub fn picker_move(&mut self, delta: isize) {
        let Some(picker) = self.overlays.picker.as_mut() else {
            return;
        };
        if picker.matches.is_empty() {
            return;
        }
        clamp_move(&mut picker.cursor, picker.matches.len(), delta);
        self.dirty = true;
    }
    pub(super) fn handle_detail_key(&mut self, key: KeyPress) {
        match key.bind {
            KeyCode::Esc | KeyCode::Char('q') => self.close_detail(),
            KeyCode::Enter => {
                if self
                    .overlays
                    .detail
                    .as_ref()
                    .is_some_and(|detail| detail.menu.is_some())
                {
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
            _ => {
                // Scrolling: arrows, the j/k pair and Page keys; Tab cycles fields.
                let page = isize::try_from(self.page_height()).unwrap_or(1);
                let delta = match key.bind {
                    KeyCode::Char('j') | KeyCode::Tab => Some(1),
                    KeyCode::Char('k') | KeyCode::BackTab => Some(-1),
                    _ => page_scroll(key.bind, page),
                };
                if let Some(delta) = delta {
                    self.detail_move(delta);
                }
            },
        }
    }

    pub(super) fn detail_apply_menu_action(&mut self, action: MenuAction) {
        let target = self.overlays.detail.as_ref().map(|detail| detail.menu).unwrap_or(None);
        if target.is_some() {
            self.detail_apply_menu();
            return;
        }
        let Some((path, value)) = self.detail_field_target() else {
            return;
        };
        self.detail_run_action(action, path, &value);
    }

    pub(super) fn handle_picker_key(&mut self, key: KeyPress) {
        match key.bind {
            KeyCode::Esc => self.close_picker(),
            KeyCode::Tab => self.cycle_focus(1),
            // Space toggles like Enter; it must be matched before the catch-all char arm.
            KeyCode::Char(' ') => self.picker_toggle(),
            KeyCode::Char(_) => {
                if let Some(ch) = key.typed_char() {
                    if let Some(picker) = self.overlays.picker.as_mut() {
                        picker.input.insert(ch);
                    }
                    self.picker_filter();
                }
            },
            KeyCode::Backspace => {
                if let Some(picker) = self.overlays.picker.as_mut() {
                    picker.input.backspace();
                }
                self.picker_filter();
            },
            KeyCode::Enter => self.picker_toggle(),
            _ => {
                let page = isize::try_from(self.page_height()).unwrap_or(1);
                if let Some(delta) = page_scroll(key.bind, page) {
                    self.picker_move(delta);
                    return;
                }
                let edited = self
                    .overlays
                    .picker
                    .as_mut()
                    .is_some_and(|picker| picker.input.handle_edit_key(key));
                if edited {
                    self.picker_filter();
                }
            },
        }
    }
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
