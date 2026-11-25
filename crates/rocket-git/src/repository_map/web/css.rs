use super::{CancellationToken, LABEL_BYTES, Result};
use std::borrow::Cow;

fn space(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\n' | b'\r' | 0x0c)
}
fn name(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-') || b >= 0x80
}

/// Return the end of one CSS escape and its decoded character. A quoted
/// backslash-newline continues the string without adding a character.
fn escape(s: &str, at: usize) -> (usize, Option<char>) {
    let bytes = s.as_bytes();
    let start = at + 1;
    let mut end = start;
    while end < bytes.len() && end - start < 6 && bytes[end].is_ascii_hexdigit() {
        end += 1;
    }
    if end > start {
        let value = u32::from_str_radix(&s[start..end], 16)
            .ok()
            .and_then(char::from_u32)
            .filter(|c| *c != '\0')
            .unwrap_or('\u{fffd}');
        if bytes.get(end).copied().is_some_and(space) {
            end += if bytes[end..].starts_with(b"\r\n") { 2 } else { 1 };
        }
        return (end, Some(value));
    }
    let Some(c) = s[start..].chars().next() else {
        return (start, Some('\u{fffd}'));
    };
    if matches!(c, '\r' | '\n' | '\x0c') {
        return (start + if bytes[start..].starts_with(b"\r\n") { 2 } else { 1 }, None);
    }
    (start + c.len_utf8(), Some(c))
}

fn decoded(s: &str) -> Option<Cow<'_, str>> {
    if s.len() > LABEL_BYTES * 6 {
        return None;
    }
    if !s.contains('\\') {
        return Some(Cow::Borrowed(s));
    }
    let mut out = String::new();
    let mut rest = s;
    while let Some(at) = rest.find('\\') {
        out.push_str(&rest[..at]);
        let (end, value) = escape(rest, at);
        if let Some(c) = value {
            out.push(c);
        }
        rest = &rest[end..];
        if out.len() > LABEL_BYTES {
            return None;
        }
    }
    out.push_str(rest);
    Some(Cow::Owned(out))
}

struct Tokens<'a> {
    s: &'a str,
    at: usize,
}
impl<'a> Tokens<'a> {
    fn byte(&self) -> Option<u8> {
        self.s.as_bytes().get(self.at).copied()
    }
    fn spaces(&mut self) {
        while self.byte().is_some_and(space) {
            self.at += 1;
        }
    }
    fn trivia(&mut self) {
        loop {
            self.spaces();
            if !self.s[self.at..].starts_with("/*") {
                return;
            }
            self.at = self.s[self.at + 2..]
                .find("*/")
                .map_or(self.s.len(), |n| self.at + n + 4);
        }
    }
    fn ident(&mut self) -> &'a str {
        let start = self.at;
        while let Some(b) = self.byte() {
            if name(b) {
                self.at += 1;
            } else if b == b'\\' {
                self.at = escape(self.s, self.at).0;
            } else {
                break;
            }
        }
        &self.s[start..self.at]
    }
    fn string(&mut self) -> Option<&'a str> {
        let quote = self.byte()?;
        self.at += 1;
        let start = self.at;
        while let Some(b) = self.byte() {
            match b {
                b if b == quote => {
                    let end = self.at;
                    self.at += 1;
                    return Some(&self.s[start..end]);
                }
                b'\n' | b'\r' | 0x0c => return None,
                b'\\' => self.at = escape(self.s, self.at).0,
                _ => self.at += 1,
            }
        }
        None
    }
    fn bad_url(&mut self) -> Option<&'a str> {
        while let Some(b) = self.byte() {
            if b == b'\\' {
                self.at = escape(self.s, self.at).0;
            } else {
                self.at += 1;
                if b == b')' {
                    break;
                }
            }
        }
        None
    }
    fn url(&mut self) -> Option<&'a str> {
        if self.byte() != Some(b'(') {
            return None;
        }
        self.at += 1;
        self.spaces();
        let value = if matches!(self.byte(), Some(b'"' | b'\'')) {
            self.string()
        } else {
            let start = self.at;
            while let Some(b) = self.byte() {
                match b {
                    b')' => break,
                    b if space(b) => break,
                    b'"' | b'\'' | b'(' | 0..=8 | 11 | 14..=31 | 127 => return self.bad_url(),
                    b'\\' => {
                        let (end, c) = escape(self.s, self.at);
                        if c.is_none() {
                            return self.bad_url();
                        }
                        self.at = end;
                    }
                    _ => self.at += 1,
                }
            }
            Some(&self.s[start..self.at])
        };
        self.spaces();
        if self.byte() != Some(b')') {
            return self.bad_url();
        }
        self.at += 1;
        value
    }
}

fn keyword(s: &str, word: &str) -> bool {
    s.len() <= word.len() * 7 && decoded(s).is_some_and(|s| s.eq_ignore_ascii_case(word))
}

pub(super) fn read(
    source: &str,
    cancel: &CancellationToken,
    emit: &mut impl FnMut(&'static str, &str) -> bool,
) -> Result<bool> {
    let mut t = Tokens {
        s: source.trim_start_matches('\u{feff}'),
        at: 0,
    };
    let mut next_check = 0;
    while t.at < t.s.len() {
        if t.at >= next_check {
            cancel.check()?;
            next_check = t.at + 4096;
        }
        t.trivia();
        let Some(b) = t.byte() else { break };
        let (kind, raw) = match b {
            b'"' | b'\'' => {
                t.string();
                continue;
            }
            b'#' => {
                t.at += 1;
                t.ident();
                continue;
            }
            b'@' => {
                t.at += 1;
                if !keyword(t.ident(), "import") {
                    continue;
                }
                t.trivia();
                let raw = if matches!(t.byte(), Some(b'"' | b'\'')) {
                    t.string()
                } else if keyword(t.ident(), "url") {
                    t.url()
                } else {
                    None
                };
                ("@import", raw)
            }
            b if name(b) || b == b'\\' => {
                if !keyword(t.ident(), "url") {
                    continue;
                }
                ("url()", t.url())
            }
            _ => {
                t.at += 1;
                continue;
            }
        };
        if let Some(value) = raw.and_then(decoded)
            && !emit(kind, &value)
        {
            return Ok(false);
        }
    }
    cancel.check()?;
    Ok(true)
}
