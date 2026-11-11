//! Layout independent key bindings.
//!
//! Terminals report the character the active layout produced: crossterm takes the `u_char`
//! of a Windows console record, unix terminals receive the UTF-8 bytes. A Russian layout
//! therefore delivers `Char('ф')` for the key bound to `a`, and no letter binding fires.
//!
//! Every key press is resolved twice: `typed` keeps the layout character for text entry,
//! `bind` carries the character of the same physical key on the US QWERTY layout, which is
//! what the bindings match. The QWERTY position is found per platform:
//!
//! - Windows: virtual key codes are enumerated with `MapVirtualKeyExW` and translated with
//!   `ToUnicodeEx`; the scan code they yield is looked up in the ANSI scan code table. Every
//!   layout Windows provides is covered, no per-layout table and no layout needs installing.
//! - macOS: the current layout is translated with `UCKeyTranslate` and compared against the
//!   current ASCII-capable layout, which is the US one for every non-Latin layout. The input source
//!   API is main thread only, see `macos.rs`.
//! - Linux and other unix: X11 keymap tables in `layouts/`, generated from xkeyboard-config
//!   and selected by `XKB_DEFAULT_LAYOUT`.
//! - Everywhere: `builtin.txt` as a fallback, so legacy consoles keep the Russian bindings,
//!   plus `--key-map <name>` and `--key-map file:<path>` for explicit choices.

use std::path::Path;

use ratatui::crossterm::event::KeyCode;
use ratatui::crossterm::event::KeyModifiers;

use crate::cli::KeyMapMode;
use crate::error::SilvError;

#[cfg(target_os = "macos")]
mod macos;
#[cfg(windows)]
mod scan;
#[cfg(windows)]
mod windows;
#[cfg(all(unix, not(target_os = "macos")))]
mod xkb;

/// Table of the built-in fallback layout.
const BUILTIN_TEXT: &str = include_str!("keymap/builtin.txt");

/// Names the built-in table answers to on platforms without shipped xkb tables.
const BUILTIN_NAMES: [&str; 2] = ["ru", "ua"];

/// Where a resolved map came from, shown by `--key-map-show`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyMapOrigin {
    /// No mapping: bindings match the typed character.
    Raw,
    /// Only the built-in table.
    Builtin,
    /// Platform map of the active layout plus the built-in table.
    Auto { platform: &'static str, detail: String },
    /// A shipped table picked by name.
    Layout(String),
    /// A user supplied file.
    File(String),
}

impl std::fmt::Display for KeyMapOrigin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Raw => f.write_str("raw (no mapping)"),
            Self::Builtin => f.write_str("builtin table"),
            Self::Auto { platform, detail } => write!(f, "auto/{platform} ({detail})"),
            Self::Layout(name) => write!(f, "layout table `{name}`"),
            Self::File(path) => write!(f, "file `{path}`"),
        }
    }
}

/// A resolved layout character to QWERTY character map, sorted for binary search.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyMap {
    entries: Vec<(char, char)>,
    origin: KeyMapOrigin,
}

impl KeyMap {
    /// Empty map: every key binds as typed.
    pub const fn raw() -> Self {
        Self {
            entries: Vec::new(),
            origin: KeyMapOrigin::Raw,
        }
    }

    /// Builds a map from pairs. The first pair of a character wins; pairs whose target the app
    /// does not bind, identity pairs and control characters are dropped, the rest is sorted.
    pub fn from_entries(entries: impl IntoIterator<Item = (char, char)>, origin: KeyMapOrigin) -> Self {
        let mut kept: Vec<(char, char)> = Vec::new();
        for (from, to) in entries {
            let usable = from != to && !from.is_control() && is_bindable(to);
            if usable && !kept.iter().any(|(known, _)| *known == from) {
                kept.push((from, to));
            }
        }
        kept.sort_unstable_by_key(|entry| entry.0);
        Self { entries: kept, origin }
    }

