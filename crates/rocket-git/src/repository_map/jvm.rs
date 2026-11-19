//! A small lexical reader for Java and Kotlin: the package clause, the imports, the names
//! a file uses, and what it declares at its top level (types, and Kotlin's functions and
//! properties), each type with its own functions counted. Comments, strings, Java's text
//! blocks and Kotlin's raw strings are skipped; Kotlin's `${…}` templates are read as code.
//! Nothing is compiled, so a name reached through inheritance is matched by name only.
use super::{OutlineItem, OutlineKind};
use std::collections::BTreeSet;

#[derive(Clone, Debug, PartialEq, Eq)]
enum Token {
    Ident(String),
    /// A string, a character or a number.
    Value,
    Punct(char),
}

/// What one file declares and imports.
#[derive(Clone, Debug, Default)]
pub(super) struct Facts {
    /// The package clause's names: `com.acme.store` is three; none for the default package.
    pub package: Vec<String>,
    pub imports: Vec<Import>,
    /// The names the file uses outside its package clause and imports.
    pub names: BTreeSet<String>,
    /// Names written out with their package (`com.acme.store.Db`), as their parts.
    pub qualified: Vec<Vec<String>>,
    /// Types, and Kotlin's functions and properties, at the top level, in order.
    pub outline: Vec<OutlineItem>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Import {
    /// The dotted path, without a trailing `*`: Java's `import static` names a class and
    /// its member, Kotlin's may name a top-level function.
    pub path: Vec<String>,
    /// `import com.acme.store.*`: everything in a package, or a class's members.
    pub all: bool,
    /// Java's `import module java.base;`: a module, not a package.
    pub module: bool,
}

/// Names written out with their package, kept per file.
const QUALIFIED: usize = 20_000;
/// How far a declaration's words are looked through, and a parameter list: past these a
/// run of tokens is no declaration, and an unfinished one costs no more.
const LOOK: usize = 256;
const PARAMETERS: usize = 4096;
/// Names kept per file.
const NAMES: usize = 100_000;
/// Outline items kept per file.
const OUTLINE: usize = 4096;

/// Words before a declaration that are no part of its name.
/// (Kotlin's `enum` is one too; it is taken where `class` is.)
const MODIFIERS: &[&str] = &[
    "public", "protected", "private", "internal", "abstract", "static", "final", "sealed", "non", "strictfp", "open", "data", "annotation",
    "inner", "value", "inline", "expect", "actual", "external", "const", "lateinit", "suspend", "tailrec", "operator", "infix", "override",
];
const ACCESS: &[&str] = &["public", "protected", "private", "internal"];
/// Words that can stand before a method's name inside a Java body yet are no type.
const NOT_TYPES: &[&str] = &["new", "return", "throw", "else", "case", "assert", "yield", "do", "this", "super"];

fn ident_start(c: char, kotlin: bool) -> bool {
    c == '_' || c.is_alphabetic() || !kotlin && c == '$'
}
fn ident_part(c: char, kotlin: bool) -> bool {
    c == '_' || c.is_alphanumeric() || !kotlin && c == '$'
}

/// The rest of a string from `c[i]`: where it ends (after its quote, or before the line
/// break that cuts an unfinished one short), and whether it stopped after a Kotlin `${`
/// instead. `raw` is a triple-quoted string: Java's text block, which keeps its escapes,
/// or Kotlin's raw string, which has none.
fn string_rest(c: &[char], mut i: usize, raw: bool, kotlin: bool) -> (usize, bool) {
    while i < c.len() {
        match c[i] {
            '\\' if !(raw && kotlin) => i += 2,
            '$' if kotlin && c.get(i + 1) == Some(&'{') => return (i + 2, true),
            '"' if !raw => return (i + 1, false),
            '"' if c.get(i + 1) == Some(&'"') && c.get(i + 2) == Some(&'"') => {
                // Kotlin's `""""` ends with a quote inside the string: the last three close it.
                let mut end = i + 3;
                while kotlin && c.get(end) == Some(&'"') {
                    end += 1;
                }
                return (end, false);
            }
            '\n' if !raw => return (i, false),
            _ => i += 1,
        }
    }
    (c.len(), false)
}

/// Tokens, each with whether a line break came before it.
fn lex(source: &str, kotlin: bool) -> Vec<(Token, bool)> {
    let c: Vec<char> = source.chars().collect();
    let mut out = Vec::new();
    let mut line = true;
    // Kotlin's `${…}` templates being read as code: whether each one's string is raw,
    // and the braces opened inside it so far.
    let mut templates: Vec<(bool, usize)> = Vec::new();
    // Inside a string: whether it is raw.
    let mut string: Option<bool> = None;
    let mut i = 0;
    while i < c.len() {
        if let Some(raw) = string.take() {
            let start = i;
            let (end, template) = string_rest(&c, i, raw, kotlin);
            line |= c[start..end.min(c.len())].contains(&'\n');
            if template {
                templates.push((raw, 0));
            }
            i = end;
            continue;
        }
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
        if x == '/' && c.get(i + 1) == Some(&'/') {
            while i < c.len() && c[i] != '\n' {
                i += 1;
            }
            continue;
        }
        if x == '/' && c.get(i + 1) == Some(&'*') {
            // Kotlin's block comments nest; Java's end at the first `*/`.
            let start = i;
            let mut depth = 0usize;
            while i < c.len() {
                if c[i] == '/' && c.get(i + 1) == Some(&'*') && (kotlin || depth == 0) {
                    depth += 1;
                    i += 2;
                } else if c[i] == '*' && c.get(i + 1) == Some(&'/') {
                    depth -= 1;
                    i += 2;
                    if depth == 0 {
                        break;
                    }
                } else {
                    i += 1;
                }
            }
            line |= c[start..i.min(c.len())].contains(&'\n');
            continue;
        }
        let token = if x == '"' {
            let raw = c.get(i + 1) == Some(&'"') && c.get(i + 2) == Some(&'"');
            i += if raw { 3 } else { 1 };
            string = Some(raw);
            Token::Value
        } else if x == '\'' {
            let mut j = i + 1;
            while j < c.len() && c[j] != '\'' && c[j] != '\n' {
                j += if c[j] == '\\' { 2 } else { 1 };
            }
            i = if c.get(j) == Some(&'\'') { j + 1 } else { j.min(c.len()) };
            Token::Value
        } else if x == '`' && kotlin {
            let mut j = i + 1;
            while j < c.len() && c[j] != '`' && c[j] != '\n' {
                j += 1;
            }
            let name = c[i + 1..j.min(c.len())].iter().collect();
            i = if c.get(j) == Some(&'`') { j + 1 } else { j.min(c.len()) };
            Token::Ident(name)
        } else if ident_start(x, kotlin) {
            let start = i;
            while i < c.len() && ident_part(c[i], kotlin) {
                i += 1;
            }
            Token::Ident(c[start..i].iter().collect())
        } else if x.is_ascii_digit() {
            while i < c.len() && (c[i].is_alphanumeric() || c[i] == '_' || c[i] == '.' && c.get(i + 1).is_some_and(char::is_ascii_digit)) {
                i += 1;
            }
            Token::Value
        } else {
            i += 1;
            match x {
                '{' => {
                    if let Some(t) = templates.last_mut() {
                        t.1 += 1;
                    }
                }
                '}' => match templates.last_mut() {
                    // The template ends: its string goes on.
                    Some((raw, 0)) => {
                        string = Some(*raw);
                        templates.pop();
                        continue;
                    }
                    Some(t) => t.1 -= 1,
                    None => {}
                },
                _ => {}
            }
            Token::Punct(x)
        };
        out.push((token, line));
        line = false;
    }
    out
}

/// Reads one file; `kotlin` for a `.kt` file.
pub(super) fn read(source: &str, kotlin: bool) -> Facts {
    // A byte-order mark is no part of the first line.
    let source = source.strip_prefix('\u{feff}').unwrap_or(source);
    let tokens = lex(source, kotlin);
    let t: Vec<&Token> = tokens.iter().map(|(t, _)| t).collect();
    let ident = |k: usize| match t.get(k) {
        Some(Token::Ident(x)) => Some(x.as_str()),
        _ => None,
    };
    let punct = |k: usize, p: char| matches!(t.get(k), Some(Token::Punct(x)) if *x == p);
    // A dotted name from `k`: its parts and where it ends.
    let dotted = |mut k: usize| -> (Vec<String>, usize) {
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
    // The end of a balanced `<…>` from `k` (at its `<`), or `k` when there is none.
    let angles = |k: usize| -> usize {
        if !punct(k, '<') {
            return k;
        }
        let mut depth = 0usize;
        let mut j = k;
        let limit = (k + LOOK).min(t.len());
        while j < limit {
            match t[j] {
                Token::Punct('<') => depth += 1,
                Token::Punct('>') => {
                    depth -= 1;
                    if depth == 0 {
                        return j + 1;
                    }
                }
                Token::Punct('{' | '}' | ';' | '=') => return j,
                _ => {}
            }
            j += 1;
        }
        j
    };
    // The names in tokens a declaration's reading passes over: a receiver's type is used.
    let passed = |facts: &mut Facts, from: usize, to: usize| {
        for token in t.iter().take(to).skip(from) {
            if let Token::Ident(x) = token
                && facts.names.len() < NAMES
                && !facts.names.contains(x.as_str())
            {
                facts.names.insert(x.clone());
            }
        }
    };
    let mut facts = Facts::default();
    let (mut braces, mut parens, mut squares) = (0usize, 0usize, 0usize);
    let mut modifiers: Vec<String> = Vec::new();
    // A type whose header was read and whose body has not opened yet, by outline index.
    let mut pending: Option<usize> = None;
    // The top-level type whose body is open: its outline index and name.
    let mut body: Option<(usize, String)> = None;
    // Inside a Java body: an `=` since the member began (a field's value, not a method).
    let mut assigned = false;
    let mut k = 0;
    while k < t.len() {
        let top = braces == 0 && parens == 0 && squares == 0;
        let in_body = body.is_some() && braces == 1 && parens == 0 && squares == 0;
        match t[k] {
            Token::Punct('{') => {
                if top && let Some(at) = pending.take() {
                    body = Some((at, ident_name(&facts.outline[at])));
                }
                braces += 1;
                assigned = false;
            }
            Token::Punct('}') => {
                braces = braces.saturating_sub(1);
                if braces == 0 {
                    body = None;
                    modifiers.clear();
                }
                assigned = false;
            }
            Token::Punct('(') => parens += 1,
            Token::Punct(')') => parens = parens.saturating_sub(1),
            Token::Punct('[') => squares += 1,
            Token::Punct(']') => squares = squares.saturating_sub(1),
            Token::Punct(';') => {
                if top {
                    modifiers.clear();
                    pending = None;
                }
                assigned = false;
            }
            Token::Punct('=') if in_body => assigned = true,
            Token::Ident(w) if top && w == "package" && facts.package.is_empty() && facts.imports.is_empty() => {
                let (parts, end) = dotted(k + 1);
                facts.package = parts;
                k = end;
                continue;
            }
            Token::Ident(w) if top && w == "import" => {
                let mut j = k + 1;
                // `import static a.b.C.m` and Java 25's `import module java.base`; in Kotlin
                // `static` and `module` can be a package's own first name.
                let mut module = false;
                if matches!(ident(j), Some("static" | "module")) && ident(j + 1).is_some() {
                    module = ident(j) == Some("module");
                    j += 1;
                }
                let (path, mut end) = dotted(j);
                let all = punct(end, '.') && punct(end + 1, '*');
                if all {
                    end += 2;
                }
                if !path.is_empty() {
                    facts.imports.push(Import { path, all, module });
                }
                // Kotlin's `import a.B as C`: the alias names nothing else.
                if ident(end) == Some("as") && ident(end + 1).is_some() {
                    end += 2;
                }
                k = end;
                continue;
            }
            Token::Punct('@') if top => {
                // An annotation, `@file:JvmName("x")` too, or Java's `@interface`.
                if ident(k + 1) == Some("interface") {
                    k = declare(&t, k + 1, OutlineKind::Annotation, &modifiers, kotlin, &mut facts, &mut pending);
                    modifiers.clear();
                    continue;
                }
                let mut j = k + 1;
                if ident(j).is_some() && punct(j + 1, ':') {
                    j += 2;
                }
                // `@Marker` uses Marker; `@com.acme.ann.Marker` names its package too.
                let (parts, end) = dotted(j);
                passed(&mut facts, j, end);
                if parts.len() >= 3 && parts[0].starts_with(|c: char| c.is_lowercase()) && facts.qualified.len() < QUALIFIED {
                    facts.qualified.push(parts.into_iter().take(32).collect());
                }
                k = end.max(k + 1);
                continue;
            }
            Token::Ident(w) if top && MODIFIERS.contains(&w.as_str()) && !punct(k + 1, '.') => {
                modifiers.push(w.clone());
            }
            // Not a class literal: Java's `Foo.class`, Kotlin's `Foo::class`.
            Token::Ident(w) if top && matches!(w.as_str(), "class" | "interface" | "object" | "enum" | "record" | "typealias") && !(k > 0 && (punct(k - 1, '.') || punct(k - 1, ':'))) => {
                let kind = match w.as_str() {
                    "class" if modifiers.iter().any(|m| m == "enum") => Some(OutlineKind::Enum),
                    "class" if modifiers.iter().any(|m| m == "annotation") => Some(OutlineKind::Annotation),
                    "class" => Some(OutlineKind::Class),
                    "interface" => Some(OutlineKind::Interface),
                    "object" if kotlin => Some(OutlineKind::Object),
                    // Java's `enum Color { … }`; in Kotlin `enum` is a modifier before `class`.
                    "enum" if !kotlin => Some(OutlineKind::Enum),
                    // A record has its components next: `record Point(int x, int y)`.
                    "record" if !kotlin && ident(k + 1).is_some() && (punct(k + 2, '(') || punct(k + 2, '<')) => Some(OutlineKind::Record),
                    "typealias" if kotlin => Some(OutlineKind::Type),
                    _ => None,
                };
                if let Some(kind) = kind {
                    k = declare(&t, k, kind, &modifiers, kotlin, &mut facts, &mut pending);
                    modifiers.clear();
                    continue;
                }
                if w == "enum" && kotlin {
                    modifiers.push(w.clone());
                }
            }
            Token::Ident(w) if top && kotlin && w == "fun" => {
                if ident(k + 1) == Some("interface") {
                    k = declare(&t, k + 1, OutlineKind::Interface, &modifiers, kotlin, &mut facts, &mut pending);
                    modifiers.clear();
                    continue;
                }
                // fun <T> List<T>.second(): the name is the last one before `(`.
                let mut j = angles(k + 1);
                let mut name = None;
                let limit = (j + LOOK).min(t.len());
                while j < limit && !punct(j, '(') {
                    match t[j] {
                        Token::Ident(x) => name = Some(x.clone()),
                        Token::Punct('<') => {
                            j = angles(j);
                            continue;
                        }
                        Token::Punct('.' | '?') => {}
                        _ => break,
                    }
                    j += 1;
                }
                if let Some(name) = name.filter(|_| punct(j, '(')) {
                    push(&mut facts, OutlineKind::Function, &name, &modifiers, kotlin);
                }
                modifiers.clear();
                pending = None;
                passed(&mut facts, k + 1, j);
                k = j.max(k + 1);
                continue;
            }
            Token::Ident(w) if top && kotlin && (w == "val" || w == "var") => {
                // val String.lastChar: Char get() = …: the name is the last one before its type.
                let mut j = angles(k + 1);
                let mut name = None;
                let limit = (j + LOOK).min(t.len());
                while j < limit && !tokens[j].1 {
                    match t[j] {
                        Token::Ident(x) if x == "by" => break,
                        Token::Ident(x) => name = Some(x.clone()),
                        Token::Punct('<') => {
                            j = angles(j);
                            continue;
                        }
                        Token::Punct('.' | '?') => {}
                        _ => break,
                    }
                    j += 1;
                }
                if let Some(name) = name {
                    let kind = if w == "val" { OutlineKind::Const } else { OutlineKind::Var };
                    push(&mut facts, kind, &name, &modifiers, kotlin);
                }
                modifiers.clear();
                pending = None;
                passed(&mut facts, k + 1, j);
                k = j.max(k + 1);
                continue;
            }
            Token::Ident(w) if in_body && kotlin && w == "fun" => count(&mut facts, &body),
            Token::Ident(w) if in_body && !kotlin && punct(k + 1, '(') && !assigned => {
                // A method: `Type name(…)` then a body, `throws`, `;` or `default`; or the
                // constructor, named as its type.
                let prev = k.checked_sub(1).map(|p| t[p]);
                let typed = match prev {
                    Some(Token::Ident(p)) => !NOT_TYPES.contains(&p.as_str()),
                    Some(Token::Punct('>' | ']')) => true,
                    _ => false,
                };
                let created = matches!(prev, Some(Token::Punct('.'))) || matches!(prev, Some(Token::Ident(p)) if p == "new");
                let constructor = body.as_ref().is_some_and(|(_, name)| name == w) && !created;
                if (typed || constructor)
                    && let Some(close) = matching(&t, k + 1)
                    && (punct(close + 1, '{') || punct(close + 1, ';') || matches!(ident(close + 1), Some("throws" | "default")))
                {
                    count(&mut facts, &body);
                }
            }
            _ => {}
        }
        // The names the file uses, and dotted ones that may start with a package.
        if let Token::Ident(w) = t[k] {
            if facts.names.len() < NAMES && !facts.names.contains(w.as_str()) {
                facts.names.insert(w.clone());
            }
            // A package of two names or more, then a type: `java.util.List`.
            let starts = !(k > 0 && punct(k - 1, '.')) && punct(k + 1, '.') && ident(k + 2).is_some() && punct(k + 3, '.') && ident(k + 4).is_some();
            if starts && w.starts_with(|c: char| c.is_lowercase()) && facts.qualified.len() < QUALIFIED {
                let (parts, _) = dotted(k);
                if parts.len() >= 2 {
                    facts.qualified.push(parts.into_iter().take(32).collect());
                }
            }
        }
        k += 1;
    }
    facts
}

/// The package clause alone, as the code map names other projects' packages: read line
/// by line past comments and annotations, up to the first line that is neither.
pub(super) fn package_clause(source: &str) -> Vec<String> {
    let mut comment = false;
    for line in source.strip_prefix('\u{feff}').unwrap_or(source).lines() {
        let mut line = line.trim();
        if comment {
            match line.find("*/") {
                Some(end) => {
                    comment = false;
                    line = line[end + 2..].trim();
                }
                None => continue,
            }
        }
        while let Some(rest) = line.strip_prefix("/*") {
            match rest.find("*/") {
                Some(end) => line = rest[end + 2..].trim(),
                None => {
                    comment = true;
                    line = "";
                }
            }
        }
        if line.is_empty() || line.starts_with("//") || line.starts_with('@') {
            continue;
        }
        let Some(rest) = line.strip_prefix("package") else { return Vec::new() };
        if !rest.starts_with(char::is_whitespace) {
            return Vec::new();
        }
        let name: String = rest.trim().chars().take_while(|c| c.is_alphanumeric() || matches!(c, '_' | '.' | '$' | '`')).collect();
        return name.replace('`', "").split('.').filter(|p| !p.is_empty()).map(str::to_owned).collect();
    }
    Vec::new()
}

fn ident_name(item: &OutlineItem) -> String {
    String::from_utf8_lossy(&item.name).into_owned()
}

/// The `)` that closes the `(` at `k`, within a parameter list's reach.
fn matching(t: &[&Token], k: usize) -> Option<usize> {
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

/// A type declared at `k` (its keyword): its outline item; the next `{` at the top level
/// opens its body. Returns where reading goes on.
fn declare(t: &[&Token], k: usize, kind: OutlineKind, modifiers: &[String], kotlin: bool, facts: &mut Facts, pending: &mut Option<usize>) -> usize {
    match t.get(k + 1) {
        Some(Token::Ident(name)) => {
            if push(facts, kind, name, modifiers, kotlin) {
                *pending = Some(facts.outline.len() - 1);
            }
            k + 2
        }
        _ => k + 1,
    }
}

fn push(facts: &mut Facts, kind: OutlineKind, name: &str, modifiers: &[String], kotlin: bool) -> bool {
    if facts.outline.len() >= OUTLINE {
        return false;
    }
    let visibility = modifiers.iter().find(|m| ACCESS.contains(&m.as_str())).cloned().unwrap_or_default();
    // Seen from another package: Java's public; Kotlin's public (the default) and internal.
    let exported = if kotlin { visibility != "private" } else { visibility == "public" || visibility == "protected" };
    facts.outline.push(OutlineItem {
        kind,
        name: name.as_bytes().to_vec(),
        visibility: visibility.into_bytes(),
        exported,
        of_trait: None,
        functions: 0,
    });
    true
}

fn count(facts: &mut Facts, body: &Option<(usize, String)>) {
    if let Some((at, _)) = body {
        facts.outline[*at].functions += 1;
    }
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
    fn imports(facts: &Facts) -> Vec<(String, bool, bool)> {
        facts.imports.iter().map(|i| (i.path.join("."), i.all, i.module)).collect()
    }

    #[test]
    fn java_package_imports_and_types_with_their_methods() {
        let facts = read(
            r#"/* Licence: package fake; import fake.Nope; */
@NonNullApi
package com.acme.store;

import java.util.List;
import java.util.*;
import static com.acme.util.Strings.trim;
import static org.junit.Assert.*;
import module java.base;

/** A store. */
public final class Store<K> extends Base implements AutoCloseable {
    private final List<K> keys = new ArrayList<>();
    private Runnable r = new Runnable() { public void run() {} };
    String text = """
        import not.this.One;
        class NotAClass { void no() {} }
        """;
    public Store(int size) { this.size = size; }
    public <T> T get(Class<T> type) throws IOException { return null; }
    abstract void close();
    int[] values() { if (x) { other(); } return new int[0]; }
    static class Inner { void hidden() {} }
    @Override public String toString() { return "Store(" + com.acme.util.Strings.trim(name) + ")"; }
}
enum Color { RED(1), GREEN(2); Color(int v) {} int value() { return 0; } }
record Point(int x, int y) { Point { } static Point origin() { return null; } }
@interface Inject { String value() default ""; }
interface Repo { void save(); }
"#,
            false,
        );
        assert_eq!(facts.package, ["com", "acme", "store"]);
        assert_eq!(
            imports(&facts),
            [
                ("java.util.List".to_owned(), false, false),
                ("java.util".into(), true, false),
                ("com.acme.util.Strings.trim".into(), false, false),
                ("org.junit.Assert".into(), true, false),
                ("java.base".into(), false, true),
            ]
        );
        assert_eq!(
            rows(&facts),
            [
                // The constructor, get, close, values and toString; not Inner's or run's.
                (OutlineKind::Class, "Store".into(), "public".into(), true, 5),
                (OutlineKind::Enum, "Color".into(), "".into(), false, 2),
                (OutlineKind::Record, "Point".into(), "".into(), false, 1),
                (OutlineKind::Annotation, "Inject".into(), "".into(), false, 1),
                (OutlineKind::Interface, "Repo".into(), "".into(), false, 1),
            ]
        );
        assert!(facts.names.contains("ArrayList") && facts.names.contains("IOException") && !facts.names.contains("NotAClass"));
        assert!(facts.qualified.iter().any(|q| q.join(".") == "com.acme.util.Strings.trim"));
    }

    #[test]
    fn kotlin_declarations_templates_and_raw_strings() {
        let facts = read(
            r#"@file:JvmName("Stores")
package com.acme.`store`

import com.acme.db.Database
import com.acme.util.*
import kotlinx.coroutines.flow.map as mapFlow

/* outer /* nested */ still a comment: class Fake */
data class User(val name: String, val age: Int) {
    fun greet() = "Hello ${name.uppercase()} and ${ if (age > 1) "{" else "}" }"
    val raw = """a ${Database.open()} "quoted" """"
    companion object { fun create(): User = User("a", 1) }
}
sealed interface Shape
enum class Kind { A, B; fun label() = name }
object Registry { fun all() = listOf<User>() }
annotation class Marker
typealias Users = List<User>
private fun helper(x: Int): Int = x
fun <T> List<T>.second(): T = this[1]
internal val String.lastChar: Char get() = this[length - 1]
const val LIMIT = 10
var counter = 0
fun interface Action { fun run() }
val (a, b) = Pair(1, 2)
"#,
            true,
        );
        assert_eq!(facts.package, ["com", "acme", "store"]);
        assert_eq!(imports(&facts), [("com.acme.db.Database".to_owned(), false, false), ("com.acme.util".into(), true, false), ("kotlinx.coroutines.flow.map".into(), false, false)]);
        assert_eq!(
            rows(&facts),
            [
                (OutlineKind::Class, "User".into(), "".into(), true, 1),
                (OutlineKind::Interface, "Shape".into(), "".into(), true, 0),
                (OutlineKind::Enum, "Kind".into(), "".into(), true, 1),
                (OutlineKind::Object, "Registry".into(), "".into(), true, 1),
                (OutlineKind::Annotation, "Marker".into(), "".into(), true, 0),
                (OutlineKind::Type, "Users".into(), "".into(), true, 0),
                (OutlineKind::Function, "helper".into(), "private".into(), false, 0),
                (OutlineKind::Function, "second".into(), "".into(), true, 0),
                (OutlineKind::Const, "lastChar".into(), "internal".into(), true, 0),
                (OutlineKind::Const, "LIMIT".into(), "".into(), true, 0),
                (OutlineKind::Var, "counter".into(), "".into(), true, 0),
                (OutlineKind::Interface, "Action".into(), "".into(), true, 1),
            ]
        );
        // Names inside templates are code; text in strings and comments is not.
        assert!(facts.names.contains("Database") && facts.names.contains("uppercase") && !facts.names.contains("Fake") && !facts.names.contains("quoted"));
    }

    #[test]
    fn names_after_val_are_no_modifiers_and_annotations_are_used() {
        let facts = read("val enum = 1\nclass Foo\nval private = 2\nclass Bar\n@Marker class App\n@com.acme.ann.Other class B\n", true);
        assert_eq!(
            rows(&facts),
            [
                (OutlineKind::Const, "enum".into(), "".into(), true, 0),
                (OutlineKind::Class, "Foo".into(), "".into(), true, 0),
                (OutlineKind::Const, "private".into(), "".into(), true, 0),
                (OutlineKind::Class, "Bar".into(), "".into(), true, 0),
                (OutlineKind::Class, "App".into(), "".into(), true, 0),
                (OutlineKind::Class, "B".into(), "".into(), true, 0),
            ]
        );
        assert!(facts.names.contains("Marker"));
        assert!(facts.qualified.iter().any(|q| q.join(".") == "com.acme.ann.Other"));
        // An extension's receiver is a name the file uses.
        assert!(read("fun Db.helper() = 1\n", true).names.contains("Db"));
    }

    #[test]
    fn unfinished_declarations_read_in_linear_time() {
        let started = std::time::Instant::now();
        read(&"fun ".repeat(20_000), true);
        read(&"val <".repeat(20_000), true);
        read(&format!("class A {{ {} }}", "int a(".repeat(20_000)), false);
        assert!(started.elapsed() < std::time::Duration::from_secs(5), "{:?}", started.elapsed());
    }

    #[test]
    fn a_default_package_a_byte_order_mark_and_unfinished_text() {
        let facts = read("\u{feff}import a.B;\nclass C { void m() { String s = \"unfinished\n; } }\n", false);
        assert!(facts.package.is_empty());
        assert_eq!(imports(&facts), [("a.B".to_owned(), false, false)]);
        assert_eq!(rows(&facts), [(OutlineKind::Class, "C".into(), "".into(), false, 1)]);
        // A Kotlin package can be named `module` or `static`.
        let facts = read("package x\nimport module.store.Db\nimport static.Thing\n", true);
        assert_eq!(imports(&facts), [("module.store.Db".to_owned(), false, false), ("static.Thing".into(), false, false)]);
    }
}

