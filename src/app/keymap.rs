//! Keyboard layout normalization for key bindings.
//!
//! Terminals report the character the active layout produced: crossterm takes the
//! `u_char` of a Windows console record, unix terminals receive the UTF-8 bytes. A
//! Russian layout therefore delivers `Char('ф')` for the key bound to `a`, and none of
//! the letter bindings fire. Bindings are matched against the physical US QWERTY
//! position of the same key, while text entry keeps the typed character, so searches
//! and filters can contain Cyrillic.

use ratatui::crossterm::event::KeyCode;
use ratatui::crossterm::event::KeyModifiers;

use crate::cli::KeyMapMode;

/// A key press seen twice: the typed character for text entry, the QWERTY position for bindings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyPress {
    /// Character as the active layout produced it, inserted into text inputs.
    pub typed: KeyCode,
    /// Same physical key expressed as a US QWERTY character, matched by bindings.
    pub bind: KeyCode,
    pub modifiers: KeyModifiers,
}

impl KeyPress {
    pub fn new(code: KeyCode, modifiers: KeyModifiers, mode: KeyMapMode) -> Self {
        let bind = match mode {
            KeyMapMode::Auto => qwerty_key(code),
            KeyMapMode::Raw => code,
        };
        Self {
            typed: code,
            bind,
            modifiers,
        }
    }

    /// Character typed in the active layout, for text entry.
    pub const fn typed_char(self) -> Option<char> {
        match self.typed {
            KeyCode::Char(ch) => Some(ch),
            _ => None,
        }
    }

    /// Character of the QWERTY position, for shortcuts bound to a letter.
    pub const fn bind_char(self) -> Option<char> {
        match self.bind {
            KeyCode::Char(ch) => Some(ch),
            _ => None,
        }
    }
}

/// The US QWERTY key that produced `code`, when `code` comes from a mapped layout.
pub fn qwerty_key(code: KeyCode) -> KeyCode {
    match code {
        KeyCode::Char(ch) => qwerty_char(ch).map_or(code, KeyCode::Char),
        _ => code,
    }
}

/// Physical US QWERTY character of the key that yields `ch` on a mapped layout.
pub fn qwerty_char(ch: char) -> Option<char> {
    MAPPED.iter().find(|(from, _)| *from == ch).map(|(_, to)| *to)
}

/// Layout key -> US QWERTY key, for the Russian layout (the standard PC/JCUKEN one) and
/// the Ukrainian keys that share the position (`ї`, `є`, `ґ`). Shifted keys map to the
/// shifted QWERTY symbol, matching what a Latin layout reports with the Shift modifier.
const MAPPED: &[(char, char)] = &[
    // Letter rows, unshifted.
    ('й', 'q'),
    ('ц', 'w'),
    ('у', 'e'),
    ('к', 'r'),
    ('е', 't'),
    ('н', 'y'),
    ('г', 'u'),
    ('ш', 'i'),
    ('щ', 'o'),
    ('з', 'p'),
    ('ф', 'a'),
    ('ы', 's'),
    ('в', 'd'),
    ('а', 'f'),
    ('п', 'g'),
    ('р', 'h'),
    ('о', 'j'),
    ('л', 'k'),
    ('д', 'l'),
    ('я', 'z'),
    ('ч', 'x'),
    ('с', 'c'),
    ('м', 'v'),
    ('и', 'b'),
    ('т', 'n'),
    ('ь', 'm'),
    // Letter rows, Shift pressed: the QWERTY key is uppercase.
    ('Й', 'Q'),
    ('Ц', 'W'),
    ('У', 'E'),
    ('К', 'R'),
    ('Е', 'T'),
    ('Н', 'Y'),
    ('Г', 'U'),
    ('Ш', 'I'),
    ('Щ', 'O'),
    ('З', 'P'),
    ('Ф', 'A'),
    ('Ы', 'S'),
    ('В', 'D'),
    ('А', 'F'),
    ('П', 'G'),
    ('Р', 'H'),
    ('О', 'J'),
    ('Л', 'K'),
    ('Д', 'L'),
    ('Я', 'Z'),
    ('Ч', 'X'),
    ('С', 'C'),
    ('М', 'V'),
    ('И', 'B'),
    ('Т', 'N'),
    ('Ь', 'M'),
    // Punctuation keys, unshifted and shifted QWERTY symbol of the same position.
    ('х', '['),
    ('ъ', ']'),
    ('ж', ';'),
    ('э', '\''),
    ('б', ','),
    ('ю', '.'),
    ('ё', '`'),
    ('і', '\\'),
    ('ї', ']'),
    ('є', '\''),
    ('ґ', 'g'),
    ('Х', '{'),
    ('Ъ', '}'),
    ('Ж', ':'),
    ('Э', '"'),
    ('Б', '<'),
    ('Ю', '>'),
    ('Ё', '~'),
    ('І', '|'),
    ('Ї', '}'),
    ('Є', '"'),
    ('Ґ', 'G'),
    // The QWERTY `/` key is `.` on the Russian layout, Shift makes it `,`: the search
    // and help bindings. `/` itself stays as-is: the Russian `\` position yields it.
    ('.', '/'),
    (',', '?'),
];

