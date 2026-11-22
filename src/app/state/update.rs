//! Self-update state: the update line, changelog overlay, repository prompt, worker events.

use super::*;

/// The changelog overlay: release notes between the running and the newest version.
/// `content_height`/`page` are recorded while drawing so keys and wheel can clamp scroll;
/// `lines` caches the wrapped notes for `width`, and rebuilds when `width` or `version`
/// change. The cached notes are the notes at overlay-open time: `update_available` is
/// fixed while a modal is open (checks only drive the state between modals), so the
/// cache can never render a note that is not on the update line.
#[derive(Clone, Debug, Default)]
pub struct ChangelogState {
    pub scroll: usize,
    pub content_height: usize,
    pub page: usize,
    /// Cached wrapped rendering of the notes; `width` is the width it was wrapped to,
    /// `version` the newest release it was built from.
    pub width: usize,
    pub version: String,
    pub lines: Vec<Line<'static>>,
}

/// The repository url pop-up: shown when the release repo is unreachable, opened with `u`.
#[derive(Clone, Debug)]
pub struct RepoPrompt {
    pub message: String,
    pub editor: Editor,
}

impl AppState {
    /// The focused bottom update line: Left/Right choose the button, Enter presses it.
    pub(super) fn handle_update_line_key(&mut self, key: KeyPress) {
        if !self.update_line_visible() {
            self.nav.focus = Focus::Rows;
            return;
        }
        match key.bind {
            KeyCode::Char('q') => self.quit(),
            KeyCode::Left | KeyCode::Char('h') => self.update.focus_index = 0,
            KeyCode::Right | KeyCode::Char('l') => self.update.focus_index = 1,
            KeyCode::Tab => self.cycle_focus(1),
            KeyCode::BackTab => self.cycle_focus(-1),
            KeyCode::Esc => self.nav.focus = Focus::Rows,
            KeyCode::Enter => {
                if self.update.focus_index == 0 {
                    self.start_update();
                } else {
                    self.open_changelog();
                }
            },
            KeyCode::Char('u') if !self.config.no_update => self.open_repo_prompt(),
            _ => {},
        }
    }
    /// True when the bottom update line (update + changelog buttons) is drawn.
    pub fn update_line_visible(&self) -> bool {
        !self.config.no_update && self.update.available.is_some() && !self.update.updating
    }

    /// The newest available version, when an update is available.
    pub fn update_latest(&self) -> Option<&str> {
        self.update
            .available
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
        self.update.check_generation = self.update.check_generation.wrapping_add(1);
        let generation = self.update.check_generation;
        let tx = self.update.events.0.clone();
        let target = UpdateTarget::new(self.update.repo.clone());
        std::thread::spawn(move || {
            let outcome = target.check();
            let _ = tx.send(UpdateEvent::Checked(generation, outcome));
        });
    }

    /// Spawns the download-and-replace worker, triggered by the update button.
    /// A cargo-managed install is not self-updated: cargo tracks its version, so the
    /// button prints the cargo command that keeps the bookkeeping in sync.
    pub fn start_update(&mut self) {
        if self.config.no_update || self.update.updating {
            return;
        }
        if self.update.cargo_managed {
            self.say(format!(
                "update: this binary is a cargo install, update with `cargo install --git {} --force` \
                 (a self-update would desync cargo's install list)",
                self.update.repo.url()
            ));
            return;
        }
        self.update.updating = true;
        self.update.changelog = None;
        // The line hides while updating; a focus on it returns to the rows.
        if self.nav.focus == Focus::UpdateLine {
            self.nav.focus = Focus::Rows;
        }
        let latest = self.update_latest().unwrap_or("the latest release").to_owned();
        self.message = Some(format!("update: updating to {latest} ..."));
        let version = self.update_latest().map(str::to_owned);
        let tx = self.update.events.0.clone();
        let target = UpdateTarget::new(self.update.repo.clone());
        std::thread::spawn(move || {
            let outcome = target.perform(version.as_deref());
            let _ = tx.send(UpdateEvent::Performed(outcome));
        });
    }

    /// Applies update worker events that arrived since the last poll.
    /// Returns true when at least one event changed the state.
    pub fn poll_update_events(&mut self) -> bool {
        let mut applied = false;
        while let Ok(event) = self.update.events.1.try_recv() {
            self.apply_update_event(event);
            applied = true;
        }
        applied
    }

