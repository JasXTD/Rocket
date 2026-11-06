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
    German,
    /// German as Switzerland and Liechtenstein write it, with ss for ß. Made from the
    /// German catalog, so it has no files of its own.
    GermanSwiss,
    French,
    Swedish,
    Russian,
    Ukrainian,
    /// Laid out right to left by the vendored epaint's bidirectional patch, with Latin
    /// words and numbers in their own order. Rocket's interface itself is not mirrored.
    Arabic,
    ChineseSimplified,
    Japanese,
    Korean,
    /// Accented, bracketed and stretched English, for finding text that skips [`tr`] and
    /// labels that clip. Only `ROCKET_LANG=pseudo` shows it.
    Pseudo,
}

/// The national forms Han characters take: the same character is drawn differently in
/// Chinese, Japanese and Korean fonts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Han {
    Chinese,
    Japanese,
    Korean,
}

/// A CLDR plural category, for whole counts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Plural {
    Zero,
    One,
    Two,
    Few,
    Many,
    Other,
}

impl Plural {
    /// The name a catalog keys a counted phrase's form by.
    pub(crate) fn key(self) -> &'static str {
        match self {
            Plural::Zero => "zero",
            Plural::One => "one",
            Plural::Two => "two",
            Plural::Few => "few",
            Plural::Many => "many",
            Plural::Other => "other",
        }
    }
}

impl Language {
    /// What Preferences can offer, in its order.
    pub const ALL: [Language; 11] = [
        Language::English,
        Language::German,
        Language::GermanSwiss,
        Language::French,
        Language::Swedish,
        Language::Russian,
        Language::Ukrainian,
        Language::Arabic,
        Language::ChineseSimplified,
        Language::Japanese,
        Language::Korean,
    ];

