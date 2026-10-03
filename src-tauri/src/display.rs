//! Linux names as text the window can show safely (spec §5.8).

use std::fmt::Write;

/// `raw` as text: bytes that are not UTF-8 become U+FFFD, and characters that are invisible
/// or reorder text are shown as `⟨U+XXXX⟩`, so a name cannot disguise its extension
/// (`photo\u{202E}gpj.exe` reads as `photo⟨U+202E⟩gpj.exe`).
pub fn display_name(raw: &[u8]) -> String {
    let mut out = String::with_capacity(raw.len());
    for c in String::from_utf8_lossy(raw).chars() {
        if hidden(c) {
            let _ = write!(out, "⟨U+{:04X}⟩", u32::from(c));
        } else {
            out.push(c);
        }
    }
    out
}

/// Control characters, bidirectional formatting and zero-width characters. Variation
/// selectors and emoji tag characters are left alone, so emoji keep their look.
fn hidden(c: char) -> bool {
    c.is_control()
        || matches!(
            c,
            '\u{00AD}'
                | '\u{034F}'
                | '\u{061C}'
                | '\u{115F}'
                | '\u{1160}'
                | '\u{180E}'
                | '\u{200B}'..='\u{200F}'
                | '\u{202A}'..='\u{202E}'
                | '\u{2060}'..='\u{2064}'
                | '\u{2066}'..='\u{206F}'
                | '\u{3164}'
                | '\u{FEFF}'
                | '\u{FFA0}'
                | '\u{FFF9}'..='\u{FFFB}'
        )
}

#[cfg(test)]
mod tests {
    use super::display_name;

    #[test]
    fn invisible_and_reordering_characters_are_shown() {
        assert_eq!(
            display_name("photo\u{202E}gpj.exe".as_bytes()),
            "photo⟨U+202E⟩gpj.exe"
        );
        assert_eq!(display_name(b"a\x01b\x7f"), "a⟨U+0001⟩b⟨U+007F⟩");
        assert_eq!(display_name("a\u{200B}b".as_bytes()), "a⟨U+200B⟩b");
        assert_eq!(display_name(b"bad-\xff.txt"), "bad-\u{FFFD}.txt");
    }

    #[test]
    fn ordinary_names_in_any_script_are_unchanged() {
        for name in [
            "türkçe-çğıöşü.txt",
            "日本語.txt",
            "emoji-😀.txt",
            "❤️",
            "a b.c",
        ] {
            assert_eq!(display_name(name.as_bytes()), name);
        }
    }
}
