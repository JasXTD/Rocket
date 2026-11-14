//! A small lexical reader for JavaScript and TypeScript: import and export lines,
//! `require()` and `import()` with a fixed path, the names each one takes, and what a
//! file declares at its top level. Comments, strings, template text and regular
//! expressions are skipped. Nothing is compiled or run, so a path built while the
//! program runs is not followed.
use super::{OutlineItem, OutlineKind};
use std::collections::{BTreeSet, HashMap};

#[derive(Clone, Debug, PartialEq, Eq)]
enum Token {
    Ident(String),
    Str(String),
    /// A number, a regular expression or a template with `${…}` in it.
    Value,
    Punct(char),
}

/// What one file imports and declares.
#[derive(Clone, Debug, Default)]
pub(super) struct Facts {
    pub imports: Vec<Import>,
    pub outline: Vec<OutlineItem>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Import {
    /// The path as written: `./view`, `react`, `node:fs`.
    pub specifier: String,
    /// The names it takes: `default` for a default import, `*` for everything, and the
    /// names used through a namespace (`ns.card()` takes `card`).
    pub names: BTreeSet<String>,
}

/// Names kept per import.
const NAMES: usize = 64;

fn ident_start(c: char) -> bool {
    c == '_' || c == '$' || c.is_alphabetic()
}
fn ident_part(c: char) -> bool {
    c == '_' || c == '$' || c.is_alphanumeric() || c == '\u{200c}' || c == '\u{200d}'
}

/// A quoted string starting at `c[i]`; it ends at its quote or, unfinished, at the
/// line's end (JSX text such as `don't` is not a string).
fn string(c: &[char], i: usize) -> (usize, String) {
    let quote = c[i];
    let mut j = i + 1;
    let mut text = String::new();
    while j < c.len() && c[j] != quote && c[j] != '\n' {
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
    // The closing quote is part of the string; a line break that ends one is not.
    let end = if c.get(j) == Some(&quote) { j + 1 } else { j.min(c.len()) };
    (end - i, text)
}
/// A template literal starting at `c[i] == '`'`: its length, and its text when it has
/// no `${…}`.
fn template(c: &[char], i: usize, depth: usize) -> (usize, Option<String>) {
    let mut j = i + 1;
    let mut text = String::new();
    let mut plain = true;
    while j < c.len() {
        match c[j] {
            '\\' => {
                if let Some(&next) = c.get(j + 1) {
                    text.push(next);
                }
                j += 2;
            }
            '`' => return (j + 1 - i, plain.then_some(text)),
            '$' if c.get(j + 1) == Some(&'{') && depth < 32 => {
                plain = false;
                j = expression(c, j + 2, depth + 1);
            }
            x => {
                text.push(x);
                j += 1;
            }
        }
    }
    (c.len() - i, None)
}
/// The end of a `${…}` expression starting at `j`, just past its `}`.
fn expression(c: &[char], mut j: usize, depth: usize) -> usize {
    let mut braces = 1;
    while j < c.len() {
        match c[j] {
            '{' => braces += 1,
            '}' => {
                braces -= 1;
                if braces == 0 {
                    return j + 1;
                }
            }
            '\'' | '"' => {
                j += string(c, j).0;
                continue;
            }
            '`' => {
                j += template(c, j, depth).0;
                continue;
            }
            '/' if c.get(j + 1) == Some(&'/') => {
                while j < c.len() && c[j] != '\n' {
                    j += 1;
                }
                continue;
            }
            '/' if c.get(j + 1) == Some(&'*') => {
                j += comment_end(c, j);
                continue;
            }
            _ => {}
        }
        j += 1;
    }
    c.len()
}
fn comment_end(c: &[char], i: usize) -> usize {
    let mut j = i + 2;
    while j < c.len() && !(c[j] == '*' && c.get(j + 1) == Some(&'/')) {
        j += 1;
    }
    (j + 2).min(c.len()) - i
}
/// A regular expression literal starting at `c[i] == '/'`: its length.
fn regex(c: &[char], i: usize) -> usize {
    let mut j = i + 1;
    let mut class = false;
    while j < c.len() && c[j] != '\n' {
        match c[j] {
            '\\' => j += 1,
            '[' => class = true,
            ']' => class = false,
            '/' if !class => {
                j += 1;
                while j < c.len() && ident_part(c[j]) {
                    j += 1;
                }
                return j - i;
            }
            _ => {}
        }
        j += 1;
    }
    j - i
}
/// Whether a `/` after `previous` starts a regular expression rather than dividing.
fn regex_may_start(previous: Option<&Token>) -> bool {
    match previous {
        None => true,
        Some(Token::Punct(p)) => !matches!(p, ')' | ']'),
        Some(Token::Ident(w)) => matches!(
            w.as_str(),
            "return" | "typeof" | "case" | "do" | "else" | "in" | "of" | "new" | "delete" | "void" | "throw" | "instanceof" | "yield" | "await" | "export" | "default"
        ),
        Some(Token::Str(_) | Token::Value) => false,
    }
}

/// Tokens, each with whether a line break came before it.
fn lex(source: &str) -> Vec<(Token, bool)> {
    let c: Vec<char> = source.chars().collect();
    let mut out: Vec<(Token, bool)> = Vec::new();
    let mut line = true;
    let mut i = 0;
    while i < c.len() {
        let x = c[i];
        if x == '\n' {
            line = true;
            i += 1;
            continue;
        }
        if x.is_whitespace() {
            i += 1;
            continue;
        }
        let token = if x == '/' && c.get(i + 1) == Some(&'/') {
            while i < c.len() && c[i] != '\n' {
                i += 1;
            }
            continue;
        } else if x == '/' && c.get(i + 1) == Some(&'*') {
            let end = comment_end(&c, i);
            line |= c[i..i + end].contains(&'\n');
            i += end;
            continue;
        } else if x == '\'' || x == '"' {
            let (len, text) = string(&c, i);
            i += len;
            Token::Str(text)
        } else if x == '`' {
            let (len, text) = template(&c, i, 0);
            i += len;
            text.map_or(Token::Value, Token::Str)
        } else if x == '/' && regex_may_start(out.last().map(|(t, _)| t)) {
            i += regex(&c, i);
            Token::Value
        } else if ident_start(x) || x == '#' && c.get(i + 1).is_some_and(|n| ident_start(*n)) {
            let start = i;
            i += 1;
            while i < c.len() && ident_part(c[i]) {
                i += 1;
            }
            Token::Ident(c[start..i].iter().collect())
        } else if x.is_ascii_digit() || x == '.' && c.get(i + 1).is_some_and(char::is_ascii_digit) {
            i += 1;
            while i < c.len() && (ident_part(c[i]) || c[i] == '.') {
                i += 1;
            }
            Token::Value
        } else {
            i += 1;
            Token::Punct(x)
        };
        out.push((token, line));
        line = false;
    }
    out
}

/// Words that start a declaration's line without declaring a name themselves.
const MODIFIERS: [&str; 4] = ["declare", "async", "abstract", "default"];

/// Reads one file.
pub(super) fn read(source: &str) -> Facts {
    // A byte-order mark is no part of the first line.
    let source = source.strip_prefix('﻿').unwrap_or(source);
    let tokens = lex(source);
    let t: Vec<&Token> = tokens.iter().map(|(t, _)| t).collect();
    let fresh_line = |k: usize| tokens.get(k).is_some_and(|(_, line)| *line);
    let ident = |k: usize| match t.get(k) {
        Some(Token::Ident(x)) => Some(x.as_str()),
        _ => None,
    };
    let text = |k: usize| match t.get(k) {
        Some(Token::Str(x)) => Some(x.as_str()),
        _ => None,
    };
    let punct = |k: usize, p: char| matches!(t.get(k), Some(Token::Punct(x)) if *x == p);
    let after_dot = |k: usize| k > 0 && punct(k - 1, '.') && !(k > 1 && punct(k - 2, '.'));
    // A declaration starts a statement: first in the file, after `;` or `}`, or on a new
    // line whose previous line does not carry on into it.
    let statement = |k: usize| {
        k == 0
            || punct(k - 1, ';')
            || punct(k - 1, '}')
            || fresh_line(k) && !matches!(t[k - 1], Token::Punct('=' | '(' | ',' | ':' | '?' | '|' | '&' | '+' | '-' | '*' | '/' | '.' | '[' | '!' | '<' | '>'))
    };
    let mut facts = Facts::default();
    // Bindings through which a namespace's names are used: (binding, import).
    let mut namespaces: Vec<(String, usize)> = Vec::new();
    let mut exported_locals: Vec<String> = Vec::new();
    let mut depth = 0usize;
    let mut k = 0;
    while k < t.len() {
        match t[k] {
            Token::Punct('{') => depth += 1,
            Token::Punct('}') => depth = depth.saturating_sub(1),
            Token::Ident(w) if w == "import" && !after_dot(k) => {
                if punct(k + 1, '(') {
                    // import('./lazy'), with a fixed path only.
                    if let Some(spec) = text(k + 2)
                        && punct(k + 3, ')')
                    {
                        facts.imports.push(import(spec, []));
                    }
                } else if !punct(k + 1, '.')
                    && let Some((found, binding, end)) = static_import(&t, k)
                {
                    let at = facts.imports.len();
                    facts.imports.push(found);
                    if let Some(binding) = binding {
                        namespaces.push((binding, at));
                    }
                    k = end;
                    continue;
                }
            }
            Token::Ident(w) if w == "require" && !after_dot(k) && punct(k + 1, '(') && punct(k + 3, ')') => {
                if let Some(spec) = text(k + 2) {
                    let mut names = BTreeSet::new();
                    let mut binding = None;
                    if k >= 2 && punct(k - 1, '=') {
                        if let Some(name) = ident(k - 2)
                            && matches!(ident(k.wrapping_sub(3)), Some("const" | "let" | "var"))
                        {
                            binding = Some(name.to_owned());
                        } else if punct(k - 2, '}') {
                            names.extend(destructured(&t, k - 2));
                        }
                    }
                    // require('./x').name
                    if punct(k + 4, '.')
                        && let Some(name) = ident(k + 5)
                    {
                        names.insert(name.to_owned());
                    }
                    let at = facts.imports.len();
                    facts.imports.push(import(spec, names));
                    if let Some(binding) = binding {
                        namespaces.push((binding, at));
                    }
                }
            }
            Token::Ident(w) if w == "export" && depth == 0 && !after_dot(k) => {
                k = export(&t, k, &mut facts, &mut exported_locals);
                continue;
            }
            // module.exports = …, module.exports.name = …, exports.name = …
            Token::Ident(w) if (w == "module" || w == "exports") && depth == 0 && !after_dot(k) && statement(k) => {
                let start = if w == "module" {
                    if !(punct(k + 1, '.') && ident(k + 2) == Some("exports")) {
                        k += 1;
                        continue;
                    }
                    k + 3
                } else {
                    k + 1
                };
                if punct(start, '=') && !punct(start + 1, '=') {
                    k = common_exports(&t, start + 1, &mut facts, &mut exported_locals);
                    continue;
                }
                if punct(start, '.')
                    && let Some(name) = ident(start + 1)
                    && punct(start + 2, '=')
                    && !punct(start + 3, '=')
                {
                    let kind = value_kind(&t, start + 3);
                    exported_locals.push(name.to_owned());
                    add(&mut facts.outline, kind, name, true, 0);
                    k = start + 3;
                    continue;
                }
            }
            Token::Ident(_) if depth == 0 && statement(k) && !after_dot(k) => {
                let mut j = k;
                while ident(j).is_some_and(|m| MODIFIERS.contains(&m)) {
                    j += 1;
                }
                if let Some((kind, name, end)) = declaration(&t, j) {
                    // Constants and variables are listed when exported: a file's own
                    // `const fs = require('fs')` lines are not what it offers.
                    if !matches!(kind, OutlineKind::Const | OutlineKind::Let | OutlineKind::Var) {
                        add(&mut facts.outline, kind, &name, false, if kind == OutlineKind::Class { methods(&t, j) } else { 0 });
                    }
                    k = end;
                    continue;
                }
            }
            _ => {}
        }
        k += 1;
    }
    // Names used through a namespace: `ns.card()`, `relay.gitRelay(…)`.
    // One pass over the file for every binding at once: a large file can hold hundreds.
    let mut by_binding: HashMap<&str, Vec<usize>> = HashMap::new();
    for (binding, at) in &namespaces {
        by_binding.entry(binding.as_str()).or_default().push(*at);
    }
    if !by_binding.is_empty() {
        for i in 0..t.len().saturating_sub(2) {
            let Some(imports) = ident(i).and_then(|word| by_binding.get(word)) else { continue };
            if after_dot(i) || !punct(i + 1, '.') {
                continue;
            }
            let Some(name) = ident(i + 2) else { continue };
            for &at in imports {
                if facts.imports[at].names.len() < NAMES {
                    facts.imports[at].names.insert(name.to_owned());
                }
            }
        }
    }
    // `export { helper }` and `module.exports = { helper }` export a name declared elsewhere.
    for name in exported_locals {
        for o in facts.outline.iter_mut().filter(|o| o.name == name.as_bytes()) {
            if !o.exported {
                o.exported = true;
                o.visibility = b"export".to_vec();
            }
        }
    }
    facts
}

fn import(spec: &str, names: impl IntoIterator<Item = String>) -> Import {
    Import {
        specifier: spec.to_owned(),
        names: names.into_iter().take(NAMES).collect(),
    }
}

fn add(outline: &mut Vec<OutlineItem>, kind: OutlineKind, name: &str, exported: bool, functions: usize) {
    if outline.len() >= 4096 {
        return;
    }
    if let Some(o) = outline.iter_mut().find(|o| o.name == name.as_bytes() && (o.kind == kind || matches!(o.kind, OutlineKind::Const))) {
        // `exports.run = run` after `function run`: the function is what is exported.
        if exported {
            o.exported = true;
            o.visibility = b"export".to_vec();
        }
        if matches!(o.kind, OutlineKind::Const) && kind != OutlineKind::Const {
            o.kind = kind;
        }
        return;
    }
    outline.push(OutlineItem {
        kind,
        name: name.as_bytes().to_vec(),
        visibility: if exported { b"export".to_vec() } else { Vec::new() },
        exported,
        of_trait: None,
        functions,
    });
}

/// `import … from '…'`, `import '…'` and TypeScript's `import x = require('…')` at
/// `k`: the import, the namespace binding its names are used through, and where it ends.
fn static_import(t: &[&Token], k: usize) -> Option<(Import, Option<String>, usize)> {
    let ident = |j: usize| match t.get(j) {
        Some(Token::Ident(x)) => Some(x.as_str()),
        _ => None,
    };
    let text = |j: usize| match t.get(j) {
        Some(Token::Str(x)) => Some(x.as_str()),
        _ => None,
    };
    let punct = |j: usize, p: char| matches!(t.get(j), Some(Token::Punct(x)) if *x == p);
    let mut j = k + 1;
    // import type { T } from …; import type T from …
    if ident(j) == Some("type") && !punct(j + 1, ',') && ident(j + 1) != Some("from") && !punct(j + 1, '=') {
        j += 1;
    }
    if let Some(spec) = text(j) {
        return Some((import(spec, []), None, j + 1));
    }
    let mut names = BTreeSet::new();
    let mut binding = None;
    if let Some(local) = ident(j)
        && local != "from"
    {
        // TypeScript: import fs = require('fs')
        if punct(j + 1, '=') && ident(j + 2) == Some("require") && punct(j + 3, '(') {
            let spec = text(j + 4)?;
            return Some((import(spec, []), Some(local.to_owned()), j + 6));
        }
        names.insert("default".to_owned());
        j += 1;
        if punct(j, ',') {
            j += 1;
        }
    }
    if punct(j, '*') && ident(j + 1) == Some("as") {
        binding = ident(j + 2).map(str::to_owned);
        names.insert("*".to_owned());
        j += 3;
    }
    if punct(j, '{') {
        let (taken, end) = braced(t, j);
        names.extend(taken);
        j = end;
    }
    if ident(j) == Some("from")
        && let Some(spec) = text(j + 1)
    {
        // `* as ns` takes `*`; the names used through `ns` are added once the file is read.
        return Some((import(spec, names), binding, j + 2));
    }
    None
}

/// `{ a, b as c, type T, "d-e" as f }` at `open`: the names as they are exported, and
/// where the list ends.
fn braced(t: &[&Token], open: usize) -> (Vec<String>, usize) {
    let mut names = Vec::new();
    let mut j = open + 1;
    let mut first = true;
    while j < t.len() && j < open + 4096 {
        match t[j] {
            Token::Punct('}') => return (names, j + 1),
            Token::Punct(',') => first = true,
            Token::Ident(w) if first && w == "type" && matches!(t.get(j + 1), Some(Token::Ident(_) | Token::Str(_))) && !matches!(t.get(j + 1), Some(Token::Ident(x)) if x == "as") => {}
            Token::Ident(w) | Token::Str(w) if first => {
                names.push(w.clone());
                first = false;
            }
            _ => {}
        }
        j += 1;
    }
    (names, j)
}

/// The keys a destructuring pattern ending at `close` takes: `{ a, b: c, d = 1 }`.
fn destructured(t: &[&Token], close: usize) -> Vec<String> {
    let mut open = close;
    let mut depth = 0;
    loop {
        match t[open] {
            Token::Punct('}') => depth += 1,
            Token::Punct('{') => {
                depth -= 1;
                if depth == 0 {
                    break;
                }
            }
            _ => {}
        }
        if open == 0 || close - open > 4096 {
            return Vec::new();
        }
        open -= 1;
    }
    let mut names = Vec::new();
    let mut nested = 0;
    for j in open + 1..close {
        match t[j] {
            Token::Punct('{' | '[') => nested += 1,
            Token::Punct('}' | ']') => nested -= 1,
            Token::Ident(w) if nested == 0 && matches!(t[j - 1], Token::Punct('{' | ',')) => names.push(w.clone()),
            _ => {}
        }
    }
    names
}

/// An `export` line at `k`: adds what it exports or re-exports and says where it ends.
fn export(t: &[&Token], k: usize, facts: &mut Facts, exported_locals: &mut Vec<String>) -> usize {
    let ident = |j: usize| match t.get(j) {
        Some(Token::Ident(x)) => Some(x.as_str()),
        _ => None,
    };
    let text = |j: usize| match t.get(j) {
        Some(Token::Str(x)) => Some(x.as_str()),
        _ => None,
    };
    let punct = |j: usize, p: char| matches!(t.get(j), Some(Token::Punct(x)) if *x == p);
    let mut j = k + 1;
    if ident(j) == Some("type") && punct(j + 1, '{') {
        j += 1;
    }
    // export * from '…'; export * as ns from '…'
    if punct(j, '*') {
        let mut end = j + 1;
        if ident(end) == Some("as") {
            end += 2;
        }
        if ident(end) == Some("from")
            && let Some(spec) = text(end + 1)
        {
            facts.imports.push(import(spec, ["*".to_owned()]));
            return end + 2;
        }
        return j + 1;
    }
    // export { a, b as c } [from '…']
    if punct(j, '{') {
        let (names, end) = braced(t, j);
        if ident(end) == Some("from")
            && let Some(spec) = text(end + 1)
        {
            facts.imports.push(import(spec, names));
            return end + 2;
        }
        exported_locals.extend(names);
        return end;
    }
    // export default function name() {}; export default class {}; export default <value>
    let default = ident(j) == Some("default");
    while ident(j).is_some_and(|m| MODIFIERS.contains(&m)) {
        j += 1;
    }
    if let Some((kind, name, end)) = declaration(t, j) {
        add(&mut facts.outline, kind, &name, true, if kind == OutlineKind::Class { methods(t, j) } else { 0 });
        return end;
    }
    if default && matches!(ident(j), Some("function" | "class")) {
        let kind = if ident(j) == Some("class") { OutlineKind::Class } else { OutlineKind::Function };
        add(&mut facts.outline, kind, "default", true, if kind == OutlineKind::Class { methods(t, j) } else { 0 });
        return j + 1;
    }
    if default && let Some(name) = ident(j) {
        // export default App;
        if punct(j + 1, ';') || t.get(j + 1).is_none() {
            exported_locals.push(name.to_owned());
        }
    }
    j
}

/// `module.exports = …` from the token after `=`.
fn common_exports(t: &[&Token], j: usize, facts: &mut Facts, exported_locals: &mut Vec<String>) -> usize {
    let ident = |i: usize| match t.get(i) {
        Some(Token::Ident(x)) => Some(x.as_str()),
        _ => None,
    };
    let punct = |i: usize, p: char| matches!(t.get(i), Some(Token::Punct(x)) if *x == p);
    if punct(j, '{') {
        // module.exports = { a, b: value, c() {} }
        let mut depth = 0;
        let mut i = j;
        while i < t.len() {
            match t[i] {
                Token::Punct('{' | '(' | '[') => depth += 1,
                Token::Punct('}' | ')' | ']') => {
                    depth -= 1;
                    if depth == 0 {
                        return i + 1;
                    }
                }
                Token::Ident(w) | Token::Str(w) if depth == 1 && matches!(t[i - 1], Token::Punct('{' | ',')) => {
                    if punct(i + 1, ':') {
                        let kind = value_kind(t, i + 2);
                        add(&mut facts.outline, kind, w, true, 0);
                    } else if punct(i + 1, '(') {
                        add(&mut facts.outline, OutlineKind::Function, w, true, 0);
                    } else {
                        exported_locals.push(w.clone());
                        if !facts.outline.iter().any(|o| o.name == w.as_bytes()) {
                            add(&mut facts.outline, OutlineKind::Const, w, true, 0);
                        }
                    }
                }
                _ => {}
            }
            i += 1;
        }
        return i;
    }
    if let Some((kind, name, end)) = declaration(t, j) {
        add(&mut facts.outline, kind, &name, true, if kind == OutlineKind::Class { methods(t, j) } else { 0 });
        return end;
    }
    if let Some(name) = ident(j)
        && !punct(j + 1, '(')
        && !punct(j + 1, '.')
    {
        exported_locals.push(name.to_owned());
    }
    j
}

/// What a value assigned at `j` is: a function, a class, else a constant.
fn value_kind(t: &[&Token], j: usize) -> OutlineKind {
    match t.get(j) {
        Some(Token::Ident(w)) if w == "function" || w == "async" => OutlineKind::Function,
        Some(Token::Ident(w)) if w == "class" => OutlineKind::Class,
        Some(Token::Punct('(')) => {
            // (a, b) => …
            let mut depth = 0;
            let mut i = j;
            while i < t.len() && i < j + 512 {
                match t[i] {
                    Token::Punct('(') => depth += 1,
                    Token::Punct(')') => {
                        depth -= 1;
                        if depth == 0 {
                            return if matches!(t.get(i + 1), Some(Token::Punct('='))) && matches!(t.get(i + 2), Some(Token::Punct('>'))) {
                                OutlineKind::Function
                            } else {
                                OutlineKind::Const
                            };
                        }
                    }
                    _ => {}
                }
                i += 1;
            }
            OutlineKind::Const
        }
        Some(Token::Ident(_)) if matches!(t.get(j + 1), Some(Token::Punct('='))) && matches!(t.get(j + 2), Some(Token::Punct('>'))) => OutlineKind::Function,
        _ => OutlineKind::Const,
    }
}

/// A declaration at `j`: `function name`, `function* name`, `class Name`, `const name`,
/// `let`/`var`, TypeScript's `interface`, `type Name =` and `enum`. Its kind, name and
/// the token after the name.
fn declaration(t: &[&Token], j: usize) -> Option<(OutlineKind, String, usize)> {
    let ident = |i: usize| match t.get(i) {
        Some(Token::Ident(x)) => Some(x.as_str()),
        _ => None,
    };
    let punct = |i: usize, p: char| matches!(t.get(i), Some(Token::Punct(x)) if *x == p);
    let word = ident(j)?;
    let (kind, at) = match word {
        "function" if punct(j + 1, '*') => (OutlineKind::Function, j + 2),
        "function" => (OutlineKind::Function, j + 1),
        "class" => (OutlineKind::Class, j + 1),
        "const" if ident(j + 1) == Some("enum") => (OutlineKind::Enum, j + 2),
        "const" => (OutlineKind::Const, j + 1),
        "let" => (OutlineKind::Let, j + 1),
        "var" => (OutlineKind::Var, j + 1),
        "interface" => (OutlineKind::Interface, j + 1),
        "enum" => (OutlineKind::Enum, j + 1),
        "type" if ident(j + 1).is_some() && (punct(j + 2, '=') || punct(j + 2, '<')) => (OutlineKind::Type, j + 1),
        _ => return None,
    };
    let name = ident(at)?;
    // `class extends Base` and `function (…)` declare no name; nor do reserved words.
    if ["extends", "implements", "of", "in", "instanceof"].contains(&name) {
        return None;
    }
    Some((kind, name.to_owned(), at + 1))
}

/// The methods a class at `k` declares in its own body.
fn methods(t: &[&Token], k: usize) -> usize {
    const BEFORE: [&str; 11] = ["static", "async", "get", "set", "public", "private", "protected", "readonly", "override", "abstract", "declare"];
    let mut j = k + 1;
    let mut parens = 0i32;
    while j < t.len() && !(matches!(t[j], Token::Punct('{')) && parens == 0) {
        match t[j] {
            Token::Punct('(') => parens += 1,
            Token::Punct(')') => parens -= 1,
            Token::Punct(';') if parens == 0 => return 0,
            _ => {}
        }
        j += 1;
    }
    let mut depth = 0;
    let mut count = 0;
    while j < t.len() {
        match t[j] {
            Token::Punct('{') => depth += 1,
            Token::Punct('}') => {
                depth -= 1;
                if depth == 0 {
                    break;
                }
            }
            Token::Ident(name) if depth == 1 && matches!(t.get(j + 1), Some(Token::Punct('(' | '<'))) => {
                let starts = match t[j - 1] {
                    Token::Punct('{' | '}' | ';' | '*') => true,
                    Token::Ident(m) => BEFORE.contains(&m.as_str()),
                    _ => false,
                };
                if starts && !BEFORE.contains(&name.as_str()) {
                    count += 1;
                }
            }
            _ => {}
        }
        j += 1;
    }
    count
}

#[cfg(test)]
mod tests {
    use super::*;

    fn imports(source: &str) -> Vec<(String, Vec<String>)> {
        read(source).imports.into_iter().map(|i| (i.specifier, i.names.into_iter().collect())).collect()
    }
    fn row(spec: &str, names: &[&str]) -> (String, Vec<String>) {
        (spec.to_owned(), names.iter().map(|n| n.to_string()).collect())
    }

    #[test]
    fn imports_requires_and_reexports_with_the_names_they_take() {
        let found = imports(
            "import React, { useState as state, type FC } from 'react';\nimport * as view from './view';\nimport './side.css';\nimport type { Props } from \"./types\";\nexport { card, list as rows } from './cards';\nexport * from './all';\nexport * as ns from './ns';\nconst { gitRelay, other: alias } = require('./git-relay.cjs');\nconst fs = require('node:fs');\nconst fixture = require('./fixture').make;\nconst lazy = await import('./lazy.js');\nview.card(); view.list(); fs.readFileSync(); obj.view.not();\nimport fsx = require('fs-extra');\n",
        );
        assert_eq!(
            found,
            [
                row("react", &["FC", "default", "useState"]),
                row("./view", &["*", "card", "list"]),
                row("./side.css", &[]),
                row("./types", &["Props"]),
                row("./cards", &["card", "list"]),
                row("./all", &["*"]),
                row("./ns", &["*"]),
                row("./git-relay.cjs", &["gitRelay", "other"]),
                row("node:fs", &["readFileSync"]),
                row("./fixture", &["make"]),
                row("./lazy.js", &[]),
                row("fs-extra", &[]),
            ]
        );
    }

    #[test]
    fn comments_strings_templates_and_regexes_hide_their_text() {
        let found = imports(
            "// import x from './comment';\n/* require('./block') */\nconst s = \"import y from './string'\";\nconst t = `require('./template') ${ require('./inside') } ${`${ '}' }`}`;\nconst r = /require\\('\\.\\/regex'\\)[/]/g;\nconst d = a / b / require('./after-division');\nconst jsx = <p>Don't</p>;\nconst real = require('./real');\n",
        );
        assert_eq!(found, [row("./after-division", &[]), row("./real", &[])]);
    }

    #[test]
    fn the_outline_lists_top_level_names_exports_first_marked() {
        let facts = read(
            "export function card() {}\nexport default class Board extends Base { constructor() {} static make() {} get size() { return 1 } #hidden() {} field = 1; arrow = () => {} }\nexport const LIMIT = 4, OTHER = 5;\nexport interface Props { a: string }\nexport type Id = string;\nexport enum Mode { A }\nfunction helper() { function inner() {} }\nconst local = 1\nfunction later() {}\nclass Private {}\nexport { helper };\nconst f = function named() {};\nlet x = y\n  function notDeclared() {}\n",
        );
        let rows: Vec<(OutlineKind, String, bool, usize)> = facts.outline.iter().map(|o| (o.kind, String::from_utf8(o.name.clone()).unwrap(), o.exported, o.functions)).collect();
        assert_eq!(
            rows,
            [
                (OutlineKind::Function, "card".into(), true, 0),
                (OutlineKind::Class, "Board".into(), true, 4),
                (OutlineKind::Const, "LIMIT".into(), true, 0),
                (OutlineKind::Interface, "Props".into(), true, 0),
                (OutlineKind::Type, "Id".into(), true, 0),
                (OutlineKind::Enum, "Mode".into(), true, 0),
                (OutlineKind::Function, "helper".into(), true, 0),
                (OutlineKind::Function, "later".into(), false, 0),
                (OutlineKind::Class, "Private".into(), false, 0),
                (OutlineKind::Function, "notDeclared".into(), false, 0),
            ],
            "{rows:#?}"
        );
    }

    #[test]
    fn common_js_exports_mark_their_names() {
        let facts = read(
            "const fs = require('fs');\nfunction gitRelay() {}\nfunction unused() {}\nmodule.exports = { gitRelay, run: () => 1, value: 2, method() {} };\nexports.extra = function () {};\nmodule.exports.Klass = class {};\n",
        );
        let rows: Vec<(OutlineKind, String, bool)> = facts.outline.iter().map(|o| (o.kind, String::from_utf8(o.name.clone()).unwrap(), o.exported)).collect();
        assert_eq!(
            rows,
            [
                (OutlineKind::Function, "gitRelay".into(), true),
                (OutlineKind::Function, "unused".into(), false),
                (OutlineKind::Function, "run".into(), true),
                (OutlineKind::Const, "value".into(), true),
                (OutlineKind::Function, "method".into(), true),
                (OutlineKind::Function, "extra".into(), true),
                (OutlineKind::Class, "Klass".into(), true),
            ],
            "{rows:#?}"
        );
    }
}
