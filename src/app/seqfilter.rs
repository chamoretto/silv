//! Filters leaked mouse escape sequences out of the key stream.
//!
//! Some Windows consoles deliver wheel and click events as raw X10 (`ESC [ M b x y`)
//! or SGR (`ESC [ < n ; x ; y M`) byte sequences that crossterm does not consume.
//! They arrive as plain `Char` key events (`[`, `M`, `C`, ...) and trigger arbitrary
//! hotkeys. The filter swallows those bursts, maps wheel codes to scroll actions,
//! and re-emits a lone `[` (real user input) when a burst times out.

use ratatui::crossterm::event::KeyCode;
use ratatui::crossterm::event::KeyModifiers;

/// A key pass-through or a synthesized wheel scroll.
#[derive(Debug, PartialEq, Eq)]
pub enum KeyAction {
    Key(KeyCode, KeyModifiers),
    Wheel(isize),
}

enum Phase {
    Idle,
    AfterBracket,
    X10Button,
    X10Position(u8),
    Sgr,
}

pub struct SequenceFilter {
    phase: Phase,
    digits: String,
    out: Option<KeyAction>,
}

impl Default for SequenceFilter {
    fn default() -> Self {
        Self::new()
    }
}

impl SequenceFilter {
    pub const fn new() -> Self {
        Self {
            phase: Phase::Idle,
            digits: String::new(),
            out: None,
        }
    }

    /// Feeds one key event, returning the action it produced, if any.
    pub fn push(&mut self, code: KeyCode, modifiers: KeyModifiers) -> Option<KeyAction> {
        let action = self.classify(code, modifiers);
        match (action, self.out.take()) {
            (Some(earlier), Some(later)) => {
                self.out = Some(later);
                Some(earlier)
            },
            (Some(only), None) => Some(only),
            (None, Some(queued)) => Some(queued),
            (None, None) => None,
        }
    }

    /// Next queued action, drained after `push` in a loop.
    pub fn take(&mut self) -> Option<KeyAction> {
        self.out.take()
    }

    /// Resolves a half-received sequence on a poll timeout: a buffered `[` is real
    /// input, an incomplete mouse burst is discarded.
    pub fn flush(&mut self) -> Option<KeyAction> {
        if let Some(action) = self.out.take() {
            return Some(action);
        }
        match self.phase {
            Phase::Idle => None,
            Phase::AfterBracket => {
                self.phase = Phase::Idle;
                Some(KeyAction::Key(KeyCode::Char('['), KeyModifiers::NONE))
            },
            _ => {
                self.reset();
                None
            },
        }
    }

    fn classify(&mut self, code: KeyCode, modifiers: KeyModifiers) -> Option<KeyAction> {
        if !modifiers.is_empty() {
            self.reset();
            return Some(KeyAction::Key(code, modifiers));
        }
        let ch = match code {
            KeyCode::Char(ch) => ch,
            _ => return self.on_non_char(code, modifiers),
        };
        match self.phase {
            Phase::Idle => {
                if ch == '[' {
                    self.phase = Phase::AfterBracket;
                    None
                } else {
                    Some(KeyAction::Key(code, modifiers))
                }
            },
            Phase::AfterBracket => match ch {
                'M' => {
                    self.phase = Phase::X10Button;
                    None
                },
                '<' => {
                    self.phase = Phase::Sgr;
                    self.digits.clear();
                    None
                },
                '[' => {
                    self.phase = Phase::AfterBracket;
                    Some(KeyAction::Key(KeyCode::Char('['), KeyModifiers::NONE))
                },
                _ => {
                    self.phase = Phase::Idle;
                    self.out = Some(KeyAction::Key(code, KeyModifiers::NONE));
                    Some(KeyAction::Key(KeyCode::Char('['), KeyModifiers::NONE))
                },
            },
            Phase::X10Button => {
                self.phase = Phase::X10Position(2);
                x10_wheel(ch).map(KeyAction::Wheel)
            },
            Phase::X10Position(remaining) => {
                // Back-to-back wheel bursts touch: the y byte of one click equals the `[`
                // that opens the next one, so `[` always restarts, it is never a coordinate.
                if ch == '[' {
                    self.phase = Phase::AfterBracket;
                } else {
                    self.phase = if remaining <= 1 {
                        Phase::Idle
                    } else {
                        Phase::X10Position(remaining - 1)
                    };
                }
                None
            },
            Phase::Sgr => match ch {
                'M' => {
                    let direction = sgr_wheel(&self.digits);
                    self.digits.clear();
                    self.phase = Phase::Idle;
                    direction.map(KeyAction::Wheel)
                },
                'm' => {
                    self.reset();
                    None
                },
                c if c.is_ascii_digit() => {
                    if self.digits.len() < 8 {
                        self.digits.push(c);
                    }
                    None
                },
                ';' => {
                    self.digits.push(';');
                    None
                },
                _ => {
                    self.reset();
                    None
                },
            },
        }
    }

