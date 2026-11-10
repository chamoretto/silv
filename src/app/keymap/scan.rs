//! ANSI scan codes to QWERTY characters: the physical position of every printable key.
//!
//! Scan codes come from the keyboard hardware, so they do not depend on the layout: the
//! probe translates a key with the active layout and looks up the same scan code here to
//! learn the QWERTY character of that position. Extended (prefixed) codes and the ISO extra
//! key are deliberately absent: layouts assign them freely, the numpad is layout
//! independent, and neither carries an app binding.

/// QWERTY characters (unshifted, shifted) of the key with this scan code.
pub const fn qwerty_pair(scan: u32) -> Option<(char, char)> {
    match scan {
        // Number row.
        0x02 => Some(('1', '!')),
        0x03 => Some(('2', '@')),
        0x04 => Some(('3', '#')),
        0x05 => Some(('4', '$')),
        0x06 => Some(('5', '%')),
        0x07 => Some(('6', '^')),
        0x08 => Some(('7', '&')),
        0x09 => Some(('8', '*')),
        0x0A => Some(('9', '(')),
        0x0B => Some(('0', ')')),
        0x0C => Some(('-', '_')),
        0x0D => Some(('=', '+')),
        // Top letter row.
        0x10 => Some(('q', 'Q')),
        0x11 => Some(('w', 'W')),
        0x12 => Some(('e', 'E')),
        0x13 => Some(('r', 'R')),
        0x14 => Some(('t', 'T')),
        0x15 => Some(('y', 'Y')),
        0x16 => Some(('u', 'U')),
        0x17 => Some(('i', 'I')),
        0x18 => Some(('o', 'O')),
        0x19 => Some(('p', 'P')),
        0x1A => Some(('[', '{')),
        0x1B => Some((']', '}')),
        0x2B => Some(('\\', '|')),
        // Home row.
        0x1E => Some(('a', 'A')),
        0x1F => Some(('s', 'S')),
        0x20 => Some(('d', 'D')),
        0x21 => Some(('f', 'F')),
        0x22 => Some(('g', 'G')),
        0x23 => Some(('h', 'H')),
        0x24 => Some(('j', 'J')),
        0x25 => Some(('k', 'K')),
        0x26 => Some(('l', 'L')),
        0x27 => Some((';', ':')),
        0x28 => Some(('\'', '"')),
        // Bottom letter row.
        0x2C => Some(('z', 'Z')),
        0x2D => Some(('x', 'X')),
        0x2E => Some(('c', 'C')),
        0x2F => Some(('v', 'V')),
        0x30 => Some(('b', 'B')),
        0x31 => Some(('n', 'N')),
        0x32 => Some(('m', 'M')),
        0x33 => Some((',', '<')),
        0x34 => Some(('.', '>')),
        0x35 => Some(('/', '?')),
        // Grave accent and space.
        0x29 => Some(('`', '~')),
        0x39 => Some((' ', ' ')),
        _ => None,
    }
}
