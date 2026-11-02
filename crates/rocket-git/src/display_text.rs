//! Bounded, visible representations of untrusted labels. Never use for Git inputs.

#[derive(Clone)]
pub(crate) struct DisplayText {
    pub text: String,
    pub escaped: bool,
    pub truncated: bool,
}

pub(crate) fn hidden(c: char) -> bool {
    c.is_control()
        || matches!(c, '\u{ad}' | '\u{61c}' | '\u{200b}'..='\u{200f}'
            | '\u{2028}'..='\u{202e}' | '\u{2060}'..='\u{206f}'
            | '\u{feff}' | '\u{fff9}'..='\u{fffb}')
}

/// Limit output bytes, including escapes and the ellipsis. Stop reading once full.
pub(crate) fn bounded(value: &str, limit: usize) -> DisplayText {
    let mut result = DisplayText {
        text: String::with_capacity(value.len().min(limit)),
        escaped: false,
        truncated: false,
    };
    // Retain character/escape boundaries so adding an ellipsis cannot split either.
    let mut boundaries = Vec::new();
    for c in value.chars() {
        let escape = hidden(c) || c == '\\';
        let piece = if escape {
            if hidden(c) && !c.is_control() {
                format!("\\u{{{:x}}}", c as u32)
            } else {
                c.escape_debug().to_string()
            }
        } else {
            c.to_string()
        };
        if result.text.len() + piece.len() > limit {
            result.truncated = true;
            if limit >= '…'.len_utf8() {
                while result.text.len() + '…'.len_utf8() > limit {
                    result.text.truncate(boundaries.pop().unwrap_or(0));
                }
                result.text.push('…');
            }
            break;
        }
        boundaries.push(result.text.len());
        result.text.push_str(&piece);
        result.escaped |= escape;
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ordinary_unicode_is_preserved_and_controls_are_visible() {
        let ordinary = "作者 العربية עברית 界 🚀";
        let text = bounded(ordinary, 256);
        assert_eq!(text.text, ordinary);
        assert!(!text.escaped && !text.truncated);
        let text = bounded("safe\n\r\t\u{1b}\u{202e}\u{2066}\u{61c}\u{200f}\\end", 256);
        assert_eq!(
            text.text,
            "safe\\n\\r\\t\\u{1b}\\u{202e}\\u{2066}\\u{61c}\\u{200f}\\\\end"
        );
        assert!(text.escaped && !text.truncated);
        assert!(!text.text.chars().any(hidden));
    }

    #[test]
    fn output_caps_include_unicode_escapes_and_ellipsis() {
        for value in ["abcdef", "界🚀界🚀", "\u{202e}\u{2066}\n", "a\\b"] {
            for limit in 0..32 {
                let text = bounded(value, limit);
                assert!(text.text.len() <= limit);
                assert!(!text.text.chars().any(hidden));
                if !text.truncated {
                    assert_eq!(text.text, bounded(value, 256).text);
                }
            }
        }
        assert_eq!(bounded("abc", 3).text, "abc");
        assert_eq!(bounded("\u{202e}abcdef", 9).text, "…");
        assert_eq!(bounded(&"x".repeat(1024 * 1024), 64).text.len(), 64);
    }
}
