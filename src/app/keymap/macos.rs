//! QWERTY positions of the active macOS layout, read through the Text Input Source API.
//!
//! macOS knows both sides of the question: the current keyboard layout translates a virtual
//! key code to the typed character, and the current ASCII-capable layout (the US one for every
//! non-Latin layout) translates the same key code to the QWERTY character. Translating every
//! key code through both and joining the results gives the map, so all layouts work without a
//! per-layout table. Latin layouts (QWERTZ, AZERTY, Dvorak) are ASCII-capable, they report
//! themselves, the map is empty and bindings follow the typed character.

use std::collections::BTreeMap;
use std::ffi::c_void;

/// Highest virtual key code of the built-in keyboard (ANSI keys and the keypad block).
const LAST_KEY_CODE: u16 = 0x60;

/// `kUCKeyActionDisplay`: report the glyph a key produces.
const ACTION_DISPLAY: u32 = 3;

/// `kUCKeyTranslateNoDeadKeysBit` (bit 0): report the key itself, not a dead-key composition.
const IGNORE_DEAD_KEYS: u32 = 1;

/// Modifier mask bit indices of `UCKeyTranslate`: shift is bit 1.
const MOD_SHIFT: u32 = 1 << 1;

/// `kCFStringEncodingUTF8`.
const UTF8_ENCODING: u32 = 0x0800_0100;

#[allow(
    non_camel_case_types,
    non_snake_case,
    non_upper_case_globals,
    dead_code,
    reason = "carbon and core foundation declarations mirror the C headers"
)]
mod ffi {
    use super::c_void;

    // Opaque Core Foundation references: pointers, typed only to keep the signatures readable.
    pub type CFStringRef = *const c_void;
    pub type CFDataRef = *const c_void;
    pub type CFTypeRef = *const c_void;
    pub type OSStatus = i32;

    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        pub unsafe fn CFRelease(cf: CFTypeRef);
        pub unsafe fn CFDataGetBytePtr(data: CFDataRef) -> *const u8;
        pub unsafe fn CFStringGetCString(string: CFStringRef, buffer: *mut i8, size: i64, encoding: u32) -> i8;
    }

    #[link(name = "Carbon", kind = "framework")]
    unsafe extern "C" {
        pub static kTISPropertyUnicodeKeyLayoutData: super::ffi::CFStringRef;
        pub static kTISPropertyLocalizedName: super::ffi::CFStringRef;

        pub unsafe fn TISCopyCurrentKeyboardLayoutInputSource() -> CFTypeRef;
        pub unsafe fn TISCopyCurrentASCIICapableKeyboardLayoutInputSource() -> CFTypeRef;
        pub unsafe fn TISGetInputSourceProperty(source: CFTypeRef, key: CFStringRef) -> CFTypeRef;
        #[allow(clippy::too_many_arguments, reason = "the C signature has ten parameters")]
        pub unsafe fn UCKeyTranslate(
            keyboard: *const u8,
            virtual_key: u16,
            key_action: u32,
            modifier_key_state: u32,
            keyboard_type: u32,
            options: u32,
            dead_key_state: *mut u32,
            max_text_length: u32,
            actual_text_length: *mut u32,
            text: *mut u16,
        ) -> OSStatus;
        pub unsafe fn LMGetKbdType() -> u8;
    }
}

