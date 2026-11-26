use super::{CancellationToken, Result, css};
use std::borrow::Cow;

fn ws(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\r' | '\n' | '\x0c')
}
fn one_of(name: &str, names: &[&str]) -> bool {
    names.iter().any(|n| name.eq_ignore_ascii_case(n))
}

/// The first occurrence of an attribute wins. Values stay borrowed until a
/// character reference actually needs decoding.
struct Tag<'a> {
    name: &'a str,
    attrs: &'a str,
    body: &'a str,
    picture: bool,
}
impl<'a> Tag<'a> {
    fn attr(&self, wanted: &str) -> Option<&'a str> {
        let mut rest = self.attrs;
        while !rest.is_empty() {
            rest = rest.trim_start_matches(|c| ws(c) || c == '/');
            let end = rest.find(|c| ws(c) || matches!(c, '=' | '/')).unwrap_or(rest.len());
            if end == 0 {
                rest = rest.strip_prefix('=')?;
                continue;
            }
            let name = &rest[..end];
            rest = rest[end..].trim_start_matches(ws);
            let mut value = &rest[..0];
            if let Some(tail) = rest.strip_prefix('=') {
                rest = tail.trim_start_matches(ws);
                if let Some(quote @ ('\'' | '"')) = rest.chars().next() {
                    let (v, next) = rest[1..].split_once(quote)?;
                    value = v;
                    rest = next;
                } else {
                    let end = rest.find(ws).unwrap_or(rest.len());
                    value = &rest[..end];
                    rest = &rest[end..];
                }
            }
            if name.eq_ignore_ascii_case(wanted) {
                return Some(value);
            }
        }
        None
    }
}

fn tag_end(s: &str, mut at: usize) -> Option<usize> {
    let mut quote = 0;
    let bytes = s.as_bytes();
    while let Some(&b) = bytes.get(at) {
        at += 1;
        if quote != 0 {
            if quote == b {
                quote = 0;
            }
        } else if matches!(b, b'\'' | b'"') {
            quote = b;
        } else if b == b'>' {
            return Some(at);
        }
    }
    None
}

/// Find an actual closing tag, so markup inside script strings or CSS is
/// never interpreted as a page element.
fn raw_end(s: &str, mut at: usize, name: &str) -> (usize, usize) {
    while let Some(n) = s[at..].find("</") {
        let start = at + n;
        at = start + 2;
        let end = at + name.len();
        if s.get(at..end).is_some_and(|n| n.eq_ignore_ascii_case(name))
            && s.as_bytes()
                .get(end)
                .is_some_and(|b| b.is_ascii_whitespace() || matches!(b, b'/' | b'>'))
        {
            return (start, tag_end(s, end).unwrap_or(s.len()));
        }
    }
    (s.len(), s.len())
}

struct Tags<'a> {
    s: &'a str,
    at: usize,
    template: usize,
    picture: usize,
}
impl<'a> Tags<'a> {
    fn new(s: &'a str) -> Self {
        Self {
            s,
            at: 0,
            template: 0,
            picture: 0,
        }
    }
    fn next(&mut self, cancel: &CancellationToken) -> Result<Option<Tag<'a>>> {
        let s = self.s;
        while let Some(offset) = s[self.at..].find('<') {
            cancel.check()?;
            let start = self.at + offset;
            self.at = start + 1;
            let rest = &s[self.at..];
            if let Some(comment) = rest.strip_prefix("!--") {
                self.at = comment
                    .match_indices('>')
                    .find_map(|(i, _)| {
                        let before = &comment[..i];
                        (before.is_empty() || before == "-" || before.ends_with("--") || before.ends_with("--!"))
                            .then_some(start + 4 + i + 1)
                    })
                    .unwrap_or(s.len());
                continue;
            }
            if rest.starts_with(['!', '?']) {
                self.at = tag_end(s, self.at).unwrap_or(s.len());
                continue;
            }
            let closing = rest.starts_with('/');
            if closing {
                self.at += 1;
            }
            if !s.as_bytes().get(self.at).is_some_and(u8::is_ascii_alphabetic) {
                continue;
            }
            let end = s[self.at..]
                .find(|c| ws(c) || matches!(c, '/' | '>'))
                .map_or(s.len(), |n| self.at + n);
            let name = &s[self.at..end];
            let Some(after) = tag_end(s, end) else {
                self.at = s.len();
                break;
            };
            self.at = after;
            if closing {
                if name.eq_ignore_ascii_case("template") {
                    self.template = self.template.saturating_sub(1);
                }
                if self.template == 0 && name.eq_ignore_ascii_case("picture") {
                    self.picture = self.picture.saturating_sub(1);
                }
                continue;
            }
            if name.eq_ignore_ascii_case("template") {
                self.template += 1;
                continue;
            }
            if name.eq_ignore_ascii_case("plaintext") {
                self.at = s.len();
                break;
            }
            let attrs = &s[end..self.at - 1];
            let mut body = "";
            if one_of(
                name,
                &[
                    "script", "style", "textarea", "title", "xmp", "iframe", "noembed", "noframes", "noscript",
                ],
            ) {
                let (close, after) = raw_end(s, self.at, name);
                body = &s[self.at..close];
                self.at = after;
            }
            if self.template != 0 {
                continue;
            }
            if name.eq_ignore_ascii_case("picture") {
                self.picture += 1;
            }
            return Ok(Some(Tag {
                name,
                attrs,
                body,
                picture: self.picture != 0,
            }));
        }
        Ok(None)
    }
}

