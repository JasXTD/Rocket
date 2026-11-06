//! The interface language. English is the key: `tr("Stage all")` shows the active
//! language's translation, or the English itself while it has none. Each language's
//! text lives in `locales/<tag>/<area>.json` and is built into the program.
//!
//! Rocket's code starts in English. Only the programs' `main` calls [`start`], so unit
//! tests and the MCP test host read English unless `ROCKET_LANG` asks otherwise.

use std::{
    collections::HashMap,
    fmt::{Display, Write},
    sync::{
        Mutex, OnceLock,
        atomic::{AtomicBool, AtomicU8, Ordering::Relaxed},
    },
};

/// A language Rocket's interface can show.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Language {
    English,
    ChineseSimplified,
    Japanese,
    /// Accented, bracketed and stretched English, for finding text that skips [`tr`] and
    /// labels that clip. Only `ROCKET_LANG=pseudo` shows it.
    Pseudo,
}

impl Language {
    /// What Preferences can offer, in its order.
    pub const ALL: [Language; 3] = [Language::English, Language::ChineseSimplified, Language::Japanese];

    /// The name saved state and `ROCKET_LANG` use.
    pub fn tag(self) -> &'static str {
        match self {
            Language::English => "en",
            Language::ChineseSimplified => "zh-Hans",
            Language::Japanese => "ja",
            Language::Pseudo => "pseudo",
        }
    }

    pub fn from_tag(tag: &str) -> Option<Self> {
        [Language::English, Language::ChineseSimplified, Language::Japanese, Language::Pseudo]
            .into_iter()
            .find(|language| language.tag().eq_ignore_ascii_case(tag))
    }

    /// The language's own name, so anyone can find theirs whatever the interface shows.
    pub fn name(self) -> &'static str {
        match self {
            Language::English => "English",
            Language::ChineseSimplified => "简体中文",
            Language::Japanese => "日本語",
            Language::Pseudo => "Pseudo",
        }
    }

    /// The language a system language name stands for, written the Windows way (zh-CN,
    /// zh-Hans-SG, ja-JP) or the Linux way (zh_CN.UTF-8). Traditional Chinese has none.
    pub fn from_system(name: &str) -> Option<Self> {
        let name = name.split(['.', '@']).next().unwrap_or_default().to_ascii_lowercase().replace('_', "-");
        let mut parts = name.split('-');
        match parts.next()? {
            "en" | "c" | "posix" => Some(Language::English),
            "ja" => Some(Language::Japanese),
            "zh" => {
                let rest: Vec<&str> = parts.collect();
                if rest.contains(&"hans") {
                    Some(Language::ChineseSimplified)
                } else if rest.contains(&"hant") {
                    None
                } else {
                    match rest.first() {
                        None | Some(&("cn" | "sg" | "my")) => Some(Language::ChineseSimplified),
                        Some(_) => None,
                    }
                }
            }
            _ => None,
        }
    }

    fn code(self) -> u8 {
        match self {
            Language::English => 0,
            Language::ChineseSimplified => 1,
            Language::Japanese => 2,
            Language::Pseudo => 3,
        }
    }

    fn from_code(code: u8) -> Self {
        match code {
            1 => Language::ChineseSimplified,
            2 => Language::Japanese,
            3 => Language::Pseudo,
            _ => Language::English,
        }
    }
}

/// Languages whose text is complete, offered without `ROCKET_LANG`. A language joins
/// once every string has its translation; the catalog test then holds it there.
const RELEASED: &[Language] = &[Language::English];

static CURRENT: AtomicU8 = AtomicU8::new(0);
static SYSTEM: AtomicU8 = AtomicU8::new(0);
static PREVIEW: AtomicBool = AtomicBool::new(false);
static JAPANESE_HAN: AtomicBool = AtomicBool::new(false);

