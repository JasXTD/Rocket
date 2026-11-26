use super::super::{LABEL_BYTES, MAP_LINKS, ObjectId, packages};
use super::{Facts, SourceLocation, Token, after_group, close};
use crate::{CancellationToken, Result};
use std::collections::BTreeMap;
use std::ops::Bound;

/// Decode a cooked Rust string after the lexer has found its closing quote.
pub(super) fn string_value(chars: &[char]) -> Option<String> {
    if !chars.contains(&'\\') {
        return Some(chars.iter().collect());
    }
    let mut value = String::new();
    let mut chars = chars.iter().copied().peekable();
    while let Some(c) = chars.next() {
        if c != '\\' {
            value.push(c);
            continue;
        }
        let c = match chars.next()? {
            c @ ('\\' | '"' | '\'') => c,
            'n' => '\n',
            'r' => '\r',
            't' => '\t',
            '0' => '\0',
            'x' => {
                let n = chars.next()?.to_digit(16)? * 16 + chars.next()?.to_digit(16)?;
                if n > 0x7f {
                    return None;
                }
                char::from_u32(n)?
            }
            'u' => {
                if chars.next()? != '{' {
                    return None;
                }
                let mut n = chars.next()?.to_digit(16)?;
                let mut digits = 1;
                loop {
                    match chars.next()? {
                        '}' => break,
                        '_' => continue,
                        digit if digits < 6 => {
                            n = n * 16 + digit.to_digit(16)?;
                            digits += 1;
                        }
                        _ => return None,
                    }
                }
                char::from_u32(n)?
            }
            c @ ('\n' | '\r') => {
                if c == '\r' && chars.next()? != '\n' {
                    return None;
                }
                while chars.next_if(|c| matches!(c, ' ' | '\t' | '\n' | '\r')).is_some() {}
                continue;
            }
            _ => return None,
        };
        value.push(c);
    }
    Some(value)
}

/// A direct call may name a built-in explicitly. Exclude lookalike paths such
/// as custom::std::include_str and unqualified names shadowed in this file.
fn builtin(prefix: &[Token], shadowed: bool) -> bool {
    use Token::{Ident, Punct, Raw};
    match prefix {
        [before @ .., Ident(ns), Punct("::")] if matches!(ns.as_str(), "std" | "core") => match before {
            [.., Ident(prior), Punct("::")] => matches!(
                prior.as_str(),
                "return" | "break" | "yield" | "in" | "else" | "if" | "match"
            ),
            [.., Raw(_) | Punct(">"), Punct("::")] => false,
            _ => true,
        },
        [.., Punct("::" | "." | "$")] => false,
        _ => !shadowed,
    }
}

fn embed_bit(name: &str) -> u8 {
    match name { "include_str" => 1, "include_bytes" => 2, _ => 0 }
}

#[derive(Clone, Copy, Default)]
pub(super) struct Scope {
    pub imports: u8,
    pub export_macros: bool,
}

pub(super) fn import_shadows(imports: &[super::Use]) -> u8 {
    let mut mask = 0;
    for u in imports.iter().filter(|u| !u.glob) {
        let Some(name) = u.alias.as_ref().or(u.segments.last()) else { continue };
        let path = match u.segments.as_slice() {
            [root, path @ ..] if root.is_empty() => path,
            path => path,
        };
        if !matches!(path, [ns, item] if matches!(ns.as_str(), "std" | "core") && item == name) {
            mask |= embed_bit(name);
        }
    }
    mask
}

