//! Linux: data is sealed with XChaCha20-Poly1305 under one random 256-bit key that the
//! desktop keyring keeps (the Secret Service, through GNOME's libsecret), as DPAPI keeps
//! its key behind the Windows login. Only the sealed bytes reach Rocket's files:
//! "RKS1", a random 24-byte nonce, then the ciphertext and its 16-byte tag.
//!
//! libsecret is loaded when first needed, so Rocket builds and runs without it; it
//! handles the keyring's encrypted session and any unlock prompt. Its calls can wait
//! for that prompt, so they are made only from workers, never the UI thread.
#[cfg(test)]
use super::sealed::TAG;
use super::sealed::{self, NONCE};
use crate::{Error, Result};
use std::{
    ffi::{CStr, c_char, c_int, c_void},
    sync::{Mutex, OnceLock, PoisonError},
    time::{Duration, Instant},
};

const MAGIC: &[u8; 4] = b"RKS1";
/// Binds each blob to this use: data sealed for another purpose never opens here.
const AAD: &[u8] = b"Rocket protected credential v1";

fn refused() -> Error {
    Error::InvalidInput(
        "Hosting credentials could not be protected or decrypted with Rocket's key in your desktop keyring. Existing data was preserved.".into(),
    )
}

/// Seal `bytes` under `key`. The nonce must be fresh random bytes for every call.
fn seal(key: &[u8; 32], nonce: [u8; NONCE], bytes: &[u8]) -> Result<Vec<u8>> {
    sealed::seal(MAGIC, AAD, key, nonce, bytes).ok_or_else(refused)
}
/// Keep the existing Linux envelope and associated data; only the cipher primitive
/// is shared with macOS. Other platforms' protected files remain incompatible.
fn open(key: &[u8; 32], blob: &[u8]) -> Result<Vec<u8>> {
    sealed::open(MAGIC, AAD, key, blob).ok_or_else(refused)
}
pub(super) fn protect(bytes: &[u8], encrypt: bool) -> Result<Vec<u8>> {
    let key = key(encrypt)?;
    if encrypt {
        let mut nonce = [0u8; NONCE];
        random(&mut nonce)?;
        seal(&key, nonce, bytes)
    } else {
        open(&key, bytes)
    }
}

/// Fill `buffer` from the kernel's random source (getrandom(2)).
fn random(buffer: &mut [u8]) -> Result<()> {
    unsafe extern "C" {
        fn getrandom(buffer: *mut u8, length: usize, flags: u32) -> isize;
    }
    let mut filled = 0;
    while filled < buffer.len() {
        // SAFETY: the rest of a live, writable buffer of exactly that length.
        let read = unsafe { getrandom(buffer[filled..].as_mut_ptr(), buffer.len() - filled, 0) };
        if read < 0 {
            let error = std::io::Error::last_os_error();
            if error.kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            return Err(Error::io("generate credential key", error));
        }
        filled += read as usize;
    }
    Ok(())
}

/// The key, read from the keyring once per process. Encrypting creates it the first time;
/// decrypting never does, since a new key could not open earlier data.
fn key(create: bool) -> Result<[u8; 32]> {
    static KEY: Mutex<Option<[u8; 32]>> = Mutex::new(None);
    let mut cached = KEY.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some(key) = *cached {
        return Ok(key);
    }
    let keyring = Keyring::load()?;
    let key = match keyring.lookup()? {
        Some(key) => key,
        None if create => {
            // Storing replaces a key with the same attributes, so two Rocket processes must
            // not create one at once: the second would orphan what the first sealed.
            let _creating = creation_lock()?;
            match keyring.lookup()? {
                Some(key) => key,
                None => {
                    let mut key = [0u8; 32];
                    random(&mut key)?;
                    keyring.store(&key)?;
                    // Use what the keyring holds now, never only what was sent.
                    keyring.lookup()?.ok_or_else(missing)?
                }
            }
        }
        None => return Err(missing()),
    };
    *cached = Some(key);
    Ok(key)
}

fn missing() -> Error {
    Error::InvalidInput(
        "Rocket's key is not in your desktop keyring, so data saved with it can't be decrypted. Save the token or passphrase again. Existing files were preserved.".into(),
    )
}

