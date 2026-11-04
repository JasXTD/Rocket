//! Bounded lexical patterns, not language execution or a general source parser.
//! Callers pass at most SCAN_WINDOW bytes of annotation/value lookahead.
pub(super) const SCAN_WINDOW: usize = 512;

/// Only an explicit constant declaration at the start of this line supplies
/// context for a short diagnostic marker. Never infer this from a file/name,
/// a comment containing `const`, or a configuration/object field.
pub(super) fn constant_binding(prefix: &[u8]) -> bool {
    if prefix.len() > SCAN_WINDOW {
        return false;
    }
    let mut words = prefix
        .split(|b| b.is_ascii_whitespace())
        .filter(|s| !s.is_empty());
    let first = words.next();
    let declaration = if matches!(
        first,
        Some(b"pub" | b"pub(crate)" | b"pub(super)" | b"pub(self)" | b"export")
    ) {
        words.next()
    } else {
        first
    };
    declaration == Some(b"const".as_slice()) && words.next().is_none()
}

pub(super) struct Literal<'a> {
    pub value: &'a [u8],
    pub quoted: bool,
    /// First byte after the complete closing delimiter, including raw hashes.
    pub end: Option<usize>,
}

pub(super) fn literal(right: &[u8]) -> Option<Literal<'_>> {
    let mut at = 0;
    let mut raw = false;
    let mut verbatim = false;
    let mut hashes = 0;
    if right.starts_with(b"@\"") {
        at = 1;
        verbatim = true;
    } else if right
        .first()
        .is_some_and(|b| matches!(b, b'r' | b'R' | b'b' | b'B'))
    {
        let prefix = right[0].to_ascii_lowercase();
        at = 1;
        raw = prefix == b'r';
        if right
            .get(at)
            .is_some_and(|b| b.to_ascii_lowercase() == if raw { b'b' } else { b'r' })
        {
            raw = true;
            at += 1;
        }
        if raw {
            while right.get(at) == Some(&b'#') {
                hashes += 1;
                at += 1;
            }
        }
        if !matches!(right.get(at), Some(b'\'' | b'"')) || (hashes > 0 && right[at] != b'"') {
            at = 0;
            raw = false;
            hashes = 0;
        }
    }
    let Some(&quote @ (b'\'' | b'"' | b'`')) = right.get(at) else {
        return Some(Literal {
            value: right
                .split(|b| b.is_ascii_whitespace() || matches!(b, b';' | b',' | b'#' | b'`'))
                .next()?,
            quoted: false,
            end: None,
        });
    };
    let start = at + 1;
    at = start;
    while at < right.len() {
        if right[at] == quote {
            if verbatim && right.get(at + 1) == Some(&quote) {
                at += 2;
                continue;
            }
            let end = at + 1 + hashes;
            if (hashes == 0 || right.get(end) != Some(&b'#'))
                && right
                    .get(at + 1..end)
                    .is_some_and(|s| s.iter().all(|b| *b == b'#'))
            {
                return Some(Literal {
                    value: &right[start..at],
                    quoted: true,
                    end: Some(end),
                });
            }
        }
        at += if !raw && !verbatim && right[at] == b'\\' {
            2
        } else {
            1
        };
    }
    // An incomplete prefix is still credential material, never a complete
    // placeholder/protocol exemption with a fabricated closing boundary.
    Some(Literal {
        value: &right[start..],
        quoted: true,
        end: None,
    })
}

/// Accept a bounded type expression, stopping at its assignment or at the end.
/// Top-level commas, comments, single colons and statement delimiters stop it;
/// they must not attach another field's value to the preceding sensitive name.
fn type_end(input: &[u8]) -> Option<usize> {
    if input.len() > SCAN_WINDOW {
        return None;
    }
    let mut stack = [0; 32];
    let mut depth = 0;
    let mut identifier = false;
    let mut at = 0;
    while let Some(&b) = input.get(at) {
        match b {
            b'=' if depth == 0 && identifier && !matches!(input.get(at + 1), Some(b'=' | b'>')) => {
                return Some(at);
            }
            b'a'..=b'z' | b'A'..=b'Z' | b'_' => identifier = true,
            b'0'..=b'9' if identifier => {}
            b' ' | b'\t' | b'\r' | b'&' | b'*' | b'?' | b'|' => {}
            b'.' if identifier => {}
            b'\''
                if input
                    .get(at + 1)
                    .is_some_and(|b| b.is_ascii_alphabetic() || *b == b'_') => {}
            b':' if input.get(at + 1) == Some(&b':') && identifier => at += 1,
            b'<' | b'[' if depth < stack.len() => {
                stack[depth] = if b == b'<' { b'>' } else { b']' };
                depth += 1;
            }
            b'>' | b']' if depth > 0 && stack[depth - 1] == b => depth -= 1,
            b',' if depth > 0 => {}
            _ => return None,
        }
        at += 1;
    }
    (identifier && depth == 0).then_some(at)
}

pub(super) fn annotated_value(right: &[u8]) -> Option<&[u8]> {
    let bounded = &right[..right.len().min(SCAN_WINDOW)];
    // In particular, a single-quoted YAML value containing '=' is not a Rust
    // lifetime/type annotation. Resolve literal prefixes before type syntax.
    if literal(bounded)?.quoted {
        return None;
    }
    let at = type_end(bounded)?;
    (bounded.get(at) == Some(&b'=')).then(|| right[at + 1..].trim_ascii_start())
}

/// Go places its type after the variable name without an annotation colon.
/// Limit this form to a `var name TYPE` declaration, never arbitrary prose.
pub(super) fn go_binding(left: &[u8]) -> Option<(&[u8], usize)> {
    let start = left.len().saturating_sub(SCAN_WINDOW);
    let tail = left[start..]
        .rsplit(|b| matches!(b, b';' | b'{' | b'}'))
        .next()?
        .trim_ascii_start();
    let declaration = tail.strip_prefix(b"var")?;
    if !declaration.first().is_some_and(u8::is_ascii_whitespace) {
        return None;
    }
    let declaration = declaration.trim_ascii_start();
    let length = declaration
        .iter()
        .take_while(|b| b.is_ascii_alphanumeric() || **b == b'_')
        .count();
    let name = &declaration[..length];
    if name.is_empty() || !name[0].is_ascii_alphabetic() && name[0] != b'_' {
        return None;
    }
    let rest = &declaration[length..];
    if !rest.first().is_some_and(u8::is_ascii_whitespace) {
        return None;
    }
    let ty = rest.trim_ascii();
    if type_end(ty)? != ty.len() {
        return None;
    }
    Some((name, left.len() - declaration.len()))
}
