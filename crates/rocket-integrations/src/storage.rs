//! A separate, user-protected integration store. Existing Git/hosting credentials
//! are never accessed. Exact snapshots refuse external edits and damaged files.
use crate::{invalid, profiles};
use rocket_git::{Result, protect_hosting_secret};
use serde_json::Value;
use std::{
    fs,
    io::{Read, Write},
    path::Path,
};
const FILE: &str = "integration-accounts.dpapi";
const LOCK: &str = "integration-accounts.lock";
const LIMIT: usize = 8 * 1024 * 1024;

fn regular_file(meta: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if meta.file_attributes() & 0x400 != 0 {
            return false;
        }
    }
    meta.is_file() && !meta.file_type().is_symlink()
}

fn save_lock(directory: &Path) -> Result<fs::File> {
    let path = directory.join(LOCK);
    let failure = || {
        invalid("integration_store: Cannot lock protected settings. Existing files were preserved.")
    };
    match fs::symlink_metadata(&path) {
        Ok(meta) if !regular_file(&meta) => return Err(failure()),
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(failure()),
        _ => {}
    }
    let mut options = fs::OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        // Do not follow a substituted reparse point or let the stable lock file
        // be deleted/replaced while any cooperating writer has it open.
        options.share_mode(1 | 2).custom_flags(0x0020_0000);
    }
    let file = options.open(path).map_err(|_| failure())?;
    if !regular_file(&file.metadata().map_err(|_| failure())?) {
        return Err(failure());
    }
    match file.try_lock() {
        Ok(()) => Ok(file),
        Err(fs::TryLockError::WouldBlock) => Err(invalid(
            "integration_store_busy: Another Rocket window is saving extension settings. Retry after it finishes; existing files were preserved.",
        )),
        Err(fs::TryLockError::Error(_)) => Err(failure()),
    }
}

fn read(directory: &Path) -> Result<Option<Vec<u8>>> {
    let path = directory.join(FILE);
    let meta = match fs::symlink_metadata(&path) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => {
            return Err(invalid(
                "integration_store: Could not read protected extension settings.",
            ));
        }
    };
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if meta.file_attributes() & 0x400 != 0 {
            return Err(invalid(
                "integration_store: Linked settings are refused; existing files were preserved.",
            ));
        }
    }
    if !meta.is_file() || meta.file_type().is_symlink() || meta.len() > LIMIT as u64 {
        return Err(invalid(
            "integration_store: Invalid or oversized settings; existing files were preserved.",
        ));
    }
    let mut bytes = vec![];
    fs::File::open(path)
        .and_then(|f| f.take((LIMIT + 1) as u64).read_to_end(&mut bytes))
        .map_err(|_| invalid("integration_store: Could not read protected settings."))?;
    if bytes.len() > LIMIT {
        return Err(invalid("integration_store: Settings exceed the limit."));
    }
    Ok(Some(bytes))
}
pub fn load(directory: &Path) -> Result<(Value, Option<Vec<u8>>)> {
    let raw = read(directory)?;
    let Some(bytes) = &raw else {
        return Ok((profiles::empty(), None));
    };
    let mut plain = protect_hosting_secret(bytes, false).map_err(|_| {
        invalid("integration_store: Settings cannot be decrypted. Existing files were preserved.")
    })?;
    let parsed = serde_json::from_slice::<Value>(&plain);
    plain.fill(0);
    let value=parsed.map_err(|_|invalid("integration_store: Protected settings contain invalid data. Existing files were preserved."))?;
    let value = profiles::upgrade(value)?;
    Ok((value, raw))
}
pub fn save(directory: &Path, value: &Value, expected: Option<&[u8]>) -> Result<Vec<u8>> {
    profiles::validate(value)?;
    // Lock a persistent sidecar, not FILE: rename replaces the store's identity.
    // Hold ownership from the first comparison through replacement and cleanup.
    // Never unlink/truncate LOCK; closing this handle (also on process exit)
    // releases the OS lock without leaving stale ownership behind.
    let _lock = save_lock(directory)?;
    if read(directory)?.as_deref() != expected {
        return Err(invalid(
            "integration_store_changed: Settings changed outside this window. Reload before saving; existing files were preserved.",
        ));
    }
    let mut plain = serde_json::to_vec(value)
        .map_err(|_| invalid("integration_store: Could not encode settings."))?;
    if plain.len() > LIMIT - 4096 {
        plain.fill(0);
        return Err(invalid(
            "integration_store_limit: Saved extensions exceed the storage limit.",
        ));
    }
    let encrypted = protect_hosting_secret(&plain, true);
    plain.fill(0);
    let encrypted = encrypted?;
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| invalid("integration_store: Invalid system clock."))?
        .as_nanos();
    let temp = directory.join(format!("integration-{}-{stamp}.tmp", std::process::id()));
    let mut file = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temp)
        .map_err(|_| invalid("integration_store: Cannot prepare protected settings."))?;
    let result = (|| {
        file.write_all(&encrypted)
            .and_then(|()| file.sync_all())
            .map_err(|_| invalid("integration_store: Could not flush protected settings."))?;
        drop(file);
        if read(directory)?.as_deref() != expected {
            return Err(invalid(
                "integration_store_changed: Settings changed while saving. Existing files were preserved.",
            ));
        }
        fs::rename(&temp,directory.join(FILE)).map_err(|_|invalid("integration_store: Could not replace protected settings. Existing files were preserved."))?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result?;
    Ok(encrypted)
}