/// Held while a key is created: `credential-key.lock` in Rocket's folder.
fn creation_lock() -> Result<std::fs::File> {
    let directory = crate::local_storage::directory()
        .map_err(|e| Error::io("open private credential key storage", e))?;
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(directory.join("credential-key.lock"))
        .map_err(|e| Error::io("open the credential key lock", e))?;
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        match file.try_lock() {
            Ok(()) => return Ok(file),
            Err(std::fs::TryLockError::WouldBlock) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(std::fs::TryLockError::WouldBlock) => {
                return Err(Error::InvalidInput(
                    "Another Rocket window is setting up its credential key. Try again in a moment.".into(),
                ));
            }
            Err(std::fs::TryLockError::Error(e)) => return Err(Error::io("lock the credential key", e)),
        }
    }
}

// libsecret's ABI, from secret-schema.h and glib's GError.
#[repr(C)]
#[derive(Clone, Copy)]
struct SchemaAttribute {
    name: *const c_char,
    kind: c_int,
}
#[repr(C)]
struct Schema {
    name: *const c_char,
    flags: c_int,
    attributes: [SchemaAttribute; 32],
    reserved: c_int,
    reserved_pointers: [*mut c_void; 7],
}
// SAFETY: the schema is immutable static data pointing only at static strings.
unsafe impl Sync for Schema {}
#[repr(C)]
struct GError {
    domain: u32,
    code: c_int,
    message: *mut c_char,
}
type Store = unsafe extern "C" fn(
    *const Schema,
    *const c_char,
    *const c_char,
    *const c_char,
    *mut c_void,
    *mut *mut GError,
    ...
) -> c_int;
type Lookup = unsafe extern "C" fn(*const Schema, *mut c_void, *mut *mut GError, ...) -> *mut c_char;
type Free = unsafe extern "C" fn(*mut c_char);
type ErrorFree = unsafe extern "C" fn(*mut GError);

const NO_ATTRIBUTE: SchemaAttribute = SchemaAttribute { name: std::ptr::null(), kind: 0 };
const PURPOSE: &CStr = c"purpose";
const PURPOSE_VALUE: &CStr = c"credential-protection-key";
const VERSION: &CStr = c"version";
const VERSION_VALUE: &CStr = c"1";
static SCHEMA: Schema = Schema {
    name: c"io.github.JasXTD.Rocket.CredentialKey".as_ptr(),
    flags: 0,
    attributes: {
        let mut attributes = [NO_ATTRIBUTE; 32];
        attributes[0] = SchemaAttribute { name: PURPOSE.as_ptr(), kind: 0 };
        attributes[1] = SchemaAttribute { name: VERSION.as_ptr(), kind: 0 };
        attributes
    },
    reserved: 0,
    reserved_pointers: [std::ptr::null_mut(); 7],
};

struct Keyring {
    store: Store,
    lookup: Lookup,
    free: Free,
    error_free: ErrorFree,
}