    /// The built-in table: the portable fallback of every platform.
    pub fn builtin() -> Self {
        Self::from_entries(builtin_entries().iter().copied(), KeyMapOrigin::Builtin)
    }

    /// Resolves the map a mode asks for: platform probing, a shipped table, a file, or raw.
    pub fn resolve(mode: &KeyMapMode) -> Self {
        match mode {
            KeyMapMode::Raw => Self::raw(),
            KeyMapMode::Auto => Self::auto(),
            KeyMapMode::Layout(name) => {
                let (entries, _) = parse_table(table_text(name), name);
                Self::from_entries(entries, KeyMapOrigin::Layout(name.clone()))
            },
            KeyMapMode::File(path) => Self::from_file(path).unwrap_or_else(|_| Self::raw()),
        }
    }

    /// Platform map of the active layout plus the built-in table; platform entries win.
    pub fn auto() -> Self {
        let platform = platform_map();
        let detail = platform_detail();
        let origin = |detail: String| KeyMapOrigin::Auto {
            platform: PLATFORM,
            detail,
        };
        if platform.is_empty() {
            return Self::from_entries(builtin_entries().iter().copied(), origin(format!("{detail}, builtin only")));
        }
        let builtin = Self::builtin();
        let merged = platform
            .entries()
            .iter()
            .copied()
            .chain(builtin.entries().iter().copied());
        Self::from_entries(merged, origin(detail))
    }

    /// Reads and parses a map file, reporting malformed lines with the file name.
    pub fn from_file(path: &Path) -> Result<Self, SilvError> {
        let label = path.display().to_string();
        let text = std::fs::read_to_string(path).map_err(|cause| SilvError::KeyMapRead {
            path: label.clone(),
            cause,
        })?;
        let (entries, reason) = parse_table(&text, &label);
        if let Some(reason) = reason {
            return Err(SilvError::KeyMapLine { path: label, reason });
        }
        Ok(Self::from_entries(entries, KeyMapOrigin::File(label)))
    }

    /// QWERTY character of a layout character.
    pub fn map_char(&self, ch: char) -> Option<char> {
        self.entries
            .binary_search_by(|(from, _)| from.cmp(&ch))
            .ok()
            .map(|index| self.entries[index].1)
    }

    /// QWERTY position of a key code.
    pub fn key(&self, code: KeyCode) -> KeyCode {
        match code {
            KeyCode::Char(ch) => self.map_char(ch).map_or(code, KeyCode::Char),
            _ => code,
        }
    }

    pub fn entries(&self) -> &[(char, char)] {
        &self.entries
    }

    pub fn origin(&self) -> &KeyMapOrigin {
        &self.origin
    }

    pub const fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub const fn len(&self) -> usize {
        self.entries.len()
    }
}

/// Characters a map entry may name: the app binds ASCII letters and these symbols, so a pair
/// with any other target would move a bound key onto an unbound one, and is dropped.
const fn is_bindable(target: char) -> bool {
    target.is_ascii_alphabetic() || matches!(target, '/' | '?' | '~' | '`' | ' ')
}

/// A key press: the typed character for text entry, the QWERTY position for bindings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyPress {
    /// Character as the active layout produced it, inserted into text inputs.
    pub typed: KeyCode,
    /// Same physical key as a US QWERTY character, matched by bindings.
    pub bind: KeyCode,
    pub modifiers: KeyModifiers,
}

