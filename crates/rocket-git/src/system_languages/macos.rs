//! Read-only CoreFoundation preferences, including Finder launches without LANG.
use super::SystemLanguages;
use std::ffi::{c_char, c_void, CStr};
type Cf = *const c_void;
#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFLocaleCopyPreferredLanguages() -> Cf;
    fn CFLocaleCopyCurrent() -> Cf;
    fn CFLocaleGetIdentifier(locale: Cf) -> Cf;
    fn CFArrayGetCount(array: Cf) -> isize;
    fn CFArrayGetValueAtIndex(array: Cf, index: isize) -> Cf;
    fn CFStringGetTypeID() -> usize;
    fn CFGetTypeID(value: Cf) -> usize;
    fn CFStringGetCString(value: Cf, buffer: *mut c_char, capacity: isize, encoding: u32) -> u8;
    fn CFRelease(value: Cf);
}
struct Owned(Cf);
impl Drop for Owned {
    fn drop(&mut self) { if !self.0.is_null() { unsafe { CFRelease(self.0) }; } }
}
/// `value` is borrowed from a live CF collection/locale (or null).
unsafe fn name(value: Cf) -> Option<String> {
    if value.is_null() || unsafe { CFGetTypeID(value) != CFStringGetTypeID() } { return None; }
    let mut buffer = [0 as c_char; 256];
    // CF writes at most capacity bytes, including a terminator on success.
    if unsafe { CFStringGetCString(value, buffer.as_mut_ptr(), buffer.len() as isize, 0x0800_0100) } == 0 { return None; }
    let text = unsafe { CStr::from_ptr(buffer.as_ptr()) }.to_str().ok()?;
    (!text.is_empty()).then(|| text.to_owned())
}
pub(super) fn languages() -> SystemLanguages {
    // Copy functions transfer +1 ownership; all borrowed values are used before release.
    let list = Owned(unsafe { CFLocaleCopyPreferredLanguages() });
    let mut display = Vec::new();
    if !list.0.is_null() {
        let count = unsafe { CFArrayGetCount(list.0) }.clamp(0, 32);
        for index in 0..count {
            if let Some(text) = unsafe { name(CFArrayGetValueAtIndex(list.0, index)) } { display.push(text); }
        }
    }
    let locale = Owned(unsafe { CFLocaleCopyCurrent() });
    let region = if locale.0.is_null() { None } else { unsafe { name(CFLocaleGetIdentifier(locale.0)) } };
    SystemLanguages { display, region }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn macos_reads_preferred_languages_and_region_without_writing_preferences() {
        let languages = languages();
        assert!(!languages.display.is_empty(), "macOS has a preferred language");
        assert!(languages.display.iter().all(|s| !s.is_empty() && s.len() < 256));
        assert!(languages.region.is_some());
        assert!(unsafe { name(std::ptr::null()) }.is_none());
        let non_string = Owned(unsafe { CFLocaleCopyCurrent() });
        assert!(unsafe { name(non_string.0) }.is_none());
    }
}
