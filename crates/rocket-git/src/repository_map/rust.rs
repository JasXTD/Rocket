//! A small lexical reader for Rust source: module declarations, `use` trees, paths
//! into this crate and the items a file declares at its top level. Comments and
//! literals are skipped except where a known construct takes a string. Macros are not expanded, `cfg` is not evaluated and nothing
//! is compiled or run, so what it reports is what the text says, not what rustc sees.
use super::{OutlineItem, OutlineKind};
mod embedded;

#[derive(Default)]
pub(super) struct ReadBudget {
    import_bytes: usize,
    embed_candidates: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Token {
    Ident(String),
    /// A raw identifier (`r#type`): a name, never a keyword.
    Raw(String),
    Str(String),
    Other,
    Punct(&'static str),
}

fn close(token: &Token) -> Option<&'static str> {
    match token {
        Token::Punct("(") => Some(")"),
        Token::Punct("[") => Some("]"),
        Token::Punct("{") => Some("}"),
        _ => None,
    }
}

/// Skip an opaque token tree in one pass. An unterminated group consumes the remainder.
fn after_group(tokens: &[Token], open: usize) -> usize {
    let mut depth = 0usize;
    for (i, t) in tokens.iter().enumerate().skip(open) {
        match t {
            Token::Punct("(" | "[" | "{") => depth += 1,
            Token::Punct(")" | "]" | "}") => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return i + 1;
                }
            }
            _ => {}
        }
    }
    tokens.len()
}