    pub fn apply_update_event(&mut self, event: UpdateEvent) {
        match event {
            UpdateEvent::Checked(generation, outcome) => {
                if generation != self.update.check_generation {
                    // A repository url was submitted while this check ran: its answer
                    // is about the old repository, drop it.
                    return;
                }
                match outcome {
                    UpdateOutcome::Available { notes, .. } => {
                        // No status bar message: the update line itself is the prompt,
                        // the rows line stays clean.
                        self.update.available = Some(notes);
                    },
                    UpdateOutcome::UpToDate => {
                        // The running version is the newest release: clear a line a
                        // previous check painted (a rollback of the repository's releases).
                        self.update.available = None;
                        self.update.changelog = None;
                    },
                    UpdateOutcome::Unreachable { repo, reason } => {
                        self.message = Some(format!("update: {repo} is unreachable: {reason}"));
                        self.open_repo_prompt();
                    },
                }
                if self.nav.focus == Focus::UpdateLine && !self.update_line_visible() {
                    self.nav.focus = Focus::Rows;
                }
            },
            UpdateEvent::Performed(outcome) => {
                self.update.updating = false;
                self.update.quit_confirm_pending = false;
                match outcome {
                    PerformOutcome::Updated { version } => {
                        self.update.available = None;
                        self.update.changelog = None;
                        self.message = Some(format!("update: silv updated to {version}, restart silv to use it"));
                    },
                    PerformOutcome::UpToDate => {
                        self.update.available = None;
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
                if self.nav.focus == Focus::UpdateLine && !self.update_line_visible() {
                    self.nav.focus = Focus::Rows;
                }
            },
        }
    }

    /// Opens the repository url pop-up (also the answer to an unreachable repo).
    pub fn open_repo_prompt(&mut self) {
        if self.config.no_update {
            return;
        }
        let mut editor = Editor::with_text(&self.update.repo.url());
        if self.update.repo_prompt.is_none() {
            editor.clear();
        }
        self.update.repo_prompt = Some(RepoPrompt {
            message: "enter repository url: owner/repo or https://github.com/owner/repo \
                      (Esc closes, --no-update / SILV_NO_UPDATE=true disables checks)"
                .to_owned(),
            editor,
        });
    }

    pub fn close_repo_prompt(&mut self) {
        self.update.repo_prompt = None;
    }

    /// Applies the typed repository url and re-checks in the background.
    /// The submit supersedes a check that is still in flight: its answer is about
    /// the previous repository and is dropped when it lands.
    pub fn submit_repo_url(&mut self) {
        let text = self.update.repo_prompt.as_ref().map(|prompt| prompt.editor.text());
        let Some(text) = text else { return };
        match text.parse::<RepoRef>() {
            Ok(repo) => {
                self.message = Some(format!("update: checking {} ...", repo.url()));
                self.update.repo = repo;
                self.update.available = None;
                self.update.check_generation = self.update.check_generation.wrapping_add(1);
                self.close_repo_prompt();
                // The line hid with the old notes: a focus parked on it returns to the rows.
                if self.nav.focus == Focus::UpdateLine && !self.update_line_visible() {
                    self.nav.focus = Focus::Rows;
                }
                self.start_update_check();
            },
            Err(reason) => {
                self.message = Some(format!("update: {reason}"));
            },
        }
    }

    pub fn open_changelog(&mut self) {
        if let Some(notes) = &self.update.available {
            self.update.changelog = Some(ChangelogState {
                version: notes.first().map(|note| note.version.clone()).unwrap_or_default(),
                ..Default::default()
            });
        }
    }

    pub fn close_changelog(&mut self) {
        self.update.changelog = None;
    }

    pub fn changelog_move(&mut self, delta: isize) {
        let Some(changelog) = self.update.changelog.as_mut() else {
            return;
        };
        let max = changelog.content_height.saturating_sub(changelog.page.max(1));
        let current = isize::try_from(changelog.scroll).unwrap_or(0);
        changelog.scroll = usize::try_from(current + delta)
            .unwrap_or(if delta < 0 { 0 } else { max })
            .min(max);
    }

    pub fn changelog_first(&mut self) {
        if let Some(changelog) = self.update.changelog.as_mut() {
            changelog.scroll = 0;
        }
    }

    pub fn changelog_last(&mut self) {
        let max = self
            .update
            .changelog
            .as_ref()
            .map(|c| c.content_height.saturating_sub(c.page.max(1)))
            .unwrap_or(0);
        if let Some(changelog) = self.update.changelog.as_mut() {
            changelog.scroll = max;
        }
    }

    pub(super) fn handle_changelog_key(&mut self, key: KeyPress) {
        let page = self
            .update
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

    pub(super) fn handle_repo_prompt_key(&mut self, key: KeyPress) {
        let Some(prompt) = self.update.repo_prompt.as_mut() else {
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