#[cfg(test)]
mod tests {
    use super::*;

    fn bind(ch: char, mode: KeyMapMode) -> KeyCode {
        KeyPress::new(KeyCode::Char(ch), KeyModifiers::NONE, mode).bind
    }

    #[test]
    fn russian_letters_take_their_qwerty_position() {
        for (ch, expected) in [('ф', 'a'), ('о', 'j'), ('к', 'r'), ('с', 'c'), ('й', 'q'), ('Ь', 'M')] {
            assert_eq!(bind(ch, KeyMapMode::Auto), KeyCode::Char(expected), "{ch}");
        }
    }

    #[test]
    fn shifted_letters_take_the_shifted_qwerty_symbol() {
        for (ch, expected) in [
            ('Х', '{'),
            ('Ъ', '}'),
            ('Ж', ':'),
            ('Э', '"'),
            ('Б', '<'),
            ('Ю', '>'),
            ('Ё', '~'),
        ] {
            assert_eq!(bind(ch, KeyMapMode::Auto), KeyCode::Char(expected), "{ch}");
        }
    }

    #[test]
    fn search_and_help_keys_are_reachable() {
        assert_eq!(bind('.', KeyMapMode::Auto), KeyCode::Char('/'));
        assert_eq!(bind(',', KeyMapMode::Auto), KeyCode::Char('?'));
        assert_eq!(bind('/', KeyMapMode::Auto), KeyCode::Char('/'), "the Russian / key stays /");
        assert_eq!(bind('і', KeyMapMode::Auto), KeyCode::Char('\\'));
    }

    #[test]
    fn latin_keys_and_their_symbols_stay_themselves() {
        for ch in ['a', 'q', 'G', '?', '/', '[', '~', ' ', '7'] {
            assert_eq!(bind(ch, KeyMapMode::Auto), KeyCode::Char(ch), "{ch}");
        }
        assert_eq!(qwerty_key(KeyCode::Enter), KeyCode::Enter);
        assert_eq!(qwerty_key(KeyCode::Esc), KeyCode::Esc);
    }

    #[test]
    fn text_entry_keeps_the_typed_character() {
        let key = KeyPress::new(KeyCode::Char('ф'), KeyModifiers::SHIFT, KeyMapMode::Auto);
        assert_eq!(key.typed, KeyCode::Char('ф'));
        assert_eq!(key.bind, KeyCode::Char('a'));
        assert_eq!(key.modifiers, KeyModifiers::SHIFT);
    }

    #[test]
    fn raw_mode_matches_the_typed_character() {
        let key = KeyPress::new(KeyCode::Char('ф'), KeyModifiers::NONE, KeyMapMode::Raw);
        assert_eq!(key.typed, KeyCode::Char('ф'));
        assert_eq!(key.bind, KeyCode::Char('ф'));
    }

    #[test]
    fn every_layout_key_is_mapped_once() {
        let mut keys: Vec<char> = MAPPED.iter().map(|(from, _)| *from).collect();
        let total = keys.len();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(keys.len(), total, "duplicate layout key in the table");
    }

    #[test]
    fn table_covers_the_alphabet() {
        for ch in "абвгдеёжзийклмнопрстуфхцчшщъыьэюя".chars() {
            assert!(qwerty_char(ch).is_some(), "unmapped {ch}");
            assert!(qwerty_char(ch.to_uppercase().next().unwrap_or(ch)).is_some(), "unmapped {ch} upper");
        }
    }
}