/// What one file says about modules and names.
#[derive(Clone, Debug, Default)]
pub(super) struct Facts {
    /// The source exceeded a nesting, expanded-import or literal embed limit.
    pub limited: bool,
    /// Literal built-in include calls, relative to this source file.
    pub embeds: Vec<(&'static str, String, u32)>,
    /// `mod name;`: a module in its own file.
    pub declarations: Vec<Declaration>,
    /// `mod name { … }`: the module path inside the file, ending in its own name.
    pub inline: Vec<Vec<String>>,
    pub uses: Vec<Use>,
    /// `crate::…`, `super::…`, `self::…` and `name::…` paths outside `use` lines.
    pub paths: Vec<Path>,
    /// Items each module of the file declares: inline module path and name.
    pub items: Vec<(Vec<String>, String)>,
    /// Inline module, Rust binding and linker symbol of an extern "C" function.
    pub native: Vec<(Vec<String>, String, String)>,
    /// The file's own top level, in order.
    pub outline: Vec<OutlineItem>,
    /// Names the file uses on their own, for `use …::*` lines: inline module path and name.
    pub names: Vec<(Vec<String>, String)>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Declaration {
    pub line: u32,
    /// Inline modules the declaration sits in.
    pub chain: Vec<String>,
    pub name: String,
    /// `#[path = "…"]`, as written.
    pub path: Option<String>,
    /// Behind `#[cfg(test)]` or another `cfg` naming `test`.
    pub test: bool,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Use {
    pub line: u32,
    pub kind: &'static str,
    pub chain: Vec<String>,
    pub segments: Vec<String>,
    pub alias: Option<String>,
    pub glob: bool,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Path {
    pub chain: Vec<String>,
    pub segments: Vec<String>,
}

fn ident_start(c: char) -> bool {
    c == '_' || c.is_alphabetic()
}
fn ident_part(c: char) -> bool {
    c == '_' || c.is_alphanumeric()
}

/// A quoted literal starting at `c[i] == '"'`: its length and text.
fn quoted(c: &[char], i: usize) -> (usize, Token) {
    let mut j = i + 1;
    while j < c.len() && c[j] != '"' {
        j += if c[j] == '\\' { 2 } else { 1 };
    }
    let token = if j < c.len() {
        embedded::string_value(&c[i + 1..j]).map_or(Token::Other, Token::Str)
    } else { Token::Other };
    ((j + 1).min(c.len()) - i, token)
}
/// A raw literal whose `r` is at `c[i]`: `r"…"`, `r#"…"#`. None when it isn't one.
fn raw(c: &[char], i: usize) -> Option<(usize, Token)> {
    let mut j = i + 1;
    while c.get(j) == Some(&'#') {
        j += 1;
    }
    let hashes = j - i - 1;
    if c.get(j) != Some(&'"') {
        return None;
    }
    let start = j + 1;
    let mut k = start;
    while k < c.len() {
        if c[k] == '"' && (1..=hashes).all(|h| c.get(k + h) == Some(&'#')) {
            let token = if hashes <= 255 { Token::Str(c[start..k].iter().collect()) } else { Token::Other };
            return Some((k + 1 + hashes - i, token));
        }
        k += 1;
    }
    Some((c.len() - i, Token::Other))
}
/// A character literal or a lifetime at `c[i] == '\''`: how much to skip.
fn quote(c: &[char], i: usize) -> usize {
    if c.get(i + 1) == Some(&'\\') {
        let mut j = i + 3;
        while j < c.len() && c[j] != '\'' && j < i + 16 {
            j += 1;
        }
        return j + 1 - i;
    }
    if c.get(i + 2) == Some(&'\'') {
        return 3;
    }
    // A lifetime or a label.
    let mut j = i + 1;
    while j < c.len() && ident_part(c[j]) {
        j += 1;
    }
    (j - i).max(1)
}

/// First token on each occupied line after line one. Multiline literals are one token.
#[derive(Default)]
struct TokenLines(Vec<(usize, u32)>);
impl TokenLines {
    fn at(&self, token: usize) -> u32 {
        let n = self.0.partition_point(|&(first, _)| first <= token);
        n.checked_sub(1).map_or(1, |i| self.0[i].1)
    }
}

fn lex(source: &str, mut new_line: impl FnMut(usize, u32)) -> Vec<Token> {
    let c: Vec<char> = source.chars().collect();
    let n = c.len();
    let (mut counted, mut line) = (0, 1u32);
    let mut out = Vec::new();
    let mut i = 0;
    while i < n {
        let ch = c[i];
        let next = c.get(i + 1).copied();
        if ch == '/' && next == Some('/') {
            while i < n && c[i] != '\n' {
                i += 1;
            }
            continue;
        }
        if ch == '/' && next == Some('*') {
            let mut depth = 1;
            i += 2;
            while i < n && depth > 0 {
                if c[i] == '/' && c.get(i + 1) == Some(&'*') {
                    depth += 1;
                    i += 2;
                } else if c[i] == '*' && c.get(i + 1) == Some(&'/') {
                    depth -= 1;
                    i += 2;
                } else {
                    i += 1;
                }
            }
            continue;
        }
        if ch.is_whitespace() {
            i += 1;
            continue;
        }
        let breaks = c[counted..i].iter().filter(|&&ch| ch == '\n').count() as u32;
        counted = i;
        if breaks > 0 {
            line += breaks;
            new_line(out.len(), line);
        }
        // Prefixed literals and raw identifiers: b"", b'', br"", r"", r#""#, r#name, c"", cr"".
        if ch == 'r' && next == Some('#') && c.get(i + 2).is_some_and(|&a| ident_start(a)) {
            let start = i + 2;
            let mut j = start;
            while j < n && ident_part(c[j]) {
                j += 1;
            }
            out.push(Token::Raw(c[start..j].iter().collect()));
            i = j;
            continue;
        }
        if matches!(ch, 'b' | 'c') && next == Some('"') {
            let (len, _) = quoted(&c, i + 1);
            out.push(Token::Other);
            i += 1 + len;
            continue;
        }
        if ch == 'b' && next == Some('\'') {
            out.push(Token::Other);
            i += 1 + quote(&c, i + 1);
            continue;
        }
        let raw_at = match (ch, next) {
            ('r', _) => Some(i),
            ('b' | 'c', Some('r')) => Some(i + 1),
            _ => None,
        };
        if let Some((len, token)) = raw_at.and_then(|at| raw(&c, at).map(|(len, token)| (len + at - i, token))) {
            out.push(if ch == 'r' { token } else { Token::Other });
            i += len;
            continue;
        }
        if ch == '"' {
            let (len, token) = quoted(&c, i);
            out.push(token);
            i += len;
            continue;
        }
        if ch == '\'' {
            out.push(Token::Other);
            i += quote(&c, i);
            continue;
        }
        if ch.is_ascii_digit() {
            out.push(Token::Other);
            // Digits, suffixes and a decimal point followed by a digit (`1.5`, not `1..2`).
            while i < n && (ident_part(c[i]) || c[i] == '.' && c.get(i + 1).is_some_and(char::is_ascii_digit)) {
                i += 1;
            }
            continue;
        }
        if ident_start(ch) {
            let start = i;
            while i < n && ident_part(c[i]) {
                i += 1;
            }
            out.push(Token::Ident(c[start..i].iter().collect()));
            continue;
        }
        let pair = |a: char, b: char| ch == a && next == Some(b);
        let punct = if pair(':', ':') {
            "::"
        } else if pair('-', '>') {
            "->"
        } else if pair('=', '>') {
            "=>"
        } else {
            match ch {
                '{' => "{",
                '}' => "}",
                '(' => "(",
                ')' => ")",
                '[' => "[",
                ']' => "]",
                ';' => ";",
                ',' => ",",
                '*' => "*",
                '#' => "#",
                '=' => "=",
                '!' => "!",
                '.' => ".",
                '<' => "<",
                '>' => ">",
                '$' => "$",
                _ => "",
            }
        };
        if !punct.is_empty() {
            out.push(Token::Punct(punct));
        } else {
            out.push(Token::Other);
        }
        i += punct.len().max(1);
    }
    out
}

const ITEMS: [&str; 8] = ["fn", "struct", "enum", "trait", "type", "const", "static", "union"];

/// Literal native sources in complete build statements in the build script's main.
/// A string in a macro, an unused helper, or a partially understood builder is no input.
pub(super) fn native_inputs(source: &str, helper_exists: impl Fn(&str) -> bool) -> Vec<(String, bool)> {
    use Token::{Ident, Punct, Str};
    if !source.contains("build_objc") && !source.contains("Build") { return Vec::new(); }
    let tokens = lex(source, |_, _| {});
    let mut depth = 0usize;
    let mut main = None;
    let mut objc_helper = false;
    for (i, t) in tokens.iter().enumerate() {
        if depth == 0 {
            match &tokens[i..] {
                [Ident(f), Ident(name), Punct("("), Punct(")"), Punct("{"), ..] if f == "fn" && name == "main" => main = Some(i + 5),
                [Ident(f), Punct("!"), Punct("("), Str(path), Punct(")"), Punct(";"), ..]
                    if f == "include" && path.rsplit('/').next() == Some("build-objc.rs") => objc_helper |= helper_exists(path),
                _ => {}
            }
        }
        match t {
            Punct("{" | "(" | "[") => depth += 1,
            Punct("}" | ")" | "]") => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    let Some(start) = main else { return Vec::new() };
    let mut result = Vec::new();
    let mut statement = start;
    depth = 0;
    for (i, t) in tokens.iter().enumerate().skip(start) {
        match t {
            Punct("}") if depth == 0 => break,
            Punct("{" | "(" | "[") => depth += 1,
            Punct("}" | ")" | "]") => {
                depth = depth.saturating_sub(1);
                if depth == 0 && *t == Punct("}") { statement = i + 1; }
            }
            Punct(";") if depth == 0 => {
                match &tokens[statement..i] {
                    [Ident(f), Punct("("), Str(file), Punct(","), Str(archive), Punct(")")]
                        if objc_helper && f == "build_objc" && !archive.is_empty() => result.push((file.clone(), false)),
                    [Ident(f), Punct("("), ..] if objc_helper && f == "build_objc" => return Vec::new(),
                    [Ident(cc), Punct("::"), Ident(build), Punct("::"), Ident(new), Punct("("), Punct(")"), tail @ ..]
                        if cc == "cc" && build == "Build" && new == "new" => {
                        let mut tail = tail;
                        let before = result.len();
                        let mut cpp = false;
                        while let [Punct("."), Ident(method), Punct("("), arg, Punct(")"), rest @ ..] = tail {
                            match (method.as_str(), arg) {
                                ("file", Str(file)) => result.push((file.clone(), false)),
                                ("cpp", Ident(v)) if v == "true" || v == "false" => cpp = v == "true",
                                _ => break,
                            }
                            tail = rest;
                        }
                        if !matches!(tail, [Punct("."), Ident(m), Punct("("), Str(archive), Punct(")")] if m == "compile" && !archive.is_empty()) {
                            // An unknown input could define the same symbol as a known one.
                            return Vec::new();
                        } else if cpp {
                            for (_, cpp) in &mut result[before..] { *cpp = true; }
                        }
                    }
                    _ => {}
                }
                statement = i + 1;
            }
            _ => {}
        }
    }
    result
}

/// Read one file and charge imports and literal embed candidates to the whole map request.
pub(super) fn read_with_budget(source: &str, budget: &mut ReadBudget) -> Facts {
    // A byte-order mark is no part of the first line.
    let source = source.strip_prefix('﻿').unwrap_or(source);
    let mut lines = TokenLines::default();
    let t = lex(source, |token, line| lines.0.push((token, line)));
    let mut facts = Facts::default();
    let allowance = FILE_IMPORT_BYTES.min(MAP_IMPORT_BYTES.saturating_sub(budget.import_bytes));
    let mut import_budget = allowance;
    let is = |k: usize, p: &str| matches!(t.get(k), Some(Token::Punct(x)) if *x == p);
    let name = |k: usize| match t.get(k) {
        Some(Token::Ident(x) | Token::Raw(x)) => Some(x.clone()),
        _ => None,
    };
    // Some(name) for an inline module's braces, None for any other.
    let mut scopes: Vec<Option<String>> = Vec::new();
    let mut foreign_depth = None;
    let chain = |scopes: &[Option<String>]| scopes.iter().flatten().cloned().collect::<Vec<_>>();
    let module_level = |scopes: &[Option<String>]| scopes.iter().all(Option::is_some);
    let mut attributes: Vec<Vec<Token>> = Vec::new();
    let mut visibility: Option<String> = None;
    // At the start of a statement or item: `use` and `mod` count only there.
    let mut fresh = true;
    let mut k = 0;
    let mut macro_end = 0;
    while k < t.len() {
        if scopes.len() > MAX_NESTING {
            facts.limited = true;
            break;
        }
        match &t[k] {
            Token::Punct("!") if k >= macro_end && k > 0 && t.get(k + 1).and_then(close).is_some()
                && matches!(&t[k - 1], Token::Raw(_) | Token::Ident(_))
                && !matches!(&t[k - 1], Token::Ident(w) if matches!(w.as_str(), "if" | "while" | "return" | "break" | "match" | "yield" | "in")) => {
                // Keep existing name/path analysis inside expressions, but macro input is
                // arbitrary data and cannot establish a module declaration or import site.
                macro_end = after_group(&t, k + 1);
            }
            Token::Punct("#") if is(k + 1, "[") || is(k + 1, "!") && is(k + 2, "[") => {
                let outer = is(k + 1, "[");
                let open = if outer { k + 1 } else { k + 2 };
                let mut depth = 0;
                let mut end = open;
                while end < t.len() {
                    if is(end, "[") {
                        depth += 1;
                    } else if is(end, "]") {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                    }
                    end += 1;
                }
                if outer {
                    attributes.push(t[open + 1..end.min(t.len())].to_vec());
                }
                k = end + 1;
                fresh = true;
                continue;
            }
            Token::Punct(p @ ("{" | "}" | ";")) => {
                match *p {
                    "{" => scopes.push(None),
                    "}" => {
                        if foreign_depth == Some(scopes.len()) { foreign_depth = None; }
                        scopes.pop();
                    }
                    _ => {}
                }
                attributes.clear();
                visibility = None;
                fresh = true;
                k += 1;
                continue;
            }
            Token::Ident(w) if w == "pub" => {
                let mut text = String::from("pub");
                if is(k + 1, "(") {
                    let mut j = k + 2;
                    let mut inner = Vec::new();
                    while j < t.len() && !is(j, ")") {
                        match &t[j] {
                            Token::Ident(x) | Token::Raw(x) => inner.push(x.clone()),
                            Token::Punct("::") => inner.push("::".into()),
                            _ => {}
                        }
                        j += 1;
                    }
                    text = format!("pub({})", inner.join(" ").replace(" :: ", "::"));
                    k = j;
                }
                visibility = Some(text);
                k += 1;
                continue;
            }
            Token::Ident(w) if w == "extern" && module_level(&scopes)
                && matches!(t.get(k + 1), Some(Token::Str(abi)) if abi == "C")
                && is(k + 2, "{") => {
                // Explicit library attributes can name an SDK or another archive.
                // Only unqualified blocks participate in the local build match.
                let linked = attributes.iter().any(|a| matches!(a.first(), Some(Token::Ident(s)) if s == "link" || s == "cfg_attr"));
                if !linked { foreign_depth = Some(scopes.len() + 1); }
                k += 2;
                continue;
            }
            Token::Ident(w) if w == "fn" && foreign_depth == Some(scopes.len()) => {
                if let Some(binding) = name(k + 1) && is(k + 2, "(") {
                    let mut symbol = Some(binding.clone());
                    for attr in &attributes {
                        match attr.as_slice() {
                            [Token::Ident(n), Token::Punct("="), Token::Str(s)] if n == "link_name" => symbol = Some(s.clone()),
                            [Token::Ident(n), ..] if matches!(n.as_str(), "link_name" | "cfg_attr" | "link_ordinal") => { symbol = None; break; }
                            _ => {}
                        }
                    }
                    if let Some(symbol) = symbol { facts.native.push((chain(&scopes), binding, symbol)); }
                }
            }
            Token::Ident(w) if w == "mod" && fresh && k >= macro_end && module_level(&scopes) => {
                if let Some(module) = name(k + 1) {
                    if is(k + 2, ";") {
                        let path = attributes.iter().find_map(|a| match a.as_slice() {
                            [Token::Ident(p), Token::Punct("="), Token::Str(value)] if p == "path" => Some(value.clone()),
                            _ => None,
                        });
                        // cfg(test) and cfg(all(test, …)); not cfg(not(test)) or cfg(any(test, …)),
                        // which also build outside tests.
                        let test = attributes.iter().any(|a| {
                            let word = |w: &str| a.iter().any(|x| matches!(x, Token::Ident(v) if v == w));
                            matches!(a.first(), Some(Token::Ident(c)) if c == "cfg") && word("test") && !word("not") && !word("any")
                        });
                        facts.declarations.push(Declaration {
                            line: lines.at(k + 1),
                            chain: chain(&scopes),
                            name: module,
                            path,
                            test,
                        });
                        k += 2;
                        continue;
                    }
                    if is(k + 2, "{") {
                        let mut inner = chain(&scopes);
                        inner.push(module.clone());
                        facts.inline.push(inner);
                        scopes.push(Some(module));
                        attributes.clear();
                        visibility = None;
                        fresh = true;
                        k += 3;
                        continue;
                    }
                }
            }
            Token::Ident(w) if w == "use" && fresh && k >= macro_end => {
                let start = k + 1;
                let mut end = start;
                while end < t.len() && !is(end, ";") {
                    end += 1;
                }
                let kind = match visibility.as_deref() {
                    None => "use",
                    Some("pub") => "pub use",
                    Some("pub(crate)") => "pub(crate) use",
                    Some("pub(super)") => "pub(super) use",
                    Some("pub(self)") => "pub(self) use",
                    Some(_) => "pub(in …) use",
                };
                expand(&t[start..end], &chain(&scopes), kind, |i| lines.at(start + i), &mut import_budget, &mut facts);
                k = end;
                continue;
            }
            Token::Ident(w) if w == "macro_rules" && is(k + 1, "!") && k >= macro_end => {
                if module_level(&scopes) && let Some(item) = name(k + 2) {
                    let here = chain(&scopes);
                    if here.is_empty() {
                        facts.outline.push(outline(OutlineKind::Macro, &item, visibility.take()));
                    }
                    facts.items.push((here, item));
                }
                let open = k + 3;
                fresh = is(open, "{");
                k = if t.get(open).and_then(close).is_some() { after_group(&t, open) } else { open };
                attributes.clear();
                visibility = None;
                continue;
            }
            Token::Ident(w) if ITEMS.contains(&w.as_str()) && module_level(&scopes) => {
                // A keyword that declares nothing here (`const fn`) keeps the visibility for
                // the item that follows.
                if let Some((kind, item)) = item(&t, k) {
                    let here = chain(&scopes);
                    if here.is_empty() {
                        facts.outline.push(outline(kind, &item, visibility.take()));
                    }
                    facts.items.push((here, item));
                    visibility = None;
                }
            }
            // An impl block starts a statement; `-> impl Trait` and `x: impl Trait` are types.
            Token::Ident(w)
                if w == "impl"
                    && module_level(&scopes)
                    && (fresh || matches!(t.get(k.wrapping_sub(1)), Some(Token::Ident(p)) if p == "unsafe" || p == "default")) =>
            {
                if scopes.is_empty() {
                    facts.outline.push(implementation(&t, k));
                }
                visibility = None;
            }
            Token::Ident(first) if !is(k.wrapping_sub(1), "::") && !is(k.wrapping_sub(1), ".") => {
                if is(k + 1, "::") && name(k + 2).is_some() {
                    let mut segments = vec![first.clone()];
                    let mut j = k + 1;
                    while is(j, "::") {
                        let Some(next) = name(j + 1) else { break };
                        segments.push(next);
                        j += 2;
                    }
                    let here = chain(&scopes);
                    facts.names.push((here.clone(), first.clone()));
                    facts.paths.push(Path { chain: here, segments });
                    k = j;
                    fresh = false;
                    continue;
                }
                let called = is(k + 1, "(") || is(k + 1, "!") || is(k + 1, "::");
                let declared = matches!(t.get(k.wrapping_sub(1)), Some(Token::Ident(p)) if ITEMS.contains(&p.as_str()) || ["let", "mut", "mod", "impl", "for", "as"].contains(&p.as_str()));
                if (called || first.starts_with(|c: char| c.is_uppercase())) && !declared {
                    facts.names.push((chain(&scopes), first.clone()));
                }
            }
            _ => {}
        }
        fresh = false;
        k += 1;
    }
    budget.import_bytes += allowance - import_budget;
    if source.contains("include_str") || source.contains("include_bytes") {
        embedded::calls(&t, &lines, &mut budget.embed_candidates, &mut facts);
    }
    facts
}

#[cfg(test)]
pub(super) fn read(source: &str) -> Facts {
    read_with_budget(source, &mut ReadBudget::default())
}

fn outline(kind: OutlineKind, name: &str, visibility: Option<String>) -> OutlineItem {
    let visibility = visibility.unwrap_or_default().into_bytes();
    OutlineItem {
        kind,
        name: name.as_bytes().to_vec(),
        exported: !visibility.is_empty(),
        visibility,
        of_trait: None,
        functions: 0,
    }
}

/// The item a keyword at `k` declares, if it declares one here.
fn item(t: &[Token], k: usize) -> Option<(OutlineKind, String)> {
    let Token::Ident(word) = &t[k] else { return None };
    let after = |j: usize| match t.get(j) {
        Some(Token::Ident(x) | Token::Raw(x)) => Some(x.as_str()),
        _ => None,
    };
    let punct = |j: usize, p: &str| matches!(t.get(j), Some(Token::Punct(x)) if *x == p);
    // `<const N: usize>` and `, const N` are generic parameters, not items.
    // `*const T` is a pointer type.
    if word == "const" && (punct(k.wrapping_sub(1), "<") || punct(k.wrapping_sub(1), ",") || punct(k.wrapping_sub(1), "*")) {
        return None;
    }
    let kind = match word.as_str() {
        "fn" => OutlineKind::Function,
        "struct" => OutlineKind::Struct,
        "enum" => OutlineKind::Enum,
        "trait" => OutlineKind::Trait,
        "type" => OutlineKind::Type,
        "const" => OutlineKind::Const,
        "static" => OutlineKind::Static,
        "union" => OutlineKind::Union,
        _ => return None,
    };
    let mut j = k + 1;
    if matches!(kind, OutlineKind::Static) && after(j) == Some("mut") {
        j += 1;
    }
    let name = after(j)?;
    // `const fn`, `const unsafe fn` and `const _` are not constants; the `fn` is read on its own.
    if matches!(kind, OutlineKind::Const) && ["fn", "unsafe", "async", "extern", "_"].contains(&name) {
        return None;
    }
    // `union` is a keyword only before a name and its body or generics.
    if matches!(kind, OutlineKind::Union) && !(punct(j + 1, "{") || punct(j + 1, "<")) {
        return None;
    }
    Some((kind, name.to_owned()))
}

/// An `impl` block at `k`: the type it is for, the trait and its functions.
fn implementation(t: &[Token], k: usize) -> OutlineItem {
    let punct = |j: usize, p: &str| matches!(t.get(j), Some(Token::Punct(x)) if *x == p);
    let mut j = k + 1;
    let mut angle = 0i32;
    let mut last: Option<String> = None;
    let mut of_trait = None;
    while j < t.len() && !(punct(j, "{") && angle <= 0) && !punct(j, ";") {
        match &t[j] {
            Token::Punct("<") => angle += 1,
            Token::Punct(">") => angle -= 1,
            Token::Ident(w) if angle <= 0 && w == "for" => of_trait = last.take(),
            Token::Ident(w) if angle <= 0 && w == "where" => break,
            Token::Ident(w) | Token::Raw(w) if angle <= 0 && !["dyn", "mut", "unsafe", "const"].contains(&w.as_str()) => last = Some(w.clone()),
            _ => {}
        }
        j += 1;
    }
    while j < t.len() && !punct(j, "{") && !punct(j, ";") {
        j += 1;
    }
    let mut functions = 0;
    if punct(j, "{") {
        let mut depth = 0;
        while j < t.len() {
            if punct(j, "{") {
                depth += 1;
            } else if punct(j, "}") {
                depth -= 1;
                if depth == 0 {
                    break;
                }
            } else if depth == 1 && matches!(&t[j], Token::Ident(w) if w == "fn") {
                functions += 1;
            }
            j += 1;
        }
    }
    OutlineItem {
        kind: OutlineKind::Impl,
        name: last.unwrap_or_default().into_bytes(),
        visibility: Vec::new(),
        exported: false,
        of_trait: of_trait.map(String::into_bytes),
        functions,
    }
}

const MAX_NESTING: usize = 64;
// Charge each flattened import for its owned strings, including repeated prefixes.
const FILE_IMPORT_BYTES: usize = 4 * 1024 * 1024;
const MAP_IMPORT_BYTES: usize = 32 * 1024 * 1024;

fn expand(body: &[Token], chain: &[String], kind: &'static str, line_at: impl Fn(usize) -> u32, budget: &mut usize, facts: &mut Facts) {
    let cost = |part: &str| size_of::<String>() + part.len();
    let base = size_of::<Use>() + chain.iter().map(|part| cost(part)).sum::<usize>();
    let mut groups = Vec::new();
    let mut segments = Vec::new();
    let mut bytes = base;
    let mut i = 0;
    while let Some(token) = body.get(i) {
        let leaf = i;
        i += 1;
        let mut alias = None;
        let glob = match token {
            Token::Punct("::") => {
                if segments.is_empty() {
                    segments.push("");
                    bytes += cost("");
                }
                continue;
            }
            Token::Punct("{") => {
                if groups.len() == MAX_NESTING {
                    facts.limited = true;
                    return;
                }
                groups.push((segments.len(), bytes));
                continue;
            }
            Token::Punct("}") => {
                groups.pop();
                None
            }
            Token::Punct("*") => Some(true),
            Token::Ident(part) | Token::Raw(part) => {
                segments.push(part.as_str());
                bytes += cost(part);
                if matches!(body.get(i), Some(Token::Punct("::"))) {
                    continue;
                }
                if matches!(body.get(i), Some(Token::Ident(word)) if word == "as") {
                    if let Some(Token::Ident(name) | Token::Raw(name)) = body.get(i + 1) {
                        alias = Some(name);
                        bytes += cost(name);
                    }
                    i += 2;
                }
                Some(false)
            }
            _ => None,
        };
        if let Some(glob) = glob && !segments.is_empty() {
            let Some(left) = budget.checked_sub(bytes) else {
                facts.limited = true;
                return;
            };
            *budget = left;
            facts.uses.push(Use {
                line: line_at(leaf),
                kind,
                chain: chain.to_vec(),
                segments: segments.iter().map(|part| (*part).to_owned()).collect(),
                alias: alias.cloned(),
                glob,
            });
        }
        let (len, prefix_bytes) = groups.last().copied().unwrap_or((0, base));
        segments.truncate(len);
        bytes = prefix_bytes;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(list: &[(Vec<String>, String)]) -> Vec<&str> {
        list.iter().map(|(_, n)| n.as_str()).collect()
    }

    #[test]
    fn comments_strings_and_lifetimes_hide_nothing_and_invent_nothing() {
        let facts = read(
            r####"
// mod fake_line;
/* mod fake_block; /* nested mod deeper; */ */
const TEXT: &str = "mod fake_string; use crate::fake;";
const RAW: &str = r#"use crate::raw_fake; "quoted" mod x;"#;
const BYTES: &[u8] = br##"mod bytes_fake;"##;
fn tick<'a>(x: &'a str) -> char { let _q = '"'; let _e = '\''; 'outer: loop { break 'outer; } 'x' }
mod real;
#[cfg(test)]
mod tests;
#[path = "elsewhere/other.rs"]
mod other;
#[cfg(not(test))]
mod real_only;
#[cfg(any(test, feature = "x"))]
mod both;
"####,
        );
        let declared: Vec<(&str, Option<&str>, bool)> = facts.declarations.iter().map(|d| (d.name.as_str(), d.path.as_deref(), d.test)).collect();
        assert_eq!(
            declared,
            [("real", None, false), ("tests", None, true), ("other", Some("elsewhere/other.rs"), false), ("real_only", None, false), ("both", None, false)]
        );
        assert!(facts.uses.is_empty() && facts.paths.is_empty(), "{facts:?}");
        let items: Vec<&str> = names(&facts.items);
        assert_eq!(items, ["TEXT", "RAW", "BYTES", "tick"]);
    }

    #[test]
    fn use_trees_expand_with_aliases_globs_and_self() {
        let facts = read("use crate::{app::{self, Intent as I}, style::*};\npub use super::lists::Line;\nuse ::std::fmt;\nuse r#type::Thing;\n");
        let uses: Vec<(String, Option<&str>, bool)> = facts
            .uses
            .iter()
            .map(|u| (u.segments.join("::"), u.alias.as_deref(), u.glob))
            .collect();
        assert_eq!(
            uses,
            [
                ("crate::app::self".to_owned(), None, false),
                ("crate::app::Intent".to_owned(), Some("I"), false),
                ("crate::style".to_owned(), None, true),
                ("super::lists::Line".to_owned(), None, false),
                ("::std::fmt".to_owned(), None, false),
                ("type::Thing".to_owned(), None, false),
            ]
        );
    }

    #[test]
    fn paths_names_and_inline_modules_are_kept_apart() {
        let facts = read(
            "mod inner {\n    use super::Thing;\n    pub fn helper() { crate::style::icon(); }\n    mod deeper;\n}\nfn main() { details::card(); value.method(); Vec::new(); let local = Icon::Map; label(1); }\n",
        );
        assert_eq!(facts.inline, [vec!["inner".to_owned()]]);
        assert_eq!(facts.declarations[0].chain, ["inner"]);
        assert_eq!(facts.uses[0].chain, ["inner"]);
        let paths: Vec<(Vec<String>, String)> = facts.paths.iter().map(|p| (p.chain.clone(), p.segments.join("::"))).collect();
        assert!(paths.contains(&(vec!["inner".into()], "crate::style::icon".into())));
        assert!(paths.contains(&(vec![], "details::card".into())));
        assert!(paths.contains(&(vec![], "Icon::Map".into())));
        // A method call and a local binding are not names to look up.
        let found = names(&facts.names);
        assert!(found.contains(&"label") && found.contains(&"Icon") && found.contains(&"Vec"));
        assert!(!found.contains(&"method") && !found.contains(&"local"), "{found:?}");
        // Only the file's own top level is outlined.
        let outline: Vec<String> = facts.outline.iter().map(|o| String::from_utf8_lossy(&o.name).into_owned()).collect();
        assert_eq!(outline, ["main"]);
        assert_eq!(names(&facts.items), ["helper", "main"]);
    }

    #[test]
    fn outline_reads_visibility_impls_and_skips_generic_consts() {
        let facts = read(
            "pub struct Browser<const N: usize> { a: u8 }\npub(crate) enum Mode { A, B }\nimpl<T: Clone> Default for Browser<T> where T: Copy { fn default() -> Self { todo!() } }\nimpl Browser { pub fn a(&self) {} fn b(&self) { fn nested() {} } }\nconst fn make() {}\nstatic mut COUNT: u8 = 0;\npub(in crate::app) type Alias = u8;\nmacro_rules! helper { () => {} }\nunion Raw { a: u8 }\nfn union() {}\nfn rows() -> impl Iterator<Item = u8> { todo!() }\nfn raw(p: *const Foo, q: impl AsRef<str>) {}\npub const fn made() {}\nunsafe impl Send for Browser {}\n",
        );
        let shown: Vec<String> = facts
            .outline
            .iter()
            .map(|o| {
                format!(
                    "{}|{:?}|{}|{}|{}",
                    String::from_utf8_lossy(&o.visibility),
                    o.kind,
                    String::from_utf8_lossy(&o.name),
                    o.of_trait.as_ref().map(|t| String::from_utf8_lossy(t).into_owned()).unwrap_or_default(),
                    o.functions
                )
            })
            .collect();
        assert_eq!(
            shown,
            [
                "pub|Struct|Browser||0",
                "pub(crate)|Enum|Mode||0",
                "|Impl|Browser|Default|1",
                "|Impl|Browser||2",
                "|Function|make||0",
                "|Static|COUNT||0",
                "pub(in crate::app)|Type|Alias||0",
                "|Macro|helper||0",
                "|Union|Raw||0",
                "|Function|union||0",
                "|Function|rows||0",
                "|Function|raw||0",
                "pub|Function|made||0",
                "|Impl|Browser|Send|0",
            ]
        );
        // Types in signatures declare nothing.
        let items: Vec<&str> = names(&facts.items);
        assert!(!items.contains(&"Foo") && !items.contains(&"Iterator"), "{items:?}");
    }

    #[test]
    fn unicode_and_unterminated_text_never_panic() {
        let facts = read("fn 結果() { let é = '🚀'; } mod 検索; use crate::界面::*; \"unterminated");
        assert_eq!(facts.declarations[0].name, "検索");
        assert_eq!(facts.uses[0].segments, ["crate", "界面"]);
        for source in ["r#\"open", "'", "/* open", "b'", "r#", "#[", "use crate::{a, b", "mod"] {
            let _ = read(source);
        }
    }

    #[test]
    fn imports_report_limits_and_keep_independent_items() {
        let nested = |n: usize| format!("use {}Item{}; use next::Value;", "{".repeat(n), "}".repeat(n));
        let within = read(&nested(MAX_NESTING));
        assert!(!within.limited);
        assert_eq!(within.uses[0].segments, ["Item"]);
        let beyond = read(&nested(MAX_NESTING + 1));
        assert!(beyond.limited);
        assert_eq!(beyond.uses.len(), 1);
        assert_eq!(beyond.uses[0].segments, ["next", "Value"]);

        let mut facts = Facts::default();
        let mut budget = size_of::<Use>() + 2 * size_of::<String>() + "groupOne".len();
        expand(&lex("group::{One, Two}", |_, _| {}), &[], "use", |_| 1, &mut budget, &mut facts);
        assert!(facts.limited);
        assert_eq!(facts.uses.len(), 1);
        assert_eq!(facts.uses[0].segments, ["group", "One"]);
    }

    #[test]
    fn import_budget_is_shared_across_source_files() {
        let one = size_of::<Use>() + size_of::<String>() + "Item".len();
        let mut used = ReadBudget { import_bytes: MAP_IMPORT_BYTES - one, ..Default::default() };
        let first = read_with_budget("use Item;", &mut used);
        assert!(!first.limited);
        assert_eq!(first.uses.len(), 1);
        assert_eq!(used.import_bytes, MAP_IMPORT_BYTES);
        let next = read_with_budget("use Other; pub fn independent() {}", &mut used);
        assert!(next.limited);
        assert!(next.uses.is_empty());
        assert_eq!(next.outline[0].name, b"independent");
    }
}