impl KeyPress {
    pub fn new(code: KeyCode, modifiers: KeyModifiers, map: &KeyMap) -> Self {
        Self {
            typed: code,
            bind: map.key(code),
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

    /// A non-ASCII character stayed unmapped: the layout may have changed since the map was
    /// built, so probing once more is worth the microseconds it takes.
    pub const fn needs_rebuild(self) -> bool {
        matches!((self.typed, self.bind), (KeyCode::Char(typed), KeyCode::Char(bound))
            if typed == bound && !typed.is_ascii())
    }
}

/// Names accepted by `--key-map <name>`.
pub fn layout_names() -> Vec<String> {
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        xkb::names()
    }
    #[cfg(not(all(unix, not(target_os = "macos"))))]
    {
        BUILTIN_NAMES.iter().map(|name| (*name).to_owned()).collect()
    }
}

/// Table text of a named layout, empty when the platform has none.
fn table_text(name: &str) -> &'static str {
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        xkb::table(name)
            .or_else(|| BUILTIN_NAMES.contains(&name).then_some(BUILTIN_TEXT))
            .unwrap_or_default()
    }
    #[cfg(not(all(unix, not(target_os = "macos"))))]
    {
        if BUILTIN_NAMES.contains(&name) {
            BUILTIN_TEXT
        } else {
            ""
        }
    }
}

/// Parses a `from<TAB>to` table, `#` starts a comment. The second result describes the first
/// malformed line; entries collected before it stay usable.
pub fn parse_table(text: &str, label: &str) -> (Vec<(char, char)>, Option<String>) {
    let mut entries = Vec::new();
    for (index, line) in text.lines().enumerate() {
        let line = line.split('#').next().unwrap_or_default().trim_end();
        if line.trim().is_empty() {
            continue;
        }
        let mut fields = line.split(['\t', ' ']).filter(|field| !field.is_empty());
        let (Some(from), Some(to), None) = (fields.next(), fields.next(), fields.next()) else {
            let reason = format!("line {} of `{label}` is not a `from<TAB>to` pair", index + 1);
            return (entries, Some(reason));
        };
        let from = from.chars().collect::<Vec<char>>();
        let to = to.chars().collect::<Vec<char>>();
        if from.len() != 1 || to.len() != 1 {
            let reason = format!("line {} of `{label}` is not a single character pair", index + 1);
            return (entries, Some(reason));
        }
        entries.push((from[0], to[0]));
    }
    (entries, None)
}

/// Built-in entries, parsed once.
fn builtin_entries() -> &'static [(char, char)] {
    static TABLE: std::sync::OnceLock<Vec<(char, char)>> = std::sync::OnceLock::new();
    TABLE.get_or_init(|| parse_table(BUILTIN_TEXT, "builtin").0)
}

/// Platform the QWERTY positions come from.
pub const PLATFORM: &str = if cfg!(windows) {
    "windows"
} else if cfg!(target_os = "macos") {
    "macos"
} else if cfg!(unix) {
    "xkb"
} else {
    "none"
};

/// Map of the layout the system is currently using, empty when the platform cannot tell, or when
/// probing is switched off by `SILV_KEYMAP_PROBE`.
fn platform_map() -> KeyMap {
    if !probe_enabled() {
        return KeyMap::raw();
    }
    #[cfg(windows)]
    {
        KeyMap::from_entries(windows::build(), KeyMapOrigin::Builtin)
    }
    #[cfg(target_os = "macos")]
    {
        KeyMap::from_entries(macos::build(), KeyMapOrigin::Builtin)
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        xkb::active_map()
    }
    #[cfg(not(any(windows, unix)))]
    {
        KeyMap::raw()
    }
}

/// One line description of the platform probe, for `--key-map-show`.
fn platform_detail() -> String {
    if !probe_enabled() {
        return format!("probe off, {PROBE_ENV} switched it off");
    }
    #[cfg(windows)]
    {
        windows::detail()
    }
    #[cfg(target_os = "macos")]
    {
        macos::detail()
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        xkb::detail()
    }
    #[cfg(not(any(windows, unix)))]
    {
        "no layout probing on this platform".to_owned()
    }
}

/// Environment variable that switches the live layout probe on and off.
const PROBE_ENV: &str = "SILV_KEYMAP_PROBE";

/// Values that switch the probe off.
const PROBE_OFF: [&str; 4] = ["0", "off", "no", "false"];

