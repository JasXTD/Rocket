//! The Windows Spell Checking API (spellcheck.h, Windows 8 and later) through COM.
//! Each vtable lists its interface's methods in the SDK header's order, up to the
//! last one called here. Everything runs on the worker thread that created it.
use crate::{Action, Error, Misspelling, byte_range, unchecked_script};
use std::{ffi::c_void, ptr};

#[repr(C)]
struct Guid(u32, u16, u16, [u8; 8]);

const CLSID_SPELL_CHECKER_FACTORY: Guid = Guid(
    0x7AB3_6653,
    0x1796,
    0x484B,
    [0xBD, 0xFA, 0xE7, 0x4F, 0x1D, 0xB7, 0xC1, 0xDC],
);
const IID_ISPELL_CHECKER_FACTORY: Guid = Guid(
    0x8E01_8A9D,
    0x2415,
    0x4677,
    [0xBF, 0x08, 0x79, 0x4E, 0xA6, 0x1F, 0x94, 0xBB],
);
const COINIT_MULTITHREADED: u32 = 0;
const CLSCTX_INPROC_SERVER: u32 = 1;
const S_OK: i32 = 0;
const S_FALSE: i32 = 1;
const REGDB_E_CLASSNOTREG: i32 = 0x8004_0154_u32 as i32;
const LOCALE_NAME_MAX_LENGTH: usize = 85;
const CORRECTIVE_ACTION_REPLACE: i32 = 2;
const CORRECTIVE_ACTION_DELETE: i32 = 3;

type This = *mut c_void;
#[repr(C)]
struct IUnknownVtbl {
    _query_interface: unsafe extern "system" fn(This, *const Guid, *mut This) -> i32,
    _add_ref: unsafe extern "system" fn(This) -> u32,
    release: unsafe extern "system" fn(This) -> u32,
}
#[repr(C)]
struct FactoryVtbl {
    base: IUnknownVtbl,
    _get_supported_languages: unsafe extern "system" fn(This, *mut This) -> i32,
    is_supported: unsafe extern "system" fn(This, *const u16, *mut i32) -> i32,
    create_spell_checker: unsafe extern "system" fn(This, *const u16, *mut This) -> i32,
}
#[repr(C)]
struct CheckerVtbl {
    base: IUnknownVtbl,
    _get_language_tag: unsafe extern "system" fn(This, *mut *mut u16) -> i32,
    check: unsafe extern "system" fn(This, *const u16, *mut This) -> i32,
    suggest: unsafe extern "system" fn(This, *const u16, *mut This) -> i32,
    add: unsafe extern "system" fn(This, *const u16) -> i32,
    ignore: unsafe extern "system" fn(This, *const u16) -> i32,
}
#[repr(C)]
struct EnumErrorsVtbl {
    base: IUnknownVtbl,
    next: unsafe extern "system" fn(This, *mut This) -> i32,
}
#[repr(C)]
struct ErrorVtbl {
    base: IUnknownVtbl,
    get_start_index: unsafe extern "system" fn(This, *mut u32) -> i32,
    get_length: unsafe extern "system" fn(This, *mut u32) -> i32,
    get_corrective_action: unsafe extern "system" fn(This, *mut i32) -> i32,
    get_replacement: unsafe extern "system" fn(This, *mut *mut u16) -> i32,
}
/// IEnumString from objidl.h.
#[repr(C)]
struct EnumStringVtbl {
    base: IUnknownVtbl,
    next: unsafe extern "system" fn(This, u32, *mut *mut u16, *mut u32) -> i32,
}

#[link(name = "ole32")]
unsafe extern "system" {
    fn CoInitializeEx(reserved: *mut c_void, model: u32) -> i32;
    fn CoUninitialize();
    fn CoCreateInstance(
        class: *const Guid,
        outer: This,
        context: u32,
        interface: *const Guid,
        object: *mut This,
    ) -> i32;
    fn CoTaskMemFree(memory: *mut c_void);
}
#[link(name = "kernel32")]
unsafe extern "system" {
    fn GetUserDefaultLocaleName(name: *mut u16, length: i32) -> i32;
}

fn failed(what: &str, hr: i32) -> Error {
    Error::Failed(format!("{what} (HRESULT 0x{:08X})", hr as u32))
}
fn check_hr(what: &str, hr: i32) -> Result<(), Error> {
    if hr < 0 {
        Err(failed(what, hr))
    } else {
        Ok(())
    }
}

/// A null-terminated UTF-16 copy. NUL would end the text early, so it becomes a
/// space, which keeps every UTF-16 offset in place.
fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16()
        .map(|unit| if unit == 0 { u16::from(b' ') } else { unit })
        .chain(std::iter::once(0))
        .collect()
}

