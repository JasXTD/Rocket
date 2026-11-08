//! Optional account persistence, independent of Git/GCM and SSH. One encrypted
//! file in Rocket's already locked profile. No plaintext credential fallback.
use rocket_git::{Error, ForgeConfig, ForgeProvider, Result};
use serde_json::{Value, json};
use std::{
    fs::{self, OpenOptions},
    io::{Read, Write},
    path::Path,
};
const LIMIT: usize = 512 * 1024;
const FILE: &str = "hosting-accounts.dpapi";
#[derive(Clone)]
pub struct Account {
    pub label: String,
    pub provider: ForgeProvider,
    pub server: String,
    pub username: String,
    pub token: String,
    pub azure_api_version: Option<String>,
}
pub struct Loaded {
    pub accounts: Vec<Account>,
    pub snapshot: Option<Vec<u8>>,
}
fn refused() -> Error {
    Error::InvalidInput(if cfg!(windows) {
        "Saved hosting accounts are invalid, changed externally, or cannot be decrypted for this Windows user. The existing file was preserved."
    } else if cfg!(target_os = "macos") {
        "Saved hosting accounts changed externally, are invalid, or could not be decrypted. Unlock the default macOS Keychain and review Rocket's access, then retry. The existing file was preserved."
    } else {
        "Saved hosting accounts are invalid, changed externally, or cannot be decrypted with the key in your desktop keyring. The existing file was preserved."
    }.into())
}
fn read(directory: &Path) -> Result<Option<Vec<u8>>> {
    let path = directory.join(FILE);
    let meta = match fs::symlink_metadata(&path) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => {
            return Err(Error::Io {
                action: "read hosting accounts",
                source: e,
            });
        }
    };
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if meta.file_attributes() & 0x400 != 0 {
            return Err(refused());
        }
    }
    if !meta.is_file() || meta.len() > LIMIT as u64 {
        return Err(refused());
    }
    let mut bytes = Vec::new();
    fs::File::open(path)
        .and_then(|f| f.take(LIMIT as u64 + 1).read_to_end(&mut bytes))
        .map_err(|source| Error::Io {
            action: "read hosting accounts",
            source,
        })?;
    if bytes.len() > LIMIT {
        return Err(refused());
    }
    Ok(Some(bytes))
}
pub fn load(directory: &Path) -> Result<Loaded> {
    let snapshot = read(directory)?;
    let Some(bytes) = &snapshot else {
        return Ok(Loaded {
            accounts: Vec::new(),
            snapshot,
        });
    };
    let mut plain = protect(bytes, false)?;
    let parsed = serde_json::from_slice::<Value>(&plain);
    plain.fill(0);
    let v = parsed.map_err(|_| refused())?;
    if v["version"] != 1 {
        return Err(refused());
    }
    let list = v["accounts"]
        .as_array()
        .filter(|a| a.len() <= 32)
        .ok_or_else(refused)?;
    let mut accounts = Vec::new();
    for row in list {
        let s = |k: &str, max: usize| {
            row[k]
                .as_str()
                .filter(|v| !v.is_empty() && v.len() <= max && !v.contains('\0'))
                .map(str::to_owned)
                .ok_or_else(refused)
        };
        let provider = ForgeProvider::from_key(&s("provider", 20)?).ok_or_else(refused)?;
        let server = s("server", 512)?;
        let config = ForgeConfig::new(provider, &server).map_err(|_| refused())?;
        let azure_api_version = if provider == ForgeProvider::AzureServer {
            let version = s("azure_api_version", 8)?;
            config.with_azure_api_version(&version).map_err(|_| refused())?;
            Some(version)
        } else { None };
        let username = row["username"]
            .as_str()
            .filter(|v| v.len() <= 320 && !v.chars().any(char::is_control))
            .ok_or_else(refused)?
            .to_owned();
        accounts.push(Account {
            label: s("label", 128)?,
            provider,
            server,
            username,
            token: s("token", 4096)?,
            azure_api_version,
        });
    }
    Ok(Loaded { accounts, snapshot })
}
pub fn save(directory: &Path, accounts: Vec<Account>, expected: Option<Vec<u8>>) -> Result<Loaded> {
    if accounts.len() > 32 {
        return Err(Error::InvalidInput(
            "Up to 32 hosting accounts can be remembered.".into(),
        ));
    }
    if read(directory)? != expected {
        return Err(refused());
    }
    for account in &accounts {
        let config = ForgeConfig::new(account.provider, &account.server).map_err(|_| refused())?;
        if account.provider == ForgeProvider::AzureServer {
            config.with_azure_api_version(account.azure_api_version.as_deref().ok_or_else(refused)?).map_err(|_| refused())?;
        }
    }
    let value = json!({"version":1,"accounts":accounts.iter().map(|a|json!({"label":a.label,"provider":a.provider.key(),"server":a.server,"username":a.username,"token":a.token,"azure_api_version":a.azure_api_version})).collect::<Vec<_>>()});
    let mut plain = serde_json::to_vec(&value).map_err(|_| refused())?;
    if plain.len() > LIMIT {
        plain.fill(0);
        return Err(refused());
    }
    let encrypted = protect(&plain, true);
    plain.fill(0);
    let encrypted = encrypted?;
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| refused())?
        .as_nanos();
    let temporary = directory.join(format!("hosting-{}-{stamp}.tmp", std::process::id()));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .map_err(|source| Error::Io {
            action: "prepare protected hosting accounts",
            source,
        })?;
    let result = (|| {
        file.write_all(&encrypted)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temporary, directory.join(FILE))
    })();
    if let Err(source) = result {
        let _ = fs::remove_file(&temporary);
        return Err(Error::Io {
            action: "save protected hosting accounts",
            source,
        });
    }
    Ok(Loaded {
        accounts,
        snapshot: Some(encrypted),
    })
}
fn protect(bytes: &[u8], encrypt: bool) -> Result<Vec<u8>> {
    rocket_git::protect_hosting_secret(bytes, encrypt)
}
#[cfg(all(test, any(windows, target_os = "linux")))]
mod tests {
    use super::*;
    #[test]
    #[cfg_attr(target_os = "linux", ignore = "needs an unlocked desktop keyring")]
    fn azure_server_version_roundtrip_and_invalid_profile_preserved() {
        let dir = std::env::temp_dir().join(format!("rocket-azure-vault-{}-{}", std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        fs::create_dir(&dir).unwrap();
        let a = Account { label: "Server".into(), provider: ForgeProvider::AzureServer,
            server: ForgeProvider::AzureServer.default_server().into(), username: String::new(),
            token: "disposable-azure-fixture".into(), azure_api_version: Some("6.0".into()) };
        let first = save(&dir, vec![a], None).unwrap();
        let loaded = load(&dir).unwrap();
        assert_eq!(loaded.accounts[0].provider, ForgeProvider::AzureServer);
        assert_eq!(loaded.accounts[0].azure_api_version.as_deref(), Some("6.0"));
        let token = b"disposable-azure-fixture";
        assert!(!first.snapshot.as_ref().unwrap().windows(token.len()).any(|w| w == token));
        let plain = protect(first.snapshot.as_ref().unwrap(), false).unwrap();
        let mut value: Value = serde_json::from_slice(&plain).unwrap();
        for version in [json!("5.0"), Value::Null] {
            value["accounts"][0]["azure_api_version"] = version;
            let bytes = protect(&serde_json::to_vec(&value).unwrap(), true).unwrap();
            fs::write(dir.join(FILE), &bytes).unwrap();
            assert!(load(&dir).is_err());
            assert_eq!(fs::read(dir.join(FILE)).unwrap(), bytes);
        }
        // Older public-host profiles without the optional field still load.
        value["accounts"][0]["provider"] = json!("azure");
        value["accounts"][0]["server"] = json!("https://dev.azure.com/fixture");
        value["accounts"][0].as_object_mut().unwrap().remove("azure_api_version");
        fs::write(dir.join(FILE), protect(&serde_json::to_vec(&value).unwrap(), true).unwrap()).unwrap();
        assert!(load(&dir).unwrap().accounts[0].azure_api_version.is_none());
        fs::remove_file(dir.join(FILE)).unwrap(); fs::remove_dir(&dir).unwrap();
    }
    #[test]
    #[cfg_attr(target_os = "linux", ignore = "needs an unlocked desktop keyring")]
    fn user_protection_roundtrip_and_tampering() {
        let raw = b"disposable-hosting-token-ONLY";
        let enc = protect(raw, true).unwrap();
        assert!(!enc.windows(raw.len()).any(|w| w == raw));
        assert_eq!(protect(&enc, false).unwrap(), raw);
        let mut bad = enc;
        let i = bad.len() / 2;
        bad[i] ^= 42;
        assert!(protect(&bad, false).is_err());
    }
    #[test]
    #[cfg_attr(target_os = "linux", ignore = "needs an unlocked desktop keyring")]
    fn external_edit_is_preserved() {
        let dir = std::env::temp_dir().join(format!(
            "rocket-vault-test-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&dir).unwrap();
        let a = Account {
            label: "fixture".into(),
            provider: ForgeProvider::GitHub,
            server: "https://github.com".into(),
            username: String::new(),
            token: "fixture-test-token".into(),
            azure_api_version: None,
        };
        let first = save(&dir, vec![a.clone()], None).unwrap();
        assert_eq!(load(&dir).unwrap().accounts[0].token, a.token);
        fs::write(dir.join(FILE), b"external edit").unwrap();
        assert!(save(&dir, vec![], first.snapshot).is_err());
        assert_eq!(fs::read(dir.join(FILE)).unwrap(), b"external edit");
        fs::remove_file(dir.join(FILE)).unwrap();
        fs::remove_dir(&dir).unwrap();
    }
}
