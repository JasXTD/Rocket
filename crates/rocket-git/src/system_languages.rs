//! The languages the user's system shows, most preferred first, as the system names
//! them: zh-CN and ja-JP on Windows/macOS, zh_CN.UTF-8 on Linux. Rocket's interface picks the
//! first one it has; nothing here decides which languages that is.

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SystemLanguages {
    /// Windows/macOS: the preferred display languages, most preferred first.
    /// Elsewhere: LANGUAGE's list, then the locale LC_ALL, LC_MESSAGES or LANG names.
    pub display: Vec<String>,
    /// Windows/macOS: the Region locale, which can differ from the display language.
    pub region: Option<String>,
}

#[cfg(windows)]
pub fn system_languages() -> SystemLanguages {
    SystemLanguages { display: windows::display(), region: windows::region() }
}

#[cfg(not(any(windows, target_os = "macos")))]
pub fn system_languages() -> SystemLanguages {
    SystemLanguages { display: from_environment(|name| std::env::var(name).ok()), region: None }
}

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
pub fn system_languages() -> SystemLanguages { macos::languages() }

/// The locale variables in gettext's order. LANGUAGE's list counts only when the
/// locale itself is not C, as gettext has it.
#[cfg_attr(any(windows, target_os = "macos"), allow(dead_code))]
fn from_environment(var: impl Fn(&str) -> Option<String>) -> Vec<String> {
    let set = |name: &str| var(name).filter(|value| !value.is_empty());
    let locale = set("LC_ALL").or_else(|| set("LC_MESSAGES")).or_else(|| set("LANG"));
    let plain = locale.as_deref().is_none_or(|l| l == "C" || l == "POSIX" || l.starts_with("C."));
    let mut names: Vec<String> = Vec::new();
    if !plain && let Some(list) = set("LANGUAGE") {
        names.extend(list.split(':').filter(|n| !n.is_empty()).map(str::to_owned));
    }
    names.extend(locale);
    names
}

#[cfg(windows)]
mod windows {
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetUserPreferredUILanguages(flags: u32, count: *mut u32, buffer: *mut u16, length: *mut u32) -> i32;
        fn GetUserDefaultLocaleName(name: *mut u16, length: i32) -> i32;
    }
    const MUI_LANGUAGE_NAME: u32 = 0x8;
    const LOCALE_NAME_MAX_LENGTH: usize = 85;
    /// Far beyond any real list; a larger answer is refused rather than allocated.
    const MOST_UNITS: u32 = 16 * 1024;

    pub(super) fn display() -> Vec<String> {
        let mut count = 0u32;
        let mut length = 0u32;
        // SAFETY: a null buffer with length 0 asks only for the length needed.
        let sized = unsafe { GetUserPreferredUILanguages(MUI_LANGUAGE_NAME, &mut count, std::ptr::null_mut(), &mut length) };
        if sized == 0 || length == 0 || length > MOST_UNITS {
            return Vec::new();
        }
        let mut buffer = vec![0u16; length as usize];
        // SAFETY: the buffer holds `length` units, as the length passed says. A list that
        // grew in between fails the call, and Rocket then shows English.
        let read = unsafe { GetUserPreferredUILanguages(MUI_LANGUAGE_NAME, &mut count, buffer.as_mut_ptr(), &mut length) };
        if read == 0 {
            return Vec::new();
        }
        // Names, each ending in NUL, then one more NUL.
        let units = &buffer[..(length as usize).min(buffer.len())];
        units.split(|unit| *unit == 0).filter(|name| !name.is_empty()).map(String::from_utf16_lossy).collect()
    }

    pub(super) fn region() -> Option<String> {
        let mut name = [0u16; LOCALE_NAME_MAX_LENGTH];
        // SAFETY: the buffer holds LOCALE_NAME_MAX_LENGTH units, as the length says.
        let length = unsafe { GetUserDefaultLocaleName(name.as_mut_ptr(), name.len() as i32) };
        // The length counts the closing NUL; 0 means failure.
        (length > 1).then(|| String::from_utf16_lossy(&name[..length as usize - 1]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn locale_variables_follow_gettexts_order() {
        let env = |pairs: &'static [(&'static str, &'static str)]| move |name: &str| pairs.iter().find(|(n, _)| *n == name).map(|(_, v)| v.to_string());
        assert_eq!(from_environment(env(&[("LANG", "zh_CN.UTF-8")])), ["zh_CN.UTF-8"]);
        assert_eq!(from_environment(env(&[("LANGUAGE", "ja:en"), ("LANG", "en_US.UTF-8")])), ["ja", "en", "en_US.UTF-8"]);
        assert_eq!(from_environment(env(&[("LC_ALL", "ja_JP.UTF-8"), ("LC_MESSAGES", "de_DE"), ("LANG", "fr_FR")])), ["ja_JP.UTF-8"]);
        assert_eq!(from_environment(env(&[("LC_MESSAGES", ""), ("LANG", "de_DE")])), ["de_DE"], "an empty variable is unset");
        assert_eq!(from_environment(env(&[("LANGUAGE", "ja"), ("LANG", "C.UTF-8")])), ["C.UTF-8"], "LANGUAGE waits for a real locale");
        assert!(from_environment(env(&[])).is_empty());
    }

    #[cfg(windows)]
    #[test]
    fn windows_reports_its_display_language() {
        let languages = system_languages();
        assert!(!languages.display.is_empty(), "Windows always has a display language");
        let tag = |name: &String| !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-');
        assert!(languages.display.iter().all(tag), "{languages:?}");
        assert!(languages.region.as_ref().is_none_or(tag), "{languages:?}");
    }
}
