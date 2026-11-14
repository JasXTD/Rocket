//! A small lexical reader for Python: `import` and `from … import` lines, also those
//! inside functions, and the classes and functions a file declares at its top level.
//! Comments and strings, triple-quoted ones too, are skipped. Nothing is compiled or
//! run, so modules loaded through importlib or a changed sys.path are not followed.
use super::{OutlineItem, OutlineKind};

#[derive(Clone, Debug, PartialEq, Eq)]
enum Token {
    Name(String),
    Str,
    Op(char),
}
/// One logical line: brackets and backslashes join physical lines into one.
struct Line {
    indent: usize,
    tokens: Vec<Token>,
}

/// What one file imports and declares.
#[derive(Clone, Debug, Default)]
pub(super) struct Facts {
    pub imports: Vec<Import>,
    pub outline: Vec<OutlineItem>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Import {
    /// The dots of a relative import (`from ..core import x` has 2); 0 otherwise.
    pub level: usize,
    /// The module path after the dots: `email.mime` is `["email", "mime"]`.
    pub module: Vec<String>,
    /// `from … import a, b` takes a and b, `*` everything; `import a.b` takes none.
    pub names: Vec<String>,
}

fn name_start(c: char) -> bool {
    c == '_' || c.is_alphabetic()
}
fn name_part(c: char) -> bool {
    c == '_' || c.is_alphanumeric()
}
/// A string literal whose quote is at `c[i]`: its length. Triple-quoted strings run
/// across lines; others end at their line.
fn string(c: &[char], i: usize) -> usize {
    let q = c[i];
    let triple = c.get(i + 1) == Some(&q) && c.get(i + 2) == Some(&q);
    let mut j = if triple { i + 3 } else { i + 1 };
    while j < c.len() {
        match c[j] {
            '\\' => j += 2,
            x if x == q && (!triple || c.get(j + 1) == Some(&q) && c.get(j + 2) == Some(&q)) => return j + (if triple { 3 } else { 1 }) - i,
            '\n' if !triple => return j - i,
            _ => j += 1,
        }
    }
    c.len() - i
}

fn lines(source: &str) -> Vec<Line> {
    let c: Vec<char> = source.chars().collect();
    let mut out = Vec::new();
    let mut tokens = Vec::new();
    let mut indent = 0;
    let mut start = true;
    let mut brackets = 0usize;
    let mut i = 0;
    while i < c.len() {
        if start {
            let mut column = 0;
            while i < c.len() && matches!(c[i], ' ' | '\t' | '\x0c') {
                column = if c[i] == '\t' { column + 8 - column % 8 } else { column + 1 };
                i += 1;
            }
            match c.get(i) {
                None => break,
                Some('\n' | '\r') => {
                    i += 1;
                    continue;
                }
                Some('#') => {
                    while i < c.len() && c[i] != '\n' {
                        i += 1;
                    }
                    continue;
                }
                _ => {}
            }
            indent = column;
            start = false;
        }
        let x = c[i];
        match x {
            '\n' => {
                if brackets == 0 {
                    if !tokens.is_empty() {
                        out.push(Line {
                            indent,
                            tokens: std::mem::take(&mut tokens),
                        });
                    }
                    start = true;
                }
                i += 1;
            }
            // A backslash at the end of a line joins the next one, with \n or \r\n.
            '\\' if c.get(i + 1) == Some(&'\n') => i += 2,
            '\\' if c.get(i + 1) == Some(&'\r') && c.get(i + 2) == Some(&'\n') => i += 3,
            '#' => {
                while i < c.len() && c[i] != '\n' {
                    i += 1;
                }
            }
            '\'' | '"' => {
                i += string(&c, i);
                tokens.push(Token::Str);
            }
            _ if name_start(x) => {
                let begin = i;
                while i < c.len() && name_part(c[i]) {
                    i += 1;
                }
                let word: String = c[begin..i].iter().collect();
                // r"…", b'…', f"…", rb'…' and the like.
                if word.len() <= 2 && word.chars().all(|p| "rRbBuUfF".contains(p)) && matches!(c.get(i), Some('\'' | '"')) {
                    i += string(&c, i);
                    tokens.push(Token::Str);
                } else {
                    tokens.push(Token::Name(word));
                }
            }
            _ if x.is_ascii_digit() => {
                while i < c.len() && (name_part(c[i]) || c[i] == '.') {
                    i += 1;
                }
                tokens.push(Token::Op('0'));
            }
            '(' | '[' | '{' => {
                brackets += 1;
                tokens.push(Token::Op(x));
                i += 1;
            }
            ')' | ']' | '}' => {
                brackets = brackets.saturating_sub(1);
                tokens.push(Token::Op(x));
                i += 1;
            }
            _ if x.is_whitespace() => i += 1,
            _ => {
                tokens.push(Token::Op(x));
                i += 1;
            }
        }
    }
    if !tokens.is_empty() {
        out.push(Line { indent, tokens });
    }
    out
}

/// Reads one file.
pub(super) fn read(source: &str) -> Facts {
    // A byte-order mark is no part of the first line.
    let source = source.strip_prefix('﻿').unwrap_or(source);
    let lines = lines(source);
    let mut facts = Facts::default();
    let mut class: Option<(usize, Option<usize>)> = None;
    for line in &lines {
        // Statements a `;` joins on one line, and the one after a `:` (`if x: import y`).
        for statement in line.tokens.split(|t| *t == Token::Op(';')) {
            let statement = match statement.iter().position(|t| matches!(t, Token::Name(w) if w == "import" || w == "from")) {
                Some(0) => statement,
                Some(at) if statement[at - 1] == Token::Op(':') => &statement[at..],
                _ => continue,
            };
            facts.imports.extend(imports(statement));
        }
        // The class whose body this line is in counts its functions.
        if line.indent == 0 {
            class = None;
        } else if let Some((at, body)) = class.as_mut() {
            let indent = *body.get_or_insert(line.indent);
            let def = match line.tokens.as_slice() {
                [Token::Name(d), ..] if d == "def" => true,
                [Token::Name(a), Token::Name(d), ..] if a == "async" && d == "def" => true,
                _ => false,
            };
            if def && line.indent == indent {
                facts.outline[*at].functions += 1;
            }
            continue;
        }
        // Only the file's own top level: a function inside a function or an `if` is not listed.
        if line.indent != 0 {
            continue;
        }
        let (kind, name) = match line.tokens.as_slice() {
            [Token::Name(k), Token::Name(n), ..] if k == "class" => (OutlineKind::Class, n),
            [Token::Name(k), Token::Name(n), ..] if k == "def" => (OutlineKind::Function, n),
            [Token::Name(a), Token::Name(k), Token::Name(n), ..] if a == "async" && k == "def" => (OutlineKind::Function, n),
            _ => continue,
        };
        if facts.outline.len() >= 4096 {
            continue;
        }
        if kind == OutlineKind::Class {
            class = Some((facts.outline.len(), None));
        }
        facts.outline.push(OutlineItem {
            kind,
            name: name.as_bytes().to_vec(),
            visibility: Vec::new(),
            exported: !name.starts_with('_'),
            of_trait: None,
            functions: 0,
        });
    }
    facts
}

/// One `import …` or `from … import …` statement.
fn imports(t: &[Token]) -> Vec<Import> {
    let name = |k: usize| match t.get(k) {
        Some(Token::Name(x)) => Some(x.as_str()),
        _ => None,
    };
    let dotted = |mut k: usize| -> (Vec<String>, usize) {
        let mut path = Vec::new();
        while let Some(part) = name(k) {
            if part == "import" || part == "as" {
                break;
            }
            path.push(part.to_owned());
            k += 1;
            if t.get(k) != Some(&Token::Op('.')) {
                break;
            }
            k += 1;
        }
        (path, k)
    };
    let mut out = Vec::new();
    match name(0) {
        Some("import") => {
            let mut k = 1;
            while k < t.len() {
                let (module, end) = dotted(k);
                if module.is_empty() {
                    break;
                }
                out.push(Import { level: 0, module, names: Vec::new() });
                k = end;
                if name(k) == Some("as") {
                    k += 2;
                }
                if t.get(k) != Some(&Token::Op(',')) {
                    break;
                }
                k += 1;
            }
        }
        Some("from") => {
            let mut k = 1;
            let mut level = 0;
            while t.get(k) == Some(&Token::Op('.')) {
                level += 1;
                k += 1;
            }
            let (module, end) = dotted(k);
            k = end;
            if name(k) != Some("import") || level == 0 && module.is_empty() {
                return out;
            }
            k += 1;
            let mut names = Vec::new();
            while k < t.len() {
                match &t[k] {
                    Token::Op('*') => names.push("*".to_owned()),
                    Token::Name(w) if w != "as" && !matches!(t.get(k.wrapping_sub(1)), Some(Token::Name(p)) if p == "as") => names.push(w.clone()),
                    _ => {}
                }
                k += 1;
            }
            out.push(Import { level, module, names });
        }
        _ => {}
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn found(source: &str) -> Vec<(usize, String, Vec<String>)> {
        read(source).imports.into_iter().map(|i| (i.level, i.module.join("."), i.names)).collect()
    }
    fn row(level: usize, module: &str, names: &[&str]) -> (usize, String, Vec<String>) {
        (level, module.to_owned(), names.iter().map(|n| n.to_string()).collect())
    }

    #[test]
    fn imports_absolute_relative_and_inside_functions() {
        let found = found(
            "\"\"\"A docstring: import fake\nfrom fake import nothing\n\"\"\"\nimport re, email.utils as u\nfrom email import errors, charset as _charset\nfrom . import base64mime\nfrom ..core.parts import (\n    Part,\n    Other as O,\n)\nfrom .mime import *\nimport x; import y\ndef f():\n    from email.generator import Generator  # import inside a function\n    s = 'import not_this'\n    return r\"from raw import nope\"\nif TYPE_CHECKING: import typing_only\nfrom __future__ import annotations\n",
        );
        assert_eq!(
            found,
            [
                row(0, "re", &[]),
                row(0, "email.utils", &[]),
                row(0, "email", &["errors", "charset"]),
                row(1, "", &["base64mime"]),
                row(2, "core.parts", &["Part", "Other"]),
                row(1, "mime", &["*"]),
                row(0, "x", &[]),
                row(0, "y", &[]),
                row(0, "email.generator", &["Generator"]),
                row(0, "typing_only", &[]),
                row(0, "__future__", &["annotations"]),
            ]
        );
    }

    #[test]
    fn the_outline_counts_a_class_functions_and_marks_private_names() {
        let facts = read(
            "import os\n\n@decorator\nclass Message(Base):\n    \"\"\"Doc.\n    def not_a_method(self): pass\n    \"\"\"\n    x = 1\n    def get(self):\n        def inner(): pass\n        return 1\n\n    async def fetch(self): ...\n    @property\n    def size(self): return 2\n\ndef _split(value):\n    pass\n\nasync def run():\n    pass\nclass _Hidden: pass\n",
        );
        let rows: Vec<(OutlineKind, String, bool, usize)> = facts.outline.iter().map(|o| (o.kind, String::from_utf8(o.name.clone()).unwrap(), o.exported, o.functions)).collect();
        assert_eq!(
            rows,
            [
                (OutlineKind::Class, "Message".into(), true, 3),
                (OutlineKind::Function, "_split".into(), false, 0),
                (OutlineKind::Function, "run".into(), true, 0),
                (OutlineKind::Class, "_Hidden".into(), false, 0),
            ]
        );
    }
}