#[derive(Default)]
struct Attribute<'a> {
    raw: &'a str,
    value: Cow<'a, str>,
    entities: Vec<EntitySpan>,
}
struct EntitySpan {
    decoded: std::ops::Range<usize>,
    raw: std::ops::Range<usize>,
}
impl std::ops::Deref for Attribute<'_> {
    type Target = str;
    fn deref(&self) -> &str { &self.value }
}
impl Attribute<'_> {
    fn source_span(&self, html: &str, span: std::ops::Range<usize>) -> std::ops::Range<usize> {
        let base = super::offset(html, self.raw);
        let original = |offset, end| {
            let through = self.entities.partition_point(|e| e.decoded.end <= offset);
            if let Some(e) = self.entities.get(through) && e.decoded.start < offset {
                // A named reference can expand to two Unicode scalars.
                return if end { e.raw.end } else { e.raw.start };
            }
            through.checked_sub(1).map_or(offset, |i| {
                let e = &self.entities[i];
                e.raw.end + (offset - e.decoded.end)
            })
        };
        base + original(span.start, false)..base + original(span.end, true)
    }
}
fn character_refs(s: &str) -> Option<Attribute<'_>> {
    if !s.contains('&') {
        return Some(Attribute { raw: s, value: Cow::Borrowed(s), entities: Vec::new() });
    }
    let mut entities = Vec::new();
    let mut out = String::with_capacity(s.len());
    let mut tail = s;
    while let Some((before, after)) = tail.split_once('&') {
        out.push_str(before);
        let decoded_start = out.len();
        let raw_start = s.len() - after.len() - 1;
        let prefix = usize::from(after.starts_with('#'));
        let count = prefix + after[prefix..]
            .bytes()
            .take_while(u8::is_ascii_alphanumeric)
            .count();
        let (name, rest) = after.split_at(count);
        if let Some(number) = name.strip_prefix('#') && rest.starts_with(';') {
            let n = if let Some(hex) = number.strip_prefix(['x', 'X']) {
                u32::from_str_radix(hex, 16).ok()?
            } else {
                number.parse().ok()?
            };
            out.push(char::from_u32(n)?);
            tail = &rest[1..];
        } else {
            let end = count + usize::from(rest.starts_with(';'));
            let spelling = &after[..end];
            let table = super::named_entities::NAMED;
            // In an attribute, a legacy semicolonless name followed by '=' or
            // another alphanumeric is literal. The scan already includes all alphanumerics.
            if !rest.starts_with('=')
                && let Ok(i) = table.binary_search_by_key(&spelling, |&(key, _)| key)
            {
                out.push_str(table[i].1);
                tail = &after[end..];
            } else {
                out.push('&');
                tail = after;
                continue;
            }
        }
        entities.push(EntitySpan { decoded: decoded_start..out.len(), raw: raw_start..s.len() - tail.len() });
    }
    out.push_str(tail);
    Some(Attribute { raw: s, value: Cow::Owned(out), entities })
}

/// Look for the first explicit base before emitting any edges. It can appear
/// after a reference and can point to another origin entirely.
pub(super) fn base_directory(
    s: &str,
    path: &[u8],
    root_site: bool,
    cancel: &CancellationToken,
) -> Result<Option<Vec<u8>>> {
    let mut tags = Tags::new(s);
    while let Some(tag) = tags.next(cancel)? {
        if tag.name.eq_ignore_ascii_case("base")
            && let Some(href) = tag.attr("href")
        {
            return Ok(character_refs(href).and_then(|s| super::page_base(path, &s, root_site)));
        }
    }
    Ok(Some(super::packages::parent(path).to_vec()))
}

