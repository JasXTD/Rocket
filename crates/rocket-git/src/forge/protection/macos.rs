//! File-based macOS Keychain, usable by desktop/CLI helpers without an entitlement.
//! Only a random wrapping key is stored there; encrypted envelopes stay in Rocket's
//! private files. No security CLI, plaintext file, inherited key or Keychain reset.
use super::macos_policy::{self, Key, Store};
use crate::darwin_acl as acl;
use crate::{Error, Result};
use std::{
    ffi::{CString, c_char, c_void},
    ptr,
    sync::{Mutex, MutexGuard},
};
type Cf = *const c_void;
#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFRelease(object: Cf);
    fn CFGetTypeID(object: Cf) -> usize;
    fn CFDataGetTypeID() -> usize;
    fn CFDataCreate(allocator: Cf, bytes: *const u8, count: isize) -> Cf;
    fn CFDataGetLength(data: Cf) -> isize;
    fn CFDataGetBytePtr(data: Cf) -> *const u8;
    fn CFStringCreateWithBytes(
        allocator: Cf,
        bytes: *const u8,
        count: isize,
        encoding: u32,
        external: u8,
    ) -> Cf;
    fn CFArrayCreate(
        allocator: Cf,
        values: *const Cf,
        count: isize,
        callbacks: *const c_void,
    ) -> Cf;
    fn CFDictionaryCreate(
        allocator: Cf,
        keys: *const Cf,
        values: *const Cf,
        count: isize,
        keys_callbacks: *const c_void,
        values_callbacks: *const c_void,
    ) -> Cf;
    // Opaque callback symbols: only their addresses are passed back to CF.
    static kCFTypeArrayCallBacks: c_void;
    static kCFTypeDictionaryKeyCallBacks: c_void;
    static kCFTypeDictionaryValueCallBacks: c_void;
    static kCFBooleanTrue: Cf;
}
#[link(name = "Security", kind = "framework")]
unsafe extern "C" {
    fn SecRandomCopyBytes(random: Cf, count: usize, bytes: *mut c_void) -> i32;
    fn SecKeychainCopyDefault(keychain: *mut Cf) -> i32;
    fn SecKeychainGetUserInteractionAllowed(allowed: *mut u8) -> i32;
    fn SecKeychainSetUserInteractionAllowed(allowed: u8) -> i32;
    fn SecItemCopyMatching(query: Cf, result: *mut Cf) -> i32;
    fn SecItemAdd(query: Cf, result: *mut Cf) -> i32;
    fn SecTrustedApplicationCreateFromPath(path: *const c_char, app: *mut Cf) -> i32;
    fn SecAccessCreate(description: Cf, trusted: Cf, access: *mut Cf) -> i32;
    static kSecClass: Cf;
    static kSecClassGenericPassword: Cf;
    static kSecAttrService: Cf;
    static kSecAttrAccount: Cf;
    static kSecAttrLabel: Cf;
    static kSecAttrAccess: Cf;
    static kSecMatchSearchList: Cf;
    static kSecMatchLimit: Cf;
    static kSecMatchLimitOne: Cf;
    static kSecReturnData: Cf;
    static kSecValueData: Cf;
    static kSecUseKeychain: Cf;
}

fn failure(status: i32) -> Error {
    let hint = match status {
        -25293 | -25308 | -128 => {
            "macos_keychain_denied: Unlock your default Keychain and review Rocket's access in Keychain Access, then retry. No credential was replaced."
        }
        -25294 | -25307 => {
            "macos_keychain_unavailable: No usable default Keychain is available. Open Keychain Access in your desktop session, then retry. Existing files were preserved."
        }
        _ => {
            "macos_keychain_failed: The Keychain operation failed. Existing credentials were preserved."
        }
    };
    Error::InvalidInput(format!("{hint} (OSStatus {status})"))
}
fn check(status: i32) -> Result<()> {
    if status == 0 {
        Ok(())
    } else {
        Err(failure(status))
    }
}

struct Owned(Cf);
impl Owned {
    fn new(value: Cf) -> Result<Self> {
        if value.is_null() {
            Err(failure(-108))
        } else {
            Ok(Self(value))
        }
    }
    fn string(value: &str) -> Result<Self> {
        // SAFETY: borrowed UTF-8 bytes; CF copies them. kCFStringEncodingUTF8.
        Self::new(unsafe {
            CFStringCreateWithBytes(
                ptr::null(),
                value.as_ptr(),
                value.len() as isize,
                0x08000100,
                0,
            )
        })
    }
    fn array(values: &[Cf]) -> Result<Self> {
        // SAFETY: CF retains each live object using its own supplied callbacks.
        Self::new(unsafe {
            CFArrayCreate(
                ptr::null(),
                values.as_ptr(),
                values.len() as isize,
                ptr::addr_of!(kCFTypeArrayCallBacks),
            )
        })
    }
    fn dictionary(pairs: &[(Cf, Cf)]) -> Result<Self> {
        let keys: Vec<_> = pairs.iter().map(|p| p.0).collect();
        let values: Vec<_> = pairs.iter().map(|p| p.1).collect();
        // SAFETY: equally sized live object arrays; CF owns retained copies.
        Self::new(unsafe {
            CFDictionaryCreate(
                ptr::null(),
                keys.as_ptr(),
                values.as_ptr(),
                pairs.len() as isize,
                ptr::addr_of!(kCFTypeDictionaryKeyCallBacks),
                ptr::addr_of!(kCFTypeDictionaryValueCallBacks),
            )
        })
    }
}
impl Drop for Owned {
    fn drop(&mut self) {
        unsafe { CFRelease(self.0) };
    }
}

