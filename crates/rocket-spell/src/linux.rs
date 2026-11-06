//! Linux has no system spell checker: words are checked against the system's Hunspell
//! dictionaries (Ubuntu's hunspell-en-us, for example) by spellbook, a pure-Rust reader of
//! that format, so no C code runs in Rocket. Ignored words last until Rocket restarts;
//! added words go to Rocket's own word list, one per line, in its state folder.
use super::{Action, Error, Misspelling, unchecked_script};
use std::{
    cell::RefCell,
    collections::HashSet,
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
};

/// Where distributions install Hunspell dictionaries.
const DICTIONARIES: [&str; 3] = ["/usr/share/hunspell", "/usr/share/myspell", "/usr/share/myspell/dicts"];
/// A dictionary bigger than this is refused rather than read.
const MAX_DICTIONARY: u64 = 64 * 1024 * 1024;
/// The added-word list Rocket reads back: words, and bytes.
const MAX_WORDS: usize = 10_000;
const MAX_WORD_LIST: u64 = 1024 * 1024;

pub(crate) struct Checker {
    dictionary: RefCell<spellbook::Dictionary>,
    language: String,
    ignored: RefCell<HashSet<String>>,
    word_list: Option<PathBuf>,
}

impl Checker {
    /// `language` is a BCP 47 tag such as `en-US`; None means the user's language, then
    /// `en-US`. A language without an installed dictionary is unavailable.
    pub fn new(language: Option<&str>) -> Result<Self, Error> {
        Self::with_word_list(language, &DICTIONARIES.map(PathBuf::from), word_list_folder())
    }

    pub(crate) fn with_word_list(
        language: Option<&str>,
        folders: &[PathBuf],
        word_lists: Option<PathBuf>,
    ) -> Result<Self, Error> {
        let candidates = match language {
            Some(tag) => vec![tag.to_owned()],
            None => [user_language(|name| std::env::var(name).ok()), Some("en-US".to_owned())]
                .into_iter()
                .flatten()
                .collect(),
        };
        let (tag, aff, dic) = candidates
            .into_iter()
            .find_map(|tag| {
                let stem = file_stem(&tag)?;
                folders.iter().find_map(|folder| {
                    let (aff, dic) = (folder.join(format!("{stem}.aff")), folder.join(format!("{stem}.dic")));
                    (aff.is_file() && dic.is_file()).then(|| (tag.clone(), aff, dic))
                })
            })
            .ok_or(Error::Unavailable)?;
        let mut dictionary = spellbook::Dictionary::new(&read(&aff)?, &read(&dic)?)
            .map_err(|e| Error::Failed(format!("read the {tag} dictionary: {e}")))?;
        let word_list = word_lists.and_then(|folder| file_stem(&tag).map(|stem| folder.join(format!("{stem}.dic"))));
        if let Some(path) = &word_list {
            for word in added_words(path)? {
                // Only words this list accepts are written, so each one parses.
                let _ = dictionary.add(&word);
            }
        }
        Ok(Self {
            dictionary: RefCell::new(dictionary),
            language: tag,
            ignored: RefCell::new(HashSet::new()),
            word_list,
        })
    }

    pub fn language(&self) -> &str {
        &self.language
    }

    pub fn check(&self, text: &str) -> Result<Vec<Misspelling>, Error> {
        let dictionary = self.dictionary.borrow();
        let ignored = self.ignored.borrow();
        let mut found = Vec::new();
        let mut previous: Option<(String, usize)> = None;
        for (range, word) in words(text) {
            // "the the": the second is flagged for deletion, as Windows does.
            let lower = word.to_lowercase();
            if let Some((last, end)) = &previous
                && *last == lower
                && text[*end..range.start].chars().all(|c| c == ' ' || c == '\t')
            {
                found.push(Misspelling { range: range.clone(), action: Action::Delete });
                previous = Some((lower, range.end));
                continue;
            }
            previous = Some((lower, range.end));
            if checked(word) && !ignored.contains(word) && !dictionary.check(word) {
                found.push(Misspelling { range, action: Action::Suggest });
            }
        }
        Ok(found)
    }

    pub fn suggest(&self, word: &str) -> Result<Vec<String>, Error> {
        let mut suggestions = Vec::new();
        self.dictionary.borrow().suggest(word, &mut suggestions);
        Ok(suggestions)
    }

    pub fn ignore(&self, word: &str) -> Result<(), Error> {
        self.ignored.borrow_mut().insert(word.to_owned());
        Ok(())
    }

    /// Kept in Rocket's word list for this language, then added to this checker.
    pub fn add(&self, word: &str) -> Result<(), Error> {
        if !addable(word) {
            return Err(Error::Failed("a dictionary word is one line of letters, without / or spaces".into()));
        }
        if let Some(path) = &self.word_list {
            append(path, word).map_err(|e| Error::Failed(format!("save the word: {e}")))?;
        }
        self.dictionary
            .borrow_mut()
            .add(word)
            .map_err(|e| Error::Failed(format!("add the word: {e}")))
    }
}