/// Picks the interface language once, before the first window: the first of the
/// system's display languages that Rocket offers, else English. `ROCKET_LANG` (en,
/// zh-Hans, ja or pseudo) stands in for the system's language and lets Preferences
/// offer the languages that are not complete yet. `region` is the Region format's
/// locale; it only decides the Han font order of an English interface.
pub fn start(display: &[String], region: Option<&str>) {
    let forced = std::env::var("ROCKET_LANG").ok().and_then(|tag| Language::from_tag(tag.trim()));
    PREVIEW.store(forced.is_some(), Relaxed);
    let system = forced.unwrap_or_else(|| pick(display.iter().map(String::as_str), RELEASED));
    SYSTEM.store(system.code(), Relaxed);
    CURRENT.store(system.code(), Relaxed);
    let japanese = display.iter().map(String::as_str).chain(region).find_map(japanese_forms).unwrap_or(false);
    JAPANESE_HAN.store(japanese, Relaxed);
}

/// The first of `names` that stands for an offered language, else English.
fn pick<'a>(names: impl Iterator<Item = &'a str>, offered: &[Language]) -> Language {
    names.filter_map(Language::from_system).find(|l| offered.contains(l)).unwrap_or(Language::English)
}

/// Whether a system language reads Han characters in their Japanese forms (true) or
/// their Chinese ones (false); None when it reads neither.
fn japanese_forms(name: &str) -> Option<bool> {
    match name.split(['-', '_', '.', '@']).next()?.to_ascii_lowercase().as_str() {
        "ja" => Some(true),
        "zh" => Some(false),
        _ => None,
    }
}

/// Languages Preferences offers: the released ones, or all of them under `ROCKET_LANG`.
pub fn offered() -> &'static [Language] {
    if PREVIEW.load(Relaxed) { &Language::ALL } else { RELEASED }
}

/// The language "Match Windows" shows.
pub fn system() -> Language {
    Language::from_code(SYSTEM.load(Relaxed))
}

/// What a saved choice shows: the choice while it is offered, else the system's language.
pub fn resolve(choice: Option<Language>) -> Language {
    resolve_in(choice, offered(), system())
}

fn resolve_in(choice: Option<Language>, offered: &[Language], system: Language) -> Language {
    choice.filter(|language| offered.contains(language)).unwrap_or(system)
}

#[cfg(test)]
thread_local! {
    static TEST_LANGUAGE: std::cell::Cell<Option<Language>> = const { std::cell::Cell::new(None) };
}

/// The language the interface shows now.
pub fn current() -> Language {
    #[cfg(test)]
    if let Some(language) = TEST_LANGUAGE.with(std::cell::Cell::get) {
        return language;
    }
    Language::from_code(CURRENT.load(Relaxed))
}

/// Shows `language` from the next string looked up on, in every window and thread.
pub fn set(language: Language) {
    CURRENT.store(language.code(), Relaxed);
}

/// Runs `f` with this test thread showing `language`; parallel tests keep English.
#[cfg(test)]
pub(crate) fn scoped<R>(language: Language, f: impl FnOnce() -> R) -> R {
    struct Restore(Option<Language>);
    impl Drop for Restore {
        fn drop(&mut self) {
            TEST_LANGUAGE.with(|l| l.set(self.0));
        }
    }
    let _restore = Restore(TEST_LANGUAGE.with(|l| l.replace(Some(language))));
    f()
}

/// Japanese fonts go before Chinese ones: for a Japanese interface, and for an English one
/// on a system whose first Han language is Japanese.
pub(crate) fn japanese_han() -> bool {
    match current() {
        Language::Japanese => true,
        Language::ChineseSimplified => false,
        Language::English | Language::Pseudo => JAPANESE_HAN.load(Relaxed),
    }
}