/// Take ownership of a CoTaskMemAlloc'd, null-terminated UTF-16 string.
///
/// # Safety
/// `text` is null or such a string, not used afterwards.
unsafe fn take_string(text: *mut u16) -> String {
    if text.is_null() {
        return String::new();
    }
    // SAFETY: the caller passes a valid null-terminated string, freed once here.
    unsafe {
        let length = (0..).take_while(|&i| *text.add(i) != 0).count();
        let value = String::from_utf16_lossy(std::slice::from_raw_parts(text, length));
        CoTaskMemFree(text.cast());
        value
    }
}

/// One counted reference to a COM interface whose vtable starts with `V`.
struct Com<V>(ptr::NonNull<*const V>);
impl<V> Com<V> {
    /// # Safety
    /// `this` is null or an owned reference to an interface with vtable `V`.
    unsafe fn new(this: This) -> Option<Self> {
        ptr::NonNull::new(this.cast()).map(Self)
    }
    fn this(&self) -> This {
        self.0.as_ptr().cast()
    }
    fn vtable(&self) -> &V {
        // SAFETY: a live COM object's first field points to its vtable, which
        // starts with V's methods (checked against the SDK header).
        unsafe { &**self.0.as_ptr() }
    }
}
impl<V> Drop for Com<V> {
    fn drop(&mut self) {
        // SAFETY: every vtable starts with IUnknown; this releases our reference.
        unsafe {
            let unknown = &**self.0.as_ptr().cast::<*const IUnknownVtbl>();
            (unknown.release)(self.this());
        }
    }
}

/// Call a method that returns a new interface reference through its last argument.
fn out_interface<V>(
    what: &str,
    call: impl FnOnce(*mut This) -> i32,
) -> Result<Option<Com<V>>, Error> {
    let mut object = ptr::null_mut();
    let hr = call(&raw mut object);
    check_hr(what, hr)?;
    // SAFETY: on success the method stored null or an owned reference of type V.
    Ok(unsafe { Com::new(object) })
}

/// COM for this thread, released when the checker is dropped.
struct Apartment;
impl Apartment {
    fn enter() -> Result<Self, Error> {
        // SAFETY: plain initialization of COM for the calling worker thread.
        check_hr("initialize COM", unsafe {
            CoInitializeEx(ptr::null_mut(), COINIT_MULTITHREADED)
        })?;
        Ok(Self)
    }
}
impl Drop for Apartment {
    fn drop(&mut self) {
        // SAFETY: balances the successful CoInitializeEx on this thread.
        unsafe { CoUninitialize() }
    }
}

pub(crate) struct Checker {
    // Field order is drop order: the checker before COM itself.
    checker: Com<CheckerVtbl>,
    language: String,
    _apartment: Apartment,
}

impl Checker {
    pub fn new(language: Option<&str>) -> Result<Self, Error> {
        let apartment = Apartment::enter()?;
        let mut object = ptr::null_mut();
        // SAFETY: valid GUIDs and out pointer; the factory is an in-process server.
        let hr = unsafe {
            CoCreateInstance(
                &CLSID_SPELL_CHECKER_FACTORY,
                ptr::null_mut(),
                CLSCTX_INPROC_SERVER,
                &IID_ISPELL_CHECKER_FACTORY,
                &raw mut object,
            )
        };
        // Windows before 8 has no spell checking service.
        if hr == REGDB_E_CLASSNOTREG {
            return Err(Error::Unavailable);
        }
        check_hr("create the spell checker factory", hr)?;
        // SAFETY: on success CoCreateInstance stored an owned factory reference.
        let factory = unsafe { Com::<FactoryVtbl>::new(object) }.ok_or(Error::Unavailable)?;
        let candidates = match language {
            Some(tag) => vec![tag.to_owned()],
            None => [user_language(), Some("en-US".to_owned())]
                .into_iter()
                .flatten()
                .collect(),
        };
        let mut chosen = None;
        for tag in candidates {
            let wide_tag = wide(&tag);
            let mut supported = 0;
            // SAFETY: a live factory, a null-terminated tag and an out pointer.
            check_hr("check the language", unsafe {
                (factory.vtable().is_supported)(factory.this(), wide_tag.as_ptr(), &mut supported)
            })?;
            if supported != 0 {
                chosen = Some((tag, wide_tag));
                break;
            }
        }
        let (tag, wide_tag) = chosen.ok_or(Error::Unavailable)?;
        let checker = out_interface::<CheckerVtbl>("create the spell checker", |out| {
            // SAFETY: a live factory, a supported null-terminated tag and an out pointer.
            unsafe {
                (factory.vtable().create_spell_checker)(factory.this(), wide_tag.as_ptr(), out)
            }
        })?
        .ok_or(Error::Unavailable)?;
        drop(factory);
        Ok(Self {
            checker,
            language: tag,
            _apartment: apartment,
        })
    }

