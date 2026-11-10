#!/usr/bin/env python3
"""Generate layout character to QWERTY character tables from xkeyboard-config data.

The tables let the TUI bind keys to the physical QWERTY position on platforms whose keymap
data is readable (X11), where the terminal only reports the typed character. Reference
positions come from us(basic) plus pc(pc105), so a pair means: the key that types `from` on
this layout types `to` on the US layout.

Run it on a machine with xkeyboard-config installed, e.g. from WSL:

    python3 scripts/gen-keymap-tables.py --out src/app/keymap/layouts
    python3 scripts/gen-keymap-tables.py --out src/app/keymap/layouts --print-rust

Only pairs whose target carries an app binding are kept: ASCII letters and the bound symbols
`/`, `?`, `~`, backquote and space. Pairs that would move a bound symbol to an unbound one
(the Russian `/` key on X11) are dropped, so bindings stay predictable.
"""

from __future__ import annotations

import argparse
import re
import sys
from pathlib import Path

# Reference layout: US symbols for the standard PC key names.
REFERENCE = [("us", "basic"), ("pc", "pc105")]

# (name, symbols file, xkb_symbols section), the names `--key-map` accepts.
LAYOUTS = [
    ("ru", "ru", "winkeys"),
    ("ru-typewriter", "ru", "typewriter"),
    ("ru-phonetic", "ru", "phonetic_winkeys"),
    ("ru-legacy", "ru", "legacy"),
    ("ua", "ua", "winkeys"),
    ("ua-typewriter", "ua", "typewriter"),
    ("ua-phonetic", "ua", "phonetic"),
    ("by", "by", "basic"),
    ("bg", "bg", "bds"),
    ("mk", "mk", "basic"),
    ("rs", "rs", "basic"),
    ("hr", "hr", "basic"),
    ("si", "si", "basic"),
    ("cz", "cz", "basic"),
    ("sk", "sk", "basic"),
    ("hu", "hu", "basic"),
    ("md", "md", "basic"),
    ("de", "de", "basic"),
    ("at", "at", "basic"),
    ("ch", "ch", "basic"),
    ("fr", "fr", "basic"),
    ("be", "be", "basic"),
    ("es", "es", "basic"),
    ("pt", "pt", "basic"),
    ("it", "it", "basic"),
    ("nl", "nl", "basic"),
    ("se", "se", "basic"),
    ("no", "no", "basic"),
    ("dk", "dk", "basic"),
    ("fi", "fi", "winkeys"),
    ("is", "is", "basic"),
    ("ee", "ee", "basic"),
    ("tr", "tr", "basic"),
    ("gr", "gr", "basic"),
    ("kz", "kz", "basic"),
    ("th", "th", "basic"),
    ("ir", "ir", "pes"),
    ("ara", "ara", "basic"),
    ("ge", "ge", "basic"),
    ("am", "am", "basic"),
    ("il", "il", "basic"),
    ("br", "br", "abnt2"),
    ("us-dvorak", "us", "dvorak"),
    ("us-colemak", "us", "colemak"),
    ("us-workman", "us", "workman"),
    ("us-norman", "us", "norman"),
    ("us-dvp", "us", "dvp"),
]

SECTION_RE = re.compile(r'xkb_symbols\s+"([^"]+)"\s*\{')
KEY_RE = re.compile(r"key\s*<([A-Za-z0-9]+)>\s*\{([^{}]*)\}", re.S)
INCLUDE_RE = re.compile(r'include\s+"([^"]+)"')
QUALIFIER_RE = re.compile(r"(?:type|symbols|groups|interpret)\s*(?:\[[^\]]*\])?\s*=\s*", re.I)
LIST_RE = re.compile(r"\[([^\]]*)\]")
MODIFIER_RE = re.compile(r"\+\w+")

# Characters the app binds, so pairs with any other target are noise.
BOUND_TARGETS = set("/?~` ")


def bindable(target: str) -> bool:
    return target.isascii() and (target.isalpha() or target in BOUND_TARGETS)


# X11 keysym names SILV tables meet: punctuation, Latin-1 letters, Cyrillic letters.
NAMES = {
    "exclam": "!",
    "at": "@",
    "numbersign": "#",
    "dollar": "$",
    "percent": "%",
    "asciicircum": "^",
    "ampersand": "&",
    "asterisk": "*",
    "parenleft": "(",
    "parenright": ")",
    "minus": "-",
    "equal": "=",
    "plus": "+",
    "underscore": "_",
    "comma": ",",
    "period": ".",
    "slash": "/",
    "colon": ":",
    "semicolon": ";",
    "less": "<",
    "greater": ">",
    "question": "?",
    "apostrophe": "'",
    "quotedbl": '"',
    "bracketleft": "[",
    "bracketright": "]",
    "backslash": "\\",
    "bar": "|",
    "asciitilde": "~",
    "grave": "`",
    "section": "§",
    "currency": "¤",
    "degree": "°",
    "plusminus": "±",
    "micro": "µ",
    "sterling": "£",
    "euro": "€",
    "cent": "¢",
    "yen": "¥",
    "copyright": "©",
    "registered": "®",
    "guillemotleft": "«",
    "guillemotright": "»",
    "brokenbar": "¦",
    "macron": "¯",
    "periodcentered": "·",
    "cedilla": "¸",
    "diaeresis": "¨",
    "acute": "´",
    "dieresis": "¨",
}