    /// The name saved state and `ROCKET_LANG` use.
    pub fn tag(self) -> &'static str {
        match self {
            Language::English => "en",
            Language::German => "de",
            Language::GermanSwiss => "de-CH",
            Language::French => "fr",
            Language::Swedish => "sv",
            Language::Russian => "ru",
            Language::Ukrainian => "uk",
            Language::Arabic => "ar",
            Language::ChineseSimplified => "zh-Hans",
            Language::Japanese => "ja",
            Language::Korean => "ko",
            Language::Pseudo => "pseudo",
        }
    }

    pub fn from_tag(tag: &str) -> Option<Self> {
        Language::ALL.into_iter().chain([Language::Pseudo]).find(|language| language.tag().eq_ignore_ascii_case(tag))
    }

    /// The language's own name, so anyone can find theirs whatever the interface shows.
    pub fn name(self) -> &'static str {
        match self {
            Language::English => "English",
            Language::German => "Deutsch",
            Language::GermanSwiss => "Deutsch (Schweiz)",
            Language::French => "Français",
            Language::Swedish => "Svenska",
            Language::Russian => "Русский",
            Language::Ukrainian => "Українська",
            Language::Arabic => "العربية",
            Language::ChineseSimplified => "简体中文",
            Language::Japanese => "日本語",
            Language::Korean => "한국어",
            Language::Pseudo => "Pseudo",
        }
    }

    /// The language a system language name stands for, written the Windows way (zh-CN,
    /// de-CH, ar-SA) or the Linux way (zh_CN.UTF-8). Traditional Chinese, Italian and the
    /// rest have none.
    pub fn from_system(name: &str) -> Option<Self> {
        let name = name.split(['.', '@']).next().unwrap_or_default().to_ascii_lowercase().replace('_', "-");
        let mut parts = name.split('-');
        let primary = parts.next()?;
        let rest: Vec<&str> = parts.collect();
        let swiss = rest.iter().any(|part| matches!(*part, "ch" | "li"));
        match primary {
            "en" | "c" | "posix" => Some(Language::English),
            // gsw is spoken Swiss German (and Alsatian); both read standard German.
            "de" | "gsw" if swiss => Some(Language::GermanSwiss),
            "de" | "gsw" => Some(Language::German),
            "fr" => Some(Language::French),
            "sv" => Some(Language::Swedish),
            "ru" => Some(Language::Russian),
            "uk" => Some(Language::Ukrainian),
            "ar" => Some(Language::Arabic),
            "ja" => Some(Language::Japanese),
            "ko" => Some(Language::Korean),
            "zh" => {
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

    /// The CLDR plural category of a whole count, by the CLDR cardinal rules.
    pub(crate) fn plural(self, n: u64) -> Plural {
        match self {
            Language::French if n <= 1 => Plural::One,
            Language::French if n % 1_000_000 == 0 => Plural::Many,
            Language::Russian | Language::Ukrainian => {
                let (last, last_two) = (n % 10, n % 100);
                if last == 1 && last_two != 11 {
                    Plural::One
                } else if (2..=4).contains(&last) && !(12..=14).contains(&last_two) {
                    Plural::Few
                } else {
                    Plural::Many
                }
            }
            Language::Arabic => match (n, n % 100) {
                (0, _) => Plural::Zero,
                (1, _) => Plural::One,
                (2, _) => Plural::Two,
                (_, 3..=10) => Plural::Few,
                (_, 11..=99) => Plural::Many,
                _ => Plural::Other,
            },
            Language::ChineseSimplified | Language::Japanese | Language::Korean => Plural::Other,
            _ if n == 1 && self != Language::French => Plural::One,
            _ => Plural::Other,
        }
    }

    /// The forms a counted phrase's translation gives: every category whole counts reach,
    /// except French "many" (whole millions), which may fall back to "other".
    #[cfg(test)]
    pub(crate) fn plural_forms(self) -> &'static [Plural] {
        match self {
            Language::ChineseSimplified | Language::Japanese | Language::Korean => &[Plural::Other],
            Language::Russian | Language::Ukrainian => &[Plural::One, Plural::Few, Plural::Many],
            Language::Arabic => &[Plural::Zero, Plural::One, Plural::Two, Plural::Few, Plural::Many, Plural::Other],
            _ => &[Plural::One, Plural::Other],
        }
    }

    /// Whether `form` holds a single count in this language, so its text may leave the
    /// number out: German "eine Datei", Arabic "ملفان" (two files).
    #[cfg(test)]
    pub(crate) fn single_count(self, form: Plural) -> bool {
        matches!(
            (self, form),
            (Language::German | Language::GermanSwiss | Language::Swedish, Plural::One)
                | (Language::Arabic, Plural::Zero | Plural::One | Plural::Two)
        )
    }

    /// The Han forms this language reads, if it writes Han characters.
    fn han(self) -> Option<Han> {
        match self {
            Language::ChineseSimplified => Some(Han::Chinese),
            Language::Japanese => Some(Han::Japanese),
            Language::Korean => Some(Han::Korean),
            _ => None,
        }
    }

    fn code(self) -> u8 {
        Language::ALL.into_iter().chain([Language::Pseudo]).position(|l| l == self).unwrap_or(0) as u8
    }

    fn from_code(code: u8) -> Self {
        Language::ALL.into_iter().chain([Language::Pseudo]).nth(usize::from(code)).unwrap_or(Language::English)
    }
}

/// Languages whose text is complete, offered without `ROCKET_LANG`. A language joins
/// once every string has its translation; the catalog test then holds it there.
const RELEASED: &[Language] = &[Language::English];

static CURRENT: AtomicU8 = AtomicU8::new(0);
static SYSTEM: AtomicU8 = AtomicU8::new(0);
static PREVIEW: AtomicBool = AtomicBool::new(false);
/// The system's first Han language (0 Chinese, 1 Japanese, 2 Korean), for the font order
/// of an interface that writes no Han characters.
static SYSTEM_HAN: AtomicU8 = AtomicU8::new(0);

/// Picks the interface language once, before the first window: the first of the
/// system's display languages that Rocket offers, else English. `ROCKET_LANG` (en, de,
/// de-CH, fr, sv, ru, uk, ar, zh-Hans, ja, ko or pseudo) stands in for the system's
/// language and lets Preferences offer the languages that are not complete yet.
/// `region` is the Region format's locale; it only decides the Han font order of an
/// interface that writes no Han characters.
pub fn start(display: &[String], region: Option<&str>) {
    let forced = std::env::var("ROCKET_LANG").ok().and_then(|tag| Language::from_tag(tag.trim()));
    PREVIEW.store(forced.is_some(), Relaxed);
    let system = forced.unwrap_or_else(|| pick(display.iter().map(String::as_str), RELEASED));
    SYSTEM.store(system.code(), Relaxed);
    CURRENT.store(system.code(), Relaxed);
    let han = display.iter().map(String::as_str).chain(region).find_map(han_of).unwrap_or(Han::Chinese);
    SYSTEM_HAN.store(han as u8, Relaxed);
}

/// The first of `names` that stands for an offered language, else English.
fn pick<'a>(names: impl Iterator<Item = &'a str>, offered: &[Language]) -> Language {
    names.filter_map(Language::from_system).find(|l| offered.contains(l)).unwrap_or(Language::English)
}

