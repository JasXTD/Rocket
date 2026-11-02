//! Display helpers. Original Git bytes remain the source for every operation.

/// Bound raw bytes without cutting a valid UTF-8 scalar. Invalid original bytes
/// remain in the slice so the caller can still show their replacement glyphs.
pub(crate) fn prefix(bytes: &[u8], limit: usize) -> &[u8] {
    let end = bytes.len().min(limit);
    if end == bytes.len() {
        return bytes;
    }
    let mut start = end;
    while start > 0 && end - start < 3 && bytes[start] & 0xc0 == 0x80 {
        start -= 1;
    }
    if start < end {
        for after in end + 1..=end.saturating_add(3).min(bytes.len()) {
            if std::str::from_utf8(&bytes[start..after]).is_ok() {
                return &bytes[..start];
            }
        }
    }
    &bytes[..end]
}

/// Decode one Git C-quoted path token, including its closing quote. This is only
/// for labels; raw patches and RepoPath bytes still drive all Git operations.
pub(crate) fn unquote_path(input: &[u8]) -> Option<(Vec<u8>, usize)> {
    if input.first() != Some(&b'"') {
        return None;
    }
    let mut decoded = Vec::new();
    let mut i = 1;
    while let Some(&byte) = input.get(i) {
        i += 1;
        match byte {
            b'"' => return Some((decoded, i)),
            b'\\' => {
                let escape = *input.get(i)?;
                i += 1;
                decoded.push(match escape {
                    b'"' | b'\\' => escape,
                    b'a' => 7,
                    b'b' => 8,
                    b't' => b'\t',
                    b'n' => b'\n',
                    b'v' => 11,
                    b'f' => 12,
                    b'r' => b'\r',
                    b'0'..=b'3' => {
                        let second = *input.get(i)?;
                        let third = *input.get(i + 1)?;
                        if !(b'0'..=b'7').contains(&second) || !(b'0'..=b'7').contains(&third) {
                            return None;
                        }
                        i += 2;
                        (escape - b'0') * 64 + (second - b'0') * 8 + (third - b'0')
                    }
                    _ => return None,
                });
            }
            _ => decoded.push(byte),
        }
    }
    None
}

pub(crate) fn path_label(input: &str) -> String {
    quoted_path_label(input, true)
}

/// Git appends a tab delimiter to ---/+++ names containing spaces. Literal
/// filename tabs are C-escaped inside quotes, so only strip the outside delimiter.
pub(crate) fn diff_file_label(input: &str) -> String {
    path_label(input.strip_suffix('\t').unwrap_or(input))
}

/// Rename/copy metadata uses repository-relative names, with no synthetic a/b prefix.
pub(crate) fn literal_path_label(input: &str) -> String {
    quoted_path_label(input, false)
}

fn quoted_path_label(input: &str, strip_diff_prefix: bool) -> String {
    let decoded = if input.starts_with('"') {
        match unquote_path(input.as_bytes()) {
            Some((bytes, used)) if used == input.len() => bytes,
            _ => return input.into(), // Preserve malformed/unknown quoting verbatim.
        }
    } else {
        input.as_bytes().to_vec()
    };
    let path = if strip_diff_prefix {
        decoded
            .strip_prefix(b"a/")
            .or_else(|| decoded.strip_prefix(b"b/"))
            .unwrap_or(&decoded)
    } else {
        &decoded
    };
    raw_path_label(path)
}

pub(crate) fn raw_path_label(path: &[u8]) -> String {
    let mut label = String::with_capacity(path.len());
    for c in String::from_utf8_lossy(path).chars() {
        // Keep tabs/newlines visible rather than letting a path alter the UI layout.
        if c.is_control() {
            label.extend(c.escape_default());
        } else {
            label.push(c);
        }
    }
    label
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_byte_boundary_preserves_complete_unicode_scalars() {
        let original = "ASCII é e\u{301} 中文 日本語 한국어 🚀 end";
        for limit in 0..original.len() + 4 {
            let shown = std::str::from_utf8(prefix(original.as_bytes(), limit)).unwrap();
            assert!(original.starts_with(shown));
            assert!(shown.len() <= limit);
            assert_eq!(
                shown.len(),
                original.floor_char_boundary(limit.min(original.len()))
            );
        }
    }

    #[test]
    fn invalid_original_bytes_are_not_hidden() {
        assert_eq!(prefix(b"\xffa\xe6\x96\x87", 4), b"\xffa");
        assert_eq!(prefix(b"a\x80b", 1), b"a");
        assert_eq!(prefix(b"a\xe6\x96", 9), b"a\xe6\x96");
    }

    #[test]
    fn quoted_paths_decode_bytes_before_utf8_and_keep_controls_visible() {
        assert_eq!(
            path_label(r#""b/\346\226\207\t\"x\".txt""#),
            "文\\t\"x\".txt"
        );
        assert_eq!(path_label(r#""b/\377.txt""#), "\u{fffd}.txt");
        assert_eq!(path_label("b/資料/文 space.txt"), "資料/文 space.txt");
        for malformed in [
            r#""b/\777""#,
            r#""b/\q""#,
            r#""b/\34""#,
            "\"b/truncated",
            "\"b/end\"suffix",
        ] {
            assert_eq!(path_label(malformed), malformed);
        }
    }
}
