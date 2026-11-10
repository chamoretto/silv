//! QWERTY positions of the active Windows layout, probed through user32.
//!
//! crossterm hands over the character the layout produced and drops the virtual key code of
//! printable keys, so the app cannot ask the terminal. user32 can answer the same question:
//! for every virtual key code, translate it with the active layout to get the layout
//! character, and translate its scan code through the ANSI table to get the QWERTY
//! character. This covers every layout the system provides, needs no layout installed
//! beyond the active one, and costs a few hundred fast calls once at startup.

use windows_sys::Win32::UI::Input::KeyboardAndMouse::GetKeyboardLayout;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::GetKeyboardLayoutNameW;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::MAPVK_VK_TO_VSC;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::MapVirtualKeyExW;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::ToUnicodeEx;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::VK_CONTROL;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::VK_MENU;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::VK_PACKET;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::VK_SHIFT;

use super::scan::qwerty_pair;

/// Key state high bit: the key is pressed.
const DOWN: u8 = 0x80;

/// `ToUnicodeEx` flag: ignore the dead-key state, report the key's own glyph.
const IGNORE_DEAD_KEYS: u32 = 2;

/// Name buffer of `GetKeyboardLayoutNameW` (KL_NAMELENGTH).
const LAYOUT_NAME_LEN: usize = 9;

/// Pairs of layout character and QWERTY character of the same physical key.
pub fn build() -> Vec<(char, char)> {
    let layout = unsafe { GetKeyboardLayout(0) };
    if layout.is_null() {
        return Vec::new();
    }
    let mut pairs: Vec<(char, char)> = Vec::new();
    for vk in 0x08..=0xFE_u32 {
        if matches!(vk, v if v == u32::from(VK_SHIFT) || v == u32::from(VK_CONTROL) || v == u32::from(VK_MENU) || v == u32::from(VK_PACKET))
        {
            continue;
        }
        let scan = unsafe { MapVirtualKeyExW(vk, MAPVK_VK_TO_VSC, layout) };
        // 0 means the key does not exist, a 0xE0 prefix marks an extended key: the numpad,
        // the right-hand modifiers and the ISO extra key, none of which carries a binding.
        if scan == 0 || scan & 0xFF00 == 0xE000 {
            continue;
        }
        let Some((base_key, shift_key)) = qwerty_pair(scan) else {
            continue;
        };
        let mut state = [0_u8; 256];
        let base_char = translate(vk, scan, &state, layout);
        state[usize::from(VK_SHIFT)] = DOWN;
        let shift_char = translate(vk, scan, &state, layout);
        if let Some(ch) = base_char {
            pairs.push((ch, base_key));
        }
        // A shift layer that changes nothing (the numpad slashes) carries no QWERTY meaning.
        if let Some(ch) = shift_char
            && base_char != Some(ch)
        {
            pairs.push((ch, shift_key));
        }
    }
    pairs
}

/// One line description of what was probed, for `--key-map-show`.
pub fn detail() -> String {
    let mut name = [0_u16; LAYOUT_NAME_LEN];
    let ok = unsafe { GetKeyboardLayoutNameW(name.as_mut_ptr()) };
    if ok == 0 {
        return "layout id unavailable".to_owned();
    }
    let id = String::from_utf16_lossy(&name);
    let id = id.trim_matches('\0');
    let language = u32::from_str_radix(id, 16).ok().map(|value| value & 0xFFFF);
    match language {
        Some(lang) => format!("layout {id}, language 0x{lang:04x}"),
        None => format!("layout {id}"),
    }
}

/// Character a key produces in this layout, dead keys and composites are skipped.
fn translate(vk: u32, scan: u32, state: &[u8; 256], layout: *mut core::ffi::c_void) -> Option<char> {
    let mut buffer = [0_u16; 4];
    let written = unsafe {
        ToUnicodeEx(
            vk,
            scan,
            state.as_ptr(),
            buffer.as_mut_ptr(),
            i32::try_from(buffer.len()).ok()?,
            IGNORE_DEAD_KEYS,
            layout,
        )
    };
    if written != 1 {
        return None;
    }
    let ch = char::from_u32(u32::from(buffer[0]))?;
    if ch.is_control() {
        return None;
    }
    Some(ch)
}