LATIN1 = {
    "agrave": "à",
    "aacute": "á",
    "acircumflex": "â",
    "atilde": "ã",
    "aumlaut": "ä",
    "adiaeresis": "ä",
    "aring": "å",
    "ae": "æ",
    "ccedilla": "ç",
    "egrave": "è",
    "eacute": "é",
    "ecircumflex": "ê",
    "eumlaut": "ë",
    "ediaeresis": "ë",
    "igrave": "ì",
    "iacute": "í",
    "icircumflex": "î",
    "iumlaut": "ï",
    "idiaeresis": "ï",
    "ntilde": "ñ",
    "ograve": "ò",
    "oacute": "ó",
    "ocircumflex": "ô",
    "otilde": "õ",
    "oumlaut": "ö",
    "odiaeresis": "ö",
    "oslash": "ø",
    "ugrave": "ù",
    "uacute": "ú",
    "ucircumflex": "û",
    "uumlaut": "ü",
    "udiaeresis": "ü",
    "yacute": "ý",
    "yumlaut": "ÿ",
    "thorn": "þ",
    "eth": "ð",
    "ssharp": "ß",
}

# X11 Cyrillic keysym names: suffix -> character (lowercase form).
CYRILLIC = {
    "a": "а",
    "be": "б",
    "ve": "в",
    "ghe": "г",
    "de": "д",
    "ie": "е",
    "e": "э",
    "io": "ё",
    "zhe": "ж",
    "ze": "з",
    "ka": "к",
    "el": "л",
    "em": "м",
    "en": "н",
    "o": "о",
    "pe": "п",
    "er": "р",
    "te": "т",
    "u": "у",
    "ef": "ф",
    "ha": "х",
    "tsse": "ц",
    "tse": "ц",
    "che": "ч",
    "sha": "ш",
    "shcha": "щ",
    "hardsign": "ъ",
    "yery": "ы",
    "softsign": "ь",
    "yu": "ю",
    "ya": "я",
    "shorti": "й",
    "ishort": "й",
    "i": "і",
    "yi": "ї",
    "je": "є",
    "dzhe": "џ",
    "gje": "ѓ",
    "kjje": "ќ",
    "lje": "љ",
    "nje": "њ",
    "dze": "ѕ",
    "shortu": "ў",
    "fita": "ѳ",
    "izhitsa": "ѵ",
    "zhde": "ѫ",
    "dt": "Ѧ",
    "small_dt": "ѧ",
    "cyrfolik": "Ѳ",
    "ukrainian_ge": "ґ",
    "ghe_with_upturn": "ґ",
    "komi_ik": "ӏ",
    "komi_kje": "ӂ",
    "khakassian_high_y": "ӳ",
    "che_with_descender": "җ",
    "u_straight": "ү",
    "ka_with_struke": "ҝ",
    "shha": "һ",
    "en_descender": "ң",
    "en_with_descender": "ң",
    "o_straight": "ө",
    "o_with_descender": "ө",
    "o_with_stroke": "ө",
    "zhe_with_descender": "ҁ",
    "komi_ik": "ӏ",
    "komi_kje": "ӂ",
    "khakassian_high_y": "ӳ",
    "che_with_descender": "җ",
}

# Keysym names read from the X11 headers, filled by main() when the headers are present.
KEYSYMS: dict[str, str] = {}

# Names that never denote a character the app can use: key actions, modifiers, dead keys,
# composing marks, keypad and navigation keys.
IGNORED = re.compile(
    r"^(NoSymbol|VoidSymbol|space|Return|Enter|Tab|BackSpace|Delete|Insert|Escape|Linefeed|"
    r"Pause|Print|Scroll[Ll]ock|Caps[Ll]ock|Shift.*|Control.*|Alt.*|Meta.*|Super.*|Hyper.*|"
    r"Num.*|KP_.*|.*_Lock|.*_L|.*_R|.*_Switch|AltGr|Level.*|Lock.*|Multi_.*|ISO_.*|dead_.*|"
    r"dead.*|.*[Aa]cute|.*[Bb]reve|.*[Dd]iaeresis|.*[Cc]ircumflex|.*[Tt]ilde|.*[Oo]gonek|"
    r".*[Cc]edilla|.*[Aa]bovedot|.*[Mm]acron|.*[Aa]bovering|.*[Dd]oubleacute|.*[Cc]aron|"
    r".*[Hh]ookabove|.*[Dd]otbelow|.*[Mm]acronbelow|.*cedillabelow|.*[Bb]revebelow|"
    r".*[Cc]ommaaccent|.*[Dd]blgrave|.*[Ii]nvertedbreve|.*[Oo]gonek|.*ogonekbelow|"
    r".*[Ee]mn?above|.*[Cc]ommaabove|.*[Dd]otbelow|.*[Mm]acronbelow)$",
    re.I,
)