// Legacy/file Keychains do not reliably honor SecItem's per-query no-UI option.
// Serialize this module's calls and temporarily suppress prompts in this process,
// restoring the prior setting on success, errors and unwind. No OS preference,
// Keychain lock state or other process is changed. No other Rocket module calls
// the legacy Security API; new consumers must use this boundary too.
static ACCESS: Mutex<()> = Mutex::new(());
struct NoUi {
    previous: u8,
    active: bool,
    _lock: MutexGuard<'static, ()>,
}
impl NoUi {
    fn new() -> Result<Self> {
        let lock = ACCESS.lock().map_err(|_| failure(-50))?;
        let mut previous = 0;
        check(unsafe { SecKeychainGetUserInteractionAllowed(&mut previous) })?;
        check(unsafe { SecKeychainSetUserInteractionAllowed(0) })?;
        Ok(Self {
            previous,
            active: true,
            _lock: lock,
        })
    }
    fn finish<T>(mut self, result: Result<T>) -> Result<T> {
        check(unsafe { SecKeychainSetUserInteractionAllowed(self.previous) })?;
        self.active = false;
        result
    }
}
impl Drop for NoUi {
    fn drop(&mut self) {
        if self.active {
            unsafe { SecKeychainSetUserInteractionAllowed(self.previous) };
        }
    }
}

