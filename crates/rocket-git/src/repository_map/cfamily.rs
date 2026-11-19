//! A small lexical reader for C and C++: the `#include` lines, the names a file uses, and
//! what it declares at its top level (also inside namespaces and `extern "C"`):
//! functions, types, `typedef`s, `using` aliases and macros, each class with its member
//! functions counted. Comments, strings, characters and raw strings are skipped. The
//! preprocessor is not run: `#include` lines are read in every branch but `#if 0`, and
//! declarations only in an `#if`'s first branch, so that braces split across branches
//! count once.
use super::{OutlineItem, OutlineKind};
use std::collections::{BTreeSet, HashMap};

#[derive(Clone, Debug, PartialEq, Eq)]
enum Token {
    Ident(String),
    /// A string, a character or a number.
    Value,
    Punct(char),
}

/// What one file includes and declares.
#[derive(Clone, Debug, Default)]
pub(super) struct Facts {
    pub includes: Vec<Include>,
    /// What the file declares at its top level, in order, each name once.
    pub outline: Vec<OutlineItem>,
    /// The names the file uses, to say what an include brings in.
    pub names: BTreeSet<String>,
    /// Enumerators the file declares: other files use them by name too.
    pub constants: BTreeSet<String>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Include {
    /// As written, with `\` read as `/`.
    pub path: String,
    /// `#include <…>` rather than `"…"`.
    pub angle: bool,
}

/// Outline items and names kept per file.
const OUTLINE: usize = 4096;
const NAMES: usize = 100_000;
/// How far a declaration's words are looked through, and a parameter list: past these a
/// run of tokens is no declaration, and an unfinished one costs no more.
const LOOK: usize = 256;
const PARAMETERS: usize = 4096;

/// Words that are no name a declaration declares, nor a type before one.
const KEYWORDS: &[&str] = &[
    "if", "else", "for", "while", "do", "switch", "case", "default", "return", "goto", "break", "continue", "sizeof", "alignof",
    "_Alignof", "decltype", "typeof", "__typeof__", "static_assert", "_Static_assert", "defined", "__attribute__", "__declspec",
    "alignas", "_Alignas", "noexcept", "throw", "new", "delete", "operator", "co_return", "co_await", "co_yield", "requires",
    "asm", "__asm__", "typedef", "using", "namespace", "template", "typename", "public", "private", "protected",
];
/// Words that stand before a declaration's name and are none: `int (*fp)(void)` declares fp.
const TYPE_WORDS: &[&str] = &[
    "int", "char", "void", "short", "long", "float", "double", "signed", "unsigned", "bool", "_Bool", "auto", "const", "volatile",
    "extern", "static", "inline", "__inline", "__forceinline", "register", "restrict", "__restrict", "constexpr", "consteval",
    "constinit", "virtual", "explicit", "friend", "mutable", "thread_local", "_Thread_local", "struct", "union", "enum", "class",
];
/// A word a declaration can name.
fn nameable(word: &str) -> bool {
    !KEYWORDS.contains(&word) && !TYPE_WORDS.contains(&word)
}

/// The file names whose files are headers: what they declare is seen by every file
/// that includes them.
pub(super) fn header(path: &[u8]) -> bool {
    let name = path.rsplit(|b| *b == b'/').next().unwrap_or_default();
    let ext = name.rsplit(|b| *b == b'.').next().unwrap_or_default().to_ascii_lowercase();
    matches!(ext.as_slice(), b"h" | b"hh" | b"hpp" | b"hxx" | b"h++" | b"inl" | b"ipp" | b"tpp")
}

/// One `#if` … `#endif`: whether the branch being read is followed for declarations,
/// and whether it can never be compiled (`#if 0`, or after an `#if 1`).
#[derive(Clone, Copy)]
struct Branch {
    taken: bool,
    dead: bool,
    /// An earlier branch was followed.
    followed: bool,
    /// An earlier branch was always compiled: the rest never are.
    always: bool,
}
/// `#if 0` and `#if 1`, and their `false` and `true`; None when it depends.
fn constant(condition: &str) -> Option<bool> {
    let words: Vec<&str> = condition.split_whitespace().collect();
    match words.as_slice() {
        ["0"] | ["false"] | ["(0)"] => Some(false),
        ["1"] | ["true"] | ["(1)"] => Some(true),
        _ => None,
    }
}

#[derive(Default)]
struct Lexed {
    tokens: Vec<Token>,
    includes: Vec<Include>,
    macros: Vec<String>,
}

/// A directive starting after its `#` at `c[i]`: its text, with `\` joining lines and
/// comments left out, and where the next line starts.
fn directive(c: &[char], mut i: usize) -> (String, usize) {
    let mut text = String::new();
    while i < c.len() {
        match c[i] {
            '\\' if c.get(i + 1) == Some(&'\n') => i += 2,
            '\\' if c.get(i + 1) == Some(&'\r') && c.get(i + 2) == Some(&'\n') => i += 3,
            '\n' => return (text, i + 1),
            '/' if c.get(i + 1) == Some(&'/') => {
                while i < c.len() && c[i] != '\n' {
                    i += 1;
                }
            }
            '/' if c.get(i + 1) == Some(&'*') => {
                i += 2;
                while i < c.len() && !(c[i] == '*' && c.get(i + 1) == Some(&'/')) {
                    i += 1;
                }
                i = (i + 2).min(c.len());
                text.push(' ');
            }
            x => {
                text.push(x);
                i += 1;
            }
        }
    }
    (text, i)
}

fn ident_start(c: char) -> bool {
    c == '_' || c.is_alphabetic()
}
fn ident_part(c: char) -> bool {
    c == '_' || c.is_alphanumeric()
}

fn lex(source: &str) -> Lexed {
    let c: Vec<char> = source.chars().collect();
    let mut out = Lexed::default();
    let mut branches: Vec<Branch> = Vec::new();
    // Only spaces and comments since the line began: a `#` there starts a directive.
    let mut start = true;
    // The name of the `#ifndef` just before, for an include guard's `#define`: only one
    // that opens the file is a guard; `#ifndef BUFSIZE` later on sets a default.
    let mut guard: Option<String> = None;
    let mut opened = false;
    let mut i = 0;
    while i < c.len() {
        let x = c[i];
        if x == '\n' {
            start = true;
            i += 1;
            continue;
        }
        if x.is_whitespace() {
            i += 1;
            continue;
        }
        if x == '/' && c.get(i + 1) == Some(&'/') {
            while i < c.len() && c[i] != '\n' {
                i += 1;
            }
            continue;
        }
        if x == '/' && c.get(i + 1) == Some(&'*') {
            i += 2;
            while i < c.len() && !(c[i] == '*' && c.get(i + 1) == Some(&'/')) {
                i += 1;
            }
            i = (i + 2).min(c.len());
            continue;
        }
        if x == '#' && start {
            let (text, next) = directive(&c, i + 1);
            i = next;
            let text = text.trim();
            let name_end = text.find(|ch: char| !ident_part(ch)).unwrap_or(text.len());
            let (name, rest) = (&text[..name_end], text[name_end..].trim());
            let alive = branches.iter().all(|b| !b.dead);
            let last_guard = guard.take();
            match name {
                "include" | "include_next" | "import" if alive => {
                    let rest = rest.trim();
                    let (open, close) = match rest.chars().next() {
                        Some('"') => ('"', '"'),
                        Some('<') => ('<', '>'),
                        _ => continue,
                    };
                    if let Some(end) = rest[1..].find(close) {
                        let path = rest[1..1 + end].replace('\\', "/");
                        if !path.is_empty() {
                            out.includes.push(Include { path, angle: open == '<' });
                        }
                    }
                }
                "define" if alive => {
                    let macro_name: String = rest.chars().take_while(|ch| ident_part(*ch)).collect();
                    let body = rest[macro_name.len()..].trim();
                    // `#ifndef FOO_H` then `#define FOO_H` (or `1`): an include guard, no macro of the file's.
                    let guarded = last_guard.as_deref() == Some(macro_name.as_str()) && (body.is_empty() || body == "1");
                    if !macro_name.is_empty() && !guarded {
                        out.macros.push(macro_name);
                    }
                }
                "if" | "ifdef" | "ifndef" => {
                    let value = if name == "if" { constant(rest) } else { None };
                    branches.push(Branch {
                        taken: value != Some(false),
                        dead: value == Some(false),
                        followed: value != Some(false),
                        always: value == Some(true),
                    });
                    if name == "ifndef" && !opened {
                        guard = Some(rest.chars().take_while(|ch| ident_part(*ch)).collect());
                    }
                }
                "elif" | "elifdef" | "elifndef" | "else" => {
                    if let Some(b) = branches.last_mut() {
                        let value = if name == "elif" { constant(rest) } else { None };
                        b.dead = b.always || value == Some(false);
                        b.taken = !b.followed && !b.dead;
                        b.followed |= b.taken;
                        b.always |= !b.dead && (name == "else" || value == Some(true));
                    }
                }
                "endif" => {
                    branches.pop();
                }
                _ => {}
            }
            start = true;
            opened = true;
            continue;
        }
        start = false;
        opened = true;
        let reading = branches.iter().all(|b| b.taken);
        let token = if x == '"' {
            i = string_end(&c, i);
            Token::Value
        } else if x == '\'' {
            let mut j = i + 1;
            while j < c.len() && c[j] != '\'' && c[j] != '\n' {
                j += if c[j] == '\\' { 2 } else { 1 };
            }
            i = if c.get(j) == Some(&'\'') { j + 1 } else { j.min(c.len()) };
            Token::Value
        } else if ident_start(x) {
            let begin = i;
            while i < c.len() && ident_part(c[i]) {
                i += 1;
            }
            let word: String = c[begin..i].iter().collect();
            // L"…", u8"…", R"(…)", u8R"x(…)x" and the like: the word is the literal's prefix.
            let quoted = matches!(c.get(i), Some('"' | '\''));
            if quoted && matches!(word.as_str(), "L" | "u" | "U" | "u8") {
                continue;
            }
            if c.get(i) == Some(&'"') && matches!(word.as_str(), "R" | "LR" | "uR" | "UR" | "u8R") {
                i = raw_end(&c, i);
                Token::Value
            } else {
                Token::Ident(word)
            }
        } else if x.is_ascii_digit() || x == '.' && c.get(i + 1).is_some_and(char::is_ascii_digit) {
            i += 1;
            while i < c.len() {
                let y = c[i];
                let exponent = matches!(y, '+' | '-') && matches!(c[i - 1], 'e' | 'E' | 'p' | 'P');
                // 1'000'000: a digit separator, not a character.
                let separator = y == '\'' && c.get(i + 1).is_some_and(|n| n.is_ascii_alphanumeric());
                if y.is_alphanumeric() || y == '_' || y == '.' || exponent || separator {
                    i += 1;
                } else {
                    break;
                }
            }
            Token::Value
        } else {
            i += 1;
            Token::Punct(x)
        };
        if reading {
            out.tokens.push(token);
        }
    }
    out
}

/// After the string whose quote is at `c[i]`, or at the line break that cuts it short.
fn string_end(c: &[char], i: usize) -> usize {
    let mut j = i + 1;
    while j < c.len() && c[j] != '"' && c[j] != '\n' {
        j += if c[j] == '\\' { 2 } else { 1 };
    }
    if c.get(j) == Some(&'"') { j + 1 } else { j.min(c.len()) }
}
/// After a raw string whose quote is at `c[i]`: `"delim( … )delim"`. C++ allows a
/// delimiter of up to 16 characters, none a space, parenthesis or backslash; anything
/// else (C's `R "text"` after a macro named R too), and a raw string that never closes,
/// is read as a plain string, which ends at its line.
fn raw_end(c: &[char], i: usize) -> usize {
    let mut j = i + 1;
    let mut delimiter = String::new();
    while j < c.len() && c[j] != '(' {
        if delimiter.chars().count() >= 16 || matches!(c[j], '"' | ')' | '\\') || c[j].is_whitespace() || c[j].is_control() {
            return string_end(c, i);
        }
        delimiter.push(c[j]);
        j += 1;
    }
    let close: Vec<char> = format!("){delimiter}\"").chars().collect();
    while j < c.len() {
        if c[j..].starts_with(&close) {
            return j + close.len();
        }
        j += 1;
    }
    string_end(c, i)
}

/// A scope a `{` opens.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Scope {
    /// A namespace or `extern "C"`: what is inside is at the top level; anonymous when
    /// its names are the file's own.
    Open { anonymous: bool },
    /// A class, struct or union declared at the top level, by outline index and name.
    Type(Option<usize>, String),
    Enum,
    /// A function's body, an initializer or anything else.
    Other,
}

