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
    MHeader,
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
            Phase::MHeader => {
                self.phase = Phase::Idle;
                Some(KeyAction::Key(KeyCode::Char('M'), KeyModifiers::NONE))
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
            Phase::Idle => match ch {
                '[' => {
                    self.phase = Phase::AfterBracket;
                    None
                },
                'M' => {
                    // ConEmu/winpty sends legacy mouse records as "ESC M b x y";
                    // crossterm eats the ESC and delivers M, button, x, y as keys.
                    self.phase = Phase::MHeader;
                    None
                },
                _ => Some(KeyAction::Key(code, modifiers)),
            },
            Phase::MHeader => {
                let wheel = x10_wheel(u32::from(ch));
                // Only plausible mouse bytes continue the burst; anything else is
                // real text ("My..."), so both keys are passed through.
                if wheel.is_none() && ch != 'C' {
                    self.phase = Phase::Idle;
                    self.out = Some(KeyAction::Key(code, KeyModifiers::NONE));
                    return Some(KeyAction::Key(KeyCode::Char('M'), KeyModifiers::NONE));
                }
                self.phase = Phase::X10Position(2);
                wheel.map(KeyAction::Wheel)
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
                x10_wheel(u32::from(ch)).map(KeyAction::Wheel)
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
            Phase::AfterBracket | Phase::MHeader => {
                let opener = match self.phase {
                    Phase::MHeader => KeyCode::Char('M'),
                    _ => KeyCode::Char('['),
                };
                self.phase = Phase::Idle;
                self.out = Some(KeyAction::Key(code, modifiers));
                Some(KeyAction::Key(opener, KeyModifiers::NONE))
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

/// X10 wheel presses: 98/100 up, 99/101 down (urxvt and xterm, 32-based),
/// 68/69 (xterm extended). 66/67 (press/release, motion on Windows conhost)
/// carry no wheel information, so they are swallowed without scrolling.
fn x10_wheel(code: u32) -> Option<isize> {
    match code {
        68 | 98 | 100 => Some(-1),
        69 | 99 | 101 => Some(1),
        _ => None,
    }
}

/// SGR wheel: 66 (urxvt up) / 67 (urxvt down), 68 (xterm up) / 69 (xterm down),
/// 96..99 the motion forms.
fn sgr_wheel(digits: &str) -> Option<isize> {
    let Ok(button) = digits.split(';').next().unwrap_or_default().parse::<u32>() else {
        return None;
    };
    match button {
        66 | 68 | 96 | 98 => Some(-1),
        67 | 69 | 97 | 99 => Some(1),
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
    fn x10_bursts_without_direction_are_swallowed() {
        // The shape observed on Windows: "[MCB[MCD[MCE+" (ESC is consumed by the
        // terminal layer). Button byte 'C' is release/motion and carries no wheel
        // information, so the bursts must vanish without scrolling.
        let mut filter = SequenceFilter::new();
        assert_eq!(feed(&mut filter, "[MCB[MCD[MCE+"), vec![]);
    }

    #[test]
    fn m_led_motion_bursts_are_swallowed() {
        // ConEmu/winpty form: "M b x y" per event, button byte 'C' for motion.
        let mut filter = SequenceFilter::new();
        assert_eq!(feed(&mut filter, "MCr#MCq#MCa#MC`#MC_#"), vec![]);
    }

    #[test]
    fn m_led_wheel_codes_scroll() {
        let mut filter = SequenceFilter::new();
        assert_eq!(feed(&mut filter, "MdeXY"), vec![KeyAction::Wheel(-1), key('Y')]);
    }

    #[test]
    fn typing_my_survives_the_m_header() {
        let mut filter = SequenceFilter::new();
        assert_eq!(feed(&mut filter, "MyApp"), vec![key('M'), key('y'), key('A'), key('p'), key('p')]);
    }

    #[test]
    fn lone_m_survives_as_text() {
        let mut filter = SequenceFilter::new();
        assert_eq!(filter.push(KeyCode::Char('M'), KeyModifiers::NONE), None);
        assert_eq!(filter.flush(), Some(key('M')));
    }

    #[test]
    fn sgr_wheel_press_and_release() {
        let mut filter = SequenceFilter::new();
        let actions = feed(&mut filter, "[<68;12;40M[<68;12;40m");
        assert_eq!(actions, vec![KeyAction::Wheel(-1)]);
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
