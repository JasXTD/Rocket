//! Static Lua source references; neither the interpreter nor its module searchers run.
use super::{CancellationToken, Result};
use crate::hosting::json::{self, Json};

pub(super) const MANIFEST_BYTES: usize = 32 * 1024;
pub(super) fn binary(bytes: &[u8]) -> bool {
    bytes.first() == Some(&0x1b) || bytes.contains(&0)
}

pub(super) fn integration_manifest(source: &str) -> bool {
    if source.len() > MANIFEST_BYTES {
        return false;
    }
    let Some(value) = json::parse(source.as_bytes()) else {
        return false;
    };
    value.u64("api") == Some(1)
        && ["id", "name", "version"]
            .iter()
            .all(|key| value.str(key).is_some_and(|s| !s.trim().is_empty()))
        && matches!(value.get("permissions"), Some(Json::Object(_)))
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Token<'a> {
    Name(&'a str),
    Literal(&'a str),
    Punct(u8),
    Other,
    End,
}
use Token::*;

#[derive(Clone)]
struct Lexer<'a> {
    rest: &'a [u8],
}

impl<'a> Lexer<'a> {
    fn new(source: &'a str) -> Self {
        let source = source.strip_prefix('\u{feff}').unwrap_or(source);
        let mut lexer = Self {
            rest: source.as_bytes(),
        };
        if lexer.rest.starts_with(b"#!") {
            lexer.line();
        }
        lexer
    }

    fn line(&mut self) {
        let end = self
            .rest
            .iter()
            .position(|b| matches!(b, b'\r' | b'\n'))
            .unwrap_or(self.rest.len());
        self.rest = &self.rest[end..];
    }

    // The opening '[' was consumed. A valid long bracket closes only at the same level.
    fn long(&mut self) -> Option<Token<'a>> {
        let equals = self.rest.iter().take_while(|&&b| b == b'=').count();
        if self.rest.get(equals) != Some(&b'[') {
            return None;
        }
        let body = &self.rest[equals + 1..];
        let mut at = 0;
        while let Some(next) = body[at..].iter().position(|&b| b == b']') {
            let start = at + next;
            at = start + 1;
            let found = body[at..].iter().take_while(|&&b| b == b'=').count();
            at += found;
            if found == equals && body.get(at) == Some(&b']') {
                self.rest = &body[at + 1..];
                return Some(Literal(std::str::from_utf8(&body[..start]).expect("source is UTF-8")));
            }
        }
        self.rest = &[];
        Some(Other)
    }

    fn next(&mut self) -> Token<'a> {
        loop {
            let input = self.rest;
            let Some((&b, rest)) = input.split_first() else {
                return End;
            };
            self.rest = rest;
            match b {
                b if b.is_ascii_whitespace() => continue,
                b'-' if rest.first() == Some(&b'-') => {
                    self.rest = &rest[1..];
                    if let Some(tail) = self.rest.strip_prefix(b"[") {
                        self.rest = tail;
                        if self.long().is_some() {
                            continue;
                        }
                    }
                    self.line();
                }
                b'[' => return self.long().unwrap_or(Punct(b)),
                b'\'' | b'"' => {
                    let mut at = 0;
                    let mut escaped = false;
                    while at < rest.len() {
                        match rest[at] {
                            c if c == b => {
                                self.rest = &rest[at + 1..];
                                return if escaped {
                                    Other
                                } else {
                                    Literal(std::str::from_utf8(&rest[..at]).expect("source is UTF-8"))
                                };
                            }
                            b'\\' => {
                                escaped = true;
                                at += 2;
                            }
                            _ => at += 1,
                        }
                    }
                    self.rest = &[];
                    return Other;
                }
                b if b.is_ascii_alphanumeric() || b == b'_' || b >= 128 => {
                    let len = rest
                        .iter()
                        .take_while(|&&c| c.is_ascii_alphanumeric() || c == b'_' || c >= 128)
                        .count();
                    self.rest = &rest[len..];
                    return Name(std::str::from_utf8(&input[..len + 1]).expect("source is UTF-8"));
                }
                b'.' if rest.first() == Some(&b'.') => {
                    self.rest = rest[1..].strip_prefix(b".").unwrap_or(&rest[1..]);
                    return Other;
                }
                _ => return Punct(b),
            }
        }
    }
}