/// Reads one file; `header` for a `.h`, `.hpp` and the like.
pub(super) fn read(source: &str, header: bool) -> Facts {
    let source = source.strip_prefix('\u{feff}').unwrap_or(source);
    let lexed = lex(source);
    let t = &lexed.tokens;
    let mut facts = Facts {
        includes: lexed.includes,
        ..Default::default()
    };
    for token in t {
        if let Token::Ident(x) = token
            && facts.names.len() < NAMES
            && !facts.names.contains(x.as_str())
        {
            facts.names.insert(x.clone());
        }
    }
    let ident = |k: usize| match t.get(k) {
        Some(Token::Ident(x)) => Some(x.as_str()),
        _ => None,
    };
    let punct = |k: usize, p: char| matches!(t.get(k), Some(Token::Punct(x)) if *x == p);
    // Each name once: a prototype and its definition are one function.
    let mut seen: HashMap<(OutlineKind, String), usize> = HashMap::new();
    let mut add = |facts: &mut Facts, kind: OutlineKind, name: &str, file_local: bool, exported: bool| -> Option<usize> {
        if let Some(&at) = seen.get(&(kind, name.to_owned())) {
            let item: &mut OutlineItem = &mut facts.outline[at];
            if file_local {
                item.visibility = b"static".to_vec();
                item.exported = false;
            }
            return Some(at);
        }
        if facts.outline.len() >= OUTLINE || name.is_empty() || !nameable(name) {
            return None;
        }
        facts.outline.push(OutlineItem {
            kind,
            name: name.as_bytes().to_vec(),
            visibility: if file_local { b"static".to_vec() } else { Vec::new() },
            exported: exported && !file_local,
            of_trait: None,
            functions: 0,
        });
        seen.insert((kind, name.to_owned()), facts.outline.len() - 1);
        Some(facts.outline.len() - 1)
    };
    for m in &lexed.macros {
        add(&mut facts, OutlineKind::Macro, m, false, header);
    }
    let mut scopes: Vec<Scope> = Vec::new();
    // Open scopes that are no namespace or `extern "C"`, and anonymous namespaces, counted
    // as they open and close: a file of many braces costs no more per token.
    let mut closed_in = 0usize;
    let mut anonymous_in = 0usize;
    // The statement being read at the top level.
    let mut statement_static = false;
    // `extern int counter;`: a variable other files use, unless the statement declares a function.
    let mut statement_extern = false;
    let mut statement_function = false;
    let mut typedef = false;
    let mut last_name: Option<String> = None;
    // `typedef int (*callback)(void *)`: the name inside the first parentheses.
    let mut pointer_name: Option<String> = None;
    // What the next `{` opens, when a header just read says.
    let mut next_scope: Option<Scope> = None;
    let mut parens = 0usize;
    let mut squares = 0usize;
    let mut k = 0;
    while k < t.len() {
        let top = closed_in == 0;
        let anonymous = anonymous_in > 0;
        // Directly in a class declared at the top level.
        let class = closed_in == 1 && matches!(scopes.last(), Some(Scope::Type(..)));
        match &t[k] {
            Token::Punct('{') => {
                let scope = next_scope.take().unwrap_or(Scope::Other);
                match scope {
                    Scope::Open { anonymous: true } => anonymous_in += 1,
                    Scope::Open { anonymous: false } => {}
                    _ => closed_in += 1,
                }
                scopes.push(scope);
                parens = 0;
                squares = 0;
            }
            Token::Punct('}') => {
                let closed = scopes.pop();
                match closed {
                    Some(Scope::Open { anonymous: true }) => anonymous_in -= 1,
                    Some(Scope::Open { anonymous: false }) | None => {}
                    Some(_) => closed_in -= 1,
                }
                // A function's body ends its statement; a type's goes on to its `;`.
                if matches!(closed, Some(Scope::Other)) && closed_in == 0 && !typedef {
                    statement_static = false;
                    statement_extern = false;
                    statement_function = false;
                    last_name = None;
                }
                if matches!(closed, Some(Scope::Open { .. })) {
                    statement_static = false;
                    statement_extern = false;
                    statement_function = false;
                    typedef = false;
                    last_name = None;
                }
            }
            Token::Punct('(') => {
                // `typedef int (*cb)(void *)`, `typedef BOOL (WINAPI *LPFN_X)(HANDLE)`,
                // `typedef GLuint (APIENTRYP PFNGLX)(void)`: the last name inside the first
                // parentheses, when parameters follow them.
                if top && parens == 0 && pointer_name.is_none() {
                    pointer_name = pointer_in(t, k);
                }
                parens += 1;
            }
            Token::Punct(')') => parens = parens.saturating_sub(1),
            Token::Punct('[') => squares += 1,
            Token::Punct(']') => squares = squares.saturating_sub(1),
            Token::Punct(';') if top => {
                if typedef && let Some(name) = pointer_name.take().or(last_name.take()) {
                    add(&mut facts, OutlineKind::Type, &name, false, header && !anonymous);
                } else if statement_extern
                    && (pointer_name.is_some() || !statement_function)
                    && let Some(name) = pointer_name.take().or(last_name.take())
                {
                    add(&mut facts, OutlineKind::Var, &name, false, !anonymous);
                }
                statement_static = false;
                statement_extern = false;
                statement_function = false;
                typedef = false;
                last_name = None;
                pointer_name = None;
                next_scope = None;
                parens = 0;
                squares = 0;
            }
            Token::Punct(';') => next_scope = None,
            Token::Ident(w) if top && parens == 0 => match w.as_str() {
                "static" => statement_static = true,
                "typedef" => typedef = true,
                "namespace" => {
                    // namespace a::b { … }, namespace { … }; an alias (`namespace fs = …;`) opens nothing.
                    let mut j = k + 1;
                    let mut named = false;
                    while ident(j).is_some() || punct(j, ':') {
                        named |= ident(j).is_some();
                        j += 1;
                    }
                    if punct(j, '{') {
                        next_scope = Some(Scope::Open { anonymous: !named });
                        k = j;
                        continue;
                    }
                }
                "extern" if !matches!(t.get(k + 1), Some(Token::Value)) || !punct(k + 2, '{') => statement_extern = true,
                "extern" => {
                    next_scope = Some(Scope::Open { anonymous: false });
                    k += 2;
                    continue;
                }
                "template" if punct(k + 1, '<') => {
                    // Its parameters: what follows is the declaration.
                    let mut depth = 0usize;
                    let mut j = k + 1;
                    let limit = (j + LOOK).min(t.len());
                    while j < limit {
                        match &t[j] {
                            Token::Punct('<') => depth += 1,
                            Token::Punct('>') => {
                                depth -= 1;
                                if depth == 0 {
                                    break;
                                }
                            }
                            Token::Punct('{' | ';') => break,
                            _ => {}
                        }
                        j += 1;
                    }
                    k = j + 1;
                    continue;
                }
                "using" => {
                    // using Name = type; (not `using namespace std;` nor `using std::x;`)
                    if let Some(name) = ident(k + 1)
                        && punct(k + 2, '=')
                    {
                        add(&mut facts, OutlineKind::Type, name, false, header && !anonymous);
                    }
                }
                "struct" | "class" | "union" | "enum" => {
                    let kind = match w.as_str() {
                        "struct" => OutlineKind::Struct,
                        "class" => OutlineKind::Class,
                        "union" => OutlineKind::Union,
                        _ => OutlineKind::Enum,
                    };
                    // struct [attributes] [EXPORT] Name [final] [: bases] {: the name is the
                    // last word before the body or the bases.
                    let mut j = k + 1;
                    if kind == OutlineKind::Enum && matches!(ident(j), Some("class" | "struct")) {
                        j += 1;
                    }
                    let mut name: Option<String> = None;
                    loop {
                        match t.get(j) {
                            Some(Token::Ident(x)) if x == "final" => {}
                            Some(Token::Ident(x)) if x == "__attribute__" || x == "__declspec" || x == "alignas" => {
                                j = skip_group(t, j + 1);
                                continue;
                            }
                            Some(Token::Ident(x)) => name = Some(x.clone()),
                            Some(Token::Punct('[')) if punct(j + 1, '[') => {
                                j = skip_brackets(t, j);
                                continue;
                            }
                            // A qualified name: struct a::B { … }.
                            Some(Token::Punct(':')) if punct(j + 1, ':') => j += 1,
                            _ => break,
                        }
                        j += 1;
                    }
                    let mut body = j;
                    if punct(j, ':') {
                        // The bases, then the body.
                        let limit = (body + LOOK).min(t.len());
                        while body < limit && !punct(body, '{') && !punct(body, ';') {
                            body += 1;
                        }
                    }
                    if punct(body, '{') {
                        let at = name.as_ref().and_then(|n| add(&mut facts, kind, n, false, header && !anonymous));
                        next_scope = Some(match kind {
                            OutlineKind::Enum => Scope::Enum,
                            _ => Scope::Type(at, name.clone().unwrap_or_default()),
                        });
                        if let Some(n) = name {
                            last_name = Some(n);
                        }
                        k = body;
                        continue;
                    }
                }
                _ if punct(k + 1, '(') && nameable(w) && !shouting(w) => {
                    // A definition's body is read on as any `{`; a method defined outside its
                    // class (`int Cache::get() {`) is counted where the class declares it.
                    let qualified = k >= 2 && punct(k - 1, ':') && punct(k - 2, ':');
                    if !qualified && !typedef && typed_before(t, k) && function_end(t, k).is_some() {
                        add(&mut facts, OutlineKind::Function, w, statement_static, !anonymous);
                        statement_function = true;
                    }
                    last_name = Some(w.clone());
                }
                _ => {
                    if squares == 0 {
                        last_name = Some(w.clone());
                    }
                }
            },
            Token::Ident(w) if class && parens == 0 && punct(k + 1, '(') && nameable(w) && !shouting(w) => {
                // A member function: `Type name(…)` then a body, `;`, `const`, `override` and
                // the like; the constructor and destructor by the class's name.
                let Some(Scope::Type(at, name)) = scopes.last().cloned() else { unreachable!("a class scope") };
                let constructor = *w == name && !(k > 0 && punct(k - 1, '.'));
                if (typed_before(t, k) || constructor)
                    && function_end(t, k).is_some()
                    && let Some(at) = at
                {
                    facts.outline[at].functions += 1;
                }
            }
            Token::Ident(w) if matches!(scopes.last(), Some(Scope::Enum)) && (punct(k - 1, '{') || punct(k - 1, ',')) && (punct(k + 1, ',') || punct(k + 1, '=') || punct(k + 1, '}')) => {
                facts.constants.insert(w.clone());
            }
            Token::Ident(w) if class && (w == "struct" || w == "class" || w == "union" || w == "enum") => {
                // A type inside a class: its members are not the class's.
                let mut j = k + 1;
                while ident(j).is_some() || punct(j, ':') {
                    j += 1;
                }
                if punct(j, '{') {
                    next_scope = Some(Scope::Other);
                }
            }
            _ => {}
        }
        k += 1;
    }
    facts
}

