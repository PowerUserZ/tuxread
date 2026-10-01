//! Turning Linux file names into names Windows accepts (spec §6.2, rules 1-5).
//! Collision handling (rule 6) lives in `copy`, because it depends on the destination.

const INVALID: &[char] = &['<', '>', ':', '"', '/', '\\', '|', '?', '*'];

const RESERVED: &[&str] = &[
    "CON", "PRN", "AUX", "NUL", "CONIN$", "CONOUT$", "COM1", "COM2", "COM3", "COM4", "COM5",
    "COM6", "COM7", "COM8", "COM9", "COM¹", "COM²", "COM³", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5",
    "LPT6", "LPT7", "LPT8", "LPT9", "LPT¹", "LPT²", "LPT³",
];

/// Longest name NTFS stores, in UTF-16 code units.
pub const MAX_UTF16: usize = 255;

/// A Windows-safe version of one Linux name component. Never empty.
pub fn sanitize(raw: &[u8]) -> String {
    // Rule 1: bytes that are not UTF-8 become U+FFFD.
    let lossy = String::from_utf8_lossy(raw);
    // Rule 2: characters Windows forbids become '_'.
    let mut name: String = lossy
        .chars()
        .map(|c| {
            if INVALID.contains(&c) || (c as u32) < 0x20 {
                '_'
            } else {
                c
            }
        })
        .collect();
    // Rule 3: trailing dots and spaces become '_'.
    let kept = name.trim_end_matches(['.', ' ']).len();
    let trailing = name.len() - kept;
    name.truncate(kept);
    name.extend(std::iter::repeat_n('_', trailing));
    // Rule 4: reserved device names, with or without an extension, get a '_' prefix.
    let stem = name
        .split('.')
        .next()
        .unwrap_or_default()
        .trim_end_matches(' ');
    if RESERVED
        .iter()
        .any(|r| r.to_uppercase() == stem.to_uppercase())
    {
        name.insert(0, '_');
    }
    // Rule 5: at most 255 UTF-16 units, keeping the extension.
    if name.encode_utf16().count() > MAX_UTF16 {
        name = shorten(&name);
    }
    if name.is_empty() {
        "_".to_string()
    } else {
        name
    }
}

fn shorten(name: &str) -> String {
    let (stem, ext) = match name.rfind('.') {
        Some(dot)
            if dot > 0
                && name
                    .get(dot..)
                    .is_some_and(|ext| ext.encode_utf16().count() <= 32) =>
        {
            name.split_at(dot)
        }
        _ => (name, ""),
    };
    let budget = MAX_UTF16 - ext.encode_utf16().count();
    let mut out = String::new();
    let mut used = 0;
    for c in stem.chars() {
        used += c.len_utf16();
        if used > budget {
            break;
        }
        out.push(c);
    }
    out.push_str(ext);
    out
}

#[cfg(test)]
mod tests {
    use super::sanitize;

    #[test]
    fn rules_one_to_five() {
        assert_eq!(sanitize(b"bad-\xFF\xFE.txt"), "bad-\u{FFFD}\u{FFFD}.txt");
        assert_eq!(sanitize(b"a:b<c>d\"e|f?g*h\\i\x01"), "a_b_c_d_e_f_g_h_i_");
        assert_eq!(sanitize(b"trailing."), "trailing_");
        assert_eq!(sanitize(b"x. ."), "x___");
        assert_eq!(sanitize(b"CON"), "_CON");
        assert_eq!(sanitize(b"con.txt"), "_con.txt");
        assert_eq!(sanitize(b"nul.tar.gz"), "_nul.tar.gz");
        assert_eq!(sanitize("COM¹".as_bytes()), "_COM¹");
        assert_eq!(sanitize(b"CONOUT$"), "_CONOUT$");
        assert_eq!(sanitize(b"console"), "console");
        assert_eq!(
            sanitize("türkçe-çğıöşü 日本語 😀.txt".as_bytes()),
            "türkçe-çğıöşü 日本語 😀.txt"
        );
    }

    #[test]
    fn long_names_keep_their_extension() {
        let long = format!("{}.jpeg", "é".repeat(300));
        let out = sanitize(long.as_bytes());
        assert_eq!(out.encode_utf16().count(), 255);
        assert!(out.ends_with(".jpeg"));
        let emoji = "😀".repeat(200); // 2 UTF-16 units each
        let out = sanitize(emoji.as_bytes());
        assert!(out.encode_utf16().count() <= 255);
        assert!(out.chars().all(|c| c == '😀'));
    }
}
