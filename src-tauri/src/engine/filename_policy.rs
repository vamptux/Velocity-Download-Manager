const MAX_FILENAME_CHARS: usize = 240;

pub(super) fn normalize_filename_hint(value: &str) -> String {
    sanitize_filename_leaf(value)
        .unwrap_or_else(|| decode_percent_encoded(value).unwrap_or_else(|| value.to_string()))
}

pub(super) fn sanitize_filename_leaf(value: &str) -> Option<String> {
    let decoded = value
        .contains('%')
        .then(|| decode_percent_encoded(value))
        .flatten();
    let leaf_source = decoded.as_deref().unwrap_or(value);
    let leaf = leaf_source
        .trim()
        .rsplit(['/', '\\'])
        .find(|segment| !segment.trim().is_empty())
        .unwrap_or(leaf_source)
        .trim();
    if leaf.is_empty() {
        return None;
    }

    let mut sanitized = String::with_capacity(leaf.len());
    for character in leaf.chars() {
        match character {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => sanitized.push('_'),
            c if c.is_control() => sanitized.push('_'),
            '\u{200B}'..='\u{200F}'
            | '\u{202A}'..='\u{202E}'
            | '\u{2060}'..='\u{2064}'
            | '\u{FEFF}' => {}
            c => sanitized.push(c),
        }
    }

    let sanitized = collapse_spaces(sanitized);
    let sanitized = sanitized.trim_matches([' ', '.']).to_string();
    if sanitized.is_empty() {
        return None;
    }

    let sanitized = guard_windows_reserved(sanitized);
    let sanitized = enforce_filename_length(sanitized, MAX_FILENAME_CHARS);
    if sanitized.is_empty() {
        return None;
    }

    Some(sanitized)
}

pub(super) fn decode_percent_encoded(value: &str) -> Option<String> {
    let bytes = value.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;

    while index < bytes.len() {
        if bytes[index] == b'%' {
            if index + 2 >= bytes.len() {
                return None;
            }
            let high = decode_hex_digit(bytes[index + 1])?;
            let low = decode_hex_digit(bytes[index + 2])?;
            decoded.push((high << 4) | low);
            index += 3;
            continue;
        }
        decoded.push(bytes[index]);
        index += 1;
    }

    String::from_utf8(decoded).ok()
}

pub(super) fn guard_windows_reserved(mut s: String) -> String {
    const RESERVED: [&str; 22] = [
        "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
        "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
    ];
    let stem = s.split('.').next().unwrap_or(&s);
    if RESERVED
        .iter()
        .any(|&reserved| stem.eq_ignore_ascii_case(reserved))
    {
        s.insert(0, '_');
    }
    s
}

fn collapse_spaces(s: String) -> String {
    let mut out = String::with_capacity(s.len());
    let mut prev_space = false;
    for c in s.chars() {
        if c == ' ' {
            if !prev_space {
                out.push(c);
            }
            prev_space = true;
        } else {
            prev_space = false;
            out.push(c);
        }
    }
    out
}

fn decode_hex_digit(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}

fn enforce_filename_length(s: String, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        return s;
    }
    if let Some((stem, ext)) = s.rsplit_once('.') {
        let ext_with_dot = format!(".{ext}");
        let ext_char_count = ext_with_dot.chars().count();
        let stem_budget = max_chars.saturating_sub(ext_char_count);
        let truncated: String = stem.chars().take(stem_budget).collect();
        format!("{truncated}{ext_with_dot}")
    } else {
        s.chars().take(max_chars).collect()
    }
}
