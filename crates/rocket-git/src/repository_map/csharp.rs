//! A small lexical reader for C#: the namespaces a file declares, its using directives,
//! the names it uses, and the types each namespace declares (classes, structs,
//! interfaces, enums, records and delegates), each with its own methods counted.
//! Comments, strings, verbatim and raw strings are skipped; an interpolated string's
//! holes are read as code. Preprocessor lines are skipped and only an `#if`'s first
//! branch is read, so that braces split across branches count once.
use super::{OutlineItem, OutlineKind};
use std::collections::BTreeSet;

#[derive(Clone, Debug, PartialEq, Eq)]
enum Token {
    Ident(String),
    /// A string, a character or a number.
    Value,
    Punct(char),
}

/// What one file declares and uses.
#[derive(Clone, Debug, Default)]
pub(super) struct Facts {
    /// The file's namespace: the file-scoped one, else the first it opens; empty for
    /// the global namespace.
    pub namespace: Vec<String>,
    pub usings: Vec<Using>,
    /// The names the file uses outside its namespace and using lines.
    pub names: BTreeSet<String>,
    /// Dotted names that may start with a namespace (`Acme.Store.Db`), as their parts.
    pub qualified: Vec<Vec<String>>,
    /// The types declared in a namespace, each with that namespace, in order.
    pub outline: Vec<(Vec<String>, OutlineItem)>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Using {
    /// The namespace or type it names, without generic arguments.
    pub path: Vec<String>,
    /// `using static Acme.Util.Strings;`: a type's members.
    pub members: bool,
    /// `using Db = Acme.Store.Database;`: the alias.
    pub alias: Option<String>,
    /// `global using …;`: it holds for every file of the project.
    pub global: bool,
}

const QUALIFIED: usize = 20_000;
const OUTLINE: usize = 4096;
const NAMES: usize = 100_000;
/// How far a declaration's words are looked through, and a parameter list: past these
/// a run of tokens is no declaration, and an unfinished one costs no more.
const LOOK: usize = 256;
const PARAMETERS: usize = 4096;
const MODIFIERS: &[&str] = &[
    "public", "internal", "private", "protected", "file", "static", "sealed", "abstract", "partial", "readonly", "ref", "unsafe", "new",
    "extern",
];
const ACCESS: &[&str] = &["public", "internal", "protected", "private", "file"];
/// Words that can stand before a method's name in a type's body yet are no type.
const NOT_TYPES: &[&str] = &["new", "return", "throw", "else", "case", "await", "yield", "is", "as", "in", "out", "this", "base", "nameof", "typeof", "sizeof", "default"];

fn ident_start(c: char) -> bool {
    c == '_' || c.is_alphabetic()
}
fn ident_part(c: char) -> bool {
    c == '_' || c.is_alphanumeric()
}

/// A string being read: verbatim (`@"…"`), raw with its quote count, and the `$`
/// count of an interpolated one (0 when it is not).
#[derive(Clone, Copy, Debug)]
struct Text {
    verbatim: bool,
    quotes: usize,
    dollars: usize,
}
/// Reads a string's text from `c[i]`: where it stops, after its end or after the
/// braces that open a hole.
fn text_rest(c: &[char], mut i: usize, s: Text) -> (usize, bool) {
    while i < c.len() {
        let x = c[i];
        if s.quotes >= 3 {
            if x == '"' && c[i..].iter().take_while(|q| **q == '"').count() >= s.quotes {
                return (i + c[i..].iter().take_while(|q| **q == '"').count(), false);
            }
            if x == '{' && s.dollars > 0 {
                let run = c[i..].iter().take_while(|q| **q == '{').count();
                if run >= s.dollars {
                    return (i + run, true);
                }
                i += run;
                continue;
            }
            i += 1;
            continue;
        }
        match x {
            '"' if s.verbatim && c.get(i + 1) == Some(&'"') => i += 2,
            '"' => return (i + 1, false),
            '\\' if !s.verbatim => i += 2,
            '\n' if !s.verbatim => return (i, false),
            '{' if s.dollars > 0 && c.get(i + 1) == Some(&'{') => i += 2,
            '{' if s.dollars > 0 => return (i + 1, true),
            _ => i += 1,
        }
    }
    (c.len(), false)
}

#[derive(Clone, Copy)]
struct Branch {
    taken: bool,
    followed: bool,
}

fn lex(source: &str) -> Vec<Token> {
    let c: Vec<char> = source.chars().collect();
    let mut out = Vec::new();
    // Holes of interpolated strings being read as code: their string, and the braces
    // opened inside so far.
    let mut holes: Vec<(Text, usize)> = Vec::new();
    let mut text: Option<Text> = None;
    let mut branches: Vec<Branch> = Vec::new();
    let mut start = true;
    let mut i = 0;
    while i < c.len() {
        if let Some(s) = text.take() {
            let (end, hole) = text_rest(&c, i, s);
            if hole {
                holes.push((s, 0));
            }
            i = end;
            continue;
        }
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
        if x == '#' && start && holes.is_empty() {
            let mut j = i + 1;
            while j < c.len() && c[j] != '\n' {
                j += 1;
            }
            let line: String = c[i + 1..j].iter().collect();
            // `#if false // for now`: the comment is no part of the condition.
            let line = line.split("//").next().unwrap_or_default();
            let mut words = line.split_whitespace();
            let name = words.next().unwrap_or_default();
            let rest: Vec<&str> = words.collect();
            let value = match rest.as_slice() {
                ["false"] | ["0"] => Some(false),
                ["true"] | ["1"] => Some(true),
                _ => None,
            };
            match name {
                "if" => branches.push(Branch { taken: value != Some(false), followed: value != Some(false) }),
                "elif" | "else" => {
                    if let Some(b) = branches.last_mut() {
                        b.taken = !b.followed && value != Some(false);
                        b.followed |= b.taken;
                    }
                }
                "endif" => {
                    branches.pop();
                }
                _ => {}
            }
            i = j;
            continue;
        }
        start = false;
        let reading = branches.iter().all(|b| b.taken);
        // Strings: "…", @"…", $"…", $@"…", @$"…", """…""" and $$"""…""".
        let mut j = i;
        let mut dollars = 0;
        let mut verbatim = false;
        while matches!(c.get(j), Some('$' | '@')) {
            if c[j] == '$' {
                dollars += 1;
            } else {
                verbatim = true;
            }
            j += 1;
        }
        let token = if c.get(j) == Some(&'"') && (j > i || x == '"') {
            // A verbatim string has no raw form: `@"""a"""` holds `"a"`.
            let quotes = if verbatim { 1 } else { c[j..].iter().take_while(|q| **q == '"').count() };
            let s = if quotes >= 3 {
                Text { verbatim: false, quotes, dollars }
            } else {
                Text { verbatim, quotes: 1, dollars }
            };
            // `""` is an empty string, not the start of a raw one.
            i = j + if quotes >= 3 { quotes } else { 1 };
            if quotes == 2 {
                i = j + 2;
            } else {
                text = Some(s);
            }
            Token::Value
        } else if x == '\'' {
            let mut j = i + 1;
            while j < c.len() && c[j] != '\'' && c[j] != '\n' {
                j += if c[j] == '\\' { 2 } else { 1 };
            }
            i = if c.get(j) == Some(&'\'') { j + 1 } else { j.min(c.len()) };
            Token::Value
        } else if x == '@' && c.get(i + 1).is_some_and(|n| ident_start(*n)) {
            // @class: a verbatim name.
            let begin = i + 1;
            i += 1;
            while i < c.len() && ident_part(c[i]) {
                i += 1;
            }
            Token::Ident(c[begin..i].iter().collect())
        } else if ident_start(x) {
            let begin = i;
            while i < c.len() && ident_part(c[i]) {
                i += 1;
            }
            Token::Ident(c[begin..i].iter().collect())
        } else if x.is_ascii_digit() {
            while i < c.len() && (c[i].is_alphanumeric() || c[i] == '_' || c[i] == '.' && c.get(i + 1).is_some_and(char::is_ascii_digit)) {
                i += 1;
            }
            Token::Value
        } else {
            i += 1;
            match x {
                '{' => {
                    if let Some(h) = holes.last_mut() {
                        h.1 += 1;
                    }
                }
                '}' => match holes.last_mut() {
                    Some((s, 0)) => {
                        text = Some(*s);
                        holes.pop();
                        continue;
                    }
                    Some(h) => h.1 -= 1,
                    None => {}
                },
                _ => {}
            }
            Token::Punct(x)
        };
        if reading {
            out.push(token);
        }
    }
    out
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Scope {
    Namespace(Vec<String>),
    /// A type declared in a namespace, by outline index and name.
    Type(usize, String),
    Other,
}

/// The namespace being read: the file-scoped one, then each block's.
fn current(file: &[String], scopes: &[Scope]) -> Vec<String> {
    let mut n = file.to_vec();
    for s in scopes {
        if let Scope::Namespace(parts) = s {
            n.extend(parts.iter().cloned());
        }
    }
    n
}

/// Reads one file.
pub(super) fn read(source: &str) -> Facts {
    let source = source.strip_prefix('\u{feff}').unwrap_or(source);
    let t = lex(source);
    let ident = |k: usize| match t.get(k) {
        Some(Token::Ident(x)) => Some(x.as_str()),
        _ => None,
    };
    let punct = |k: usize, p: char| matches!(t.get(k), Some(Token::Punct(x)) if *x == p);
    // A dotted name from `k` (`global::` left out), its parts and where it ends.
    let dotted = |mut k: usize| -> (Vec<String>, usize) {
        if ident(k) == Some("global") && punct(k + 1, ':') && punct(k + 2, ':') {
            k += 3;
        }
        let mut parts = Vec::new();
        while let Some(part) = ident(k) {
            parts.push(part.to_owned());
            k += 1;
            if !punct(k, '.') || ident(k + 1).is_none() {
                break;
            }
            k += 1;
        }
        (parts, k)
    };
    let mut facts = Facts::default();
    let mut scopes: Vec<Scope> = Vec::new();
    // `namespace Acme.Store;` holds for the whole file.
    let mut file_namespace: Vec<String> = Vec::new();
    // The file's namespace: its first type's, else the first one it names.
    let mut first_type: Option<Vec<String>> = None;
    let mut first_named: Option<Vec<String>> = None;
    let mut modifiers: Vec<String> = Vec::new();
    let mut next_scope: Option<Scope> = None;
    let mut assigned = false;
    let (mut parens, mut squares) = (0usize, 0usize);
    let mut k = 0;
    while k < t.len() {
        let level = scopes.iter().all(|s| matches!(s, Scope::Namespace(_))) && parens == 0 && squares == 0;
        let body = match scopes.last() {
            Some(Scope::Type(at, name)) if scopes[..scopes.len() - 1].iter().all(|s| matches!(s, Scope::Namespace(_))) && parens == 0 && squares == 0 => Some((*at, name.clone())),
            _ => None,
        };
        match &t[k] {
            Token::Punct('{') => {
                scopes.push(next_scope.take().unwrap_or(Scope::Other));
                modifiers.clear();
                assigned = false;
            }
            Token::Punct('}') => {
                scopes.pop();
                modifiers.clear();
                assigned = false;
            }
            Token::Punct('(') => parens += 1,
            Token::Punct(')') => parens = parens.saturating_sub(1),
            Token::Punct('[') => squares += 1,
            Token::Punct(']') => squares = squares.saturating_sub(1),
            Token::Punct(';') => {
                modifiers.clear();
                next_scope = None;
                assigned = false;
            }
            Token::Punct('=') if body.is_some() => assigned = true,
            Token::Ident(w) if level && w == "namespace" => {
                let (parts, end) = dotted(k + 1);
                if punct(end, ';') {
                    file_namespace = parts;
                    first_named.get_or_insert_with(|| current(&file_namespace, &scopes));
                } else if punct(end, '{') {
                    let mut full = current(&file_namespace, &scopes);
                    full.extend(parts.iter().cloned());
                    first_named.get_or_insert(full);
                    next_scope = Some(Scope::Namespace(parts));
                }
                k = end;
                continue;
            }
            Token::Ident(w) if level && (w == "using" || w == "global" && ident(k + 1) == Some("using")) => {
                let global = w == "global";
                let mut j = if global { k + 2 } else { k + 1 };
                let members = ident(j) == Some("static");
                if members {
                    j += 1;
                }
                let alias = if ident(j).is_some() && punct(j + 1, '=') {
                    let a = ident(j).map(str::to_owned);
                    j += 2;
                    a
                } else {
                    None
                };
                let (path, mut end) = dotted(j);
                // using L = System.Collections.Generic.List<int>;
                if alias.is_some() {
                    let limit = (end + LOOK).min(t.len());
                    while end < limit && !punct(end, ';') && !punct(end, '{') {
                        end += 1;
                    }
                }
                // A directive ends at its `;`; `using var scope = …;` and `using (…)` are statements.
                if !path.is_empty() && punct(end, ';') {
                    facts.usings.push(Using { path, members, alias, global });
                    k = end;
                    continue;
                }
            }
            Token::Ident(w) if level && MODIFIERS.contains(&w.as_str()) => modifiers.push(w.clone()),
            // Not a constraint: `where T : class`, `where T : struct, IComparable<T>`.
            Token::Ident(w) if level && matches!(w.as_str(), "class" | "struct" | "interface" | "enum" | "record" | "delegate") && !(k > 0 && (punct(k - 1, ':') || punct(k - 1, ','))) => {
                let (kind, mut j) = match w.as_str() {
                    "class" => (OutlineKind::Class, k + 1),
                    "struct" => (OutlineKind::Struct, k + 1),
                    "interface" => (OutlineKind::Interface, k + 1),
                    "enum" => (OutlineKind::Enum, k + 1),
                    // record, record class, record struct
                    "record" if matches!(ident(k + 1), Some("class" | "struct")) => (OutlineKind::Record, k + 2),
                    "record" => (OutlineKind::Record, k + 1),
                    _ => (OutlineKind::Type, k + 1),
                };
                let name = if kind == OutlineKind::Type {
                    // delegate ReturnType Name(…): the name is the last word before `(`.
                    let mut name = None;
                    let limit = (j + LOOK).min(t.len());
                    while j < limit && !punct(j, '(') && !punct(j, ';') {
                        if let Some(x) = ident(j) {
                            name = Some(x.to_owned());
                        }
                        if punct(j, '<') {
                            // Generic arguments: Func<int, bool> Name, or Name<T>(…).
                            let mut depth = 0usize;
                            while j < limit {
                                if punct(j, '<') {
                                    depth += 1;
                                } else if punct(j, '>') {
                                    depth -= 1;
                                    if depth == 0 {
                                        break;
                                    }
                                }
                                j += 1;
                            }
                        }
                        j += 1;
                    }
                    name
                } else {
                    ident(j).map(str::to_owned)
                };
                if let Some(name) = name {
                    let namespace = current(&file_namespace, &scopes);
                    first_type.get_or_insert_with(|| namespace.clone());
                    if facts.outline.len() < OUTLINE {
                        let visibility = modifiers.iter().find(|m| ACCESS.contains(&m.as_str())).cloned().unwrap_or_default();
                        let exported = visibility != "private" && visibility != "file";
                        facts.outline.push((
                            namespace,
                            OutlineItem {
                                kind,
                                name: name.as_bytes().to_vec(),
                                visibility: visibility.into_bytes(),
                                exported,
                                of_trait: None,
                                functions: 0,
                            },
                        ));
                        if kind != OutlineKind::Type {
                            next_scope = Some(Scope::Type(facts.outline.len() - 1, name));
                        }
                    }
                    // On past the name (a delegate's parameters next): `record struct` is one type.
                    modifiers.clear();
                    k = if kind == OutlineKind::Type { j } else { j + 1 };
                    continue;
                }
                modifiers.clear();
            }
            Token::Ident(w) if body.is_some() && !assigned && (punct(k + 1, '(') || punct(k + 1, '<')) => {
                // A method: `Type Name(…)` or `Type Name<T>(…)`, then a body, `=>`, `;`,
                // `where` or a constructor's `: base(…)`; or the constructor, named as its type.
                let mut open = k + 1;
                if punct(open, '<') {
                    let mut depth = 0usize;
                    let limit = (open + LOOK).min(t.len());
                    while open < limit {
                        if punct(open, '<') {
                            depth += 1;
                        } else if punct(open, '>') {
                            depth -= 1;
                            if depth == 0 {
                                break;
                            }
                        } else if !matches!(t[open], Token::Ident(_) | Token::Punct(',' | '.' | '?' | '[' | ']')) {
                            break;
                        }
                        open += 1;
                    }
                    open += 1;
                }
                if punct(open, '(') {
                    let (at, name) = body.clone().unwrap_or_default();
                    let prev = k.checked_sub(1).map(|p| &t[p]);
                    let typed = match prev {
                        Some(Token::Ident(p)) => !NOT_TYPES.contains(&p.as_str()),
                        Some(Token::Punct('>' | ']' | '?')) => true,
                        // A tuple: `public (int, int) Get()`.
                        Some(Token::Punct(')')) => tuple_before(&t, k - 1),
                        _ => false,
                    };
                    let created = matches!(prev, Some(Token::Punct('.'))) || matches!(prev, Some(Token::Ident(p)) if p == "new");
                    let constructor = *w == name && !created;
                    if (typed || constructor)
                        && let Some(close) = matching(&t, open)
                    {
                        let ends = punct(close + 1, '{') || punct(close + 1, ';') || punct(close + 1, ':') || punct(close + 1, '=') && punct(close + 2, '>') || ident(close + 1) == Some("where");
                        if ends {
                            facts.outline[at].1.functions += 1;
                        }
                    }
                }
            }
            _ => {}
        }
        if let Token::Ident(w) = &t[k] {
            if facts.names.len() < NAMES && !facts.names.contains(w.as_str()) {
                facts.names.insert(w.clone());
            }
            let starts = !(k > 0 && punct(k - 1, '.')) && punct(k + 1, '.') && ident(k + 2).is_some();
            if starts && facts.qualified.len() < QUALIFIED {
                let (parts, _) = dotted(k);
                if parts.len() >= 2 {
                    facts.qualified.push(parts.into_iter().take(32).collect());
                }
            }
        }
        k += 1;
    }
    facts.namespace = first_type.or(first_named).unwrap_or_default();
    facts
}

/// The first namespace a file names, read line by line past comments, as the code map
/// names other projects' namespaces; empty for none.
pub(super) fn namespace_clause(source: &str) -> Vec<String> {
    let mut comment = false;
    for line in source.lines() {
        let mut line = line.trim().trim_start_matches('\u{feff}');
        if comment {
            match line.find("*/") {
                Some(end) => {
                    comment = false;
                    line = line[end + 2..].trim();
                }
                None => continue,
            }
        }
        if let Some(rest) = line.strip_prefix("/*") {
            match rest.find("*/") {
                Some(end) => line = rest[end + 2..].trim(),
                None => {
                    comment = true;
                    continue;
                }
            }
        }
        let Some(rest) = line.strip_prefix("namespace") else { continue };
        if !rest.starts_with(char::is_whitespace) {
            continue;
        }
        let name: String = rest.trim().chars().take_while(|c| c.is_alphanumeric() || matches!(c, '_' | '.' | '@')).collect();
        return name.replace('@', "").split('.').filter(|p| !p.is_empty()).map(str::to_owned).collect();
    }
    Vec::new()
}

/// The `)` that closes the `(` at `k`, within a parameter list's reach.
fn matching(t: &[Token], k: usize) -> Option<usize> {
    let mut depth = 0usize;
    for (j, token) in t.iter().enumerate().skip(k).take(PARAMETERS) {
        match token {
            Token::Punct('(') => depth += 1,
            Token::Punct(')') => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return Some(j);
                }
            }
            _ => {}
        }
    }
    None
}