def chr_in_scope(code: int) -> str | None:
    """Character of a codepoint, limited to the scripts the tables need."""
    allowed = (
        0x20 <= code <= 0x7E
        or 0xA0 <= code <= 0x24F
        or 0x370 <= code <= 0x06FF
        or 0x1000 <= code <= 0x10FF
        or 0x0E00 <= code <= 0x109F
    )
    if not allowed:
        return None
    char = chr(code)
    return char if char.isprintable() and char != " " else None


KEYSYM_RE = re.compile(
    r"#define\s+XK_(\S+)\s+0x([0-9a-fA-F]+)(?:\s+/\*\s*U\+([0-9a-fA-F]{4,6})\s+([^*]*)\*/)?",
)


def load_keysyms(path: Path) -> dict[str, str]:
    """Keysym names to characters, read from the X11 headers when they are installed."""
    if not path.is_file():
        return {}
    mapping: dict[str, str] = {}
    for match in KEYSYM_RE.finditer(path.read_text(encoding="utf-8", errors="replace")):
        name, value, unicode_hint, label = match.groups()
        if label and label.strip().upper().startswith("COMBINING"):
            continue
        number = int(unicode_hint, 16) if unicode_hint else int(value, 16)
        if number >= 0x01000000:
            number &= 0xFFFFFF
        char = chr_in_scope(number)
        if char and char.isalpha():
            mapping[name] = char
    return mapping


def keysym_to_char(name: str) -> str | None:
    """Character a keysym name denotes, None for key actions, dead keys and unknown names."""
    name = name.strip()
    if not name:
        return None
    if name.startswith("'"):
        literal = name[1:]
        return literal if len(literal) == 1 else None
    if len(name) == 1:
        return name
    if IGNORED.match(name):
        return None
    from_header = KEYSYMS.get(name)
    if from_header:
        return from_header
    table = LATIN1
    if name in NAMES:
        return NAMES[name]
    if name in table:
        return table[name]
    lowered = name.lower()
    if lowered in NAMES:
        return NAMES[lowered]
    if lowered in table:
        return table[lowered]
    if name.startswith("uni") and len(name) in (7, 8, 9):
        return chr_from_hex(name[3:])
    if name.startswith("U") and len(name) in (5, 6, 7):
        return chr_from_hex(name[1:])
    for prefix in ("Cyrillic_", "Ukrainian_", "Macedonia_", "Serbian_", "Bulgarian_"):
        if name.startswith(prefix):
            suffix = name[len(prefix) :]
            entry = CYRILLIC.get(suffix.lower())
            if entry:
                return entry if suffix[0].islower() else entry.upper()
    return None


def chr_from_hex(digits: str) -> str | None:
    try:
        value = int(digits, 16)
    except ValueError:
        return None
    if value > 0x10FFFF:
        return None
    char = chr(value)
    return char if char.isprintable() and len(char) == 1 else None


def section_bodies(text: str) -> dict[str, str]:
    """Section name to body text, matching braces from each xkb_symbols header."""
    bodies: dict[str, str] = {}
    for match in SECTION_RE.finditer(text):
        name = match.group(1)
        depth = 1
        index = match.end()
        while index < len(text) and depth:
            if text[index] == "{":
                depth += 1
            elif text[index] == "}":
                depth -= 1
            index += 1
        if depth == 0:
            bodies.setdefault(name, text[match.end() : index - 1])
    return bodies


def default_section(bodies: dict[str, str], text: str) -> str | None:
    """Section the plain `include "file"` form resolves to: the first `default` one."""
    for match in re.finditer(r'(default|hidden|partial)\s*xkb_symbols\s+"([^"]+)"', text):
        if match.group(1) == "default" and match.group(2) in bodies:
            return match.group(2)
    return next(iter(bodies), None)


def read_file(symbols: Path, name: str) -> str:
    path = symbols / name
    if not path.is_file():
        return ""
    return path.read_text(encoding="utf-8", errors="replace")