fn descriptors(s: &str) -> bool {
    let (mut width, mut density, mut height) = (false, false, false);
    for d in s.split_ascii_whitespace() {
        let integer = |v: &str| !v.is_empty() && v.bytes().all(|b| b.is_ascii_digit()) && v.bytes().any(|b| b != b'0');
        if d.strip_suffix('w').is_some_and(integer) {
            if width || density {
                return false;
            }
            width = true;
        } else if d.strip_suffix('h').is_some_and(integer) {
            if height || density {
                return false;
            }
            height = true;
        } else if d
            .strip_suffix('x')
            .is_some_and(|v| !v.starts_with('+') && v.parse::<f64>().is_ok_and(|x| x.is_finite() && x >= 0.))
        {
            if width || height || density {
                return false;
            }
            density = true;
        } else {
            return false;
        }
    }
    !height || width
}

/// Commas can be part of the URL, in particular in data URLs. A comma following
/// a descriptor or at the end of the URL token separates candidates.
fn srcset(input: &str, cancel: &CancellationToken, emit: &mut impl FnMut(&'static str, &str, usize) -> bool) -> Result<bool> {
    let mut s = input;
    loop {
        cancel.check()?;
        s = s.trim_start_matches(|c| ws(c) || c == ',');
        if s.is_empty() {
            return Ok(true);
        }
        let end = s.find(ws).unwrap_or(s.len());
        let url = &s[..end];
        s = &s[end..];
        if url.ends_with(',') {
            if !emit("srcset", url.trim_end_matches(','), super::offset(input, url)) {
                return Ok(false);
            }
        } else {
            let mut depth = 0usize;
            let end = s
                .char_indices()
                .find_map(|(i, c)| match c {
                    '(' => {
                        depth += 1;
                        None
                    }
                    ')' => {
                        depth = depth.saturating_sub(1);
                        None
                    }
                    ',' if depth == 0 => Some(i),
                    _ => None,
                })
                .unwrap_or(s.len());
            if descriptors(&s[..end]) && !emit("srcset", url, super::offset(input, url)) {
                return Ok(false);
            }
            s = &s[end..];
        }
    }
}

pub(super) fn read(
    s: &str,
    cancel: &CancellationToken,
    emit: &mut impl FnMut(&'static str, &str, std::ops::Range<usize>) -> bool,
) -> Result<bool> {
    let mut tags = Tags::new(s);
    while let Some(tag) = tags.next(cancel)? {
        let attr = |name| tag.attr(name).and_then(character_refs);
        let mut single = None;
        let mut candidates = None;
        if tag.name.eq_ignore_ascii_case("link") {
            let rel = attr("rel").unwrap_or_default();
            let has = |v: &str| rel.split_ascii_whitespace().any(|word| word.eq_ignore_ascii_case(v));
            let kind = if has("stylesheet") {
                Some("stylesheet")
            } else if has("icon") {
                Some("image")
            } else if has("modulepreload") {
                Some("script")
            } else if has("preload") {
                match attr("as").as_deref().map(str::to_ascii_lowercase).as_deref() {
                    Some("image") => {
                        candidates = attr("imagesrcset");
                        Some("image")
                    }
                    Some("style") => Some("stylesheet"),
                    Some("script") => Some("script"),
                    Some("font") => Some("font"),
                    _ => None,
                }
            } else {
                None
            };
            single = kind.and_then(|k| attr("href").map(|s| (k, s)));
        } else if tag.name.eq_ignore_ascii_case("script") {
            single = attr("src").map(|s| ("script", s));
        } else if tag.name.eq_ignore_ascii_case("img") {
            single = attr("src").map(|s| ("image", s));
            candidates = attr("srcset");
        } else if tag.picture && tag.name.eq_ignore_ascii_case("source") {
            candidates = attr("srcset");
        } else if tag.name.eq_ignore_ascii_case("input")
            && attr("type").is_some_and(|v| v.eq_ignore_ascii_case("image"))
        {
            single = attr("src").map(|s| ("image", s));
        }
        if let Some((kind, url)) = single {
            let trimmed = url.trim_matches(ws);
            let start = super::offset(&url, trimmed);
            if !emit(kind, trimmed, url.source_span(s, start..start + trimmed.len())) { return Ok(false); }
        }
        if let Some(set) = candidates
            && !srcset(&set, cancel, &mut |k, url, at| emit(k, url, set.source_span(s, at..at + url.len())))?
        {
            return Ok(false);
        }
        if let Some(style) = attr("style")
            && !css::read(&style, cancel, &mut |k, url, span| emit(k, url, style.source_span(s, span)))?
        {
            return Ok(false);
        }
        if tag.name.eq_ignore_ascii_case("style") && !css::read(tag.body, cancel, &mut |k, url, span| {
            let base = super::offset(s, tag.body);
            emit(k, url, base + span.start..base + span.end)
        })? {
            return Ok(false);
        }
    }
    cancel.check()?;
    Ok(true)
}