/// Whether the word before the name at `k` makes it a declaration: a type, a pointer or
/// reference, a template's `>`, `~` of a destructor, or a macro that wraps the return
/// type (`GIT_EXTERN(int) git_open(…)`).
fn typed_before(t: &[Token], k: usize) -> bool {
    let Some(before) = k.checked_sub(1).map(|p| &t[p]) else { return false };
    match before {
        Token::Ident(p) => !KEYWORDS.contains(&p.as_str()),
        Token::Punct('*' | '&' | '>' | '~') => true,
        Token::Punct(')') => {
            // The `(` that opened it, and the macro before that. A type holds no value and
            // no comma: `OP(ADD, 1) OP(SUB, 2)` is a table of macro calls.
            let mut depth = 0usize;
            let mut j = k - 1;
            for _ in 0..16 {
                match &t[j] {
                    Token::Punct(')') => depth += 1,
                    Token::Punct('(') => {
                        depth -= 1;
                        if depth == 0 {
                            return j > 0 && matches!(&t[j - 1], Token::Ident(m) if shouting(m));
                        }
                    }
                    Token::Value | Token::Punct(',') => return false,
                    _ => {}
                }
                if j == 0 {
                    return false;
                }
                j -= 1;
            }
            false
        }
        _ => false,
    }
}