impl Keyring {
    /// libsecret, loaded once and kept for the life of the process.
    fn load() -> Result<&'static Self> {
        static LOADED: OnceLock<std::result::Result<Keyring, String>> = OnceLock::new();
        LOADED
            .get_or_init(Self::open_library)
            .as_ref()
            .map_err(|reason| Error::Unsupported(reason.clone()))
    }

    fn open_library() -> std::result::Result<Self, String> {
        unsafe extern "C" {
            fn dlopen(file: *const c_char, flags: c_int) -> *mut c_void;
            fn dlsym(library: *mut c_void, name: *const c_char) -> *mut c_void;
        }
        const RTLD_NOW: c_int = 2;
        let unavailable = || {
            "Remembering hosting tokens needs your desktop keyring: libsecret (Ubuntu's libsecret-1-0) and a keyring such as GNOME Keyring or KeePassXC.".to_owned()
        };
        let path = system_library("libsecret-1.so.0").ok_or_else(unavailable)?;
        // SAFETY: a NUL-terminated full path to a library the system installed.
        let library = unsafe { dlopen(path.as_ptr(), RTLD_NOW) };
        if library.is_null() {
            return Err(unavailable());
        }
        let symbol = |name: &CStr| {
            // SAFETY: a live library handle and a NUL-terminated name. g_error_free is
            // found in glib, which libsecret loads with itself.
            let address = unsafe { dlsym(library, name.as_ptr()) };
            if address.is_null() { Err(unavailable()) } else { Ok(address) }
        };
        // SAFETY: each address is the named function with exactly the C signature
        // declared above, from libsecret's and glib's public headers. The library is
        // never closed, so the pointers stay valid.
        unsafe {
            Ok(Self {
                store: std::mem::transmute::<*mut c_void, Store>(symbol(c"secret_password_store_sync")?),
                lookup: std::mem::transmute::<*mut c_void, Lookup>(symbol(c"secret_password_lookup_sync")?),
                free: std::mem::transmute::<*mut c_void, Free>(symbol(c"secret_password_free")?),
                error_free: std::mem::transmute::<*mut c_void, ErrorFree>(symbol(c"g_error_free")?),
            })
        }
    }

    /// A failed call's message, which never contains the secret; the error is freed.
    fn failure(&self, error: *mut GError) -> Error {
        let message = if error.is_null() {
            "no reason given".to_owned()
        } else {
            // SAFETY: libsecret set a live GError; its message is NUL-terminated or null.
            let text = unsafe {
                let message = (*error).message;
                if message.is_null() { String::new() } else { CStr::from_ptr(message).to_string_lossy().into_owned() }
            };
            // SAFETY: this error is owned here and freed once.
            unsafe { (self.error_free)(error) };
            text
        };
        Error::InvalidInput(format!(
            "Your desktop keyring did not answer ({message}). Unlock it or start one, such as GNOME Keyring, then try again. Existing data was preserved."
        ))
    }

    fn lookup(&self) -> Result<Option<[u8; 32]>> {
        let mut error = std::ptr::null_mut();
        // SAFETY: the static schema, no cancellable, an error out-pointer, then attribute
        // name/value pairs ended by a null pointer, as the variadic API requires.
        let found = unsafe {
            (self.lookup)(
                &SCHEMA,
                std::ptr::null_mut(),
                &mut error,
                PURPOSE.as_ptr(),
                PURPOSE_VALUE.as_ptr(),
                VERSION.as_ptr(),
                VERSION_VALUE.as_ptr(),
                std::ptr::null::<c_char>(),
            )
        };
        if !error.is_null() {
            if !found.is_null() {
                // SAFETY: a password libsecret returned; freed (and cleared) once.
                unsafe { (self.free)(found) };
            }
            return Err(self.failure(error));
        }
        if found.is_null() {
            return Ok(None);
        }
        // SAFETY: a live NUL-terminated password; it is read, then cleared and freed.
        let key = unsafe { parse_key(CStr::from_ptr(found).to_bytes()) };
        unsafe { (self.free)(found) };
        key.map(Some).ok_or_else(|| {
            Error::InvalidInput(
                "Rocket's key in your desktop keyring is damaged, so it was not used or replaced. Remove \"Rocket credential protection key\" from the keyring to start again; data saved with it can't be decrypted then.".into(),
            )
        })
    }

    fn store(&self, key: &[u8; 32]) -> Result<()> {
        // Hex digits written in place: no heap copy of the key is left behind.
        const HEX: &[u8; 16] = b"0123456789abcdef";
        let mut text = [0u8; 65];
        for (index, byte) in key.iter().enumerate() {
            text[index * 2] = HEX[usize::from(byte >> 4)];
            text[index * 2 + 1] = HEX[usize::from(byte & 15)];
        }
        let mut error = std::ptr::null_mut();
        // SAFETY: as in lookup; the key is a NUL-terminated hex string, the collection is
        // null (the default one), and the label is shown in keyring managers.
        let stored = unsafe {
            (self.store)(
                &SCHEMA,
                std::ptr::null(),
                c"Rocket credential protection key".as_ptr(),
                text.as_ptr().cast(),
                std::ptr::null_mut(),
                &mut error,
                PURPOSE.as_ptr(),
                PURPOSE_VALUE.as_ptr(),
                VERSION.as_ptr(),
                VERSION_VALUE.as_ptr(),
                std::ptr::null::<c_char>(),
            )
        };
        text.fill(0);
        std::hint::black_box(&text);
        if stored == 0 || !error.is_null() {
            return Err(self.failure(error));
        }
        Ok(())
    }
}

/// A system library by its full path in the system's own library folders, and only one
/// that root owns, in folders root owns, none of them writable by anyone else. A library
/// of that name elsewhere, as LD_LIBRARY_PATH or a planted copy could offer, is never used.
fn system_library(name: &str) -> Option<std::ffi::CString> {
    use std::os::unix::ffi::OsStringExt;
    let triplet = if cfg!(target_arch = "x86_64") {
        "x86_64-linux-gnu"
    } else if cfg!(target_arch = "aarch64") {
        "aarch64-linux-gnu"
    } else {
        ""
    };
    [format!("/usr/lib/{triplet}"), "/usr/lib64".into(), "/usr/lib".into(), format!("/lib/{triplet}"), "/lib64".into()]
        .into_iter()
        .filter(|folder| !folder.ends_with('/'))
        .find_map(|folder| {
            // The soname is a link to the versioned file; trust what it really is.
            let real = std::fs::canonicalize(std::path::Path::new(&folder).join(name)).ok()?;
            (real.is_file() && root_owned(&real)).then_some(())?;
            std::ffi::CString::new(real.into_os_string().into_vec()).ok()
        })
}

