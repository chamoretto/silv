//! Interactive TUI application: state, layout, overlays and the event loop.

pub mod detail;
pub mod help;
pub mod input;
pub mod picker;
pub mod seqfilter;
pub mod state;
pub mod ui;

use std::io::IsTerminal;
use std::time::Duration;

use miette::Context;
use miette::IntoDiagnostic;
use miette::Result;
use ratatui::crossterm::event::DisableMouseCapture;
use ratatui::crossterm::event::EnableMouseCapture;
use ratatui::crossterm::event::Event;
use ratatui::crossterm::event::KeyEventKind;
use ratatui::crossterm::execute;

use crate::app::seqfilter::KeyAction;
use crate::app::seqfilter::SequenceFilter;
use crate::app::state::AppState;
use crate::cli::Config;
use crate::data::LogStore;
use crate::error::SilvError;

/// Poll interval, keeps the UI responsive to resize without busy looping.
const POLL: Duration = Duration::from_millis(120);

/// Runs an interactive session over the given store.
pub fn run(store: LogStore, config: &Config) -> Result<()> {
    if !(std::io::stdin().is_terminal() && std::io::stdout().is_terminal()) {
        return Err(SilvError::NotATerminal.into());
    }
    let mut terminal = ratatui::try_init()
        .into_diagnostic()
        .context("failed to initialize the terminal")?;
    let mut state = AppState::new(store, config.clone());
    let _ = execute!(std::io::stdout(), EnableMouseCapture);

    let result = run_loop(&mut terminal, &mut state).context("interactive session failed");

    let _ = execute!(std::io::stdout(), DisableMouseCapture);
    ratatui::restore();
    result
}

fn run_loop(terminal: &mut ratatui::DefaultTerminal, state: &mut AppState) -> Result<()> {
    state.dirty = true;
    let mut filter = SequenceFilter::new();
    loop {
        if state.dirty {
            terminal
                .draw(|frame| ui::draw(frame, state))
                .map_err(|cause| SilvError::OutputWrite { cause })?;
            state.dirty = false;
        }
        if state.quit {
            return Ok(());
        }
        if ratatui::crossterm::event::poll(POLL).map_err(|cause| SilvError::OutputWrite { cause })? {
            match ratatui::crossterm::event::read().map_err(|cause| SilvError::OutputWrite { cause })? {
                Event::Key(key) if key.kind != KeyEventKind::Release => {
                    let mut action = filter.push(key.code, key.modifiers);
                    while let Some(next) = action {
                        apply_action(state, next);
                        state.dirty = true;
                        action = filter.take();
                    }
                },
                Event::Mouse(mouse) => {
                    state.handle_mouse(mouse.kind, mouse.column, mouse.row);
                    state.dirty = true;
                },
                Event::Resize(_, _) => {
                    state.dirty = true;
                },
                _ => {},
            }
        } else if let Some(action) = filter.flush() {
            apply_action(state, action);
            state.dirty = true;
        }
    }
}

fn apply_action(state: &mut AppState, action: KeyAction) {
    match action {
        KeyAction::Key(code, modifiers) => state.handle_key(code, modifiers),
        KeyAction::Wheel(delta) => state.wheel(delta),
    }
}