/// For a name at `k` followed by its parameters: the token that ends the declaration
/// after them, a body's `{` or a `;` (`= 0`, `= default` and `= delete` too), when what
/// follows the parameters is what a function's declaration can have. Parameters never
/// start with a value: `std::string name("x");` is a variable.
fn function_end(t: &[Token], k: usize) -> Option<usize> {
    if matches!(t.get(k + 2), Some(Token::Value)) {
        return None;
    }
    let mut depth = 0usize;
    let mut j = k + 1;
    let limit = (j + PARAMETERS).min(t.len());
    loop {
        if j >= limit {
            return None;
        }
        match &t[j] {
            Token::Punct('(') => depth += 1,
            Token::Punct(')') => {
                depth -= 1;
                if depth == 0 {
                    break;
                }
            }
            Token::Punct('{' | ';') => return None,
            _ => {}
        }
        j += 1;
    }
    j += 1;
    let limit = (j + LOOK).min(t.len());
    while j < limit {
        match &t[j] {
            Token::Punct('{' | ';') => return Some(j),
            Token::Punct('=') => return Some(j),
            // A constructor's initializers, then its body.
            Token::Punct(':') if !matches!(t.get(j + 1), Some(Token::Punct(':'))) => {
                let limit = (j + PARAMETERS).min(t.len());
                while j < limit && !matches!(t[j], Token::Punct('{' | ';')) {
                    j += 1;
                }
                return (j < limit).then_some(j);
            }
            Token::Punct('(') => j = skip_group(t, j),
            Token::Punct('[') if matches!(t.get(j + 1), Some(Token::Punct('['))) => j = skip_brackets(t, j),
            // const, noexcept, override, final, macros such as __THROW; `-> int`, `&`, `&&`.
            Token::Ident(_) | Token::Punct('-' | '>' | '&' | '*' | ':' | '<' | ',') => j += 1,
            _ => return None,
        }
    }
    None
}