fn loader_name(t: Token<'_>) -> bool {
    matches!(t, Name("require" | "_ENV"))
}

fn changes_search(token: Token<'_>, mut tail: Lexer<'_>) -> bool {
    if !matches!(token, Name("package" | "_G" | "_ENV")) {
        return false;
    }
    let key = match tail.next() {
        Punct(b'.') => match tail.next() {
            Name(s) => s,
            _ => return false,
        },
        Punct(b'[') => match tail.next() {
            Literal(s) if tail.next() == Punct(b']') => s,
            _ => return false,
        },
        _ => return false,
    };
    let relevant = match token {
        Name("package") => matches!(key, "path" | "searchers" | "preload" | "loaded"),
        _ => key == "require",
    };
    if !relevant {
        return false;
    }
    let mut next = tail.next();
    match next {
        Punct(b'[') => {
            tail.next();
            if tail.next() != Punct(b']') {
                return false;
            }
            next = tail.next();
        }
        Punct(b'.') => {
            tail.next();
            next = tail.next();
        }
        _ => (),
    }
    next == Punct(b'=') && tail.next() != Punct(b'=')
}

// A file which locally replaces require or its environment has no reliable global
// loader throughout. Conservatively omit its calls instead of guessing scope.
fn rebinds(previous: Token<'_>, token: Token<'_>, mut tail: Lexer<'_>, table: bool) -> bool {
    match token {
        Name("local" | "for") => loop {
            let t = tail.next();
            if loader_name(t) {
                return true;
            }
            if !matches!(t, Name(_)) {
                return false;
            }
            let mut separator = tail.next();
            if separator == Punct(b'<') {
                tail.next();
                if tail.next() != Punct(b'>') {
                    return false;
                }
                separator = tail.next();
            }
            if separator != Punct(b',') {
                return false;
            }
        },
        Name("function") => {
            let first = tail.next();
            let mut t = first;
            if matches!(first, Name(_)) {
                t = tail.next();
                if loader_name(first) && t == Punct(b'(') {
                    return true;
                }
                while matches!(t, Punct(b'.' | b':')) && matches!(tail.next(), Name(_)) {
                    t = tail.next();
                }
            }
            if t == Punct(b'(') {
                loop {
                    match tail.next() {
                        t if loader_name(t) => return true,
                        Name(_) | Punct(b',') => (),
                        _ => break,
                    }
                }
            }
            false
        }
        t if loader_name(t)
            && !matches!(previous, Punct(b'.' | b':'))
            && !(table && matches!(previous, Punct(b'{' | b',' | b';'))) =>
        {
            let mut t = tail.next();
            // Bound lookahead even for a generated list containing repeated loader names.
            for _ in 0..32 {
                if t != Punct(b',') {
                    break;
                }
                if !matches!(tail.next(), Name(_)) {
                    return false;
                }
                t = tail.next();
            }
            if t == Punct(b',') {
                return true;
            }
            t == Punct(b'=') && tail.next() != Punct(b'=')
        }
        _ => false,
    }
}

fn module_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 512
        && name.split(['.', '/']).all(|s| {
            !s.is_empty()
                && s.chars()
                    .all(|c| c.is_alphanumeric() || matches!(c, '-' | '_' | ' ') || (!c.is_ascii() && !c.is_control()))
        })
}

