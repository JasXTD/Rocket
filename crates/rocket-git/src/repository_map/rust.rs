//! A small lexical reader for Rust source: module declarations, `use` trees, paths
//! into this crate and the items a file declares at its top level. Comments and
//! literals are skipped. Macros are not expanded, `cfg` is not evaluated and nothing
//! is compiled or run, so what it reports is what the text says, not what rustc sees.
use super::{OutlineItem, OutlineKind};

#[derive(Clone, Debug, PartialEq, Eq)]
enum Token {
    Ident(String),
    /// A raw identifier (`r#type`): a name, never a keyword.
    Raw(String),
    Str(String),
    Punct(&'static str),
}

/// What one file says about modules and names.
#[derive(Clone, Debug, Default)]
pub(super) struct Facts {
    /// `mod name;`: a module in its own file.
    pub declarations: Vec<Declaration>,
    /// `mod name { … }`: the module path inside the file, ending in its own name.
    pub inline: Vec<Vec<String>>,
    pub uses: Vec<Use>,
    /// `crate::…`, `super::…`, `self::…` and `name::…` paths outside `use` lines.
    pub paths: Vec<Path>,
    /// Items each module of the file declares: inline module path and name.
    pub items: Vec<(Vec<String>, String)>,
    /// The file's own top level, in order.
    pub outline: Vec<OutlineItem>,
    /// Names the file uses on their own, for `use …::*` lines: inline module path and name.
    pub names: Vec<(Vec<String>, String)>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Declaration {
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
fn quoted(c: &[char], i: usize) -> (usize, String) {
    let mut j = i + 1;
    let mut text = String::new();
    while j < c.len() && c[j] != '"' {
        if c[j] == '\\' {
            if let Some(&next) = c.get(j + 1) {
                text.push(next);
            }
            j += 2;
        } else {
            text.push(c[j]);
            j += 1;
        }
    }
    ((j + 1).min(c.len()) - i, text)
}
/// A raw literal whose `r` is at `c[i]`: `r"…"`, `r#"…"#`. None when it isn't one.
fn raw(c: &[char], i: usize) -> Option<(usize, String)> {
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
            return Some((k + 1 + hashes - i, c[start..k].iter().collect()));
        }
        k += 1;
    }
    Some((c.len() - i, c[start..].iter().collect()))
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

fn lex(source: &str) -> Vec<Token> {
    let c: Vec<char> = source.chars().collect();
    let n = c.len();
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
            let (len, text) = quoted(&c, i + 1);
            out.push(Token::Str(text));
            i += 1 + len;
            continue;
        }
        if ch == 'b' && next == Some('\'') {
            i += 1 + quote(&c, i + 1);
            continue;
        }
        let raw_at = match (ch, next) {
            ('r', _) => Some(i),
            ('b' | 'c', Some('r')) => Some(i + 1),
            _ => None,
        };
        if let Some((len, text)) = raw_at.and_then(|at| raw(&c, at).map(|(len, text)| (len + at - i, text))) {
            out.push(Token::Str(text));
            i += len;
            continue;
        }
        if ch == '"' {
            let (len, text) = quoted(&c, i);
            out.push(Token::Str(text));
            i += len;
            continue;
        }
        if ch == '\'' {
            i += quote(&c, i);
            continue;
        }
        if ch.is_ascii_digit() {
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
        }
        i += punct.len().max(1);
    }
    out
}

const ITEMS: [&str; 8] = ["fn", "struct", "enum", "trait", "type", "const", "static", "union"];

/// Reads one file.
pub(super) fn read(source: &str) -> Facts {
    let t = lex(source);
    let mut facts = Facts::default();
    let is = |k: usize, p: &str| matches!(t.get(k), Some(Token::Punct(x)) if *x == p);
    let name = |k: usize| match t.get(k) {
        Some(Token::Ident(x) | Token::Raw(x)) => Some(x.clone()),
        _ => None,
    };
    // Some(name) for an inline module's braces, None for any other.
    let mut scopes: Vec<Option<String>> = Vec::new();
    let chain = |scopes: &[Option<String>]| scopes.iter().flatten().cloned().collect::<Vec<_>>();
    let module_level = |scopes: &[Option<String>]| scopes.iter().all(Option::is_some);
    let mut attributes: Vec<Vec<Token>> = Vec::new();
    let mut visibility: Option<String> = None;
    // At the start of a statement or item: `use` and `mod` count only there.
    let mut fresh = true;
    let mut k = 0;
    while k < t.len() {
        match &t[k] {
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
            Token::Ident(w) if w == "mod" && fresh && module_level(&scopes) => {
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
            Token::Ident(w) if w == "use" && fresh => {
                let start = k + 1;
                let mut end = start;
                while end < t.len() && !is(end, ";") {
                    end += 1;
                }
                let here = chain(&scopes);
                for (segments, alias, glob) in expand(&t[start..end]) {
                    if !segments.is_empty() {
                        facts.uses.push(Use {
                            chain: here.clone(),
                            segments,
                            alias,
                            glob,
                        });
                    }
                }
                k = end;
                continue;
            }
            Token::Ident(w) if w == "macro_rules" && is(k + 1, "!") && module_level(&scopes) => {
                if let Some(item) = name(k + 2) {
                    let here = chain(&scopes);
                    if here.is_empty() {
                        facts.outline.push(outline(OutlineKind::Macro, &item, visibility.take()));
                    }
                    facts.items.push((here, item));
                }
                k += 3;
                fresh = false;
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
    facts
}

fn outline(kind: OutlineKind, name: &str, visibility: Option<String>) -> OutlineItem {
    OutlineItem {
        kind,
        name: name.as_bytes().to_vec(),
        visibility: visibility.unwrap_or_default().into_bytes(),
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
        of_trait: of_trait.map(String::into_bytes),
        functions,
    }
}

type Tree = (Vec<String>, Option<String>, bool);
/// A `use` tree as flat paths: segments, `as` name, and whether it ends in `*`.
fn expand(body: &[Token]) -> Vec<Tree> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < body.len() {
        let before = i;
        i = walk(body, i, &[], &mut out);
        if i == before {
            i += 1;
        }
    }
    out
}
fn walk(body: &[Token], mut i: usize, prefix: &[String], out: &mut Vec<Tree>) -> usize {
    let mut segments = prefix.to_vec();
    let alias = |j: usize| match body.get(j) {
        Some(Token::Ident(a) | Token::Raw(a)) => Some(a.clone()),
        _ => None,
    };
    while i < body.len() {
        match &body[i] {
            Token::Punct("::") => {
                // A leading `::` names another crate.
                if segments.is_empty() {
                    segments.push(String::new());
                }
                i += 1;
            }
            Token::Punct("*") => {
                out.push((segments, None, true));
                return i + 1;
            }
            Token::Punct("{") => {
                i += 1;
                while i < body.len() && !matches!(body[i], Token::Punct("}")) {
                    if matches!(body[i], Token::Punct(",")) {
                        i += 1;
                        continue;
                    }
                    let before = i;
                    i = walk(body, i, &segments, out);
                    if i == before {
                        i += 1;
                    }
                }
                return i + 1;
            }
            Token::Ident(part) | Token::Raw(part) => {
                segments.push(part.clone());
                i += 1;
                if matches!(body.get(i), Some(Token::Punct("::"))) {
                    continue;
                }
                if matches!(body.get(i), Some(Token::Ident(a)) if a == "as") {
                    out.push((segments, alias(i + 1), false));
                    return i + 2;
                }
                out.push((segments, None, false));
                return i;
            }
            _ => return i,
        }
    }
    i
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
}
