//! Main-queue Cocoa calls are marshalled from Rocket's cancellable worker queue.
use super::{Action, Error, Misspelling, byte_range, unchecked_script};
use std::ffi::{CStr, c_char, c_void};
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct NativeRange {
    start: usize,
    length: usize,
}
unsafe extern "C" {
    fn rocket_spell_create(
        requested: *const u8,
        length: usize,
        language: *mut c_char,
        capacity: usize,
    ) -> *mut c_void;
    fn rocket_spell_check(
        handle: *mut c_void,
        bytes: *const u8,
        length: usize,
        output: *mut NativeRange,
        capacity: usize,
    ) -> i32;
    fn rocket_spell_suggest(
        handle: *mut c_void,
        bytes: *const u8,
        length: usize,
        output: *mut c_char,
        slots: usize,
        stride: usize,
    ) -> i32;
    fn rocket_spell_word(handle: *mut c_void, bytes: *const u8, length: usize, learn: i32) -> i32;
    fn rocket_spell_destroy(handle: *mut c_void);
}
pub(crate) struct Checker {
    handle: *mut c_void,
    language: String,
}
fn failed() -> Error {
    Error::Failed("macOS spelling service is unavailable or returned invalid data".into())
}
impl Checker {
    pub fn new(language: Option<&str>) -> Result<Self, Error> {
        let requested = language.unwrap_or("");
        let mut output = [0i8; 128];
        // Buffers live for the synchronous C call; Cocoa objects stay on main.
        let handle = unsafe {
            rocket_spell_create(
                requested.as_ptr(),
                requested.len(),
                output.as_mut_ptr(),
                output.len(),
            )
        };
        if handle.is_null() {
            return Err(Error::Unavailable);
        }
        let language = unsafe { CStr::from_ptr(output.as_ptr()) }
            .to_string_lossy()
            .replace('_', "-");
        Ok(Self { handle, language })
    }
    pub fn language(&self) -> &str {
        &self.language
    }
    pub fn check(&self, text: &str) -> Result<Vec<Misspelling>, Error> {
        let mut ranges = vec![NativeRange::default(); 4096];
        let count = unsafe {
            rocket_spell_check(
                self.handle,
                text.as_ptr(),
                text.len(),
                ranges.as_mut_ptr(),
                ranges.len(),
            )
        };
        if count < 0 || count as usize > ranges.len() {
            return Err(failed());
        }
        let mut output = Vec::new();
        for range in &ranges[..count as usize] {
            let range = byte_range(text, range.start, range.length).ok_or_else(failed)?;
            if !text[range.clone()].chars().all(unchecked_script) {
                output.push(Misspelling {
                    range,
                    action: Action::Suggest,
                });
            }
        }
        Ok(output)
    }
    pub fn suggest(&self, word: &str) -> Result<Vec<String>, Error> {
        let mut words = [[0i8; 1024]; 8];
        let count = unsafe {
            rocket_spell_suggest(
                self.handle,
                word.as_ptr(),
                word.len(),
                words.as_mut_ptr().cast(),
                8,
                1024,
            )
        };
        if !(0..=8).contains(&count) {
            return Err(failed());
        }
        words[..count as usize]
            .iter()
            .map(|word| {
                unsafe { CStr::from_ptr(word.as_ptr()) }
                    .to_str()
                    .map(str::to_owned)
                    .map_err(|_| failed())
            })
            .collect()
    }
    fn change(&self, word: &str, learn: bool) -> Result<(), Error> {
        if word.is_empty() || word.len() > 1024 || word.chars().any(char::is_whitespace) {
            return Err(failed());
        }
        if unsafe { rocket_spell_word(self.handle, word.as_ptr(), word.len(), i32::from(learn)) }
            == 0
        {
            Ok(())
        } else {
            Err(failed())
        }
    }
    pub fn ignore(&self, word: &str) -> Result<(), Error> {
        self.change(word, false)
    }
    pub fn add(&self, word: &str) -> Result<(), Error> {
        self.change(word, true)
    }
}
impl Drop for Checker {
    fn drop(&mut self) {
        unsafe { rocket_spell_destroy(self.handle) };
    }
}