pub(super) fn read(source: &str, cancel: &CancellationToken, mut load: impl FnMut(&str) -> bool) -> Result<()> {
    if !source.contains("require") {
        return cancel.check();
    }
    for bindings in [true, false] {
        let mut lexer = Lexer::new(source);
        let mut previous = End;
        let mut count = 0;
        let mut tables = 0usize;
        loop {
            if count % 512 == 0 {
                cancel.check()?;
            }
            count += 1;
            let token = lexer.next();
            if token == End {
                break;
            }
            if bindings {
                if rebinds(previous, token, lexer.clone(), tables > 0)
                    || (!matches!(previous, Punct(b'.' | b':')) && changes_search(token, lexer.clone()))
                {
                    return cancel.check();
                }
                match token {
                    Punct(b'{') => tables += 1,
                    Punct(b'}') => tables = tables.saturating_sub(1),
                    _ => (),
                }
            } else if token == Name("require") && !matches!(previous, Punct(b'.' | b':')) {
                let mut args = lexer.clone();
                let name = match args.next() {
                    Literal(s) => Some(s),
                    Punct(b'(') => match args.next() {
                        Literal(s) if args.next() == Punct(b')') => Some(s),
                        _ => None,
                    },
                    _ => None,
                };
                if let Some(s) = name.filter(|s| module_name(s))
                    && !load(s)
                {
                    return cancel.check();
                }
            }
            previous = token;
        }
    }
    cancel.check()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn loads(source: &str) -> Vec<String> {
        let mut out = Vec::new();
        read(source, &CancellationToken::new(), |s| {
            out.push(s.to_owned());
            true
        })
        .unwrap();
        out
    }

    #[test]
    fn literal_calls_skip_examples_and_dynamic_names() {
        let sample = r###"#!/usr/bin/lua require("shebang")
            --[==[ require("comment") ]=] require("still-comment") ]==]
            local docs = [=[ require "quoted" ]=]
            local quoted = "escaped \" require('still-quoted')"
            local mod = require -- comment before the argument
                ("ui.面板")
            require 'pkg/sub'  require [=[a.b]=]
            local r = x.require('field') .. obj:require 'method' .. require("real")
            local obj = { require = function() end }
            require("composed" .. suffix); require(runtime); require "a\046b"
            require('../dir')  require('a..b')  require('/outside')  require('C:\tool')
            require(false, 'extra'); require('first', 'second')"###;
        assert_eq!(loads(sample), ["ui.面板", "pkg/sub", "a.b", "real"]);
    }

    #[test]
    fn replacement_loaders_do_not_imply_a_source_file() {
        for bind in [
            "local require = load",
            "local one <const>, require = 1, load",
            "function obj:run(arg, require) end",
            "local function require() end",
            "for require in iter() do end",
            "local _ENV = {}",
            "require, x = fn, 2",
            "_G.require = custom",
            "_ENV['require'] = custom",
            "package.path = 'other/?.lua'",
            "package.preload['module'] = custom",
            "package.searchers[1] = custom",
        ] {
            assert!(loads(&format!("{bind}\nrequire('module')")).is_empty(), "{bind}");
        }
        assert_eq!(
            loads("function t.require(a) end; local p = package.path; if require == original then require'module' end"),
            ["module"]
        );
    }

    #[test]
    fn cancellation_and_link_budgets_can_stop_a_single_source() {
        let single = CancellationToken::new();
        assert!(matches!(
            read("require 'one'", &single, |_| {
                single.cancel();
                true
            }),
            Err(crate::Error::Cancelled)
        ));
        let cancel = CancellationToken::new();
        let source = "require('helper')\n".repeat(3000);
        let mut count = 0;
        assert!(matches!(
            read(&source, &cancel, |_| {
                count += 1;
                cancel.cancel();
                true
            }),
            Err(crate::Error::Cancelled)
        ));
        assert!((1..3000).contains(&count));
        count = 0;
        read(&source, &CancellationToken::new(), |_| {
            count += 1;
            count < 4
        })
        .unwrap();
        assert_eq!(count, 4);
    }
}