struct Keychain(Owned);
impl Keychain {
    fn default() -> Result<Self> {
        let mut handle = ptr::null();
        check(unsafe { SecKeychainCopyDefault(&mut handle) })?;
        Ok(Self(Owned::new(handle)?))
    }
    fn identity(&self) -> Result<(Owned, Owned)> {
        Ok((
            Owned::string("io.github.JasXTD.Rocket.CredentialKey")?,
            Owned::string("v1")?,
        ))
    }
    fn access(&self) -> Result<Owned> {
        let description = Owned::string("Rocket credential protection key")?;
        let mut applications = Vec::new();
        let mut current = ptr::null();
        check(unsafe { SecTrustedApplicationCreateFromPath(ptr::null(), &mut current) })?;
        applications.push(Owned::new(current)?);
        // Trust only verified sibling Rocket executables, not any application.
        // Security.framework records their code identities. Existing ACLs are
        // never broadened on load, upgrade or an access-denied response.
        for path in sibling_helpers()? {
            let file = trusted_executable(&path)?;
            use std::os::unix::ffi::OsStrExt;
            let name = CString::new(path.as_os_str().as_bytes()).map_err(|_| failure(-50))?;
            let mut app = ptr::null();
            check(unsafe { SecTrustedApplicationCreateFromPath(name.as_ptr(), &mut app) })?;
            let owned = Owned::new(app)?;
            if !crate::local_files::opened_without_link(&file, &path).map_err(|_| failure(-50))? {
                return Err(failure(-50));
            }
            applications.push(owned);
        }
        let pointers: Vec<_> = applications.iter().map(|a| a.0).collect();
        let list = Owned::array(&pointers)?;
        let mut access = ptr::null();
        check(unsafe { SecAccessCreate(description.0, list.0, &mut access) })?;
        Owned::new(access)
    }
}
fn helper_refused() -> Error {
    Error::InvalidInput("macos_keychain_helper_untrusted: Rocket's helper location or permissions could not be verified. Use a complete Rocket bundle in a folder writable only by you or the system. Existing Keychain access rules were preserved.".into())
}
fn sibling_helpers() -> Result<Vec<std::path::PathBuf>> {
    let current = std::env::current_exe().map_err(|_| helper_refused())?;
    let directory = current.parent().ok_or_else(helper_refused)?;
    // Test binaries in deps deliberately trust only themselves. Native helper
    // interoperability is tested from a staged, complete app in a disposable user.
    if directory.file_name().is_some_and(|n| n == "deps") {
        return Ok(vec![]);
    }
    ["rocket", "rocket-mcp", "rocket-cli", "rocket-ssh"]
        .into_iter()
        .filter_map(|name| {
            let path = directory.join(name);
            match std::fs::symlink_metadata(&path) {
                Ok(_) if path != current => Some(Ok(path)),
                Ok(_) => None,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
                Err(_) => Some(Err(helper_refused())),
            }
        })
        .collect()
}
fn trusted_executable(path: &std::path::Path) -> Result<std::fs::File> {
    use std::os::unix::fs::MetadataExt;
    unsafe extern "C" {
        fn geteuid() -> u32;
    }
    let user = unsafe { geteuid() };
    for ancestor in path.ancestors().skip(1) {
        let m = std::fs::symlink_metadata(ancestor).map_err(|_| helper_refused())?;
        if !m.is_dir()
            || (m.uid() != 0 && m.uid() != user)
            || (m.mode() & 0o022 != 0 && !(m.uid() == 0 && m.mode() & 0o1000 != 0))
        {
            return Err(helper_refused());
        }
        let mut options = std::fs::OpenOptions::new();
        options.read(true);
        crate::local_files::no_follow(&mut options);
        let directory = options.open(ancestor).map_err(|_| helper_refused())?;
        acl::check(&directory, ancestor, &m).map_err(|_| helper_refused())?;
        if !crate::local_files::opened_without_link(&directory, ancestor)
            .map_err(|_| helper_refused())?
        {
            return Err(helper_refused());
        }
    }
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    crate::local_files::no_follow(&mut options);
    let file = options.open(path).map_err(|_| helper_refused())?;
    let m = file.metadata().map_err(|_| helper_refused())?;
    if !m.is_file()
        || m.mode() & 0o111 == 0
        || m.mode() & 0o022 != 0
        || (m.uid() != user && m.uid() != 0)
    {
        return Err(helper_refused());
    }
    acl::check(&file, path, &m).map_err(|_| helper_refused())?;
    if !crate::local_files::opened_without_link(&file, path).map_err(|_| helper_refused())? {
        return Err(helper_refused());
    }
    Ok(file)
}
impl Store for Keychain {
    fn lookup(&self) -> Result<Option<Key>> {
        let (service, account) = self.identity()?;
        let list = Owned::array(&[self.0.0])?;
        let query = unsafe {
            Owned::dictionary(&[
                (kSecClass, kSecClassGenericPassword),
                (kSecAttrService, service.0),
                (kSecAttrAccount, account.0),
                (kSecMatchSearchList, list.0),
                (kSecMatchLimit, kSecMatchLimitOne),
                (kSecReturnData, kCFBooleanTrue),
            ])
        }?;
        let mut data = ptr::null();
        let status = unsafe { SecItemCopyMatching(query.0, &mut data) };
        if status == -25300 {
            return Ok(None);
        }
        check(status)?;
        let data = Owned::new(data)?;
        if unsafe { CFGetTypeID(data.0) != CFDataGetTypeID() || CFDataGetLength(data.0) != 32 } {
            return Err(macos_policy::refused());
        }
        let bytes = unsafe { CFDataGetBytePtr(data.0) };
        if bytes.is_null() {
            return Err(macos_policy::refused());
        }
        let mut key = Key([0; 32]);
        key.0
            .copy_from_slice(unsafe { std::slice::from_raw_parts(bytes, 32) });
        Ok(Some(key))
    }
    fn add(&self, key: &Key) -> Result<()> {
        let (service, account) = self.identity()?;
        let label = Owned::string("Rocket credential protection key")?;
        let access = self.access()?;
        let data = Owned::new(unsafe { CFDataCreate(ptr::null(), key.0.as_ptr(), 32) })?;
        let query = unsafe {
            Owned::dictionary(&[
                (kSecClass, kSecClassGenericPassword),
                (kSecAttrService, service.0),
                (kSecAttrAccount, account.0),
                (kSecAttrLabel, label.0),
                (kSecAttrAccess, access.0),
                (kSecValueData, data.0),
                (kSecUseKeychain, self.0.0),
            ])
        }?;
        // There is intentionally no SecItemUpdate or delete-and-retry here.
        let status = unsafe { SecItemAdd(query.0, ptr::null_mut()) };
        if status == -25299 {
            Ok(())
        } else {
            check(status)
        }
    }
    fn random(&self, bytes: &mut [u8]) -> Result<()> {
        check(unsafe { SecRandomCopyBytes(ptr::null(), bytes.len(), bytes.as_mut_ptr().cast()) })
    }
}

pub(super) fn protect(bytes: &[u8], encrypt: bool) -> Result<Vec<u8>> {
    macos_policy::validate(bytes, encrypt)?;
    let guard = NoUi::new()?;
    let result =
        Keychain::default().and_then(|store| macos_policy::protect(&store, bytes, encrypt));
    guard.finish(result)
}

#[cfg(test)]
#[path = "macos_tests.rs"]
mod tests;