fn read(path: &Path) -> Result<String, Error> {
    let failed = |e: &dyn std::fmt::Display| Error::Failed(format!("read {}: {e}", path.display()));
    let mut text = String::new();
    fs::File::open(path)
        .map_err(|e| failed(&e))?
        .take(MAX_DICTIONARY + 1)
        .read_to_string(&mut text)
        .map_err(|e| failed(&e))?;
    if text.len() as u64 > MAX_DICTIONARY {
        return Err(failed(&"the dictionary is too large"));
    }
    Ok(text)
}

/// `en-US` → `en_US`, the dictionary's file name. Only plain language tags are accepted.
fn file_stem(tag: &str) -> Option<String> {
    let valid = !tag.is_empty()
        && tag.len() <= 35
        && tag.split('-').all(|part| !part.is_empty() && part.bytes().all(|b| b.is_ascii_alphanumeric()));
    valid.then(|| tag.replace('-', "_"))
}

/// The user's language as a tag (`en_GB.UTF-8` → `en-GB`), from the variables that
/// choose the language of messages, in POSIX order. C and POSIX name none.
fn user_language(variable: impl Fn(&str) -> Option<String>) -> Option<String> {
    let value = ["LC_ALL", "LC_MESSAGES", "LANG"]
        .iter()
        .find_map(|name| variable(name).filter(|v| !v.is_empty()))?;
    let locale = value.split(['.', '@']).next()?;
    if matches!(locale, "C" | "POSIX") {
        return None;
    }
    let tag = locale.replace('_', "-");
    file_stem(&tag).map(|_| tag)
}

/// Rocket's word lists: `$XDG_STATE_HOME/rocket/dictionary`, by default under
/// `~/.local/state`, as Rocket's other files. Created 0700 when first written.
fn word_list_folder() -> Option<PathBuf> {
    std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| std::env::home_dir().filter(|home| home.is_absolute()).map(|home| home.join(".local/state")))
        .map(|state| state.join("rocket").join("dictionary"))
}

fn addable(word: &str) -> bool {
    !word.is_empty()
        && word.len() <= 100
        && !word.contains('/')
        && !word.chars().any(|c| c.is_whitespace() || c.is_control())
}