    pub fn language(&self) -> &str {
        &self.language
    }

    pub fn check(&self, text: &str) -> Result<Vec<Misspelling>, Error> {
        // The checker refuses empty text (E_INVALIDARG); blank text has no words anyway.
        if text.trim().is_empty() {
            return Ok(Vec::new());
        }
        let units = wide(text);
        let Some(errors) = out_interface::<EnumErrorsVtbl>("check the text", |out| {
            // SAFETY: a live checker, a null-terminated text and an out pointer.
            unsafe { (self.checker.vtable().check)(self.checker.this(), units.as_ptr(), out) }
        })?
        else {
            return Ok(Vec::new());
        };
        let mut found = Vec::new();
        loop {
            let mut item = ptr::null_mut();
            // SAFETY: a live enumerator and an out pointer.
            let hr = unsafe { (errors.vtable().next)(errors.this(), &mut item) };
            check_hr("read the next spelling error", hr)?;
            // SAFETY: Next stored null or an owned ISpellingError reference.
            let Some(error) = (unsafe { Com::<ErrorVtbl>::new(item) }) else {
                break;
            };
            let (mut start, mut length, mut action) = (0, 0, 0);
            let mut replacement = ptr::null_mut();
            let vtable = error.vtable();
            // SAFETY: a live spelling error and out pointers of the declared types.
            unsafe {
                check_hr(
                    "read a spelling error",
                    (vtable.get_start_index)(error.this(), &mut start),
                )?;
                check_hr(
                    "read a spelling error",
                    (vtable.get_length)(error.this(), &mut length),
                )?;
                check_hr(
                    "read a spelling error",
                    (vtable.get_corrective_action)(error.this(), &mut action),
                )?;
                check_hr(
                    "read a spelling error",
                    (vtable.get_replacement)(error.this(), &mut replacement),
                )?;
            }
            // SAFETY: get_Replacement returns null or a CoTaskMemAlloc'd string.
            let replacement = unsafe { take_string(replacement) };
            // Offsets that split a character cannot be shown; skip them.
            if let Some(range) = byte_range(text, start as usize, length as usize)
                && !text[range.clone()].chars().all(unchecked_script)
            {
                found.push(Misspelling {
                    range,
                    action: match action {
                        CORRECTIVE_ACTION_REPLACE => Action::Replace(replacement),
                        CORRECTIVE_ACTION_DELETE => Action::Delete,
                        _ => Action::Suggest,
                    },
                });
            }
            if hr == S_FALSE {
                break;
            }
        }
        Ok(found)
    }

    pub fn suggest(&self, word: &str) -> Result<Vec<String>, Error> {
        let units = wide(word);
        let Some(strings) = out_interface::<EnumStringVtbl>("suggest words", |out| {
            // SAFETY: a live checker, a null-terminated word and an out pointer.
            unsafe { (self.checker.vtable().suggest)(self.checker.this(), units.as_ptr(), out) }
        })?
        else {
            return Ok(Vec::new());
        };
        let mut words = Vec::new();
        loop {
            let (mut item, mut fetched) = (ptr::null_mut(), 0);
            // SAFETY: a live enumerator, room for one string and a count.
            let hr = unsafe { (strings.vtable().next)(strings.this(), 1, &mut item, &mut fetched) };
            check_hr("read a suggestion", hr)?;
            if fetched == 0 {
                break;
            }
            // SAFETY: Next returned one CoTaskMemAlloc'd string.
            words.push(unsafe { take_string(item) });
            if hr != S_OK {
                break;
            }
        }
        Ok(words)
    }

    pub fn ignore(&self, word: &str) -> Result<(), Error> {
        let units = wide(word);
        // SAFETY: a live checker and a null-terminated word.
        check_hr("ignore the word", unsafe {
            (self.checker.vtable().ignore)(self.checker.this(), units.as_ptr())
        })
    }

    pub fn add(&self, word: &str) -> Result<(), Error> {
        let units = wide(word);
        // SAFETY: a live checker and a null-terminated word.
        check_hr("add the word to the dictionary", unsafe {
            (self.checker.vtable().add)(self.checker.this(), units.as_ptr())
        })
    }
}

/// The user's Windows language, such as `en-GB`.
fn user_language() -> Option<String> {
    let mut name = [0u16; LOCALE_NAME_MAX_LENGTH];
    // SAFETY: the buffer holds LOCALE_NAME_MAX_LENGTH units, as the length says.
    let length = unsafe { GetUserDefaultLocaleName(name.as_mut_ptr(), name.len() as i32) };
    // The length includes the terminating NUL; 0 means failure.
    (length > 1).then(|| String::from_utf16_lossy(&name[..length as usize - 1]))
}