/// Probe policy for an env value: every platform probes by default, `SILV_KEYMAP_PROBE` is the
/// escape hatch that switches probing off, and an unknown value changes nothing. The macOS probe is
/// main thread only (`macos.rs`): off the main thread it degrades to the built-in table rather than
/// aborting, which is what a multi-threaded test harness would do to the process.
fn probe_policy(setting: Option<&str>) -> bool {
    let off =
        |words: &[&str]| setting.is_some_and(|value| words.iter().any(|word| value.trim().eq_ignore_ascii_case(word)));
    !off(&PROBE_OFF)
}

fn probe_enabled() -> bool {
    probe_policy(std::env::var(PROBE_ENV).ok().as_deref())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map() -> KeyMap {
        KeyMap::builtin()
    }

    fn bind(ch: char, map: &KeyMap) -> KeyCode {
        KeyPress::new(KeyCode::Char(ch), KeyModifiers::NONE, map).bind
    }

    #[test]
    fn builtin_letters_take_their_qwerty_position() {
        for (ch, expected) in [('ф', 'a'), ('о', 'j'), ('к', 'r'), ('с', 'c'), ('й', 'q'), ('Ь', 'M')] {
            assert_eq!(bind(ch, &map()), KeyCode::Char(expected), "{ch}");
        }
    }

    #[test]
    fn builtin_shifted_letters_take_the_uppercase_qwerty_letter() {
        for (ch, expected) in [('Ф', 'A'), ('Й', 'Q'), ('О', 'J'), ('Т', 'N'), ('Ё', '~')] {
            assert_eq!(bind(ch, &map()), KeyCode::Char(expected), "{ch}");
        }
    }

    #[test]
    fn search_and_help_keys_are_reachable() {
        assert_eq!(bind('.', &map()), KeyCode::Char('/'));
        assert_eq!(bind(',', &map()), KeyCode::Char('?'));
        assert_eq!(bind('/', &map()), KeyCode::Char('/'), "the Russian / key stays /");
    }

    #[test]
    fn entries_only_name_bound_targets() {
        // '{' and '\\' carry no binding, so the shifted pairs of the table are dropped.
        assert_eq!(bind('Х', &map()), KeyCode::Char('Х'));
        assert_eq!(bind('і', &map()), KeyCode::Char('і'));
        let with_targets =
            KeyMap::from_entries([('Х', 'z'), ('і', 'q'), ('а', ';')], KeyMapOrigin::Layout("x".to_owned()));
        assert_eq!(with_targets.entries(), &[('Х', 'z'), ('і', 'q')]);
    }

    #[test]
    fn latin_keys_and_their_symbols_stay_themselves() {
        for ch in ['a', 'q', 'G', '?', '/', '[', '~', ' ', '7'] {
            assert_eq!(bind(ch, &map()), KeyCode::Char(ch), "{ch}");
        }
        assert_eq!(map().key(KeyCode::Enter), KeyCode::Enter);
        assert_eq!(map().key(KeyCode::Esc), KeyCode::Esc);
    }

    #[test]
    fn text_entry_keeps_the_typed_character() {
        let key = KeyPress::new(KeyCode::Char('ф'), KeyModifiers::SHIFT, &map());
        assert_eq!(key.typed, KeyCode::Char('ф'));
        assert_eq!(key.bind, KeyCode::Char('a'));
        assert_eq!(key.modifiers, KeyModifiers::SHIFT);
    }

    #[test]
    fn raw_mode_matches_the_typed_character() {
        let raw = KeyMap::raw();
        let key = KeyPress::new(KeyCode::Char('ф'), KeyModifiers::NONE, &raw);
        assert_eq!(key.typed, KeyCode::Char('ф'));
        assert_eq!(key.bind, KeyCode::Char('ф'));
    }

    #[test]
    fn injected_maps_drive_the_bindings() {
        let german = KeyMap::from_entries([('z', 'y'), ('y', 'z')], KeyMapOrigin::Layout("de".to_owned()));
        assert_eq!(bind('z', &german), KeyCode::Char('y'));
        assert_eq!(bind('a', &german), KeyCode::Char('a'));
        assert_eq!(german.origin(), &KeyMapOrigin::Layout("de".to_owned()));
    }

    #[test]
    fn maps_drop_identity_and_keep_the_first_pair() {
        let map =
            KeyMap::from_entries([('a', 'z'), ('a', 'x'), ('q', 'q'), ('j', 'k'), ('x', '\u{1}')], KeyMapOrigin::Raw);
        assert_eq!(map.entries(), &[('a', 'z'), ('j', 'k')]);
    }

    #[test]
    fn auto_mode_keeps_the_builtin_table() {
        let map = KeyMap::auto();
        assert_eq!(bind('ф', &map), KeyCode::Char('a'), "builtin entries survive auto mode");
        assert!(matches!(map.origin(), KeyMapOrigin::Auto { .. }));
        assert!(map.entries().windows(2).all(|pair| pair[0].0 < pair[1].0), "entries stay sorted and unique");
    }

    #[test]
    fn named_layouts_resolve_to_a_table() {
        for name in layout_names() {
            let map = KeyMap::resolve(&KeyMapMode::Layout(name.clone()));
            assert!(!map.is_empty(), "`{name}` resolves to an empty map");
            assert_eq!(map.origin(), &KeyMapOrigin::Layout(name));
        }
        assert!(KeyMap::resolve(&KeyMapMode::Layout("nosuchlayout".to_owned())).is_empty());
    }

    #[test]
    fn table_files_parse_as_character_pairs() {
        let (entries, reason) = parse_table("# comment\nф\ta\nо\tj\n\nй q\n", "t");
        assert_eq!(reason, None);
        assert_eq!(entries, [('ф', 'a'), ('о', 'j'), ('й', 'q')]);
    }

    #[test]
    fn malformed_table_lines_are_reported() {
        let (entries, reason) = parse_table("ф\ta\nabc\td\n", "t");
        assert_eq!(entries, [('ф', 'a')]);
        assert!(reason.unwrap().contains("line 2"));
        let (_, reason) = parse_table("ф\ta\n", "t");
        assert!(reason.is_none());
        let (_, reason) = parse_table("ф\ta\tb\n", "t");
        assert!(reason.unwrap().contains("line 1"));
    }

    #[test]
    fn builtin_table_covers_the_letters_the_app_binds() {
        for ch in "авгдеёзийклмнопрстуфцчшщыья".chars() {
            let upper = ch.to_uppercase().next().unwrap_or(ch);
            assert!(map().map_char(ch).is_some(), "unmapped {ch}");
            assert!(map().map_char(upper).is_some(), "unmapped {upper}");
        }
        for ch in ['б', 'ю', 'х', 'ъ', 'ж', 'э'] {
            assert!(map().map_char(ch).is_none(), "{ch} sits on an unbound QWERTY key");
        }
        let mut keys: Vec<char> = builtin_entries().iter().map(|(from, _)| *from).collect();
        let total = keys.len();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(keys.len(), total, "duplicate layout key in the builtin table");
    }

    #[test]
    fn rebuild_is_requested_only_for_unmapped_non_ascii() {
        let key = KeyPress::new(KeyCode::Char('ф'), KeyModifiers::NONE, &KeyMap::raw());
        assert!(key.needs_rebuild());
        let key = KeyPress::new(KeyCode::Char('ф'), KeyModifiers::NONE, &map());
        assert!(!key.needs_rebuild(), "a mapped character needs no rebuild");
        let key = KeyPress::new(KeyCode::Char('a'), KeyModifiers::NONE, &KeyMap::raw());
        assert!(!key.needs_rebuild(), "ASCII stays put");
        let key = KeyPress::new(KeyCode::Enter, KeyModifiers::NONE, &KeyMap::raw());
        assert!(!key.needs_rebuild());
    }

    #[test]
    fn probe_policy_probes_by_default_and_takes_the_off_values() {
        assert!(probe_policy(None), "every platform probes by default");
        assert!(probe_policy(Some("1")));
        assert!(probe_policy(Some("nonsense")), "an unknown value changes nothing");
        assert!(!probe_policy(Some("0")), "an off value switches probing off");
        assert!(!probe_policy(Some("OFF")), "values are trimmed and case free");
        assert!(!probe_policy(Some(" no ")));
    }

    #[cfg(windows)]
    #[test]
    fn windows_probe_yields_usable_pairs() {
        let map = KeyMap::from_entries(windows::build(), KeyMapOrigin::Builtin);
        for (from, to) in map.entries().iter().copied() {
            assert!(!from.is_control() && !to.is_control(), "control character at {from:?}");
            assert_ne!(from, to, "identity pair {from:?} survives filtering");
            assert_eq!(map.map_char(from), Some(to), "entry stays addressable");
        }
        assert!(windows::detail().contains("layout"), "detail names the layout");
    }

    #[cfg(windows)]
    #[test]
    fn scan_table_is_the_ansi_layout_and_is_consistent() {
        use crate::app::keymap::scan::qwerty_pair;
        for (scan, base, shifted) in [
            (0x10, 'q', 'Q'),
            (0x1E, 'a', 'A'),
            (0x24, 'j', 'J'),
            (0x1F, 's', 'S'),
            (0x30, 'b', 'B'),
            (0x02, '1', '!'),
            (0x1A, '[', '{'),
            (0x27, ';', ':'),
            (0x35, '/', '?'),
            (0x33, ',', '<'),
            (0x29, '`', '~'),
            (0x2B, '\\', '|'),
            (0x34, '.', '>'),
            (0x28, '\'', '"'),
        ] {
            assert_eq!(qwerty_pair(scan), Some((base, shifted)), "scan {scan:#x}");
        }
        assert_eq!(qwerty_pair(0x1C), None, "Enter has no character");
        assert_eq!(qwerty_pair(0x39), Some((' ', ' ')), "space maps to itself and is dropped");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_probe_yields_usable_pairs() {
        if !macos::on_main_thread() {
            // The input source API is main thread only, so the harness must keep tests on it.
            eprintln!("skipped: the macOS probe needs the main thread, run with `--test-threads=1`");
            return;
        }
        let pairs = macos::build();
        if pairs.is_empty() {
            // A session without a keyboard layout input source: the probe degrades to empty.
            assert!(!macos::detail().is_empty());
            return;
        }
        let map = KeyMap::from_entries(pairs, KeyMapOrigin::Builtin);
        for (from, to) in map.entries() {
            assert!(!from.is_control() && !to.is_control(), "control character at {from:?}");
        }
        assert!(!macos::detail().is_empty());
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    #[test]
    fn shipped_tables_are_well_formed() {
        for name in xkb::names() {
            let text = xkb::table(&name).unwrap_or_default();
            assert!(!text.is_empty(), "{name} has no table");
            let (entries, reason) = parse_table(text, &name);
            assert_eq!(reason, None, "{name} table is malformed");
            assert!(!entries.is_empty(), "{name} table looks truncated: {}", entries.len());
            assert_eq!(entries.iter().filter(|(from, to)| from == to).count(), 0, "{name} table has identity pairs");
        }
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    #[test]
    fn shipped_russian_table_binds_like_the_builtin_one() {
        let text = xkb::table("ru").unwrap_or_default();
        let (entries, _) = parse_table(text, "ru");
        let ru = KeyMap::from_entries(entries, KeyMapOrigin::Layout("ru".to_owned()));
        for (ch, expected) in [('ф', 'a'), ('о', 'j'), ('к', 'r'), ('.', '/'), ('Ф', 'A')] {
            assert_eq!(ru.map_char(ch), Some(expected), "{ch}");
        }
    }
}