/// After the parenthesized group at `t[k]` (its `(`), or `k` when there is none.
fn skip_group(t: &[Token], k: usize) -> usize {
    if !matches!(t.get(k), Some(Token::Punct('('))) {
        return k;
    }
    let mut depth = 0usize;
    let mut j = k;
    let limit = (k + PARAMETERS).min(t.len());
    while j < limit {
        match &t[j] {
            Token::Punct('(') => depth += 1,
            Token::Punct(')') => {
                depth -= 1;
                if depth == 0 {
                    return j + 1;
                }
            }
            _ => {}
        }
        j += 1;
    }
    t.len()
}
/// After `[[ … ]]` at `t[k]`.
fn skip_brackets(t: &[Token], k: usize) -> usize {
    let mut depth = 0usize;
    let mut j = k;
    let limit = (k + LOOK).min(t.len());
    while j < limit {
        match &t[j] {
            Token::Punct('[') => depth += 1,
            Token::Punct(']') => {
                depth -= 1;
                if depth == 0 {
                    return j + 1;
                }
            }
            _ => {}
        }
        j += 1;
    }
    t.len()
}

/// A name in capitals, digits and underscores: a macro, not a function.
fn shouting(word: &str) -> bool {
    word.len() > 1 && word.chars().any(|c| c.is_ascii_uppercase()) && word.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
}