/// The Han forms a system language reads; None when it writes no Han characters.
fn han_of(name: &str) -> Option<Han> {
    match name.split(['-', '_', '.', '@']).next()?.to_ascii_lowercase().as_str() {
        "zh" => Some(Han::Chinese),
        "ja" => Some(Han::Japanese),
        "ko" => Some(Han::Korean),
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

/// Which forms Han characters take: the interface language's own, or, for a language
/// that writes none, the system's first Han language's (Chinese when it has none).
pub(crate) fn han() -> Han {
    current().han().unwrap_or(match SYSTEM_HAN.load(Relaxed) {
        1 => Han::Japanese,
        2 => Han::Korean,
        _ => Han::Chinese,
    })
}

/// Each language's files by area; Swiss German has none (see [`catalog`]). Shared words
/// such as Cancel live in common; the rest go with the part of Rocket that shows them.
macro_rules! files {
    ($($area:literal),* $(,)?) => {
        &[$(
            (Language::German, $area, include_str!(concat!("../locales/de/", $area, ".json"))),
            (Language::French, $area, include_str!(concat!("../locales/fr/", $area, ".json"))),
            (Language::Swedish, $area, include_str!(concat!("../locales/sv/", $area, ".json"))),
            (Language::Russian, $area, include_str!(concat!("../locales/ru/", $area, ".json"))),
            (Language::Ukrainian, $area, include_str!(concat!("../locales/uk/", $area, ".json"))),
            (Language::Arabic, $area, include_str!(concat!("../locales/ar/", $area, ".json"))),
            (Language::ChineseSimplified, $area, include_str!(concat!("../locales/zh-Hans/", $area, ".json"))),
            (Language::Japanese, $area, include_str!(concat!("../locales/ja/", $area, ".json"))),
            (Language::Korean, $area, include_str!(concat!("../locales/ko/", $area, ".json"))),
        )*]
    };
}
const FILES: &[(Language, &str, &str)] =
    files!["common", "menus", "chrome", "views", "diff", "preferences"];

/// One catalog entry: a text, or a counted phrase's forms by CLDR category.
#[derive(Clone, Debug)]
enum Entry {
    Text(String),
    Forms(HashMap<String, String>),
}

impl Entry {
    fn read(value: serde_json::Value) -> Option<Entry> {
        match value {
            serde_json::Value::String(text) => Some(Entry::Text(text)),
            serde_json::Value::Object(forms) => forms
                .into_iter()
                .map(|(form, text)| Some((form, text.as_str()?.to_owned())))
                .collect::<Option<HashMap<_, _>>>()
                .map(Entry::Forms),
            _ => None,
        }
    }
}

fn catalog(language: Language) -> &'static HashMap<String, Entry> {
    static CATALOGS: [OnceLock<HashMap<String, Entry>>; 12] = [const { OnceLock::new() }; 12];
    CATALOGS[usize::from(language.code())].get_or_init(|| {
        let source = if language == Language::GermanSwiss { Language::German } else { language };
        // A file that does not read keeps its strings in English; the catalog test fails on it.
        let mut entries: HashMap<String, Entry> = FILES
            .iter()
            .filter(|(l, ..)| *l == source)
            .filter_map(|(_, _, text)| serde_json::from_str::<serde_json::Map<String, serde_json::Value>>(text).ok())
            .flatten()
            .filter_map(|(english, value)| Some((english, Entry::read(value)?)))
            .collect();
        if language == Language::GermanSwiss {
            for entry in entries.values_mut() {
                match entry {
                    Entry::Text(text) => *text = swiss(text),
                    Entry::Forms(forms) => forms.values_mut().for_each(|text| *text = swiss(text)),
                }
            }
        }
        entries
    })
}

/// German as Switzerland writes it: ss where Germany writes ß.
fn swiss(german: &str) -> String {
    german.replace('ß', "ss").replace('ẞ', "SS")
}

/// The interface text for `english`, a literal or a [`mark`]ed one.
pub fn tr(english: &'static str) -> &'static str {
    match current() {
        Language::English => english,
        Language::Pseudo => pseudo(english),
        language => match catalog(language).get(english) {
            Some(Entry::Text(text)) => text,
            // A counted phrase shown as plain text reads its general form.
            Some(Entry::Forms(forms)) => forms.get("other").or_else(|| forms.get("many")).map_or(english, String::as_str),
            None => english,
        },
    }
}

/// Marks text in a constant table for the catalog test; [`tr`] translates it where it shows.
pub const fn mark(english: &'static str) -> &'static str {
    english
}

/// [`trn!`]'s phrase. English picks by count; a catalog keys the phrase by `other`, with
/// one text for every count or the forms of the language's plural categories.
pub fn plural(count: u64, one: &'static str, other: &'static str) -> &'static str {
    let english = if count == 1 { one } else { other };
    match current() {
        Language::English => english,
        Language::Pseudo => pseudo(english),
        language => match catalog(language).get(other) {
            Some(Entry::Text(text)) => text,
            Some(Entry::Forms(forms)) => forms
                .get(language.plural(count).key())
                .or_else(|| forms.get("other"))
                .map_or(english, String::as_str),
            None => english,
        },
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

/// A counted phrase: `trn!(count, "{n} file", "{n} files")`, `{n}` being the count. A
/// catalog keys it by the second phrase: one text, or a form per plural category.
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
            ("ko-KR", Some(Korean)),
            ("de-DE", Some(German)),
            ("de-AT", Some(German)),
            ("de_DE.UTF-8", Some(German)),
            ("de-CH", Some(GermanSwiss)),
            ("de-LI", Some(GermanSwiss)),
            ("gsw-CH", Some(GermanSwiss)),
            ("fr-FR", Some(French)),
            ("fr-CH", Some(French)),
            ("fr-CA", Some(French)),
            ("sv-SE", Some(Swedish)),
            ("sv-FI", Some(Swedish)),
            ("ru-RU", Some(Russian)),
            ("uk-UA", Some(Ukrainian)),
            ("ar-SA", Some(Arabic)),
            ("ar-EG", Some(Arabic)),
            ("it-CH", None),
            ("rm-CH", None),
            ("es-ES", None),
            ("", None),
        ] {
            assert_eq!(Language::from_system(name), expected, "{name}");
        }
        let names = ["zh-TW", "ja-JP", "en-US"];
        assert_eq!(pick(names.into_iter(), &Language::ALL), Japanese, "Traditional Chinese passes to the next");
        assert_eq!(pick(names.into_iter(), &[English]), English, "an unreleased language is passed over");
        assert_eq!(pick(["it-CH", "es-ES"].into_iter(), &Language::ALL), English);
        assert_eq!(pick(["it-CH", "de-CH"].into_iter(), &Language::ALL), GermanSwiss, "Italian Switzerland passes to the next");
        assert_eq!(resolve_in(Some(Japanese), &Language::ALL, English), Japanese);
        assert_eq!(resolve_in(Some(Japanese), &[English], ChineseSimplified), ChineseSimplified, "a choice no longer offered follows the system");
        assert_eq!(resolve_in(None, &Language::ALL, Japanese), Japanese);
        for language in Language::ALL.into_iter().chain([Pseudo]) {
            assert_eq!(Language::from_tag(language.tag()), Some(language));
            assert_eq!(Language::from_code(language.code()), language);
        }
        assert_eq!(Language::from_tag("DE-ch"), Some(GermanSwiss), "tags read in any case");
        assert_eq!(han_of("ja-JP"), Some(Han::Japanese));
        assert_eq!(han_of("zh-TW"), Some(Han::Chinese));
        assert_eq!(han_of("ko-KR"), Some(Han::Korean));
        assert_eq!(han_of("jam"), None);
    }

    #[test]
    fn plural_categories_follow_cldr() {
        use Language::*;
        use Plural::*;
        let cases: [(Language, &[(u64, Plural)]); 6] = [
            (German, &[(0, Other), (1, One), (2, Other), (21, Other)]),
            (French, &[(0, One), (1, One), (2, Other), (1_000, Other), (1_000_000, Many), (2_000_000, Many)]),
            (Russian, &[(0, Many), (1, One), (2, Few), (4, Few), (5, Many), (11, Many), (12, Many), (14, Many), (21, One), (22, Few), (25, Many), (101, One), (111, Many), (112, Many)]),
            (Ukrainian, &[(1, One), (3, Few), (13, Many), (21, One), (24, Few), (1_011, Many)]),
            (Arabic, &[(0, Zero), (1, One), (2, Two), (3, Few), (10, Few), (11, Many), (99, Many), (100, Other), (102, Other), (103, Few), (111, Many)]),
            (Korean, &[(0, Other), (1, Other), (2, Other)]),
        ];
        for (language, counts) in cases {
            for &(n, expected) in counts {
                assert_eq!(language.plural(n), expected, "{language:?} {n}");
            }
        }
        for language in Language::ALL {
            // Every whole count lands on a form the catalogs must give (French many falls back).
            for n in 0..2_000 {
                let form = language.plural(n);
                assert!(language.plural_forms().contains(&form) || (language == French && form == Many), "{language:?} {n} → {form:?}");
            }
        }
    }

    #[test]
    fn swiss_german_writes_ss_for_eszett() {
        assert_eq!(swiss("Schließen · STRAẞE"), "Schliessen · STRASSE");
        assert_eq!(swiss("Ändern"), "Ändern", "umlauts stay");
        let german = catalog(Language::German);
        let switzerland = catalog(Language::GermanSwiss);
        assert_eq!(german.len(), switzerland.len(), "Swiss German has German's entries");
        for (english, entry) in german {
            match (entry, &switzerland[english]) {
                (Entry::Text(de), Entry::Text(ch)) => assert_eq!(*ch, swiss(de), "{english}"),
                (Entry::Forms(de), Entry::Forms(ch)) => {
                    for (form, text) in de {
                        assert_eq!(ch[form], swiss(text), "{english} {form}");
                    }
                }
                _ => panic!("{english} changed shape"),
            }
        }
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
    /// Each key, and whether it is a counted phrase (trn!).
    fn code_keys() -> BTreeMap<String, bool> {
        let mut keys = BTreeMap::new();
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
                    for (key, counted) in keys_in(&std::fs::read_to_string(&path).unwrap()) {
                        *keys.entry(key).or_insert(false) |= counted;
                    }
                }
            }
        }
        keys
    }

    fn keys_in(text: &str) -> Vec<(String, bool)> {
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
                    keys.extend(phrases.pop().map(|key| (key, literal == 1)));
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

    /// A catalog file's entries, with every top-level key in order so one written twice
    /// is seen (serde_json keeps only the last).
    fn entries(text: &str) -> Result<Vec<(String, serde_json::Value)>, String> {
        let value: serde_json::Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
        let object = value.as_object().ok_or("not a JSON object")?;
        let bytes = text.as_bytes();
        let (mut depth, mut i, mut keys) = (0, 0, Vec::new());
        while i < bytes.len() {
            match bytes[i] {
                b'{' => depth += 1,
                b'}' => depth -= 1,
                b'"' => {
                    let start = i;
                    i += 1;
                    while bytes[i] != b'"' {
                        i += if bytes[i] == b'\\' { 2 } else { 1 };
                    }
                    let after = text[i + 1..].trim_start();
                    if depth == 1 && after.starts_with(':') {
                        keys.push(serde_json::from_str::<String>(&text[start..=i]).map_err(|e| e.to_string())?);
                    }
                }
                _ => {}
            }
            i += 1;
        }
        Ok(keys.into_iter().map(|key| (key.clone(), object[&key].clone())).collect())
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

    fn translations(language: Language, problems: &mut Vec<String>) -> BTreeMap<String, (serde_json::Value, &'static str)> {
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

    /// The languages with catalog files of their own.
    const CATALOG_LANGUAGES: [Language; 9] = [
        Language::German,
        Language::French,
        Language::Swedish,
        Language::Russian,
        Language::Ukrainian,
        Language::Arabic,
        Language::ChineseSimplified,
        Language::Japanese,
        Language::Korean,
    ];

    #[test]
    fn catalogs_match_the_code() {
        let code = code_keys();
        assert!(code.len() > 100, "the scan found only {} strings", code.len());
        let mut problems = Vec::new();
        for language in CATALOG_LANGUAGES {
            let found = translations(language, &mut problems);
            for (key, (value, area)) in &found {
                let file = format!("{}/{area}.json", language.tag());
                let Some(&counted) = code.get(key) else {
                    problems.push(format!("{file}: {key:?} is no longer in the code; reworded English needs its translation moved"));
                    continue;
                };
                let forms = language.plural_forms();
                match value {
                    serde_json::Value::String(text) if !counted || forms == [Plural::Other] => {
                        if slots(key) != slots(text) {
                            problems.push(format!("{file}: {key:?} → {text:?} has other {{slots}}"));
                        }
                        if text.trim().is_empty() {
                            problems.push(format!("{file}: {key:?} is empty"));
                        }
                    }
                    serde_json::Value::Object(given) if counted => {
                        for form in forms {
                            if !given.get(form.key()).and_then(|t| t.as_str()).is_some_and(|t| !t.trim().is_empty()) {
                                problems.push(format!("{file}: {key:?} lacks its {:?} form", form.key()));
                            }
                        }
                        for (name, text) in given {
                            let all = [Plural::Zero, Plural::One, Plural::Two, Plural::Few, Plural::Many, Plural::Other];
                            let Some(form) = all.into_iter().find(|f| f.key() == name) else {
                                problems.push(format!("{file}: {key:?} has a form {name:?}, which CLDR does not name"));
                                continue;
                            };
                            let Some(text) = text.as_str() else {
                                problems.push(format!("{file}: {key:?} {name} is not text"));
                                continue;
                            };
                            // A form holding one count may leave the number out; none adds a slot.
                            let ok = slots(text) == slots(key) || (language.single_count(form) && slots(text).is_subset(&slots(key)));
                            if !ok {
                                problems.push(format!("{file}: {key:?} {name} → {text:?} has other {{slots}}"));
                            }
                        }
                    }
                    _ if counted => problems.push(format!("{file}: {key:?} needs its forms {:?}", forms.iter().map(|f| f.key()).collect::<Vec<_>>())),
                    _ => problems.push(format!("{file}: {key:?} must be one text")),
                }
            }
            let missing = code.keys().filter(|key| !found.contains_key(*key)).count();
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
        assert!(FILES.iter().all(|(l, ..)| CATALOG_LANGUAGES.contains(l)));
        assert_eq!(CATALOG_LANGUAGES.len() + 2, Language::ALL.len(), "English and Swiss German have no files");
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
        let expected = [("After a test module", false), ("Marked \"quoted\"…", false), ("Pushed {branch}", false), ("Stage all", false), ("{n} files", true)];
        assert_eq!(keys, expected.map(|(key, counted)| (key.to_owned(), counted)));
    }

    /// The English for shown text that fills a translated template's slots, such as
    /// "в main" for "to main", so a cut label compares with English's.
    fn filled_english(language: Language, shown: &str) -> Option<String> {
        if matches!(language, Language::English | Language::Pseudo) {
            return None;
        }
        for (english, entry) in catalog(language) {
            let forms: Vec<&String> = match entry {
                Entry::Text(text) => vec![text],
                Entry::Forms(forms) => forms.values().collect(),
            };
            for form in forms {
                if let Some(values) = slot_values(form, shown) {
                    let values: Vec<(&str, &dyn Display)> = values.iter().map(|(n, v)| (*n, v as &dyn Display)).collect();
                    return Some(fill(english, &values));
                }
            }
        }
        None
    }

    /// The values `text` gives a template's `{slots}`, if it fills that template.
    fn slot_values<'a>(template: &'a str, text: &str) -> Option<Vec<(&'a str, String)>> {
        let names: Vec<&str> = slots(template).into_iter().collect();
        if names.is_empty() {
            return None;
        }
        // Literal pieces between the slots, in order; each slot takes the shortest text
        // up to the next piece (the last takes the rest).
        let mut pieces = Vec::new();
        let mut order = Vec::new();
        let mut rest = template;
        while let Some(open) = rest.find('{') {
            let Some(close) = rest[open..].find('}').map(|c| open + c) else { break };
            let name = &rest[open + 1..close];
            if !names.contains(&name) {
                break;
            }
            pieces.push(&rest[..open]);
            order.push(name);
            rest = &rest[close + 1..];
        }
        pieces.push(rest);
        let mut at = text.strip_prefix(pieces[0])?;
        let mut values = Vec::new();
        for (i, name) in order.iter().enumerate() {
            let next = pieces[i + 1];
            let end = if i + 1 == order.len() {
                at.len().checked_sub(next.len()).filter(|_| at.ends_with(next))?
            } else {
                at.find(next).filter(|_| !next.is_empty())?
            };
            values.push((*name, at[..end].to_owned()));
            at = &at[end + next.len()..];
        }
        at.is_empty().then_some(values)
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
            let english_of: HashMap<&str, &str> = Language::ALL
                .into_iter()
                .filter(|l| *l != Language::English)
                .flat_map(|l| {
                    catalog(l).iter().filter_map(|(english, entry)| match entry {
                        Entry::Text(shown) => Some((shown.as_str(), english.as_str())),
                        Entry::Forms(_) => None,
                    })
                })
                .collect();
            let mut english_cut: HashMap<u32, BTreeSet<String>> = HashMap::new();
            let mut problems = Vec::new();
            for language in [Language::English, Language::Pseudo].into_iter().chain(Language::ALL.into_iter().skip(1)) {
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
                        let cut: BTreeSet<String> = cut
                            .iter()
                            .map(|t| english_of.get(t.as_str()).map(|e| (*e).to_owned()).or_else(|| filled_english(language, t)).unwrap_or_else(|| t.clone()))
                            .collect();
                        match language {
                            Language::English => {
                                english_cut.insert(width as u32, cut);
                            }
                            Language::Pseudo => eprintln!("pseudo cuts {} texts at {width}: {cut:?}", cut.len()),
                            _ => {
                                let extra: Vec<&String> = cut.difference(&english_cut[&(width as u32)]).collect();
                                if !extra.is_empty() {
                                    problems.push(format!("{language:?} at {width} cuts what English shows whole: {extra:?}"));
                                }
                            }
                        }
                    }
                });
            }
            assert!(problems.is_empty(), "{}", problems.join("
"));
        });
    }

    /// Prints each language's untranslated strings as JSON to fill in:
    /// `cargo test -p rocket-ui i18n::tests::untranslated -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn untranslated() {
        let code = code_keys();
        for language in CATALOG_LANGUAGES {
            let found = translations(language, &mut Vec::new());
            let missing: BTreeMap<&String, &str> = code.keys().filter(|k| !found.contains_key(*k)).map(|k| (k, "")).collect();
            println!("{}: {}", language.tag(), serde_json::to_string_pretty(&missing).unwrap());
        }
    }
}