def layout_keys(symbols: Path, file: str, section: str, stack: tuple[str, ...] = ()) -> dict[str, list[str]]:
    """Key name to symbol list, includes first, so the section's own keys win."""
    path = f"{file}({section})"
    if path in stack:
        return {}
    text = read_file(symbols, file)
    if not text:
        return {}
    bodies = section_bodies(text)
    body = bodies.get(section)
    if body is None:
        return {}
    keys: dict[str, list[str]] = {}
    for include in INCLUDE_RE.findall(body):
        target, _, included = include.partition("(")
        target = target.strip()
        included = included.rstrip(")").strip()
        if not target:
            continue
        if not included:
            included = default_section(section_bodies(read_file(symbols, target)), read_file(symbols, target)) or ""
        for key, symbols_list in layout_keys(symbols, target, included, (*stack, path)).items():
            keys[key] = symbols_list
    for match in KEY_RE.finditer(body):
        key, statement = match.group(1), match.group(2)
        statement = QUALIFIER_RE.sub("", statement)
        listing = LIST_RE.search(statement)
        if not listing:
            continue
        fields = [field.strip() for field in listing.group(1).split(",")]
        fields = [field for field in fields if field and not MODIFIER_RE.fullmatch(field)]
        if not fields:
            continue
        inherited = keys.get(key)
        if not inherited:
            keys[key] = fields
            continue
        # `any` and NoSymbol are placeholders in partial sections: keep the inherited level.
        merged = list(inherited)
        for index, value in enumerate(fields):
            if value.lower() in ("any", "nosymbol", "voidsymbol"):
                continue
            if index < len(merged):
                merged[index] = value
            else:
                merged.append(value)
        keys[key] = merged
    return keys


def reference_keys(symbols: Path) -> dict[str, list[str]]:
    keys: dict[str, list[str]] = {}
    for file, section in REFERENCE:
        keys.update(layout_keys(symbols, file, section))
    return keys


def build_table(symbols: Path, reference: dict[str, list[str]], section_file: str, section_name: str):
    keys = layout_keys(symbols, section_file, section_name)
    pairs: list[tuple[str, str]] = []
    for key, listing in keys.items():
        base = reference.get(key)
        if not base:
            continue
        for level in (0, 1):
            if level >= len(listing) or level >= len(base):
                continue
            source = keysym_to_char(listing[level])
            target = keysym_to_char(base[level])
            if not source or not target:
                continue
            if len(source) != 1 or len(target) != 1:
                continue
            if source == target or source.isascii() and not source.isprintable():
                continue
            if not bindable(target):
                continue
            pairs.append((source, target))
    return pairs


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--symbols", default="/usr/share/X11/xkb/symbols", help="xkeyboard-config symbols directory")
    parser.add_argument("--keysyms", default="/usr/include/X11/keysymdef.h", help="X11 keysymdef.h for script names")
    parser.add_argument("--out", required=True, help="directory for the generated tables")
    parser.add_argument("--print-rust", action="store_true", help="print the LAYOUTS constant of keymap/xkb.rs")
    args = parser.parse_args()

    global KEYSYMS
    KEYSYMS = load_keysyms(Path(args.keysyms))
    print(f"{len(KEYSYMS)} keysym names from {args.keysyms}", file=sys.stderr)

    symbols = Path(args.symbols)
    if not symbols.is_dir():
        print(f"error: {symbols} is not a directory (run this on Linux with xkeyboard-config installed)", file=sys.stderr)
        return 2
    out = Path(args.out)
    out.mkdir(parents=True, exist_ok=True)
    reference = reference_keys(symbols)
    if not reference:
        print(f"error: no reference keys parsed from {symbols} (us basic, pc pc105)", file=sys.stderr)
        return 2

    written: list[str] = []
    for name, file, section in LAYOUTS:
        pairs = build_table(symbols, reference, file, section)
        if not pairs:
            print(f"skipped {name}: {file}({section}) produced no pairs", file=sys.stderr)
            continue
        unique: dict[str, str] = {}
        for source, target in pairs:
            unique.setdefault(source, target)
        lines = [
            f"# {name}: layout characters and the QWERTY character of the same key.",
            f"# generated from xkeyboard-config by scripts/gen-keymap-tables.py, do not edit.",
            f"# from {file}({section}), reference us(basic) and pc(pc105), {len(unique)} pairs.",
        ]
        lines += [f"{source}\t{target}" for source, target in sorted(unique.items())]
        (out / f"{name}.txt").write_text("\n".join(lines) + "\n", encoding="utf-8", newline="\n")
        written.append(name)
        print(f"{name}: {len(unique)} pairs")

    if args.print_rust:
        print("\n// generated by scripts/gen-keymap-tables.py")
        print("const LAYOUTS: &[(&str, &str)] = &[")
        for name in written:
            print(f'    ("{name}", include_str!("layouts/{name}.txt")),')
        print("};")
    print(f"{len(written)} tables in {out}", file=sys.stderr)
    return 0


if __name__ == "__main__":
    sys.exit(main())
