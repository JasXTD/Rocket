//! A small lexical reader for Go: the package clause, the imports, the names used
//! through each import (`http.Client`) and the types and functions a file declares at
//! its top level, with each method counted on its type. Comments, strings and runes are
//! skipped. Build constraints other than `//go:build ignore` are not evaluated, so files
//! for every platform are read together.
use super::{OutlineItem, OutlineKind};

#[derive(Clone, Debug, PartialEq, Eq)]
enum Token {
    Ident(String),
    Str(String),
    /// A number or a rune.
    Value,
    Punct(char),
}

/// What one file declares and imports.
#[derive(Clone, Debug, Default)]
pub(super) struct Facts {
    /// The package clause's name.
    pub package: Option<String>,
    /// `//go:build ignore`: a program kept beside the package, never built with it.
    pub ignored: bool,
    pub imports: Vec<Import>,
    /// `X.Name` pairs: the names used through each import's name.
    pub selectors: Vec<(String, String)>,
    /// Types and functions at the top level, in order.
    pub outline: Vec<OutlineItem>,
    /// The type of each method's receiver.
    pub methods: Vec<String>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Import {
    pub path: String,
    /// The name it is imported as: a name, `_` or `.`.
    pub alias: Option<String>,
}

/// `X.Name` pairs kept per file.
const SELECTORS: usize = 100_000;

fn ident_start(c: char) -> bool {
    c == '_' || c.is_alphabetic()
}
fn ident_part(c: char) -> bool {
    c == '_' || c.is_alphanumeric()
}
pub(super) fn exported(name: &str) -> bool {
    name.chars().next().is_some_and(char::is_uppercase)
}

/// `//go:build ignore` (or the old `// +build ignore`) before the package clause.
fn build_ignored(source: &str) -> bool {
    for line in source.lines() {
        let line = line.trim();
        if line.starts_with("package ") || line == "package" {
            return false;
        }
        let expression = line.strip_prefix("//go:build").or_else(|| line.strip_prefix("// +build"));
        if let Some(rest) = expression
            && rest.starts_with(char::is_whitespace)
            && rest.split(|c: char| c.is_whitespace() || c == ',').find(|w| !w.is_empty()) == Some("ignore")
        {
            return true;
        }
    }
    false
}

/// Tokens, each with whether a line break came before it.
fn lex(source: &str) -> Vec<(Token, bool)> {
    let c: Vec<char> = source.chars().collect();
    let mut out = Vec::new();
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
            let start = i;
            i += 2;
            while i < c.len() && !(c[i] == '*' && c.get(i + 1) == Some(&'/')) {
                i += 1;
            }
            i = (i + 2).min(c.len());
            line |= c[start..i].contains(&'\n');
            continue;
        } else if x == '"' {
            let mut j = i + 1;
            let mut text = String::new();
            while j < c.len() && c[j] != '"' && c[j] != '\n' {
                if c[j] == '\\' {
                    if let Some(&n) = c.get(j + 1) {
                        text.push(n);
                    }
                    j += 2;
                } else {
                    text.push(c[j]);
                    j += 1;
                }
            }
            i = if c.get(j) == Some(&'"') { j + 1 } else { j.min(c.len()) };
            Token::Str(text)
        } else if x == '`' {
            let mut j = i + 1;
            while j < c.len() && c[j] != '`' {
                j += 1;
            }
            let text = c[i + 1..j.min(c.len())].iter().collect();
            line |= c[i..j.min(c.len())].contains(&'\n');
            i = (j + 1).min(c.len());
            Token::Str(text)
        } else if x == '\'' {
            let mut j = i + 1;
            while j < c.len() && c[j] != '\'' && c[j] != '\n' {
                j += if c[j] == '\\' { 2 } else { 1 };
            }
            i = (j + 1).min(c.len());
            Token::Value
        } else if ident_start(x) {
            let start = i;
            while i < c.len() && ident_part(c[i]) {
                i += 1;
            }
            Token::Ident(c[start..i].iter().collect())
        } else if x.is_ascii_digit() {
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

/// Reads one file.
pub(super) fn read(source: &str) -> Facts {
    // A byte-order mark is no part of the first line.
    let source = source.strip_prefix('﻿').unwrap_or(source);
    let tokens = lex(source);
    let t: Vec<&Token> = tokens.iter().map(|(t, _)| t).collect();
    let ident = |k: usize| match t.get(k) {
        Some(Token::Ident(x)) => Some(x.as_str()),
        _ => None,
    };
    let text = |k: usize| match t.get(k) {
        Some(Token::Str(x)) => Some(x.as_str()),
        _ => None,
    };
    let punct = |k: usize, p: char| matches!(t.get(k), Some(Token::Punct(x)) if *x == p);
    let mut facts = Facts {
        ignored: build_ignored(source),
        ..Default::default()
    };
    let mut braces = 0usize;
    let mut parens = 0usize;
    let mut k = 0;
    while k < t.len() {
        let top = braces == 0 && parens == 0;
        match t[k] {
            Token::Punct('{') => braces += 1,
            Token::Punct('}') => braces = braces.saturating_sub(1),
            Token::Punct('(') => parens += 1,
            Token::Punct(')') => parens = parens.saturating_sub(1),
            Token::Ident(w) if top && w == "package" && facts.package.is_none() => {
                facts.package = ident(k + 1).map(str::to_owned);
                k += 2;
                continue;
            }
            Token::Ident(w) if top && w == "import" => {
                // import "fmt"; import f "fmt"; import ( … )
                let entry = |j: usize| -> Option<(Import, usize)> {
                    match t.get(j)? {
                        Token::Str(path) => Some((Import { path: path.clone(), alias: None }, j + 1)),
                        Token::Ident(alias) => Some((Import { path: text(j + 1)?.to_owned(), alias: Some(alias.clone()) }, j + 2)),
                        Token::Punct(p @ ('.' | '_')) => Some((Import { path: text(j + 1)?.to_owned(), alias: Some(p.to_string()) }, j + 2)),
                        _ => None,
                    }
                };
                if punct(k + 1, '(') {
                    let mut j = k + 2;
                    while j < t.len() && !punct(j, ')') {
                        match entry(j) {
                            Some((found, next)) => {
                                facts.imports.push(found);
                                j = next;
                            }
                            None => j += 1,
                        }
                    }
                    k = j + 1;
                } else if let Some((found, next)) = entry(k + 1) {
                    facts.imports.push(found);
                    k = next;
                } else {
                    k += 1;
                }
                continue;
            }
            Token::Ident(w) if top && w == "type" => {
                if punct(k + 1, '(') {
                    // type ( A struct{…}; B = int ): each line's first name.
                    let mut j = k + 2;
                    let mut depth = 0usize;
                    while j < t.len() && !(depth == 0 && punct(j, ')')) {
                        match t[j] {
                            Token::Punct('(' | '{' | '[') => depth += 1,
                            Token::Punct(')' | '}' | ']') => depth = depth.saturating_sub(1),
                            Token::Ident(name) if depth == 0 && tokens[j].1 => facts.outline.push(item(OutlineKind::Type, name)),
                            // A field's type names an import too: `Body io.Reader`.
                            Token::Ident(x) if !punct(j - 1, '.') && punct(j + 1, '.') && facts.selectors.len() < SELECTORS => {
                                if let Some(name) = ident(j + 2) {
                                    facts.selectors.push((x.clone(), name.to_owned()));
                                }
                            }
                            _ => {}
                        }
                        j += 1;
                    }
                    k = j + 1;
                    continue;
                }
                if let Some(name) = ident(k + 1) {
                    facts.outline.push(item(OutlineKind::Type, name));
                }
            }
            Token::Ident(w) if top && w == "func" => {
                if punct(k + 1, '(') {
                    // A method: func (r *T[K]) Name(…), starting its line. Elsewhere `func(` is a
                    // function type (`type H func(*Request)`) or a function literal.
                    let mut j = k + 2;
                    let mut names = Vec::new();
                    let mut square = 0usize;
                    while j < t.len() && !punct(j, ')') {
                        match t[j] {
                            Token::Punct('[') => square += 1,
                            Token::Punct(']') => square = square.saturating_sub(1),
                            Token::Ident(x) if square == 0 => names.push(x.clone()),
                            _ => {}
                        }
                        j += 1;
                    }
                    let method = tokens[k].1 && ident(j + 1).is_some() && (punct(j + 2, '(') || punct(j + 2, '['));
                    if method {
                        // `r *T` or `r T` names the receiver first; `T` and `*T` do not.
                        if let Some(receiver) = names.last() {
                            facts.methods.push(receiver.clone());
                        }
                        k = j;
                        continue;
                    }
                } else if let Some(name) = ident(k + 1) {
                    facts.outline.push(item(OutlineKind::Function, name));
                }
            }
            Token::Ident(x) if !(k > 0 && punct(k - 1, '.')) && punct(k + 1, '.') && facts.selectors.len() < SELECTORS => {
                if let Some(name) = ident(k + 2) {
                    facts.selectors.push((x.clone(), name.to_owned()));
                }
            }
            _ => {}
        }
        k += 1;
    }
    facts
}

fn item(kind: OutlineKind, name: &str) -> OutlineItem {
    OutlineItem {
        kind,
        name: name.as_bytes().to_vec(),
        visibility: Vec::new(),
        exported: exported(name),
        of_trait: None,
        functions: 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn package_imports_selectors_and_build_ignore() {
        let facts = read(
            "// Copyright\n\n// Package http serves.\npackage http\n\nimport (\n\t\"bufio\"\n\tnetpkg \"net\"\n\t_ \"embed\"\n\t. \"strings\"\n\t\"math/rand/v2\"\n)\n\nimport \"fmt\"\n\nvar s = \"io.Reader in a string\" // io.Writer in a comment\nfunc Get(url string) (*Response, error) { r := bufio.NewReader(nil); netpkg.Dial(); fmt.Println(r.x.Y); return nil, rand.IntN }\n",
        );
        assert_eq!(facts.package.as_deref(), Some("http"));
        assert!(!facts.ignored);
        let paths: Vec<(&str, Option<&str>)> = facts.imports.iter().map(|i| (i.path.as_str(), i.alias.as_deref())).collect();
        assert_eq!(paths, [("bufio", None), ("net", Some("netpkg")), ("embed", Some("_")), ("strings", Some(".")), ("math/rand/v2", None), ("fmt", None)]);
        let used: Vec<(&str, &str)> = facts.selectors.iter().map(|(a, b)| (a.as_str(), b.as_str())).collect();
        assert_eq!(used, [("bufio", "NewReader"), ("netpkg", "Dial"), ("fmt", "Println"), ("r", "x"), ("rand", "IntN")]);
        assert!(read("//go:build ignore\n\npackage main\n").ignored);
        assert!(read("// +build ignore\n\npackage main\n").ignored);
        assert!(!read("//go:build linux && !ignore\npackage main\n").ignored);
        assert!(!read("package main\n//go:build ignore\n").ignored);
    }

    #[test]
    fn types_functions_and_methods_at_the_top_level() {
        let facts = read(
            "package p\n\ntype Client struct { Timeout int }\ntype (\n\tHeader map[string][]string\n\tcookie struct{ a, b int }\n\tAlias = int\n)\n\nfunc Get() {}\nfunc (c *Client) Do() {}\nfunc (c Client) do() {}\nfunc (Client) Close() {}\nfunc (s *Set[T]) Add(v T) {}\nfunc Map[T any](f func(T) T) {}\nfunc init() { type local struct{}; func() {}() }\ntype HandlerFunc func(ResponseWriter, *Request)\nfunc Serve() {}\nvar f = func(r io.Reader) error { return nil }\ntype (\n\tBody struct{ R io.Reader }\n)\n",
        );
        let rows: Vec<(OutlineKind, String, bool)> = facts.outline.iter().map(|o| (o.kind, String::from_utf8(o.name.clone()).unwrap(), o.exported)).collect();
        assert_eq!(
            rows,
            [
                (OutlineKind::Type, "Client".into(), true),
                (OutlineKind::Type, "Header".into(), true),
                (OutlineKind::Type, "cookie".into(), false),
                (OutlineKind::Type, "Alias".into(), true),
                (OutlineKind::Function, "Get".into(), true),
                (OutlineKind::Function, "Map".into(), true),
                (OutlineKind::Function, "init".into(), false),
                (OutlineKind::Type, "HandlerFunc".into(), true),
                (OutlineKind::Function, "Serve".into(), true),
                (OutlineKind::Type, "Body".into(), true),
            ]
        );
        // A function type and a function literal are no methods.
        assert_eq!(facts.methods, ["Client", "Client", "Client", "Set"]);
        assert_eq!(facts.selectors.iter().filter(|(a, b)| a == "io" && b == "Reader").count(), 2);
    }
}