/// The file and every folder above it belong to root, and only root can change them.
fn root_owned(path: &std::path::Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    path.ancestors()
        .all(|part| std::fs::metadata(part).is_ok_and(|found| found.uid() == 0 && found.mode() & 0o022 == 0))
}

/// 64 lowercase hex digits, as `store` writes them.
fn parse_key(text: &[u8]) -> Option<[u8; 32]> {
    if text.len() != 64 {
        return None;
    }
    let digit = |c: u8| match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        _ => None,
    };
    let mut key = [0u8; 32];
    for (index, pair) in text.chunks(2).enumerate() {
        key[index] = (digit(pair[0])? << 4) | digit(pair[1])?;
    }
    Some(key)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sealed_data_opens_only_with_its_key_and_untouched() {
        let key = [7u8; 32];
        let blob = seal(&key, [9u8; NONCE], b"ghp_disposable token").unwrap();
        assert_eq!(&blob[..4], b"RKS1");
        assert_eq!(blob.len(), 4 + NONCE + 20 + TAG);
        assert!(!blob.windows(5).any(|w| w == b"token"), "the plaintext must not show");
        assert_eq!(open(&key, &blob).unwrap(), b"ghp_disposable token");
        // Another key, a changed byte anywhere, or a cut blob is refused.
        assert!(open(&[8u8; 32], &blob).is_err());
        for at in [4, 4 + NONCE, blob.len() - 1] {
            let mut changed = blob.clone();
            changed[at] ^= 1;
            assert!(open(&key, &changed).is_err(), "byte {at}");
        }
        assert!(open(&key, &blob[..blob.len() - 1]).is_err());
        assert!(open(&key, &blob[..4 + NONCE + TAG - 1]).is_err());
        // A Windows DPAPI blob, or any other format, is not mistaken for one.
        assert!(open(&key, b"\x01\x00\x00\x00\xd0\x8c\x9d\xdf\x01\x15\xd1\x11").is_err());
        // Fresh nonces: the same data seals differently each time.
        let mut nonce = [0u8; NONCE];
        random(&mut nonce).unwrap();
        assert_ne!(seal(&key, nonce, b"same").unwrap(), seal(&key, [0u8; NONCE], b"same").unwrap());
    }

    #[test]
    fn only_root_owned_system_libraries_are_loaded() {
        // A copy of a library in a folder this user owns is not trusted, wherever it is.
        let folder = std::env::temp_dir().join(format!("rocket-planted-{}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        let planted = folder.join("libsecret-1.so.0");
        std::fs::write(&planted, b"not a library").unwrap();
        assert!(!root_owned(&planted));
        assert!(root_owned(std::path::Path::new("/usr/lib")));
        assert_eq!(system_library("rocket-no-such-library.so.0"), None);
        std::fs::remove_dir_all(folder).unwrap();
    }

    #[test]
    fn keys_are_read_back_only_as_written() {
        let key: [u8; 32] = std::array::from_fn(|i| (i * 37) as u8);
        let text: String = key.iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(parse_key(text.as_bytes()), Some(key));
        assert_eq!(parse_key(text.to_uppercase().as_bytes()), None);
        assert_eq!(parse_key(&text.as_bytes()[..62]), None);
        assert_eq!(parse_key(b""), None);
        assert_eq!(std::mem::size_of::<Schema>(), 592, "libsecret's SecretSchema");
    }

    /// Against a real, unlocked Secret Service (see guide.md); creates Rocket's key there.
    #[test]
    #[ignore = "needs an unlocked desktop keyring: run with --ignored in a session that has one"]
    fn the_keyring_keeps_one_key_that_seals_and_opens() {
        let sealed = protect(b"disposable keyring test", true).unwrap();
        assert_eq!(protect(&sealed, false).unwrap(), b"disposable keyring test");
        // What was sealed is under the key the keyring holds, not a copy only in memory.
        let stored = Keyring::load().unwrap().lookup().unwrap().expect("the key is in the keyring");
        assert_eq!(open(&stored, &sealed).unwrap(), b"disposable keyring test");
    }
}