    fn on_non_char(&mut self, code: KeyCode, modifiers: KeyModifiers) -> Option<KeyAction> {
        match self.phase {
            Phase::Idle => Some(KeyAction::Key(code, modifiers)),
            Phase::AfterBracket => {
                self.phase = Phase::Idle;
                self.out = Some(KeyAction::Key(code, modifiers));
                Some(KeyAction::Key(KeyCode::Char('['), KeyModifiers::NONE))
            },
            _ => {
                self.reset();
                None
            },
        }
    }

    fn reset(&mut self) {
        self.phase = Phase::Idle;
        self.digits.clear();
    }
}

/// X10 wheel: 66/67 are the wheel-up/down presses, 68/69 the extended forms,
/// 96..101 the same codes with the 32 offset that X10 adds to every byte.
fn x10_wheel(ch: char) -> Option<isize> {
    let code = u32::from(ch);
    let base = if code >= 96 { code - 32 } else { code };
    match base {
        66 | 67 => Some(-1),
        68 | 69 => Some(1),
        _ => None,
    }
}

/// SGR wheel: 66/67 (urxvt style) and 68/69 (xterm style) presses, 96..99 motion forms.
fn sgr_wheel(digits: &str) -> Option<isize> {
    let Ok(button) = digits.split(';').next().unwrap_or_default().parse::<u32>() else {
        return None;
    };
    match button {
        66 | 67 | 96 | 97 => Some(-1),
        68 | 69 | 98 | 99 => Some(1),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn feed(filter: &mut SequenceFilter, text: &str) -> Vec<KeyAction> {
        let mut actions = Vec::new();
        for ch in text.chars() {
            if let Some(action) = filter.push(KeyCode::Char(ch), KeyModifiers::NONE) {
                actions.push(action);
            }
            while let Some(action) = filter.take() {
                actions.push(action);
            }
        }
        actions
    }

    fn key(ch: char) -> KeyAction {
        KeyAction::Key(KeyCode::Char(ch), KeyModifiers::NONE)
    }

    #[test]
    fn plain_keys_pass_through() {
        let mut filter = SequenceFilter::new();
        assert_eq!(feed(&mut filter, "abc"), vec![key('a'), key('b'), key('c')]);
    }

    #[test]
    fn x10_wheel_burst_is_swallowed_and_mapped() {
        // The exact shape observed on Windows: "[MCE+[MCB[MCD" (no ESC, it is consumed).
        let mut filter = SequenceFilter::new();
        let actions = feed(&mut filter, "[MCB[MCD[MCE+");
        assert_eq!(actions, vec![KeyAction::Wheel(-1), KeyAction::Wheel(-1), KeyAction::Wheel(-1)]);
    }

    #[test]
    fn sgr_wheel_press_and_release() {
        let mut filter = SequenceFilter::new();
        let actions = feed(&mut filter, "[<68;12;40M[<68;12;40m");
        assert_eq!(actions, vec![KeyAction::Wheel(1)]);
    }

    #[test]
    fn lone_bracket_survives_as_text() {
        let mut filter = SequenceFilter::new();
        assert_eq!(filter.push(KeyCode::Char('['), KeyModifiers::NONE), None);
        assert_eq!(filter.flush(), Some(key('[')));
        assert_eq!(filter.flush(), None);
    }

    #[test]
    fn bracket_then_text_emits_both() {
        let mut filter = SequenceFilter::new();
        assert_eq!(filter.push(KeyCode::Char('['), KeyModifiers::NONE), None);
        assert_eq!(filter.push(KeyCode::Char('a'), KeyModifiers::NONE), Some(key('[')));
        assert_eq!(filter.take(), Some(key('a')));
    }

    #[test]
    fn bracket_then_enter_emits_both() {
        let mut filter = SequenceFilter::new();
        assert_eq!(filter.push(KeyCode::Char('['), KeyModifiers::NONE), None);
        assert_eq!(
            filter.push(KeyCode::Enter, KeyModifiers::NONE),
            Some(KeyAction::Key(KeyCode::Char('['), KeyModifiers::NONE))
        );
        assert_eq!(filter.take(), Some(KeyAction::Key(KeyCode::Enter, KeyModifiers::NONE)));
    }

    #[test]
    fn modified_keys_never_start_a_sequence() {
        let mut filter = SequenceFilter::new();
        let actions = feed(&mut filter, "[b");
        assert_eq!(actions, vec![key('['), key('b')]);
    }

    #[test]
    fn control_keys_pass_through_mid_burst() {
        let mut filter = SequenceFilter::new();
        filter.push(KeyCode::Char('['), KeyModifiers::NONE);
        assert_eq!(
            filter.push(KeyCode::Char('c'), KeyModifiers::CONTROL),
            Some(KeyAction::Key(KeyCode::Char('c'), KeyModifiers::CONTROL))
        );
    }
}
