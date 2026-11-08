//! Native Security.framework test. Never use the developer's default Keychain.
use super::*;
use std::{fs, os::unix::ffi::OsStrExt, path::PathBuf};

#[link(name = "Security", kind = "framework")]
unsafe extern "C" {
    fn SecKeychainCreate(
        path: *const c_char,
        length: u32,
        password: Cf,
        prompt: u8,
        access: Cf,
        keychain: *mut Cf,
    ) -> i32;
    fn SecKeychainDelete(keychain: Cf) -> i32;
    fn SecKeychainUnlock(keychain: Cf, length: u32, password: Cf, use_password: u8) -> i32;
    fn SecKeychainLock(keychain: Cf) -> i32;
    fn SecKeychainCopySearchList(list: *mut Cf) -> i32;
    fn SecKeychainSetSearchList(list: Cf) -> i32;
}
#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFEqual(a: Cf, b: Cf) -> u8;
}

struct Fixture {
    keychain: Option<Keychain>,
    search: Owned,
    default: Keychain,
    directory: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        // SecKeychainCreate can register a new search-list entry. This explicit
        // opt-in test belongs in a disposable CI/OS user, never a personal login.
        assert_eq!(
            std::env::var("ROCKET_MACOS_DISPOSABLE_KEYCHAIN_TEST").as_deref(),
            Ok("1")
        );
        let default = Keychain::default().expect("disposable test user needs a default Keychain");
        let mut search = ptr::null();
        check(unsafe { SecKeychainCopySearchList(&mut search) }).unwrap();
        let search = Owned::new(search).unwrap();
        let directory = std::env::temp_dir().join(format!(
            "rocket-keychain-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&directory).unwrap();
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&directory, fs::Permissions::from_mode(0o700)).unwrap();
        let mut fixture = Self {
            keychain: None,
            search,
            default,
            directory,
        };
        let path = CString::new(
            fixture
                .directory
                .join("fixture.keychain")
                .as_os_str()
                .as_bytes(),
        )
        .unwrap();
        let password = b"disposable Keychain fixture";
        let mut keychain = ptr::null();
        check(unsafe {
            SecKeychainCreate(
                path.as_ptr(),
                password.len() as u32,
                password.as_ptr().cast(),
                0,
                ptr::null(),
                &mut keychain,
            )
        })
        .unwrap();
        fixture.keychain = Some(Keychain(Owned::new(keychain).unwrap()));
        check(unsafe { SecKeychainSetSearchList(fixture.search.0) }).unwrap();
        fixture.unchanged();
        fixture
    }
    fn unchanged(&self) {
        let now = Keychain::default().unwrap();
        assert_ne!(
            unsafe { CFEqual(now.0.0, self.default.0.0) },
            0,
            "default Keychain unchanged"
        );
        let mut search = ptr::null();
        check(unsafe { SecKeychainCopySearchList(&mut search) }).unwrap();
        let search = Owned::new(search).unwrap();
        assert_ne!(
            unsafe { CFEqual(search.0, self.search.0) },
            0,
            "search list unchanged"
        );
    }
    fn cleanup(&mut self) -> Result<()> {
        if let Some(keychain) = self.keychain.as_ref() {
            check(unsafe { SecKeychainDelete(keychain.0.0) })?;
            self.keychain = None;
        }
        check(unsafe { SecKeychainSetSearchList(self.search.0) })?;
        Ok(())
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        if let Err(error) = self.cleanup() {
            eprintln!("disposable Keychain cleanup failed: {error}");
        }
        // Only remove an empty owned directory. Unexpected leftovers are evidence.
        let _ = fs::remove_dir(&self.directory);
    }
}

#[test]
#[ignore = "only in an explicitly opted-in disposable macOS CI/OS user"]
fn disposable_keychain_roundtrip_duplicate_lock_and_preservation() {
    let guard = NoUi::new().unwrap();
    let previous = guard.previous;
    let mut fixture = Fixture::new();
    let store = fixture.keychain.as_ref().unwrap();
    assert!(store.lookup().unwrap().is_none());
    let blob = macos_policy::protect(store, b"disposable credential", true).unwrap();
    assert_ne!(
        blob,
        macos_policy::protect(store, b"disposable credential", true).unwrap(),
        "fresh OS nonces"
    );
    assert_eq!(
        macos_policy::protect(store, &blob, false).unwrap(),
        b"disposable credential"
    );
    let original = store.lookup().unwrap().unwrap();
    store.add(&Key([81; 32])).unwrap();
    assert_eq!(
        store.lookup().unwrap().unwrap().0,
        original.0,
        "duplicate add never replaces"
    );
    let mut corrupt = blob.clone();
    *corrupt.last_mut().unwrap() ^= 1;
    assert!(macos_policy::protect(store, &corrupt, false).is_err());
    check(unsafe { SecKeychainLock(store.0.0) }).unwrap();
    let error = macos_policy::protect(store, &blob, false)
        .unwrap_err()
        .to_string();
    assert!(error.contains("macos_keychain_denied"), "{error}");
    let password = b"disposable Keychain fixture";
    check(unsafe {
        SecKeychainUnlock(
            store.0.0,
            password.len() as u32,
            password.as_ptr().cast(),
            1,
        )
    })
    .unwrap();
    assert_eq!(
        macos_policy::protect(store, &blob, false).unwrap(),
        b"disposable credential"
    );
    assert_eq!(store.lookup().unwrap().unwrap().0, original.0);
    fixture.unchanged();
    fixture.cleanup().unwrap();
    fixture.unchanged();
    guard.finish(Ok(())).unwrap();
    let mut now = 0;
    check(unsafe { SecKeychainGetUserInteractionAllowed(&mut now) }).unwrap();
    assert_eq!(now, previous, "process UI policy restored");
}

#[test]
fn invalid_envelope_is_refused_before_default_keychain_access() {
    assert!(
        protect(b"not a credential envelope", false)
            .unwrap_err()
            .to_string()
            .contains("macos_credential_data")
    );
}

#[test]
fn helper_trust_refuses_writable_files_links_and_acl_grants() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let directory = std::env::temp_dir().join(format!(
        "rocket-keychain-helper-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir(&directory).unwrap();
    fs::set_permissions(&directory, fs::Permissions::from_mode(0o700)).unwrap();
    let directory = fs::canonicalize(directory).unwrap();
    let file = directory.join("fixture-helper");
    fs::write(&file, b"only permission validation; not executed").unwrap();
    fs::set_permissions(&file, fs::Permissions::from_mode(0o700)).unwrap();
    assert!(trusted_executable(&file).is_ok());
    fs::set_permissions(&file, fs::Permissions::from_mode(0o720)).unwrap();
    assert!(trusted_executable(&file).is_err());
    fs::set_permissions(&file, fs::Permissions::from_mode(0o700)).unwrap();
    let link = directory.join("link");
    symlink(&file, &link).unwrap();
    assert!(trusted_executable(&link).is_err());
    fs::remove_file(link).unwrap();
    assert!(
        std::process::Command::new("/bin/chmod")
            .args(["+a", "everyone allow read"])
            .arg(&file)
            .status()
            .unwrap()
            .success()
    );
    assert!(trusted_executable(&file).is_err());
    fs::remove_file(file).unwrap();
    fs::remove_dir(directory).unwrap();
}