pub(super) fn calls(tokens: &[Token], sites: &[SourceLocation], bindings: &std::collections::HashMap<Option<usize>, Scope>, used: &mut usize, facts: &mut Facts) {
    use Token::{Ident, Punct, Str};
    // Imports apply throughout their block. macro_rules! starts at the definition
    // and flows into subsequent nested scopes, including inline modules.
    let mut imported = bindings.get(&None).copied().unwrap_or_default().imports;
    let mut textual = 0;
    let mut scopes = Vec::new();
    let mut i = 0;
    while i < tokens.len() {
        match tokens[i] {
            Punct("{") => {
                if scopes.len() == super::MAX_NESTING {
                    facts.limited = true;
                    break;
                }
                let scope = bindings.get(&Some(i)).copied().unwrap_or_default();
                scopes.push((textual, imported, scope.export_macros));
                let module = matches!(&tokens[..i], [.., Ident(m), Ident(_) | Token::Raw(_)] if m == "mod");
                imported = scope.imports | if module { 0 } else { imported };
            }
            Punct("}") => {
                if let Some((outer_macros, outer_imports, export)) = scopes.pop() {
                    if !export { textual = outer_macros; }
                    imported = outer_imports;
                }
            }
            _ => {}
        }
        if tokens[i] == Punct("#") {
            let open = if tokens.get(i + 1) == Some(&Punct("!")) {
                i + 2
            } else {
                i + 1
            };
            if tokens.get(open) == Some(&Punct("[")) {
                // A doc value is a Rust expression. Other attributes may consume
                // arbitrary tokens as data, including something that looks like a call.
                if !matches!(&tokens[open + 1..], [Ident(doc), Punct("="), ..] if doc == "doc") {
                    i = after_group(tokens, open);
                    continue;
                }
            }
        }
        let [Ident(name), Punct("!"), tail @ ..] = &tokens[i..] else {
            i += 1;
            continue;
        };
        let open = i + if name == "macro_rules" { 3 } else { 2 };
        let Some(end) = tokens.get(open).and_then(close) else {
            i += 1;
            continue;
        };
        if name == "macro_rules" && let [Ident(binding) | Token::Raw(binding), ..] = tail {
            textual |= embed_bit(binding);
        }
        let kind = match name.as_str() {
            "include_str" => Some("include_str!"),
            "include_bytes" => Some("include_bytes!"),
            _ => None,
        };
        if let Some(kind) = kind {
            if builtin(&tokens[..i], (textual | imported) & embed_bit(name) != 0)
                && let [_, Str(path), rest @ ..] = tail
            {
                let rest = rest.strip_prefix(&[Punct(",")]).unwrap_or(rest);
                if matches!(rest.first(), Some(Punct(p)) if *p == end) {
                    if *used >= MAP_LINKS {
                        facts.limited = true;
                        break;
                    }
                    *used += 1;
                    if !path.is_empty() && path.len() <= LABEL_BYTES {
                        let literal = sites[open + 1];
                        let site = if literal.line == sites[i].line { literal } else { sites[i] };
                        facts.embeds.push((kind, path.clone(), site));
                    }
                }
            }
        }
        // A macro can discard or stringify its arguments. Never infer file reads
        // from a macro definition or another macro's unevaluated input.
        i = after_group(tokens, open);
    }
}

/// Only regular blobs in the pinned Git tree can be targets. Check each entered
/// directory too: link/../file need not mean file when link is a symlink.
fn target(source: &[u8], path: &str, blobs: &BTreeMap<&[u8], (ObjectId, usize)>) -> Option<Vec<u8>> {
    let separator = |c| c == '/' || cfg!(windows) && c == '\\';
    if path.starts_with(separator) || cfg!(windows) && path.contains(':') || path.chars().any(char::is_control) {
        return None;
    }
    let mut parts = path.split(separator).peekable();
    let mut dir = packages::parent(source).to_vec();
    while let Some(part) = parts.next() {
        match part {
            "" | "." | ".." if parts.peek().is_none() => return None,
            "" | "." => continue,
            ".." => {
                if dir.is_empty() {
                    return None;
                }
                dir.truncate(packages::parent(&dir).len());
            }
            p => {
                if !dir.is_empty() {
                    dir.push(b'/');
                }
                dir.extend_from_slice(p.as_bytes());
                if dir.len() > LABEL_BYTES {
                    return None;
                }
                if parts.peek().is_none() {
                    return blobs.contains_key(dir.as_slice()).then_some(dir);
                }
                dir.push(b'/');
                let first = blobs
                    .range::<[u8], _>((Bound::Included(dir.as_slice()), Bound::Unbounded))
                    .next()?;
                if !first.0.starts_with(&dir) {
                    return None;
                }
                dir.pop();
            }
        }
    }
    None
}