/// The words a list holds; a missing list holds none. Lines that aren't words are skipped.
fn added_words(path: &Path) -> Result<Vec<String>, Error> {
    let mut text = String::new();
    match fs::File::open(path) {
        Ok(file) => {
            file.take(MAX_WORD_LIST).read_to_string(&mut text).map_err(|e| Error::Failed(format!("read {}: {e}", path.display())))?;
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(Error::Failed(format!("read {}: {e}", path.display()))),
    }
    Ok(text.lines().filter(|word| addable(word)).take(MAX_WORDS).map(str::to_owned).collect())
}

fn append(path: &Path, word: &str) -> std::io::Result<()> {
    use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
    if let Some(folder) = path.parent() {
        fs::DirBuilder::new().recursive(true).mode(0o700).create(folder)?;
    }
    let mut file = fs::OpenOptions::new().append(true).create(true).mode(0o600).open(path)?;
    file.write_all(format!("{word}\n").as_bytes())?;
    file.sync_all()
}

/// The words of `text` with their byte ranges: letters, with apostrophes and hyphens
/// inside a word. Whitespace-separated chunks that look like code give none.
fn words(text: &str) -> impl Iterator<Item = (std::ops::Range<usize>, &str)> {
    let mut found = Vec::new();
    let mut offset = 0;
    for chunk in text.split_inclusive(char::is_whitespace) {
        let start = offset;
        offset += chunk.len();
        let chunk = chunk.trim_end();
        // Paths, URLs, addresses, file names, inline code and identifiers aren't prose.
        if code_like(chunk) {
            continue;
        }
        let mut word: Option<usize> = None;
        let mut chars = chunk.char_indices().peekable();
        while let Some((at, c)) = chars.next() {
            let joins = matches!(c, '\'' | '’' | '-')
                && word.is_some()
                && chars.peek().is_some_and(|(_, next)| next.is_alphabetic());
            if c.is_alphabetic() || joins {
                word.get_or_insert(at);
                continue;
            }
            if let Some(begin) = word.take() {
                found.push((start + begin..start + at, &text[start + begin..start + at]));
            }
        }
        if let Some(begin) = word {
            found.push((start + begin..start + chunk.len(), &text[start + begin..start + chunk.len()]));
        }
    }
    found.into_iter()
}

fn code_like(chunk: &str) -> bool {
    chunk.contains(['/', '\\', '@', '`', '_', '<', '>', '=', '{', '}', '[', ']', '|', '$', '~'])
        // A dot between letters or digits: main.rs, example.com, 1.2 (not "end.").
        || chunk.as_bytes().windows(3).any(|w| {
            w[1] == b'.' && w[0].is_ascii_alphanumeric() && w[2].is_ascii_alphanumeric()
        })
}

/// Whether a word is checked: not an acronym (API), an identifier (fooBar, v2) or text
/// in a script the dictionaries don't cover (中文).
fn checked(word: &str) -> bool {
    let mut letters = word.chars().filter(|c| c.is_alphabetic());
    letters.next().is_some()
        && !letters.any(char::is_uppercase)
        && !word.chars().any(|c| c.is_ascii_digit())
        && !word.chars().all(unchecked_script)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn found<'a>(text: &'a str, misspellings: &[Misspelling]) -> Vec<&'a str> {
        misspellings.iter().map(|m| &text[m.range.clone()]).collect()
    }

    #[test]
    fn prose_words_are_found_and_code_is_left_alone() {
        let listed = |text| words(text).map(|(_, w)| w).collect::<Vec<_>>();
        assert_eq!(listed("Fix the parser's  well-known bug."), ["Fix", "the", "parser's", "well-known", "bug"]);
        assert_eq!(listed("see src/main.rs, https://example.com and a@b.org"), ["see", "and"]);
        assert_eq!(listed("call foo_bar() or `fooBar` with key=value"), ["call", "or", "with"]);
        assert_eq!(listed("'quoted' -dash- end.\nnext"), ["quoted", "dash", "end", "next"]);
        assert_eq!(listed("é中文 ok"), ["é中文", "ok"]);
        for word in ["API", "fooBar", "v2", "README", "中文", "x2y"] {
            assert!(!checked(word), "{word}");
        }
        for word in ["Fix", "the", "don't", "I", "é"] {
            assert!(checked(word), "{word}");
        }
        let ranges: Vec<_> = words("ab cd").map(|(r, _)| r).collect();
        assert_eq!(ranges, [0..2, 3..5]);
    }

    #[test]
    fn languages_come_from_tags_or_the_environment_and_are_plain() {
        assert_eq!(file_stem("en-US").as_deref(), Some("en_US"));
        for bad in ["", "en_US/../x", "en US", "-en", "../en"] {
            assert_eq!(file_stem(bad), None, "{bad}");
        }
        let env = |pairs: &'static [(&'static str, &'static str)]| move |name: &str| pairs.iter().find(|(n, _)| *n == name).map(|(_, v)| (*v).to_owned());
        assert_eq!(user_language(env(&[("LANG", "en_GB.UTF-8")])).as_deref(), Some("en-GB"));
        assert_eq!(user_language(env(&[("LANG", "en_GB.UTF-8"), ("LC_MESSAGES", "de_DE@euro")])).as_deref(), Some("de-DE"));
        assert_eq!(user_language(env(&[("LC_ALL", "C.UTF-8"), ("LANG", "en_GB.UTF-8")])), None);
        assert_eq!(user_language(env(&[])), None);
        for word in ["Rocket", "Rocket's"] {
            assert!(addable(word));
        }
        for word in ["", "two words", "flag/G", "line\nbreak"] {
            assert!(!addable(word), "{word:?}");
        }
    }

    /// With the system's en_US dictionary (Ubuntu: hunspell-en-us), as the Windows tests
    /// use the en-US checker that ships with Windows.
    #[test]
    fn the_hunspell_dictionary_flags_suggests_ignores_and_adds() {
        let root = std::env::temp_dir().join(format!("rocket-spell-{}", std::process::id()));
        let folders = DICTIONARIES.map(PathBuf::from);
        let checker = Checker::with_word_list(Some("en-US"), &folders, Some(root.clone()))
            .expect("the en_US Hunspell dictionary (install hunspell-en-us)");
        assert_eq!(checker.language(), "en-US");
        let text = "中文 This is a smple sentence 🚀 with a tpyo in src/main.rs and the API.";
        let misspellings = checker.check(text).unwrap();
        assert_eq!(found(text, &misspellings), ["smple", "tpyo"]);
        assert!(misspellings.iter().all(|m| m.action == Action::Suggest));
        assert!(checker.suggest("smple").unwrap().iter().any(|s| s == "simple"));
        let repeated = checker.check("Fix the the bug").unwrap();
        assert_eq!(repeated.len(), 1);
        assert_eq!((repeated[0].range.clone(), repeated[0].action.clone()), (8..11, Action::Delete));
        // Ignored for this checker only; added words are kept in the word list.
        checker.ignore("smple").unwrap();
        assert!(checker.check("smple").unwrap().is_empty());
        checker.add("Rocketish").unwrap();
        assert!(checker.check("Rocketish").unwrap().is_empty());
        assert!(checker.add("not/a word").is_err());
        let again = Checker::with_word_list(Some("en-US"), &folders, Some(root.clone())).unwrap();
        assert!(again.check("Rocketish").unwrap().is_empty(), "added words come back");
        assert_eq!(found("smple", &again.check("smple").unwrap()), ["smple"], "ignoring ends with the checker");
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(fs::metadata(root.join("en_US.dic")).unwrap().permissions().mode() & 0o777, 0o600);
        assert!(checker.check("").unwrap().is_empty());
        assert!(matches!(Checker::with_word_list(Some("zz-ZZ"), &folders, None), Err(Error::Unavailable)));
        assert!(matches!(Checker::with_word_list(Some("en-US"), &[root.join("none")], None), Err(Error::Unavailable)));
        fs::remove_dir_all(root).unwrap();
    }
}
