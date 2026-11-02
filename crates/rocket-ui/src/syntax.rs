//! Display-only, bounded lexical highlighting. No grammars, plugins or repository
//! code are executed. Each side of a diff owns its multiline state; a hunk gap
//! resets it because the omitted source is unknown. This is not semantic analysis.
use crate::diff::{FilePatch, Kind};
use std::sync::Arc;

pub const MAX_BYTES: usize = 2 * 1024 * 1024;
pub const MAX_SPANS: usize = 100_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Token {
    Keyword,
    String,
    Comment,
    Number,
    Type,
    Function,
    Property,
    Attribute,
    Operator,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Span {
    pub start: usize,
    pub end: usize,
    pub token: Token,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Language {
    Rust,
    C,
    Cpp,
    CSharp,
    Go,
    Java,
    JavaScript,
    TypeScript,
    Python,
    PowerShell,
    Shell,
    Ruby,
    Json,
    Toml,
    Yaml,
    Html,
    Css,
    Sql,
    Dart,
    Elixir,
    Erlang,
    Matlab,
    Fortran,
    FortranFixed,
}
impl Language {
    pub fn detect(path: &str) -> Option<Self> {
        let name = path.rsplit(['/', '\\']).next()?.to_ascii_lowercase();
        if matches!(name.as_str(), "dockerfile" | "makefile") {
            return None;
        }
        Some(match name.rsplit('.').next()? {
            "rs" => Self::Rust,
            "c" | "h" => Self::C,
            "cpp" | "cc" | "cxx" | "hpp" | "hh" | "hxx" => Self::Cpp,
            "cs" => Self::CSharp,
            "go" => Self::Go,
            "java" => Self::Java,
            "js" | "jsx" | "mjs" | "cjs" => Self::JavaScript,
            "ts" | "tsx" | "mts" | "cts" => Self::TypeScript,
            "py" | "pyw" | "pyi" => Self::Python,
            "ps1" | "psm1" | "psd1" => Self::PowerShell,
            "sh" | "bash" | "zsh" => Self::Shell,
            "rb" => Self::Ruby,
            "json" => Self::Json,
            "toml" => Self::Toml,
            "yaml" | "yml" => Self::Yaml,
            "html" | "htm" | "xml" | "svg" => Self::Html,
            "css" | "scss" => Self::Css,
            "sql" => Self::Sql,
            "dart" => Self::Dart,
            "ex" | "exs" => Self::Elixir,
            "erl" | "hrl" => Self::Erlang,
            "m" => Self::Matlab,
            "f90" | "f95" | "f03" | "f08" | "f18" => Self::Fortran,
            "f" | "for" | "ftn" | "f77" => Self::FortranFixed,
            _ => return None,
        })
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Rust => "Rust",
            Self::C => "C",
            Self::Cpp => "C++",
            Self::CSharp => "C#",
            Self::Go => "Go",
            Self::Java => "Java",
            Self::JavaScript => "JavaScript",
            Self::TypeScript => "TypeScript",
            Self::Python => "Python",
            Self::PowerShell => "PowerShell",
            Self::Shell => "Shell",
            Self::Ruby => "Ruby",
            Self::Json => "JSON",
            Self::Toml => "TOML",
            Self::Yaml => "YAML",
            Self::Html => "HTML / XML",
            Self::Css => "CSS",
            Self::Sql => "SQL",
            Self::Dart => "Dart",
            Self::Elixir => "Elixir",
            Self::Erlang => "Erlang",
            Self::Matlab => "MATLAB",
            Self::Fortran | Self::FortranFixed => "Fortran",
        }
    }
    fn c_comments(self) -> bool {
        matches!(
            self,
            Self::Rust
                | Self::C
                | Self::Cpp
                | Self::CSharp
                | Self::Go
                | Self::Java
                | Self::JavaScript
                | Self::TypeScript
                | Self::Css
                | Self::Dart
        )
    }
    fn hash_comments(self) -> bool {
        matches!(
            self,
            Self::Python
                | Self::PowerShell
                | Self::Shell
                | Self::Ruby
                | Self::Toml
                | Self::Yaml
                | Self::Elixir
        )
    }
    fn word(self, word: &str) -> Option<Token> {
        let lower;
        let word = if matches!(
            self,
            Self::PowerShell | Self::Sql | Self::Fortran | Self::FortranFixed
        ) {
            lower = word.to_ascii_lowercase();
            lower.as_str()
        } else {
            word
        };
        let keywords = match self {
            Self::Rust => {
                "as async await break const continue crate dyn else enum extern false fn for if impl in let loop match mod move mut pub ref return self Self static struct super trait true type unsafe use where while abstract become box do final macro override priv typeof unsized virtual yield try"
            }
            Self::C => {
                "auto break case const continue default do else enum extern for goto if inline register restrict return sizeof static struct switch typedef union volatile while _Atomic _Generic _Static_assert"
            }
            Self::Cpp => {
                "alignas alignof asm auto break case catch class concept const consteval constexpr constinit const_cast continue co_await co_return co_yield decltype default delete do dynamic_cast else enum explicit export extern false for friend goto if inline mutable namespace new noexcept nullptr operator private protected public register reinterpret_cast requires return sizeof static static_assert static_cast struct switch template this thread_local throw true try typedef typeid typename union using virtual volatile while"
            }
            Self::CSharp => {
                "abstract as async await base break case catch checked class const continue default delegate do else enum event explicit extern false finally fixed for foreach goto if implicit in interface internal is lock namespace new null operator out override params partial private protected public readonly record ref required return sealed sizeof stackalloc static struct switch this throw true try typeof unchecked unsafe using var virtual void volatile when where while with yield"
            }
            Self::Go => {
                "break case chan const continue default defer else fallthrough for func go goto if import interface map package range return select struct switch type var true false nil"
            }
            Self::Java => {
                "abstract assert boolean break case catch class const continue default do else enum extends final finally for if implements import instanceof interface native new null package private protected public record return sealed static strictfp super switch synchronized this throw throws transient true false try var volatile while yield"
            }
            Self::JavaScript | Self::TypeScript => {
                "async await break case catch class const continue debugger default delete do else export extends false finally for from function get if import in instanceof let new null of return set static super switch this throw true try typeof undefined var void while with yield as declare enum implements infer interface is keyof namespace never private protected public readonly satisfies type abstract override"
            }
            Self::Python => {
                "False None True and as assert async await break case class continue def del elif else except finally for from global if import in is lambda match nonlocal not or pass raise return try while with yield"
            }
            Self::PowerShell => {
                "begin break catch class clean continue data define do dynamicparam else elseif end enum exit filter finally for foreach from function if in param process return switch throw trap try until using var while workflow true false null"
            }
            Self::Shell => {
                "if then else elif fi case esac for select while until do done in function time coproc export local readonly declare typeset return break continue"
            }
            Self::Ruby => {
                "BEGIN END alias and begin break case class def defined do else elsif end ensure false for if in module next nil not or redo rescue retry return self super then true undef unless until when while yield"
            }
            Self::Json => "true false null",
            Self::Toml => "true false inf nan",
            Self::Yaml => "true false null yes no on off",
            Self::Sql => {
                "select from where join left right inner outer full on as and or not null true false insert into values update set delete create alter drop table index view primary foreign key references group by order asc desc having limit offset union all distinct case when then else end with returning exists between like is count sum avg begin commit rollback"
            }
            Self::Html | Self::Css => "",
            Self::Dart => {
                "abstract as assert async await augment base break case catch class const continue covariant default deferred do dynamic else enum export extends extension external factory false final finally for Function get hide if implements import in interface is late library mixin new null of on operator part required rethrow return sealed set show static super switch sync this throw true try type typedef var void when while with yield"
            }
            Self::Elixir => {
                "after alias and case catch cond def defdelegate defexception defguard defguardp defimpl defmacro defmacrop defmodule defp defprotocol defstruct do else end false fn for if import in nil not or quote raise receive require rescue reraise super true try unless unquote unquote_splicing use when with"
            }
            Self::Erlang => {
                "after begin case catch cond end fun if let of receive try when andalso orelse maybe else bnot not div rem band and bor bxor bsl bsr or xor true false"
            }
            Self::Matlab => {
                "arguments break case catch classdef continue else elseif end enumeration events for function global if methods otherwise parfor persistent properties return spmd switch try while"
            }
            Self::Fortran | Self::FortranFixed => {
                "allocate allocatable associate asynchronous backspace bind block call case close common contains continue cycle data deallocate dimension do double precision else elseif elsewhere end enddo endif endprogram entry enum enumerator equivalence error exit external final flush forall format function generic goto if implicit import in include inout inquire intent interface intrinsic module namelist none non_intrinsic only open operator optional out parameter pass pause pointer print private procedure program protected public pure read recursive result return rewind save select sequence stop submodule subroutine target then type use value volatile wait where while write elemental concurrent rank selectrank selecttype"
            }
        };
        if keywords.split_ascii_whitespace().any(|k| k == word) {
            return Some(Token::Keyword);
        }
        let types = match self {
            Self::Rust => {
                "bool char str i8 i16 i32 i64 i128 isize u8 u16 u32 u64 u128 usize f32 f64 String Vec Option Result Box Arc Rc Self"
            }
            Self::C | Self::Cpp => {
                "bool char double float int long short signed unsigned void wchar_t size_t string vector unique_ptr shared_ptr"
            }
            Self::CSharp => {
                "bool byte char decimal double dynamic float int long object sbyte short string uint ulong ushort"
            }
            Self::Go => {
                "bool byte complex64 complex128 error float32 float64 int int8 int16 int32 int64 rune string uint uint8 uint16 uint32 uint64 uintptr"
            }
            Self::Java => "boolean byte char double float int long short void String Object",
            Self::TypeScript => "any boolean bigint number object string symbol unknown void",
            Self::Python => {
                "bool bytes bytearray complex dict float frozenset int list object range set str tuple type"
            }
            Self::Dart => {
                "int double num bool String List Set Map Object Null Never Future Stream Iterable Symbol Record"
            }
            Self::Fortran | Self::FortranFixed => "integer real complex logical character",
            _ => "",
        };
        types
            .split_ascii_whitespace()
            .any(|t| t == word)
            .then_some(Token::Type)
    }
}

#[derive(Clone, Debug)]
enum Mode {
    MatlabComment(usize),
    Block {
        end: &'static str,
        depth: usize,
        nested: bool,
    },
    Quote {
        end: String,
        escape: Option<u8>,
        multiline: bool,
        doubled: bool,
    },
    Here(String),
}
pub struct Lexer {
    language: Language,
    mode: Option<Mode>,
    tag: bool,
    expect_type: bool,
    expect_function: bool,
}
impl Lexer {
    pub fn new(language: Language) -> Self {
        Self {
            language,
            mode: None,
            tag: false,
            expect_type: false,
            expect_function: false,
        }
    }
    // Returns the end byte and whether the multiline construct is closed.
    fn continuation(mode: &mut Mode, text: &str, mut i: usize) -> (usize, bool) {
        let b = text.as_bytes();
        match mode {
            Mode::MatlabComment(depth) => {
                if text.trim() == "%{" {
                    *depth += 1;
                }
                if text.trim() == "%}" {
                    *depth -= 1;
                }
                (text.len(), *depth == 0)
            }
            Mode::Block { end, depth, nested } => {
                while i < b.len() {
                    if text[i..].starts_with(*end) {
                        i += end.len();
                        *depth -= 1;
                        if *depth == 0 {
                            return (i, true);
                        }
                    } else if *nested && text[i..].starts_with("/*") {
                        *depth += 1;
                        i += 2;
                    } else {
                        i += text[i..].chars().next().unwrap().len_utf8();
                    }
                }
                (i, false)
            }
            Mode::Quote {
                end,
                escape,
                multiline,
                doubled,
            } => {
                while i < b.len() {
                    if escape.is_some_and(|c| b[i] == c) {
                        i += 1;
                        if i < b.len() {
                            i += text[i..].chars().next().unwrap().len_utf8();
                        }
                    } else if text[i..].starts_with(end.as_str()) {
                        i += end.len();
                        if *doubled && text[i..].starts_with(end.as_str()) {
                            i += end.len();
                        } else {
                            return (i, true);
                        }
                    } else {
                        i += text[i..].chars().next().unwrap().len_utf8();
                    }
                }
                (i, !*multiline)
            }
            Mode::Here(end) => {
                if i == 0 && text.starts_with(end.as_str()) {
                    (end.len(), true)
                } else {
                    (b.len(), false)
                }
            }
        }
    }
    pub fn line(&mut self, text: &str, offset: usize) -> Vec<Span> {
        let mut out = Vec::new();
        let mut i = 0;
        let b = text.as_bytes();
        let lang = self.language;
        // Fixed-form comment indicators are meaningful only in column one.
        // Comment lines between continued literals do not consume quote state.
        if matches!(lang, Language::Fortran | Language::FortranFixed) {
            if text.trim_start().starts_with('!')
                || (lang == Language::FortranFixed && matches!(b.first(), Some(b'C' | b'c' | b'*')))
            {
                return vec![Span {
                    start: offset,
                    end: offset + b.len(),
                    token: Token::Comment,
                }];
            }
            if self.mode.is_some() {
                let continuation = if lang == Language::FortranFixed {
                    b.get(5).is_some_and(|b| !matches!(b, b' ' | b'0'))
                } else {
                    text.trim_start().starts_with('&')
                };
                if continuation {
                    i = if lang == Language::FortranFixed {
                        6
                    } else {
                        text.len() - text.trim_start().len() + 1
                    };
                } else {
                    self.mode = None;
                }
            }
        }
        while i < b.len() {
            let start = i;
            if let Some(mode) = &mut self.mode {
                let token = if matches!(mode, Mode::Block { .. } | Mode::MatlabComment(_)) {
                    Token::Comment
                } else {
                    Token::String
                };
                let (end, closed) = Self::continuation(mode, text, i);
                i = end;
                if closed {
                    self.mode = None;
                }
                if i > start {
                    out.push(Span {
                        start: start + offset,
                        end: i + offset,
                        token,
                    });
                }
                continue;
            }
            let rest = &text[i..];
            let ch = rest.chars().next().unwrap();
            if ch.is_whitespace() {
                i += ch.len_utf8();
                continue;
            }
            if lang == Language::Matlab && text.trim() == "%{" {
                self.mode = Some(Mode::MatlabComment(1));
                out.push(Span {
                    start: i + offset,
                    end: b.len() + offset,
                    token: Token::Comment,
                });
                break;
            }
            let line_comment =
                (lang.c_comments() && lang != Language::Css && rest.starts_with("//"))
                    || (lang.hash_comments()
                        && ch == '#'
                        && (lang != Language::Shell || i == 0 || b[i - 1].is_ascii_whitespace()))
                    || (lang == Language::Sql && rest.starts_with("--"))
                    || (matches!(lang, Language::Matlab | Language::Erlang) && ch == '%')
                    || (matches!(lang, Language::Fortran | Language::FortranFixed) && ch == '!');
            if line_comment {
                out.push(Span {
                    start: i + offset,
                    end: b.len() + offset,
                    token: Token::Comment,
                });
                break;
            }
            let block = if lang == Language::Html && rest.starts_with("<!--") {
                Some((4, "-->", false))
            } else if lang == Language::PowerShell && rest.starts_with("<#") {
                Some((2, "#>", false))
            } else if (lang.c_comments() || lang == Language::Sql) && rest.starts_with("/*") {
                Some((2, "*/", matches!(lang, Language::Rust | Language::Dart)))
            } else {
                None
            };
            if let Some((n, end, nested)) = block {
                let mut mode = Mode::Block {
                    end,
                    depth: 1,
                    nested,
                };
                let (end, closed) = Self::continuation(&mut mode, text, i + n);
                i = end;
                if !closed {
                    self.mode = Some(mode);
                }
                out.push(Span {
                    start: start + offset,
                    end: i + offset,
                    token: Token::Comment,
                });
                continue;
            }
            // Markup body is plain text. Embedded JS/CSS is deliberately not guessed.
            if lang == Language::Html {
                if ch == '<' {
                    self.tag = true;
                    i += 1;
                    if b.get(i) == Some(&b'/') {
                        i += 1;
                    }
                    while i < b.len() && ident_char(text[i..].chars().next().unwrap(), lang) {
                        i += text[i..].chars().next().unwrap().len_utf8();
                    }
                    out.push(Span {
                        start: start + offset,
                        end: i + offset,
                        token: Token::Type,
                    });
                    continue;
                }
                if ch == '>' {
                    self.tag = false;
                    i += 1;
                    out.push(Span {
                        start: start + offset,
                        end: i + offset,
                        token: Token::Type,
                    });
                    continue;
                }
                if !self.tag {
                    i += ch.len_utf8();
                    continue;
                }
            }
            // Adjacent apostrophes after operands in MATLAB are transpose
            // operators. Whitespace in array literals can start a new string.
            let transpose = lang == Language::Matlab
                && (ch == '\'' || rest.starts_with(".'"))
                && text[..i]
                    .chars()
                    .next_back()
                    .is_some_and(|c| c.is_alphanumeric() || "_)]}'\"".contains(c));
            if transpose {
                i += if ch == '.' { 2 } else { 1 };
                out.push(Span {
                    start: start + offset,
                    end: i + offset,
                    token: Token::Operator,
                });
                continue;
            }
            // Erlang character literals must shield $% and $" from comment and
            // string recognition. Escaped delimiters remain one token.
            if lang == Language::Erlang && ch == '$' && rest.len() > 1 {
                i += 1;
                if b.get(i) == Some(&b'\\') {
                    i += 1;
                }
                if let Some(c) = text[i..].chars().next() {
                    i += c.len_utf8();
                }
                out.push(Span {
                    start: start + offset,
                    end: i + offset,
                    token: Token::String,
                });
                continue;
            }
            if let Some((n, mut mode)) = quote(lang, rest) {
                let here = matches!(&mode, Mode::Here(_));
                let (end, closed) = if here {
                    (b.len(), false)
                } else {
                    Self::continuation(&mut mode, text, i + n)
                };
                i = end;
                if !closed {
                    self.mode = Some(mode);
                }
                let property = matches!(
                    lang,
                    Language::Json | Language::Yaml | Language::JavaScript | Language::TypeScript
                ) && text[i..].trim_start().starts_with(':');
                out.push(Span {
                    start: start + offset,
                    end: i + offset,
                    token: if property {
                        Token::Property
                    } else {
                        Token::String
                    },
                });
                continue;
            }
            if lang == Language::Toml && ch == '[' && text[..i].trim().is_empty() {
                i = text[i..].find(']').map_or(b.len(), |end| i + end + 1);
                if b.get(i) == Some(&b']') {
                    i += 1;
                }
                out.push(Span {
                    start: start + offset,
                    end: i + offset,
                    token: Token::Attribute,
                });
                continue;
            }
            if lang == Language::Rust && ch == '\'' {
                i += 1;
                while i < b.len() && ident_char(text[i..].chars().next().unwrap(), lang) {
                    i += text[i..].chars().next().unwrap().len_utf8();
                }
                out.push(Span {
                    start: start + offset,
                    end: i + offset,
                    token: Token::Type,
                });
                continue;
            }
            if (ch == '$' && matches!(lang, Language::PowerShell | Language::Shell))
                || (ch == '@' && matches!(lang, Language::Python | Language::Java | Language::Css))
                || (lang == Language::Rust && rest.starts_with("#["))
            {
                i += 1;
                while i < b.len()
                    && (ident_char(text[i..].chars().next().unwrap(), lang)
                        || b".:".contains(&b[i]))
                {
                    i += text[i..].chars().next().unwrap().len_utf8();
                }
                out.push(Span {
                    start: start + offset,
                    end: i + offset,
                    token: if ch == '$' {
                        Token::Property
                    } else {
                        Token::Attribute
                    },
                });
                continue;
            }
            if ch == '#' && matches!(lang, Language::C | Language::Cpp | Language::CSharp) {
                i += 1;
                while i < b.len() && (b[i].is_ascii_alphabetic() || b[i] == b' ') {
                    i += 1;
                }
                out.push(Span {
                    start: start + offset,
                    end: i + offset,
                    token: Token::Attribute,
                });
                continue;
            }
            if ch.is_ascii_digit() {
                i += 1;
                while i < b.len() {
                    if b[i].is_ascii_alphanumeric()
                        || b[i] == b'_'
                        || (b[i] == b'.' && b.get(i + 1).is_some_and(u8::is_ascii_digit))
                        || (b"+-".contains(&b[i]) && b"eE".contains(&b[i - 1]))
                    {
                        i += 1;
                    } else {
                        break;
                    }
                }
                out.push(Span {
                    start: start + offset,
                    end: i + offset,
                    token: Token::Number,
                });
                continue;
            }
            if ch.is_alphabetic() || ch == '_' || ch == '$' {
                i += ch.len_utf8();
                while i < b.len() && ident_char(text[i..].chars().next().unwrap(), lang) {
                    i += text[i..].chars().next().unwrap().len_utf8();
                }
                let word = &text[start..i];
                let after = text[i..].trim_start();
                let token = lang.word(word).or_else(|| {
                    if self.expect_type {
                        Some(Token::Type)
                    } else if self.expect_function
                        || after.starts_with('(')
                        || (lang == Language::Rust && after.starts_with('!'))
                    {
                        Some(Token::Function)
                    } else if (matches!(lang, Language::Toml) && after.starts_with('='))
                        || (matches!(
                            lang,
                            Language::Yaml
                                | Language::Css
                                | Language::JavaScript
                                | Language::TypeScript
                        ) && after.starts_with(':')
                            && !after.starts_with("::"))
                        || (lang == Language::Html && self.tag)
                    {
                        Some(Token::Property)
                    } else {
                        None
                    }
                });
                self.expect_type = matches!(token, Some(Token::Keyword))
                    && matches!(
                        word,
                        "struct" | "enum" | "class" | "trait" | "interface" | "type"
                    );
                self.expect_function = matches!(token, Some(Token::Keyword))
                    && matches!(word, "fn" | "def" | "func" | "function");
                if let Some(token) = token {
                    out.push(Span {
                        start: start + offset,
                        end: i + offset,
                        token,
                    });
                }
                continue;
            }
            self.expect_function = false;
            self.expect_type = false;
            i += ch.len_utf8();
            if b"=+-*/%&|!<>?:^~".contains(&b[start]) {
                out.push(Span {
                    start: start + offset,
                    end: i + offset,
                    token: Token::Operator,
                });
            }
        }
        if lang == Language::Fortran && self.mode.is_some() && !text.trim_end().ends_with('&') {
            self.mode = None;
        }
        out
    }
}
fn ident_char(c: char, lang: Language) -> bool {
    c.is_alphanumeric()
        || c == '_'
        || c == '$'
        || ('\u{300}'..='\u{36f}').contains(&c)
        || (c == '-'
            && matches!(
                lang,
                Language::PowerShell
                    | Language::Css
                    | Language::Html
                    | Language::Yaml
                    | Language::Toml
            ))
}
fn quote(lang: Language, rest: &str) -> Option<(usize, Mode)> {
    if lang == Language::Cpp {
        for prefix in ["R\"", "u8R\"", "uR\"", "UR\"", "LR\""] {
            if let Some(tail) = rest.strip_prefix(prefix) {
                let n = tail.find('(')?;
                let delimiter = &tail[..n];
                if n > 16
                    || delimiter
                        .bytes()
                        .any(|b| b <= 32 || matches!(b, b'\\' | b')'))
                {
                    return None;
                }
                return Some((
                    prefix.len() + n + 1,
                    Mode::Quote {
                        end: format!("){delimiter}\""),
                        escape: None,
                        multiline: true,
                        doubled: false,
                    },
                ));
            }
        }
    }
    if lang == Language::Elixir && rest.starts_with('~') {
        let letters = rest[1..]
            .bytes()
            .take_while(u8::is_ascii_alphabetic)
            .count();
        if letters > 0 && letters <= 16 {
            let n = letters + 1;
            let tail = &rest[n..];
            let (size, end) = if tail.starts_with("\"\"\"") {
                (3, "\"\"\"")
            } else if tail.starts_with("'''") {
                (3, "'''")
            } else {
                (
                    1,
                    match tail.as_bytes().first()? {
                        b'/' => "/",
                        b'|' => "|",
                        b'"' => "\"",
                        b'\'' => "'",
                        b'(' => ")",
                        b'[' => "]",
                        b'{' => "}",
                        b'<' => ">",
                        _ => return None,
                    },
                )
            };
            return Some((
                n + size,
                Mode::Quote {
                    end: end.into(),
                    escape: Some(b'\\'),
                    multiline: true,
                    doubled: false,
                },
            ));
        }
    }
    if lang == Language::Rust {
        let prefix = usize::from(rest.starts_with('b') || rest.starts_with('c'));
        if rest[prefix..].starts_with('r') {
            let hashes = rest[prefix + 1..]
                .bytes()
                .take_while(|b| *b == b'#')
                .count();
            let n = prefix + 1 + hashes;
            if rest.as_bytes().get(n) == Some(&b'"') {
                return Some((
                    n + 1,
                    Mode::Quote {
                        end: format!("\"{}", "#".repeat(hashes)),
                        escape: None,
                        multiline: true,
                        doubled: false,
                    },
                ));
            }
        }
        if let Some(quoted) = rest.strip_prefix('\'') {
            let mut chars = quoted.char_indices();
            let (_, c) = chars.next()?;
            if c != '\\' && chars.next().is_none_or(|(_, c)| c != '\'') {
                return None;
            }
        }
    }
    if lang == Language::PowerShell
        && (rest.starts_with("@\"") || rest.starts_with("@'"))
        && rest[2..].trim().is_empty()
    {
        return Some((2, Mode::Here(format!("{}@", &rest[1..2]))));
    }
    let (mut prefix, mut raw) = (0, false);
    if lang == Language::Dart && (rest.starts_with("r\"") || rest.starts_with("r'")) {
        prefix = 1;
        raw = true;
    }
    if lang == Language::Python {
        prefix = rest
            .bytes()
            .take(2)
            .take_while(|b| b"rRbBuUfF".contains(b))
            .count();
        raw = rest[..prefix].bytes().any(|b| b == b'r' || b == b'R');
    } else if lang == Language::Rust
        && (rest.starts_with("b\"") || rest.starts_with("c\"") || rest.starts_with("b'"))
    {
        prefix = 1;
    } else if lang == Language::CSharp {
        if rest.starts_with("$@\"") || rest.starts_with("@$\"") {
            prefix = 2;
            raw = true;
        } else if rest.starts_with("@\"") {
            prefix = 1;
            raw = true;
        } else if rest.starts_with("$\"") {
            prefix = 1;
        }
    }
    let s = &rest[prefix..];
    let q = s.chars().next()?;
    if !matches!(q, '\'' | '"' | '`')
        || (q == '\'' && lang == Language::Json)
        || (q == '`'
            && !matches!(
                lang,
                Language::JavaScript | Language::TypeScript | Language::Go | Language::Shell
            ))
    {
        return None;
    }
    let triple = matches!(
        lang,
        Language::Python
            | Language::Toml
            | Language::CSharp
            | Language::Dart
            | Language::Elixir
            | Language::Erlang
    ) && (lang != Language::Erlang || q == '"')
        && (s.starts_with("\"\"\"") || s.starts_with("'''"));
    let n = if triple && matches!(lang, Language::Erlang | Language::CSharp) {
        s.bytes().take_while(|b| *b == q as u8).count()
    } else if triple {
        3
    } else {
        1
    };
    let escape = if raw
        || (lang == Language::Erlang && triple)
        || matches!(
            lang,
            Language::Matlab | Language::Fortran | Language::FortranFixed
        )
        || (q == '`' && lang == Language::Go)
        || (q == '\''
            && matches!(
                lang,
                Language::Toml
                    | Language::Shell
                    | Language::Sql
                    | Language::Yaml
                    | Language::PowerShell
            )) {
        None
    } else if lang == Language::PowerShell {
        Some(b'`')
    } else {
        Some(b'\\')
    };
    Some((
        prefix + n,
        Mode::Quote {
            end: s[..n].into(),
            escape,
            multiline: triple
                || raw && matches!(lang, Language::Rust | Language::CSharp)
                || q == '`'
                || matches!(
                    lang,
                    Language::Rust
                        | Language::Shell
                        | Language::Ruby
                        | Language::Html
                        | Language::Sql
                        | Language::Elixir
                        | Language::Erlang
                        | Language::Fortran
                        | Language::FortranFixed
                ),
            doubled: matches!(
                lang,
                Language::Matlab | Language::Fortran | Language::FortranFixed
            ) || (lang == Language::CSharp && raw)
                || matches!(lang, Language::Sql | Language::Yaml | Language::PowerShell)
                    && q == '\'',
        },
    ))
}

pub fn prepare(file: &mut FilePatch) {
    file.syntax_limited = false;
    file.syntax_spans = 0;
    let Some(language) = Language::detect(&file.label) else {
        return;
    };
    file.syntax_language = Some(language.label());
    let (mut old, mut new) = (Lexer::new(language), Lexer::new(language));
    let (mut bytes, mut spans) = (0, 0);
    for line in Arc::make_mut(&mut file.lines) {
        line.syntax.clear();
        if line.kind == Kind::Hunk {
            old = Lexer::new(language);
            new = Lexer::new(language);
            continue;
        }
        if line.kind == Kind::Header {
            continue;
        }
        bytes += line.text.len();
        if bytes > MAX_BYTES || spans >= MAX_SPANS {
            file.syntax_limited = true;
            continue;
        }
        let offset = usize::from(matches!(
            line.text.as_bytes().first(),
            Some(b'+' | b'-' | b' ')
        ));
        let text = &line.text[offset..];
        line.syntax = match line.kind {
            Kind::Removed => old.line(text, offset),
            Kind::Added => new.line(text, offset),
            Kind::Context => {
                old.line(text, offset);
                new.line(text, offset)
            }
            _ => Vec::new(),
        };
        spans += line.syntax.len();
        if spans > MAX_SPANS {
            line.syntax.clear();
            file.syntax_limited = true;
        }
        file.syntax_spans += line.syntax.len();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn tokens(lang: Language, text: &str) -> Vec<(String, Token)> {
        Lexer::new(lang)
            .line(text, 0)
            .into_iter()
            .map(|s| (text[s.start..s.end].into(), s.token))
            .collect()
    }
    #[test]
    fn language_keywords_and_roles_are_distinct() {
        assert!(
            !tokens(Language::Python, "fn export null")
                .iter()
                .any(|(_, t)| *t == Token::Keyword)
        );
        for (lang, text, want) in [
            (
                Language::Rust,
                "pub fn 検索(x: &str) -> String",
                vec![
                    ("fn", Token::Keyword),
                    ("検索", Token::Function),
                    ("str", Token::Type),
                ],
            ),
            (
                Language::Python,
                "async def render(label: str):",
                vec![
                    ("async", Token::Keyword),
                    ("render", Token::Function),
                    ("str", Token::Type),
                ],
            ),
            (
                Language::TypeScript,
                "interface View { title: string }",
                vec![
                    ("View", Token::Type),
                    ("title", Token::Property),
                    ("string", Token::Type),
                ],
            ),
            (
                Language::Json,
                "{\"label\": \"你好\", \"count\": 42}",
                vec![
                    ("\"label\"", Token::Property),
                    ("\"你好\"", Token::String),
                    ("42", Token::Number),
                ],
            ),
            (
                Language::PowerShell,
                "param($Name) # comment",
                vec![
                    ("param", Token::Keyword),
                    ("$Name", Token::Property),
                    ("# comment", Token::Comment),
                ],
            ),
            (
                Language::Toml,
                "name = 'hello'",
                vec![("name", Token::Property), ("'hello'", Token::String)],
            ),
            (
                Language::Css,
                "color: #fff;",
                vec![("color", Token::Property)],
            ),
            (
                Language::Sql,
                "SELECT name FROM users",
                vec![("SELECT", Token::Keyword), ("FROM", Token::Keyword)],
            ),
        ] {
            let actual = tokens(lang, text);
            for (word, token) in want {
                assert!(
                    actual.contains(&(word.into(), token)),
                    "{lang:?}: {word} in {actual:?}"
                );
            }
        }
    }
    #[test]
    fn multiline_literals_and_nested_comments() {
        for (lang, first, second, token) in [
            (
                Language::Rust,
                "let s = r##\"first",
                "/* string */ \"##; let n = 2;",
                Token::String,
            ),
            (
                Language::Python,
                "text = r\"\"\"first",
                "# string\"\"\"; return 3",
                Token::String,
            ),
            (
                Language::JavaScript,
                "const value = `first",
                "// string`; return 4",
                Token::String,
            ),
            (
                Language::CSharp,
                "var text = @\"first",
                "\"\"quoted\"\" last\"; return 5;",
                Token::String,
            ),
            (
                Language::PowerShell,
                "$text = @\"",
                "# string",
                Token::String,
            ),
            (
                Language::Rust,
                "/* outer /* inner */",
                "outer */ fn visible() {}",
                Token::Comment,
            ),
        ] {
            let mut lex = Lexer::new(lang);
            lex.line(first, 0);
            let spans = lex.line(second, 0);
            assert_eq!(spans[0].token, token, "{lang:?}");
            assert_eq!(spans[0].start, 0);
        }
        let t = tokens(Language::Rust, "fn f<'a>(x: &'a str) { let c = '文'; }");
        assert!(t.contains(&("'a".into(), Token::Type)));
        assert!(t.contains(&("'文'".into(), Token::String)));
    }
    #[test]
    fn diff_sides_hunk_gaps_and_utf8_are_preserved() {
        let patch=crate::diff::Patch::parse("diff --git a/文.rs b/文.rs\n@@ -1,2 +1,2 @@\n-/* open\n+let 新 = \"你好 🚀 é\";\n-old */\n+return 新;\n@@ -20 +20 @@\n+fn other() {}\n".as_bytes());
        let file = &patch.files[0];
        for line in file.lines.iter() {
            for span in &line.syntax {
                assert!(
                    line.text.is_char_boundary(span.start) && line.text.is_char_boundary(span.end)
                );
            }
        }
        let added = file
            .lines
            .iter()
            .find(|l| l.text.starts_with("+let"))
            .unwrap();
        assert!(added.syntax.iter().any(|s| s.token == Token::Keyword));
        assert!(
            file.lines
                .iter()
                .find(|l| l.text.starts_with("-old"))
                .unwrap()
                .syntax
                .iter()
                .all(|s| s.token == Token::Comment)
        );
        assert!(
            file.lines
                .iter()
                .find(|l| l.text.starts_with("+fn"))
                .unwrap()
                .syntax
                .iter()
                .any(|s| s.token == Token::Function)
        );
        assert_eq!(added.raw_index, 3);
    }
    #[test]
    fn unknown_languages_and_limits_remain_plain_and_bounded() {
        assert_eq!(Language::detect("文/TEST.TSX"), Some(Language::TypeScript));
        assert_eq!(Language::detect("notes.unknown"), None);
        let body = format!(
            "diff --git a/a.rs b/a.rs\n@@ -0,0 +1 @@\n{}",
            format!("+// {}\n", "a".repeat(8000)).repeat(300)
        );
        let p = crate::diff::Patch::parse(body.as_bytes());
        assert!(p.files[0].syntax_limited);
        assert!(p.files[0].lines.last().unwrap().syntax.is_empty());
    }
    #[test]
    fn added_languages_detect_paths_and_keep_comments_out_of_strings() {
        for (path, lang, code, keyword) in [
            (
                "資料/main.DART",
                Language::Dart,
                "final s = r'/* literal */'; // comment",
                "final",
            ),
            (
                "資料/main.exs",
                Language::Elixir,
                "def ready, do: \"# literal\" # comment",
                "def",
            ),
            (
                "資料/main.erl",
                Language::Erlang,
                "case X of 'end' -> \"% literal\" end % comment",
                "case",
            ),
            (
                "資料/main.m",
                Language::Matlab,
                "if ready; x = 'it''s % literal'; end % comment",
                "if",
            ),
            (
                "資料/main.F90",
                Language::Fortran,
                "PRINT *, 'it''s ! literal' ! comment",
                "PRINT",
            ),
        ] {
            assert_eq!(Language::detect(path), Some(lang));
            let t = tokens(lang, code);
            assert!(
                t.contains(&(keyword.into(), Token::Keyword)),
                "{lang:?} {t:?}"
            );
            assert_eq!(t.last().unwrap().1, Token::Comment);
            assert_eq!(t.iter().filter(|(_, t)| *t == Token::Comment).count(), 1);
            assert!(t.iter().any(|(_, t)| *t == Token::String));
        }
    }
    #[test]
    fn dart_nested_comments_raw_heredocs_and_elixir_sigils_keep_state() {
        let mut lex = Lexer::new(Language::Dart);
        lex.line("/* outer /* inner */", 0);
        let text = "outer */ final value = r'''raw \\";
        let t = lex.line(text, 0);
        assert_eq!(t[0].token, Token::Comment);
        assert!(t.iter().any(|s| s.token == Token::Keyword));
        let text = "// still literal '''; return;";
        let t = lex.line(text, 0);
        assert_eq!(t[0].token, Token::String);
        assert!(t.iter().any(|s| s.token == Token::Keyword));
        let mut lex = Lexer::new(Language::Elixir);
        lex.line("@doc \"\"\"", 0);
        assert_eq!(lex.line("# text with 世界", 0)[0].token, Token::String);
        lex.line("\"\"\"", 0);
        assert!(
            tokens(Language::Elixir, "~r{[a-z]+#literal}i # comment")
                .contains(&("~r{[a-z]+#literal}".into(), Token::String))
        );
        for lang in [Language::Dart, Language::Python] {
            let mut lex = Lexer::new(lang);
            lex.line("r'unterminated", 0);
            assert_eq!(
                lex.line("# next", 0).first().map(|s| s.token),
                if lang == Language::Python {
                    Some(Token::Comment)
                } else {
                    None
                }
            );
        }
    }
    #[test]
    fn matlab_transposes_and_whole_line_block_delimiters_are_distinct() {
        let t = tokens(
            Language::Matlab,
            "y = matrix'; z = matrix.'; s = 'it''s ready';",
        );
        assert_eq!(t.iter().filter(|(_, k)| *k == Token::String).count(), 1);
        assert!(t.contains(&("'".into(), Token::Operator)));
        assert!(t.contains(&(".'".into(), Token::Operator)));
        assert_eq!(
            tokens(Language::Matlab, "['a' 'b']")
                .iter()
                .filter(|(_, k)| *k == Token::String)
                .count(),
            2
        );
        let mut lex = Lexer::new(Language::Matlab);
        lex.line("  %{ ", 0);
        assert_eq!(lex.line("function hidden", 0)[0].token, Token::Comment);
        lex.line("%} extra", 0);
        assert_eq!(lex.line("still hidden", 0)[0].token, Token::Comment);
        lex.line(" %} ", 0);
        assert_eq!(lex.line("function visible", 0)[0].token, Token::Keyword);
        let mut lex = Lexer::new(Language::Matlab);
        lex.line("x=1; %{", 0);
        assert_eq!(lex.line("function visible", 0)[0].token, Token::Keyword);
    }
    #[test]
    fn fortran_fixed_comments_continuations_and_erlang_character_literals() {
        assert_eq!(Language::detect("solver.F"), Some(Language::FortranFixed));
        let mut fixed = Lexer::new(Language::FortranFixed);
        assert_eq!(
            fixed.line("C column-one comment", 0)[0].token,
            Token::Comment
        );
        assert!(tokens(Language::Fortran, "CALL run()").contains(&("CALL".into(), Token::Keyword)));
        let mut free = Lexer::new(Language::Fortran);
        free.line("PRINT *, 'start &", 0);
        assert_eq!(
            free.line("! between continued lines", 0)[0].token,
            Token::Comment
        );
        assert_eq!(free.line(" &end'", 0)[0].token, Token::String);
        assert!(
            free.line("CALL run()", 0)
                .iter()
                .any(|s| s.token == Token::Keyword)
        );
        let t = tokens(Language::Erlang, "[$%, $\", $\\'] % comment");
        assert_eq!(t.iter().filter(|(_, k)| *k == Token::String).count(), 3);
        assert_eq!(t.iter().filter(|(_, k)| *k == Token::Comment).count(), 1);
    }
    #[test]
    fn cpp_raw_delimiters_and_new_language_diff_sides_are_bounded() {
        let mut lex = Lexer::new(Language::Cpp);
        lex.line("auto s = u8R\"tag(世界 \"quotes\"", 0);
        let text = "/* literal */)tag\"; return;";
        let t = lex.line(text, 0);
        assert_eq!(t[0].token, Token::String);
        assert!(t.iter().any(|s| s.token == Token::Keyword));
        let patch=crate::diff::Patch::parse("diff --git a/test.dart b/test.dart\n@@ -1,2 +1,2 @@\n-/* open\n+final s = '世界 🚀 é';\n-old */\n+return s;\n@@ -20 +20 @@\n+final other = 1;\n".as_bytes());
        let f = &patch.files[0];
        assert_eq!(f.syntax_language, Some("Dart"));
        for line in f.lines.iter() {
            for s in &line.syntax {
                assert!(line.text.is_char_boundary(s.start) && line.text.is_char_boundary(s.end));
            }
        }
        assert!(
            f.lines
                .iter()
                .find(|l| l.text.starts_with("+return"))
                .unwrap()
                .syntax
                .iter()
                .any(|s| s.token == Token::Keyword)
        );
        assert!(
            f.lines
                .iter()
                .find(|l| l.text.starts_with("-old"))
                .unwrap()
                .syntax
                .iter()
                .all(|s| s.token == Token::Comment)
        );
    }
    #[test]
    fn markup_attributes_comments_and_strings() {
        let mut lex = Lexer::new(Language::Html);
        let text = "<div class=\"你好\">plain <!-- comment";
        let spans = lex.line(text, 0);
        assert!(
            spans
                .iter()
                .any(|s| s.token == Token::Property && &text[s.start..s.end] == "class")
        );
        assert_eq!(spans.last().unwrap().token, Token::Comment);
        assert_eq!(lex.line("still comment -->", 0)[0].token, Token::Comment);
    }
}