impl Facts {
    pub(in super::super) fn embedded_files<'a>(
        &'a self,
        source: &'a [u8],
        blobs: &'a BTreeMap<&[u8], (ObjectId, usize)>,
        cancel: &'a CancellationToken,
    ) -> impl Iterator<Item = Result<(&'static str, Vec<u8>, SourceLocation)>> + 'a {
        self.embeds.iter().filter_map(move |(kind, path, line)| {
            cancel
                .check()
                .map(|()| target(source, path, blobs).map(|p| (*kind, p, *line)))
                .transpose()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::super::{ReadBudget, read, read_with_budget};
    use super::*;

    fn embed_lines(source: &str) -> Vec<(&'static str, String, u32)> {
        read(source).embeds.into_iter().map(|(kind, path, site)| (kind, path, site.line)).collect()
    }

    #[test]
    fn literal_embeds_read_escapes_raw_paths_and_doc_values() {
        let source = r###"
            #![doc = include_str!("readme\x20\u{96_ea}.md")]
            const A: &[u8] = ::core::include_bytes![r##"pictures/#1 "snow".bin"##,];
            fn text() -> &'static str { return ::std::include_str!{ /* relative */ "a/\
                    example.md" }; }
            #[custom(include_str!("not a read"))] struct Marker;
        "###;
        assert_eq!(
            embed_lines(source),
            [
                ("include_str!", "readme 雪.md".into(), 2),
                ("include_bytes!", "pictures/#1 \"snow\".bin".into(), 3),
                ("include_str!", "a/example.md".into(), 4)
            ]
        );
    }

    #[test]
    fn embed_shadowing_follows_the_definitions_scope_and_order() {
        let source = r#"
            const ROOT: &str = include_str!("root");
            mod inner {
                const EARLY: &str = include_str!("early");
                macro_rules! include_str { ($p:literal) => { "local" } }
                const LATE: &str = include_str!("shadowed");
                const BUILTIN: &str = std::include_str!("explicit");
            }
            fn local() {
                include_bytes!("before-import");
                use other::file as include_bytes;
                mod nested { const ASSET: &[u8] = include_bytes!("nested"); }
            }
            const AFTER: &[u8] = include_bytes!("outside");
            #[macro_use] mod exported {
                macro_rules! include_bytes { ($p:literal) => { b"local" } }
            }
            const LATE: &[u8] = include_bytes!("exported-shadow");
        "#;
        let facts = read(source);
        let paths: Vec<_> = facts.embeds.iter().map(|(_, path, _)| path.as_str()).collect();
        assert_eq!(paths, ["root", "early", "explicit", "nested", "outside"]);

        let depth = super::super::MAX_NESTING + 1;
        let nested = format!("{}include_str!(\"too-deep\");{}", "{".repeat(depth), "}".repeat(depth));
        let facts = read(&nested);
        assert!(facts.limited && facts.embeds.is_empty());
    }

    #[test]
    fn shadowed_calls_and_opaque_macro_input_are_not_file_dependencies() {
        let source = r###"
            use other::{file as include_bytes};
            macro_rules! include_str { ($arg:tt) => { std::include_str!("example") }; }
            fn demo() {
                include_str!("shadowed");
                include_bytes!("alias");
                stringify!( ::std::include_str!("descriptive text") );
                custom::std::include_bytes!("custom");
                other::include_bytes!("custom");
                std::include_str!(concat!("a", "b")); std::include_str!(env!("OUT_DIR"));
                std::include_str!(b"bytes"); std::include_str!(c"c-string"); std::include_bytes!(0 + "number");
                std::include_str!("bad\q"); std::include_str!("extra", 2);
                let x = r#"std::include_str!("not code")"#; // std::include_bytes!("comment")
                ::std::include_str!("actual");
            }
        "###;
        assert_eq!(embed_lines(source), [("include_str!", "actual".into(), 14)]);
    }

    #[test]
    fn candidate_limit_is_shared_even_for_missing_files_and_preserves_other_facts() {
        let mut budget = ReadBudget {
            embed_candidates: MAP_LINKS - 1,
            ..Default::default()
        };
        let first = read_with_budget("include_str!(\"missing\");", &mut budget);
        let second = read_with_budget("mod child; include_bytes!(\"another\");", &mut budget);
        assert_eq!(first.embeds.len(), 1);
        assert!(!first.limited);
        assert!(second.limited && second.embeds.is_empty());
        assert_eq!(second.declarations[0].name, "child");
        assert_eq!(budget.embed_candidates, MAP_LINKS);
    }

    #[test]
    fn paths_stay_inside_real_directories_of_the_revision() {
        let oid = ObjectId::parse(&"1".repeat(40)).unwrap();
        let blobs: BTreeMap<_, _> = [b"src/lib.rs".as_slice(), b"docs/a.md", b"img/p.bin"]
            .into_iter()
            .map(|p| (p, (oid.clone(), 1)))
            .collect();
        assert_eq!(
            target(b"src/lib.rs", "../docs/../img/./p.bin", &blobs).as_deref(),
            Some(&b"img/p.bin"[..])
        );
        for path in [
            "../../img/p.bin",
            "/img/p.bin",
            "../link/../img/p.bin",
            "../docs/a.md/../a.md",
            "../docs/",
            "../img/p.bin/",
        ] {
            assert_eq!(target(b"src/lib.rs", path, &blobs), None, "{path}");
        }
    }
}