/// Pairs of layout character and QWERTY character of the same physical key.
pub fn build() -> Vec<(char, char)> {
    // The sources must outlive the translations: the layout bytes come from data they own.
    let current_source = Source(unsafe { ffi::TISCopyCurrentKeyboardLayoutInputSource() });
    let ascii_source = Source(unsafe { ffi::TISCopyCurrentASCIICapableKeyboardLayoutInputSource() });
    let (Some(current), Some(ascii)) = (layout_data(current_source.0), layout_data(ascii_source.0)) else {
        return Vec::new();
    };
    let keyboard_type = u32::from(unsafe { ffi::LMGetKbdType() });

    // Key code and layer of every character the current layout produces.
    let mut typed: BTreeMap<char, (u16, bool)> = BTreeMap::new();
    for code in 0..=LAST_KEY_CODE {
        for shifted in [false, true] {
            if let Some(ch) = translate(current, keyboard_type, code, shifted) {
                typed.entry(ch).or_insert((code, shifted));
            }
        }
    }

    // Same key code and layer in the reference layout, the US one for non-Latin layouts.
    let mut reference: BTreeMap<(u16, bool), char> = BTreeMap::new();
    for code in 0..=LAST_KEY_CODE {
        for shifted in [false, true] {
            if let Some(ch) = translate(ascii, keyboard_type, code, shifted) {
                reference.entry((code, shifted)).or_insert(ch);
            }
        }
    }

    typed
        .iter()
        .filter_map(|(ch, key)| reference.get(key).map(|base| (*ch, *base)))
        .collect()
}

/// One line description of what was probed, for `--key-map-show`.
pub fn detail() -> String {
    let source = Source(unsafe { ffi::TISCopyCurrentKeyboardLayoutInputSource() });
    if source.0.is_null() {
        return "no keyboard layout input source".to_owned();
    }
    let name = unsafe { ffi::TISGetInputSourceProperty(source.0, ffi::kTISPropertyLocalizedName) };
    let label = if name.is_null() {
        String::new()
    } else {
        read_string(name as ffi::CFStringRef)
    };
    if label.is_empty() {
        "layout name unavailable".to_owned()
    } else {
        format!("layout `{label}`")
    }
}

/// An input source reference, released when the owner leaves the scope.
///
/// `TISGetInputSourceProperty` returns a value owned by the source, and its layout bytes stay valid
/// only while the source lives, so the release must come after every translation that used them.
struct Source(ffi::CFTypeRef);

impl Drop for Source {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { ffi::CFRelease(self.0) };
        }
    }
}

/// Unicode key layout data of an input source, null when the source is not a layout.
fn layout_data(source: ffi::CFTypeRef) -> Option<*const u8> {
    if source.is_null() {
        return None;
    }
    let data = unsafe { ffi::TISGetInputSourceProperty(source, ffi::kTISPropertyUnicodeKeyLayoutData) };
    if data.is_null() {
        return None;
    }
    let ptr = unsafe { ffi::CFDataGetBytePtr(data as ffi::CFDataRef) };
    (!ptr.is_null()).then_some(ptr)
}

/// Character a key code produces in this layout; composites and dead keys are skipped.
fn translate(layout: *const u8, keyboard_type: u32, code: u16, shifted: bool) -> Option<char> {
    let modifiers = if shifted { MOD_SHIFT } else { 0 };
    let mut dead_key_state = 0_u32;
    let mut produced = [0_u16; 4];
    let mut written = 0_u32;
    let status = unsafe {
        ffi::UCKeyTranslate(
            layout,
            code,
            ACTION_DISPLAY,
            modifiers,
            keyboard_type,
            IGNORE_DEAD_KEYS,
            &mut dead_key_state,
            u32::try_from(produced.len()).ok()?,
            &mut written,
            produced.as_mut_ptr(),
        )
    };
    if status != 0 || written != 1 {
        return None;
    }
    let ch = char::from_u32(u32::from(produced[0]))?;
    if ch.is_control() {
        return None;
    }
    Some(ch)
}

/// Reads a CFString as UTF-8, an empty string when it does not fit the buffer.
fn read_string(string: ffi::CFStringRef) -> String {
    let mut buffer = [0_i8; 256];
    let ok = unsafe {
        ffi::CFStringGetCString(string, buffer.as_mut_ptr(), i64::try_from(buffer.len()).unwrap_or(256), UTF8_ENCODING)
    };
    if ok == 0 {
        return String::new();
    }
    let bytes: Vec<u8> = buffer
        .iter()
        .map(|byte| *byte as u8)
        .take_while(|byte| *byte != 0)
        .collect();
    String::from_utf8(bytes).unwrap_or_default()
}