/// For the `(` at `k`: the last name inside the parentheses it opens, when another `(`
/// (the parameters) follows them: a function pointer's name.
fn pointer_in(t: &[Token], k: usize) -> Option<String> {
    let mut depth = 0usize;
    let mut last = None;
    for (j, token) in t.iter().enumerate().skip(k).take(LOOK) {
        match token {
            Token::Punct('(') => depth += 1,
            Token::Punct(')') => {
                depth -= 1;
                if depth == 0 {
                    return last.filter(|_| matches!(t.get(j + 1), Some(Token::Punct('('))));
                }
            }
            Token::Ident(name) if depth == 1 => last = Some(name.clone()),
            Token::Punct('{' | ';') => return None,
            _ => {}
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows(facts: &Facts) -> Vec<(OutlineKind, String, String, bool, usize)> {
        facts
            .outline
            .iter()
            .map(|o| (o.kind, String::from_utf8(o.name.clone()).unwrap(), String::from_utf8(o.visibility.clone()).unwrap(), o.exported, o.functions))
            .collect()
    }

    #[test]
    fn includes_in_every_branch_but_if_0_and_a_header_declarations() {
        let facts = read(
            r#"/* #include "commented.h" */
#ifndef GIT_REPOSITORY_H
#define GIT_REPOSITORY_H
#include "common.h"
# include <git2/types.h>
#include "sub\win32.h"
#if 0
#include "dead.h"
#define DEAD 1
#else
#include "alive.h"
#endif
#ifdef _WIN32
#include <windows.h>
#else
#include <unistd.h>
#endif
#define GIT_OK 0
#define GIT_MAX(a, b) \
    ((a) > (b) ? (a) : (b))
typedef struct git_repository git_repository;
typedef int (*git_cb)(const char *path, void *payload);
typedef struct { int x; int y; } git_point;
typedef enum { GIT_A = 1, GIT_B } git_kind;
struct git_config { int level; };
enum git_mode { GIT_MODE_READ, GIT_MODE_WRITE = 2 };
GIT_EXTERN(int) git_repository_open(git_repository **out, const char *path);
extern int git_libgit2_init(void);
static inline int git_inline(void) { return GIT_OK; }
const char *git_path(const git_repository *repo);
int git_counter = 0;
extern uint32_t OPENSSL_ia32cap_P[4];
extern int (*git_hook)(void *payload);
extern "C" int c_decl(void);
#endif
"#,
            true,
        );
        let includes: Vec<(&str, bool)> = facts.includes.iter().map(|i| (i.path.as_str(), i.angle)).collect();
        assert_eq!(includes, [("common.h", false), ("git2/types.h", true), ("sub/win32.h", false), ("alive.h", false), ("windows.h", true), ("unistd.h", true)]);
        assert_eq!(
            rows(&facts),
            [
                (OutlineKind::Macro, "GIT_OK".into(), "".into(), true, 0),
                (OutlineKind::Macro, "GIT_MAX".into(), "".into(), true, 0),
                (OutlineKind::Type, "git_repository".into(), "".into(), true, 0),
                (OutlineKind::Type, "git_cb".into(), "".into(), true, 0),
                (OutlineKind::Type, "git_point".into(), "".into(), true, 0),
                (OutlineKind::Type, "git_kind".into(), "".into(), true, 0),
                (OutlineKind::Struct, "git_config".into(), "".into(), true, 0),
                (OutlineKind::Enum, "git_mode".into(), "".into(), true, 0),
                (OutlineKind::Function, "git_repository_open".into(), "".into(), true, 0),
                (OutlineKind::Function, "git_libgit2_init".into(), "".into(), true, 0),
                (OutlineKind::Function, "git_inline".into(), "static".into(), false, 0),
                (OutlineKind::Function, "git_path".into(), "".into(), true, 0),
                // Variables other files use; not the definition above them, nor a function.
                (OutlineKind::Var, "OPENSSL_ia32cap_P".into(), "".into(), true, 0),
                (OutlineKind::Var, "git_hook".into(), "".into(), true, 0),
                (OutlineKind::Function, "c_decl".into(), "".into(), true, 0),
            ]
        );
        assert_eq!(facts.constants.iter().map(String::as_str).collect::<Vec<_>>(), ["GIT_A", "GIT_B", "GIT_MODE_READ", "GIT_MODE_WRITE"]);
        assert!(facts.names.contains("git_repository") && !facts.names.contains("DEAD"));
    }

    #[test]
    fn defaults_calling_conventions_macro_tables_and_raw_string_look_alikes() {
        let facts = read(
            "#ifndef UTIL_H\n#define UTIL_H 1\n#ifndef BUFSIZE\n#define BUFSIZE 4096\n#endif\n\
             typedef BOOL (WINAPI *LPFN_ISWOW64PROCESS)(HANDLE, PBOOL);\n\
             typedef GLuint (APIENTRYP PFNGLCREATEPROGRAMPROC)(void);\n\
             OP(ADD, 1)\nOP(SUB, 2)\nOP(MUL, 3)\nextern int op_count;\n\
             #define R \"\\x1b[0m\"\nconst char *reset = R\"this is not a raw string at all\";\nint after_r(void);\n#endif\n",
            true,
        );
        assert_eq!(
            rows(&facts),
            [
                // The guard that opens the file is no macro; a default further down is.
                (OutlineKind::Macro, "BUFSIZE".into(), "".into(), true, 0),
                (OutlineKind::Macro, "R".into(), "".into(), true, 0),
                (OutlineKind::Type, "LPFN_ISWOW64PROCESS".into(), "".into(), true, 0),
                (OutlineKind::Type, "PFNGLCREATEPROGRAMPROC".into(), "".into(), true, 0),
                // A table of macro calls is no function and hides nothing after it.
                (OutlineKind::Var, "op_count".into(), "".into(), true, 0),
                (OutlineKind::Function, "after_r".into(), "".into(), true, 0),
            ]
        );
    }

    #[test]
    fn deep_braces_and_long_macro_tables_read_in_linear_time() {
        let started = std::time::Instant::now();
        let braces = format!("{}{}", "{".repeat(50_000), "}".repeat(50_000));
        read(&braces, false);
        let table: String = (0..8_000).map(|i| format!("OP(X{i}, {i})\n")).collect();
        let facts = read(&format!("{table}int last(void);\n"), true);
        assert_eq!(facts.outline.iter().map(|o| String::from_utf8_lossy(&o.name).into_owned()).collect::<Vec<_>>(), ["last"]);
        assert!(started.elapsed() < std::time::Duration::from_secs(5), "{:?}", started.elapsed());
    }

    #[test]
    fn cpp_namespaces_classes_and_their_members() {
        let facts = read(
            r##"#include <vector>
namespace acme::store {
template <typename T>
class EXPORT Cache final : public Base<T>, private Other {
public:
    Cache();
    explicit Cache(int size) : size_(size) {}
    ~Cache();
    virtual T get(int key) const override;
    std::vector<T> all() const noexcept { return {}; }
    bool operator==(const Cache&) const = default;
    static Cache* make();
    struct Entry { void hidden(); };
private:
    int size_ = compute(3);
    Q_DISABLE_COPY(Cache);
};
using Map = std::map<int, int>;
enum class Color : int { Red, Green };
int Cache::get(int key) const { auto s = R"x(not )" { code)x"; return 1'000 + key; }
Cache::Cache() {}
namespace {
int hidden_helper() { return 0; }
}
}
extern "C" {
void c_api(void);
}
int main(int argc, char **argv) {
    struct Local { int a; };
    return 0;
}
std::string name("not a function");
DEFINE_LOCK(table)
int after_macro(void) { return 0; }
"##,
            false,
        );
        assert_eq!(facts.includes, [Include { path: "vector".into(), angle: true }]);
        assert_eq!(
            rows(&facts),
            [
                // A class in a source file is the file's own. Its members: both constructors,
                // the destructor, get, all and make; not operator==, Entry's or a macro's.
                (OutlineKind::Class, "Cache".into(), "".into(), false, 6),
                (OutlineKind::Type, "Map".into(), "".into(), false, 0),
                (OutlineKind::Enum, "Color".into(), "".into(), false, 0),
                (OutlineKind::Function, "hidden_helper".into(), "".into(), false, 0),
                (OutlineKind::Function, "c_api".into(), "".into(), true, 0),
                (OutlineKind::Function, "main".into(), "".into(), true, 0),
                (OutlineKind::Function, "after_macro".into(), "".into(), true, 0),
            ]
        );
        // A raw string's braces and a parameter's type.
        assert!(facts.names.contains("Base") && facts.names.contains("argv"));
        assert_eq!(facts.constants.iter().map(String::as_str).collect::<Vec<_>>(), ["Green", "Red"]);
    }
}
