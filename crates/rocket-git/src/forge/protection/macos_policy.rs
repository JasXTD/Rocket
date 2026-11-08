//! Key creation policy and envelope validation, independent of Security.framework.
use super::sealed::{self, NONCE, TAG};
use crate::{Error, Result};
const MAGIC: &[u8; 4] = b"RKM1";
const AAD: &[u8] = b"Rocket macOS credential v1";
pub(super) struct Key(pub [u8; 32]);
impl Drop for Key {
    fn drop(&mut self) {
        for byte in &mut self.0 {
            unsafe { std::ptr::write_volatile(byte, 0) };
        }
    }
}
pub(super) trait Store {
    fn lookup(&self) -> Result<Option<Key>>;
    /// Add only; a duplicate is success, never replace an existing key.
    fn add(&self, key: &Key) -> Result<()>;
    fn random(&self, bytes: &mut [u8]) -> Result<()>;
}
pub(super) fn refused() -> Error {
    Error::InvalidInput("macos_credential_data: Credentials are damaged, from another platform, or cannot be decrypted with Rocket's Keychain key. Existing data was preserved.".into())
}
pub(super) fn validate(bytes: &[u8], encrypt: bool) -> Result<()> {
    if encrypt {
        if bytes.is_empty() || bytes.len() > super::LIMIT {
            return Err(refused());
        }
    } else if !bytes.starts_with(MAGIC)
        || bytes.len() <= 4 + NONCE + TAG
        || bytes.len() > super::LIMIT + 4 + NONCE + TAG
    {
        return Err(refused());
    }
    Ok(())
}
pub(super) fn protect(store: &impl Store, bytes: &[u8], encrypt: bool) -> Result<Vec<u8>> {
    validate(bytes, encrypt)?;
    let key = match store.lookup()? {
        Some(key) => key,
        None if encrypt => {
            let mut candidate = Key([0; 32]);
            store.random(&mut candidate.0)?;
            store.add(&candidate)?;
            // Another process may have won the add. Always use the stored key.
            store.lookup()?.ok_or_else(missing)?
        }
        None => return Err(missing()),
    };
    if encrypt {
        let mut nonce = [0; NONCE];
        store.random(&mut nonce)?;
        sealed::seal(MAGIC, AAD, &key.0, nonce, bytes).ok_or_else(refused)
    } else {
        sealed::open(MAGIC, AAD, &key.0, bytes).ok_or_else(refused)
    }
}
fn missing() -> Error {
    Error::InvalidInput("macos_keychain_key_missing: Rocket's credential key is missing from the default Keychain. Restore the original Keychain or save your credentials again. Existing encrypted files were preserved.".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::{Cell, RefCell};
    #[derive(Default)]
    struct Fake {
        key: RefCell<Option<[u8; 32]>>,
        reads: Cell<u32>,
        adds: Cell<u32>,
        nonce: Cell<u8>,
        race: bool,
        denied: bool,
        random_fails: bool,
    }
    impl Store for Fake {
        fn lookup(&self) -> Result<Option<Key>> {
            self.reads.set(self.reads.get() + 1);
            if self.denied {
                return Err(Error::InvalidInput("macos_keychain_denied".into()));
            }
            Ok((*self.key.borrow()).map(Key))
        }
        fn add(&self, key: &Key) -> Result<()> {
            self.adds.set(self.adds.get() + 1);
            self.key
                .borrow_mut()
                .get_or_insert(if self.race { [73; 32] } else { key.0 });
            Ok(())
        }
        fn random(&self, bytes: &mut [u8]) -> Result<()> {
            if self.random_fails {
                return Err(Error::InvalidInput("macos_random_failed".into()));
            }
            self.nonce.set(self.nonce.get() + 1);
            bytes.fill(self.nonce.get());
            Ok(())
        }
    }
    #[test]
    fn key_is_added_once_read_back_and_never_replaced_even_after_a_race() {
        for race in [false, true] {
            let store = Fake {
                race,
                ..Default::default()
            };
            let a = protect(&store, b"disposable fixture", true).unwrap();
            let b = protect(&store, b"disposable fixture", true).unwrap();
            assert_ne!(a, b);
            assert_eq!(store.adds.get(), 1);
            assert_eq!(protect(&store, &a, false).unwrap(), b"disposable fixture");
            assert_eq!(protect(&store, &b, false).unwrap(), b"disposable fixture");
            assert!(!a.windows(18).any(|w| w == b"disposable fixture"));
        }
    }
    #[test]
    fn bad_data_and_bounds_fail_before_any_keychain_access() {
        let store = Fake::default();
        for raw in [vec![], vec![0; super::super::LIMIT + 1]] {
            assert!(protect(&store, &raw, true).is_err());
        }
        for raw in [
            vec![],
            b"RKS1invalid".to_vec(),
            vec![0; super::super::LIMIT + 45],
            b"RKM1".to_vec(),
        ] {
            assert!(protect(&store, &raw, false).is_err());
        }
        assert_eq!(store.reads.get(), 0);
        assert_eq!(store.adds.get(), 0);
    }
    #[test]
    fn missing_locked_and_random_failure_never_replace_or_create_a_decryption_key() {
        let a = protect(&Fake::default(), b"disposable", true).unwrap();
        let missing = Fake::default();
        assert!(protect(&missing, &a, false).is_err());
        assert_eq!(missing.adds.get(), 0);
        let denied = Fake {
            denied: true,
            ..Default::default()
        };
        assert!(protect(&denied, b"disposable", true).is_err());
        assert_eq!(denied.adds.get(), 0);
        let broken = Fake {
            random_fails: true,
            ..Default::default()
        };
        assert!(protect(&broken, b"disposable", true).is_err());
        assert_eq!(broken.adds.get(), 0);
    }
    #[test]
    fn tampering_wrong_key_truncation_and_linux_envelopes_are_refused() {
        let store = Fake::default();
        let blob = protect(&store, b"disposable", true).unwrap();
        for i in 0..blob.len() {
            let mut bad = blob.clone();
            bad[i] ^= 1;
            assert!(protect(&store, &bad, false).is_err());
        }
        assert!(protect(&store, &blob[..blob.len() - 1], false).is_err());
        let wrong = Fake {
            key: RefCell::new(Some([8; 32])),
            ..Default::default()
        };
        assert!(protect(&wrong, &blob, false).is_err());
        let linux = sealed::seal(
            b"RKS1",
            b"Rocket protected credential v1",
            &[1; 32],
            [2; NONCE],
            b"disposable",
        )
        .unwrap();
        assert!(protect(&store, &linux, false).is_err());
        let mut renamed = linux;
        renamed[..4].copy_from_slice(MAGIC);
        assert!(protect(&store, &renamed, false).is_err());
    }
}