/// Each language's files by area, both languages for every area. Shared words such as
/// Cancel live in common; the rest go with the part of Rocket that shows them.
macro_rules! files {
    ($($area:literal),* $(,)?) => {
        &[$(
            (Language::ChineseSimplified, $area, include_str!(concat!("../locales/zh-Hans/", $area, ".json"))),
            (Language::Japanese, $area, include_str!(concat!("../locales/ja/", $area, ".json"))),
        )*]
    };
}
const FILES: &[(Language, &str, &str)] =
    files!["common", "menus", "chrome", "views", "diff", "preferences"];

fn catalog(language: Language) -> &'static HashMap<String, String> {
    static CHINESE: OnceLock<HashMap<String, String>> = OnceLock::new();
    static JAPANESE: OnceLock<HashMap<String, String>> = OnceLock::new();
    let cell = if language == Language::Japanese { &JAPANESE } else { &CHINESE };
    cell.get_or_init(|| {
        // A file that does not read keeps its strings in English; the catalog test fails on it.
        FILES
            .iter()
            .filter(|(l, ..)| *l == language)
            .filter_map(|(_, _, text)| serde_json::from_str::<HashMap<String, String>>(text).ok())
            .flatten()
            .collect()
    })
}

/// The interface text for `english`, a literal or a [`mark`]ed one.
pub fn tr(english: &'static str) -> &'static str {
    match current() {
        Language::English => english,
        Language::Pseudo => pseudo(english),
        language => catalog(language).get(english).map_or(english, String::as_str),
    }
}

/// Marks text in a constant table for the catalog test; [`tr`] translates it where it shows.
pub const fn mark(english: &'static str) -> &'static str {
    english
}

/// [`trn!`]'s phrase: English picks by count; the catalogs hold one form, under `other`.
pub fn plural(count: u64, one: &'static str, other: &'static str) -> &'static str {
    let english = if count == 1 { one } else { other };
    match current() {
        Language::English => english,
        Language::Pseudo => pseudo(english),
        language => catalog(language).get(other).map_or(english, String::as_str),
    }
}

/// `template` with each `{name}` in `values` replaced; other braces stay as written.
pub fn fill(template: &str, values: &[(&str, &dyn Display)]) -> String {
    let mut out = String::with_capacity(template.len() + 16);
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        out.push_str(&rest[..open]);
        rest = &rest[open..];
        let found = rest[1..].find('}').and_then(|close| {
            let name = &rest[1..1 + close];
            values.iter().find(|(n, _)| *n == name).map(|(_, value)| (close, *value))
        });
        match found {
            Some((close, value)) => {
                let _ = write!(out, "{value}");
                rest = &rest[close + 2..];
            }
            None => {
                out.push('{');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// Translated text with named slots: `trf!("Pushed {branch} to {remote}", branch, remote = name)`.
/// A translation may move a slot; a slot given no value stays as written.
macro_rules! trf {
    ($text:literal $(, $name:ident $(= $value:expr)?)* $(,)?) => {
        $crate::i18n::fill(
            $crate::i18n::tr($text),
            &[$((stringify!($name), &$crate::i18n::trf!(@value $name $(= $value)?) as &dyn ::std::fmt::Display)),*],
        )
    };
    (@value $name:ident = $value:expr) => { $value };
    (@value $name:ident) => { $name };
}
pub(crate) use trf;

/// A counted phrase: `trn!(count, "{n} file", "{n} files")`, `{n}` being the count.
/// Chinese and Japanese have one form, which their catalogs key by the second phrase.
macro_rules! trn {
    ($count:expr, $one:literal, $other:literal $(, $name:ident $(= $value:expr)?)* $(,)?) => {{
        let count = $count;
        $crate::i18n::fill(
            $crate::i18n::plural(count as u64, $one, $other),
            &[("n", &count as &dyn ::std::fmt::Display)
                $(, (stringify!($name), &$crate::i18n::trf!(@value $name $(= $value)?) as &dyn ::std::fmt::Display))*],
        )
    }};
}
pub(crate) use trn;

fn pseudo(english: &'static str) -> &'static str {
    static MADE: Mutex<Option<HashMap<&'static str, &'static str>>> = Mutex::new(None);
    let mut made = MADE.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    // Each English string is stretched once and kept, as catalog text is.
    *made.get_or_insert_with(HashMap::new).entry(english).or_insert_with(|| Box::leak(stretch(english).into_boxed_str()))
}

/// "Stage all" → "[Šţåĝé åļļ ~~~~]": accented letters, slots kept, about 40% longer.
fn stretch(english: &str) -> String {
    let mut out = String::from("[");
    let mut slot = false;
    for c in english.chars() {
        slot = match c {
            '{' => true,
            '}' => false,
            _ => slot,
        };
        out.push(if slot { c } else { accent(c) });
    }
    out.push(' ');
    out.extend(std::iter::repeat_n('~', english.chars().count().div_ceil(3).max(2)));
    out.push(']');
    out
}

fn accent(c: char) -> char {
    const PLAIN: &str = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ";
    const ACCENTED: &str = "åƀçðéƒĝĥîĵķļɱñöþǫŕšţûṽŵẋýžÅƁÇÐÉƑĜĤÎĴĶĻṀÑÖÞǪŔŠŢÛṼŴẊÝŽ";
    PLAIN.chars().position(|p| p == c).and_then(|i| ACCENTED.chars().nth(i)).unwrap_or(c)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        collections::{BTreeMap, BTreeSet},
        path::Path,
    };

    #[test]
    fn system_names_pick_the_languages_rocket_offers() {
        use Language::*;
        for (name, expected) in [
            ("zh-CN", Some(ChineseSimplified)),
            ("zh-SG", Some(ChineseSimplified)),
            ("zh-Hans-HK", Some(ChineseSimplified)),
            ("zh", Some(ChineseSimplified)),
            ("zh_CN.UTF-8", Some(ChineseSimplified)),
            ("zh-TW", None),
            ("zh-HK", None),
            ("zh-Hant-TW", None),
            ("ja-JP", Some(Japanese)),
            ("ja_JP.UTF-8@cjk", Some(Japanese)),
            ("en-GB", Some(English)),
            ("C.UTF-8", Some(English)),
            ("fr-FR", None),
            ("", None),
        ] {
            assert_eq!(Language::from_system(name), expected, "{name}");
        }
        let names = ["zh-TW", "ja-JP", "en-US"];
        assert_eq!(pick(names.into_iter(), &Language::ALL), Japanese, "Traditional Chinese passes to the next");
        assert_eq!(pick(names.into_iter(), &[English]), English, "an unreleased language is passed over");
        assert_eq!(pick(["fr-FR", "de-DE"].into_iter(), &Language::ALL), English);
        assert_eq!(pick(["fr-FR", "zh-CN"].into_iter(), &Language::ALL), ChineseSimplified);
        assert_eq!(resolve_in(Some(Japanese), &Language::ALL, English), Japanese);
        assert_eq!(resolve_in(Some(Japanese), &[English], ChineseSimplified), ChineseSimplified, "a choice no longer offered follows the system");
        assert_eq!(resolve_in(None, &Language::ALL, Japanese), Japanese);
        for language in [English, ChineseSimplified, Japanese, Pseudo] {
            assert_eq!(Language::from_tag(language.tag()), Some(language));
            assert_eq!(Language::from_code(language.code()), language);
        }
        assert_eq!(japanese_forms("ja-JP"), Some(true));
        assert_eq!(japanese_forms("zh-TW"), Some(false));
        assert_eq!(japanese_forms("jam"), None);
    }

    #[test]
    fn values_fill_named_slots_and_other_braces_stay() {
        let branch = "main";
        assert_eq!(fill("Pushed {branch} to {remote}", &[("branch", &branch), ("remote", &"origin")]), "Pushed main to origin");
        assert_eq!(fill("{remote} に {branch} をプッシュ", &[("branch", &branch), ("remote", &"origin")]), "origin に main をプッシュ");
        assert_eq!(fill("@{u} and {missing} and {", &[("u", &"x")]), "@x and {missing} and {");
        assert_eq!(trf!("{n} min ago", n = 4), "4 min ago");
        let n = 2;
        assert_eq!(trn!(n, "{n} file", "{n} files"), "2 files");
        assert_eq!(trn!(1, "{n} file in {folder}", "{n} files in {folder}", folder = "src"), "1 file in src");
    }

    #[test]
    fn languages_without_text_fall_back_to_english_and_pseudo_stretches() {
        // Built at run time, so the catalog test does not count it as interface text.
        let unknown: &'static str = Box::leak(String::from("Text no catalog has").into_boxed_str());
        scoped(Language::Japanese, || {
            assert_eq!(current(), Language::Japanese);
            assert_eq!(tr(unknown), unknown);
            assert_eq!(plural(1, unknown, "Other text no catalog has"), unknown, "English's own form while untranslated");
        });
        assert_eq!(current(), Language::English, "the scope restores this thread's language");
        scoped(Language::Pseudo, || {
            let stretched = tr(unknown);
            assert!(stretched.starts_with("[Ţéẋţ ñö çåţåļöĝ ĥåš") && stretched.ends_with(" ~~~~~~~]"), "{stretched}");
            assert!(std::ptr::eq(stretched, tr(unknown)), "each string is stretched once");
            let counted = trn!(3, "{n} file", "{n} files");
            assert_eq!(counted, "[3 ƒîļéš ~~~]", "slots keep their names");
        });
    }

    /// English keys the code passes to `tr`, `mark`, `trf!` and `trn!`, leaving out
    /// comments and test modules.
    fn code_keys() -> BTreeSet<String> {
        let mut keys = BTreeSet::new();
        let mut folders = vec![Path::new(env!("CARGO_MANIFEST_DIR")).join("src")];
        while let Some(folder) = folders.pop() {
            for entry in std::fs::read_dir(&folder).unwrap().flatten() {
                let path = entry.path();
                if path.is_dir() {
                    folders.push(path);
                    continue;
                }
                let name = path.file_name().unwrap().to_string_lossy().into_owned();
                if name.ends_with(".rs") && !name.ends_with("tests.rs") {
                    keys.extend(keys_in(&std::fs::read_to_string(&path).unwrap()));
                }
            }
        }
        keys
    }

    fn keys_in(text: &str) -> Vec<String> {
        let lines: Vec<&str> = text.lines().collect();
        let mut code = String::new();
        let mut in_test = false;
        for (i, line) in lines.iter().enumerate() {
            let next = lines.get(i + 1).map_or("", |l| l.trim_start());
            if !in_test && line.starts_with("#[cfg(") && line.contains("test") && next.starts_with("mod ") && next.trim_end().ends_with('{') {
                in_test = true;
                continue;
            }
            if in_test {
                // A top-level module closes with a lone brace in the first column; code after it counts.
                in_test = line.trim_end() != "}";
                continue;
            }
            if !line.trim_start().starts_with("//") {
                code.push_str(line);
                code.push('\n');
            }
        }
        let mut keys = Vec::new();
        for (marker, literal) in [("tr(", 0), ("mark(", 0), ("trf!(", 0), ("trn!(", 1)] {
            for (at, _) in code.match_indices(marker) {
                if code[..at].chars().next_back().is_some_and(|c| c.is_alphanumeric() || c == '_') {
                    continue;
                }
                let mut rest = &code[at + marker.len()..];
                if literal == 0 && !rest.trim_start().starts_with('"') {
                    continue; // tr(label) with a marked value
                }
                let mut phrases = Vec::new();
                for _ in 0..=literal {
                    let Some(quote) = rest.find('"') else { break };
                    let Some((text, after)) = rust_string(&rest[quote + 1..]) else { break };
                    rest = after;
                    phrases.push(text);
                }
                // trn!'s catalogs key its second phrase only.
                if phrases.len() == literal + 1 {
                    keys.extend(phrases.pop());
                }
            }
        }
        keys
    }

    /// A Rust string literal's text, from just after its opening quote, and what follows it.
    fn rust_string(source: &str) -> Option<(String, &str)> {
        let mut text = String::new();
        let mut chars = source.char_indices();
        while let Some((i, c)) = chars.next() {
            match c {
                '"' => return Some((text, &source[i + 1..])),
                '\\' => match chars.next()?.1 {
                    'n' => text.push('\n'),
                    't' => text.push('\t'),
                    'r' => text.push('\r'),
                    '0' => text.push('\0'),
                    'u' => {
                        let hex: String = chars.by_ref().map(|(_, c)| c).skip(1).take_while(|c| *c != '}').collect();
                        text.push(char::from_u32(u32::from_str_radix(&hex, 16).ok()?)?);
                    }
                    'x' => {
                        let hex: String = chars.by_ref().take(2).map(|(_, c)| c).collect();
                        text.push(char::from(u8::from_str_radix(&hex, 16).ok()?));
                    }
                    '\n' => {
                        // A line continuation drops the next line's indent.
                        let rest = &source[i + 2..];
                        let skip = rest.len() - rest.trim_start().len();
                        for _ in 0..rest[..skip].chars().count() {
                            chars.next();
                        }
                    }
                    other => text.push(other),
                },
                c => text.push(c),
            }
        }
        None
    }

    /// A catalog file's entries in order, so a key written twice is seen.
    fn entries(text: &str) -> Result<Vec<(String, String)>, String> {
        let value: serde_json::Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
        let object = value.as_object().ok_or("not a JSON object")?;
        if let Some((key, _)) = object.iter().find(|(_, v)| !v.is_string()) {
            return Err(format!("{key:?} is not text"));
        }
        // serde_json keeps the last of two equal keys; the strings in order show both.
        let mut strings = Vec::new();
        let mut rest = text;
        while let Some(start) = rest.find('"') {
            let bytes = rest.as_bytes();
            let mut end = start + 1;
            while bytes[end] != b'"' {
                end += if bytes[end] == b'\\' { 2 } else { 1 };
            }
            strings.push(serde_json::from_str::<String>(&rest[start..=end]).map_err(|e| e.to_string())?);
            rest = &rest[end + 1..];
        }
        Ok(strings.chunks(2).map(|pair| (pair[0].clone(), pair[1].clone())).collect())
    }

    /// The `{name}` slots in a phrase.
    fn slots(text: &str) -> BTreeSet<&str> {
        text.match_indices('{')
            .filter_map(|(open, _)| {
                let rest = &text[open + 1..];
                let name = &rest[..rest.find('}')?];
                let identifier = name.chars().next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
                    && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
                identifier.then_some(name)
            })
            .collect()
    }

    fn translations(language: Language, problems: &mut Vec<String>) -> BTreeMap<String, (String, &'static str)> {
        let mut found = BTreeMap::new();
        for (_, area, text) in FILES.iter().filter(|(l, ..)| *l == language) {
            let file = format!("{}/{area}.json", language.tag());
            match entries(text) {
                Err(error) => problems.push(format!("{file}: {error}")),
                Ok(list) => {
                    for (key, value) in list {
                        if let Some((_, other)) = found.insert(key.clone(), (value, *area)) {
                            problems.push(format!("{file}: {key:?} is also in {other}.json"));
                        }
                    }
                }
            }
        }
        found
    }

    #[test]
    fn catalogs_match_the_code() {
        let code = code_keys();
        assert!(code.len() > 100, "the scan found only {} strings", code.len());
        let mut problems = Vec::new();
        for language in [Language::ChineseSimplified, Language::Japanese] {
            let found = translations(language, &mut problems);
            for (key, (value, area)) in &found {
                let file = format!("{}/{area}.json", language.tag());
                if !code.contains(key) {
                    problems.push(format!("{file}: {key:?} is no longer in the code; reworded English needs its translation moved"));
                }
                if slots(key) != slots(value) {
                    problems.push(format!("{file}: {key:?} → {value:?} has other {{slots}}"));
                }
                if value.trim().is_empty() {
                    problems.push(format!("{file}: {key:?} is empty"));
                }
            }
            let missing = code.iter().filter(|key| !found.contains_key(*key)).count();
            eprintln!("{}: {} of {} strings translated", language.tag(), code.len() - missing, code.len());
            if RELEASED.contains(&language) && missing > 0 {
                problems.push(format!("{} is released with {missing} strings untranslated", language.tag()));
            }
        }
        assert!(problems.is_empty(), "{}", problems.join("\n"));
    }

    #[test]
    fn every_catalog_file_is_built_in() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("locales");
        let mut on_disk = BTreeSet::new();
        for folder in std::fs::read_dir(&root).unwrap().flatten().filter(|e| e.path().is_dir()) {
            for file in std::fs::read_dir(folder.path()).unwrap().flatten() {
                let name = file.file_name().to_string_lossy().into_owned();
                if let Some(area) = name.strip_suffix(".json") {
                    on_disk.insert(format!("{}/{area}", folder.file_name().to_string_lossy()));
                }
            }
        }
        let built: BTreeSet<String> = FILES.iter().map(|(l, area, _)| format!("{}/{area}", l.tag())).collect();
        assert_eq!(on_disk, built, "list each locales/<tag>/<area>.json in FILES");
    }

    #[test]
    fn scanning_reads_each_call_form() {
        let source = concat!(
            "let a = tr(\"Stage all\");\n",
            "// tr(\"In a comment\")\n",
            "let b = trf!(\"Pushed {branch}\", branch);\n",
            "let c = trn!(items.len(), \"{n} file\", \"{n} files\");\n",
            "const T: &str = mark(\"Marked \\\"quoted\\\"\\u{2026}\");\n",
            "let d = str(\"not ours\"); let e = tr(label);\n",
            "#[cfg(test)]\nmod tests {\n    fn t() { tr(\"Only in a test\"); }\n}\n",
            "fn after_the_tests() { tr(\"After a test module\"); }\n",
        );
        let mut keys = keys_in(source);
        keys.sort();
        assert_eq!(keys, ["After a test module", "Marked \"quoted\"…", "Pushed {branch}", "Stage all", "{n} files"]);
    }

    /// Text a frame cut short: truncated labels, and text running past its clip.
    fn cut_texts(output: &eframe::egui::FullOutput) -> Vec<String> {
        use eframe::egui::{Rect, Shape};
        fn visit(shape: &Shape, clip: Rect, cut: &mut Vec<String>) {
            match shape {
                Shape::Text(text) => {
                    let rect = text.visual_bounding_rect();
                    let runs_past = rect.left() < clip.right() && rect.right() > clip.right() + 0.5;
                    if text.galley.elided || runs_past {
                        cut.push(text.galley.job.text.clone());
                    }
                }
                Shape::Vec(shapes) => shapes.iter().for_each(|s| visit(s, clip, cut)),
                _ => {}
            }
        }
        let mut cut = Vec::new();
        for clipped in &output.shapes {
            visit(&clipped.shape, clipped.clip_rect, &mut cut);
        }
        cut
    }

    /// Translations cut no text that English shows whole in the main window (menu bar,
    /// toolbar, sidebar, Changes with a staged and an unstaged file, the commit area and
    /// the status bar), from the narrowest supported width up. Pseudo's longer English
    /// prints what it would cut. (The panels shrink to the window, so cut text, not the
    /// laid-out width, shows where a label no longer fits.)
    #[test]
    fn translations_cut_no_text_english_shows_whole() {
        crate::test_common::isolated("ROCKET_I18N_UI_CASE", "i18n::tests::translations_cut_no_text_english_shows_whole", || {
            use crate::app::{Intent, Rocket, View};
            use eframe::egui::{Context, Pos2, RawInput, Rect, vec2};
            use std::time::{Duration, Instant};
            let fixture = crate::test_common::Fixture::new();
            fixture.write("README.md", "# Fixture
");
            fixture.commit("Start the fixture");
            fixture.write("src/retry.rs", "pub fn backoff() -> u64 {
    250
}
");
            fixture.run(&["add", "src/retry.rs"]);
            fixture.write("README.md", "# Fixture

With a second line.
");
            let ctx = Context::default();
            let mut app = Rocket::with_storage(&ctx, vec![fixture.repository.clone()], fixture.root.join("logs"), fixture.root.join("state"));
            let deadline = Instant::now() + Duration::from_secs(40);
            while app.busy() || app.tab().is_some_and(|t| t.snapshot.is_none()) {
                app.poll(&ctx);
                assert!(Instant::now() < deadline, "the repository loads");
                std::thread::sleep(Duration::from_millis(10));
            }
            app.dispatch(Intent::View(View::Changes));
            // The English each translation stands for, so a cut label compares with English's.
            let english_of: HashMap<&str, &str> = [Language::ChineseSimplified, Language::Japanese]
                .into_iter()
                .flat_map(|l| catalog(l).iter().map(|(english, shown)| (shown.as_str(), english.as_str())))
                .collect();
            let mut english_cut: HashMap<u32, BTreeSet<String>> = HashMap::new();
            for language in [Language::English, Language::Pseudo, Language::ChineseSimplified, Language::Japanese] {
                scoped(language, || {
                    ctx.set_fonts(crate::fonts::definitions());
                    for width in [780.0f32, 1100.0, 1480.0] {
                        let bounds = Rect::from_min_size(Pos2::ZERO, vec2(width, 920.0));
                        let mut cut = Vec::new();
                        for pass in 0..3 {
                            app.frame_widgets.clear();
                            let output = ctx.run_ui(RawInput { screen_rect: Some(bounds), ..Default::default() }, |ui| app.shell(ui, &mut Vec::new()));
                            if pass == 2 {
                                cut = cut_texts(&output);
                            }
                            output.drop_without_applying_deltas();
                        }
                        assert!(app.frame_widgets.contains_key("commit"), "{language:?} shows Commit at {width}");
                        let cut: BTreeSet<String> = cut.iter().map(|t| english_of.get(t.as_str()).map_or_else(|| t.clone(), |e| (*e).to_owned())).collect();
                        match language {
                            Language::English => {
                                english_cut.insert(width as u32, cut);
                            }
                            Language::Pseudo => eprintln!("pseudo cuts {} texts at {width}: {cut:?}", cut.len()),
                            _ => {
                                let extra: Vec<&String> = cut.difference(&english_cut[&(width as u32)]).collect();
                                assert!(extra.is_empty(), "{language:?} at {width} cuts what English shows whole: {extra:?}");
                            }
                        }
                    }
                });
            }
        });
    }

    /// Prints each language's untranslated strings as JSON to fill in:
    /// `cargo test -p rocket-ui i18n::tests::untranslated -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn untranslated() {
        let code = code_keys();
        for language in [Language::ChineseSimplified, Language::Japanese] {
            let found = translations(language, &mut Vec::new());
            let missing: BTreeMap<&String, &str> = code.iter().filter(|k| !found.contains_key(*k)).map(|k| (k, "")).collect();
            println!("{}: {}", language.tag(), serde_json::to_string_pretty(&missing).unwrap());
        }
    }
}