/// Whether the `)` at `close` ends a tuple type, as a member's first word: its `(` stands
/// after a modifier or where a member starts.
fn tuple_before(t: &[Token], close: usize) -> bool {
    let mut depth = 0usize;
    let mut j = close;
    for _ in 0..LOOK {
        match &t[j] {
            Token::Punct(')') => depth += 1,
            Token::Punct('(') => {
                depth -= 1;
                if depth == 0 {
                    return match j.checked_sub(1).map(|p| &t[p]) {
                        None => true,
                        Some(Token::Ident(m)) => MODIFIERS.contains(&m.as_str()) || matches!(m.as_str(), "async" | "virtual" | "override"),
                        Some(Token::Punct('{' | ';' | '}' | ']')) => true,
                        _ => false,
                    };
                }
            }
            _ => {}
        }
        if j == 0 {
            return false;
        }
        j -= 1;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows(facts: &Facts) -> Vec<(String, OutlineKind, String, String, bool, usize)> {
        facts
            .outline
            .iter()
            .map(|(n, o)| (n.join("."), o.kind, String::from_utf8(o.name.clone()).unwrap(), String::from_utf8(o.visibility.clone()).unwrap(), o.exported, o.functions))
            .collect()
    }

    #[test]
    fn a_file_scoped_namespace_usings_and_types_with_their_methods() {
        let facts = read(
            r#"// using Fake.Namespace;
global using Acme.Shared;
using System;
using static Acme.Util.Strings;
using Db = Acme.Store.Database;
using L = System.Collections.Generic.List<int>;

namespace Acme.Web;

/// <summary>A page.</summary>
[Route("home")]
public sealed partial class HomeController : Controller
{
    private readonly ILogger<HomeController> _log = Create();
    public HomeController(ILogger<HomeController> log) : base(log) { _log = log; }
    public async Task<IActionResult> Index() => View($"Hello {User.Name} and {(x ? "{" : "}")}");
    public T Get<T>(int id) where T : class { return null; }
    protected abstract void Render();
    public string Name { get; set; }
    private class Nested { void Hidden() { } }
    string Text = @"verbatim ""quoted"" class Fake {";
    string Raw = """
        { not code
        """;
#if DEBUG
    void Debug() { }
#else
    void Release() { }
#endif
}
internal record Person(string Name);
public record struct Point(int X, int Y);
file enum Mode { A, B }
public delegate void Handler(object sender, EventArgs e);
interface IRepo { void Save(); Task<int> Count(); }
"#,
        );
        assert_eq!(facts.namespace, ["Acme", "Web"]);
        let usings: Vec<(String, bool, Option<&str>, bool)> = facts.usings.iter().map(|u| (u.path.join("."), u.members, u.alias.as_deref(), u.global)).collect();
        assert_eq!(
            usings,
            [
                ("Acme.Shared".to_owned(), false, None, true),
                ("System".into(), false, None, false),
                ("Acme.Util.Strings".into(), true, None, false),
                ("Acme.Store.Database".into(), false, Some("Db"), false),
                ("System.Collections.Generic.List".into(), false, Some("L"), false),
            ]
        );
        assert_eq!(
            rows(&facts),
            [
                // The constructor, Index, Get, Render and Debug; not Nested's, nor the #else branch.
                ("Acme.Web".to_owned(), OutlineKind::Class, "HomeController".into(), "public".into(), true, 5),
                ("Acme.Web".into(), OutlineKind::Record, "Person".into(), "internal".into(), true, 0),
                ("Acme.Web".into(), OutlineKind::Record, "Point".into(), "public".into(), true, 0),
                ("Acme.Web".into(), OutlineKind::Enum, "Mode".into(), "file".into(), false, 0),
                ("Acme.Web".into(), OutlineKind::Type, "Handler".into(), "public".into(), true, 0),
                ("Acme.Web".into(), OutlineKind::Interface, "IRepo".into(), "".into(), true, 2),
            ]
        );
        assert!(facts.names.contains("User") && facts.names.contains("Controller") && !facts.names.contains("Fake"));
    }

    #[test]
    fn constraints_using_statements_commented_conditions_and_tuples() {
        let facts = read(
            "using var scope = app.Services.CreateScope();\nnamespace Acme.Data;\n\
             public class Cache<TKey, TValue> where TKey : class where TValue : struct\n{\n\
                 public void Put(TKey key, TValue value) { }\n\
                 public (int, int) Size() => (0, 0);\n\
             #if false // not built\n    void Never() { }\n#else\n    void Built() { }\n#endif\n}\n\
             public delegate T Make<T>() where T : class;\n",
        );
        // No type named `where`; `using var` is a statement, not a using line.
        assert_eq!(rows(&facts), [("Acme.Data".to_owned(), OutlineKind::Class, "Cache".into(), "public".into(), true, 3), ("Acme.Data".into(), OutlineKind::Type, "Make".into(), "public".into(), true, 0)]);
        assert!(facts.usings.is_empty());
    }

    #[test]
    fn block_namespaces_nest_and_qualified_names_are_kept() {
        let facts = read(
            "namespace Acme {\n  namespace Store.Sql {\n    class Db { void Open() { var x = global::Acme.Util.Strings.Trim(\"a\"); } }\n  }\n  public class Top { }\n}\nclass Global { }\n",
        );
        assert_eq!(facts.namespace, ["Acme", "Store", "Sql"]);
        let rows: Vec<(String, String)> = facts.outline.iter().map(|(n, o)| (n.join("."), String::from_utf8(o.name.clone()).unwrap())).collect();
        assert_eq!(rows, [("Acme.Store.Sql".to_owned(), "Db".to_owned()), ("Acme".into(), "Top".into()), ("".into(), "Global".into())]);
        assert!(facts.qualified.iter().any(|q| q.join(".") == "Acme.Util.Strings.Trim"));
    }
}
