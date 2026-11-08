//! Separate protected source file with an exclusive writer and exact-byte recheck.
use crate::{
    invalid,
    playground::{SOURCE_LIMIT, STARTER, validate_source},
};
use rocket_git::{CancellationToken, Result, protect_hosting_secret};
use std::{
    fs,
    io::{Read, Write},
    path::Path,
};
const FILE: &str = "playground.dpapi";
fn inspect(path: &Path, dir: bool) -> Result<()> {
    let m = fs::symlink_metadata(path)
        .map_err(|_| invalid("playground_store: Cannot inspect the scratch file."))?;
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if m.file_attributes() & 0x400 != 0 {
            return Err(invalid(
                "playground_store: Linked scratch files are refused.",
            ));
        }
    }
    if m.file_type().is_symlink()
        || (dir && !m.is_dir())
        || (!dir && (!m.is_file() || m.len() > (SOURCE_LIMIT + 4096) as u64))
    {
        return Err(invalid(
            "playground_store: Invalid or oversized scratch storage. Existing files were preserved.",
        ));
    }
    Ok(())
}
fn read(directory: &Path) -> Result<Option<Vec<u8>>> {
    for p in directory.ancestors() {
        inspect(p, true)?;
    }
    let path = directory.join(FILE);
    if matches!(fs::symlink_metadata(&path),Err(e) if e.kind()==std::io::ErrorKind::NotFound) {
        return Ok(None);
    }
    inspect(&path, false)?;
    let mut raw = vec![];
    fs::File::open(&path)
        .and_then(|f| f.take((SOURCE_LIMIT + 4097) as u64).read_to_end(&mut raw))
        .map_err(|_| invalid("playground_store: Cannot read scratch source."))?;
    if raw.len() > SOURCE_LIMIT + 4096 {
        return Err(invalid("playground_store: Oversized scratch file."));
    }
    Ok(Some(raw))
}
pub fn load(directory: &Path) -> Result<(String, Option<Vec<u8>>)> {
    let raw = read(directory)?;
    let source = if let Some(raw) = &raw {
        let plain = protect_hosting_secret(raw, false).map_err(|_| {
            invalid(
                "playground_store: Cannot decrypt scratch source. Existing files were preserved.",
            )
        })?;
        String::from_utf8(plain)
            .map_err(|_| invalid("playground_store: Invalid scratch source."))?
    } else {
        STARTER.to_owned()
    };
    validate_source(&source)?;
    Ok((source, raw))
}
pub fn save(
    directory: &Path,
    source: &str,
    expected: Option<&[u8]>,
    cancel: &CancellationToken,
) -> Result<Vec<u8>> {
    validate_source(source)?;
    cancel.check()?;
    for p in directory.ancestors() {
        inspect(p, true)?;
    }
    let lock = directory.join("playground.lock");
    if lock.exists() {
        inspect(&lock, false)?;
    }
    let mut options = fs::OpenOptions::new();
    options.write(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.create(true).truncate(false).share_mode(0);
    }
    #[cfg(not(windows))]
    {
        options.create_new(true);
    }
    let handle = options.open(&lock).map_err(|_| {
        invalid(
            "playground_busy: Another window is saving scratch source. Retry after it finishes.",
        )
    })?;
    let result = (|| {
        if read(directory)?.as_deref() != expected {
            return Err(invalid(
                "playground_changed: Scratch source changed outside this window. Your code is retained; reload before saving.",
            ));
        }
        let raw = protect_hosting_secret(source.as_bytes(), true).map_err(|_|invalid(if cfg!(windows) {"playground_store: Cannot protect scratch source for this Windows user. Your code is retained."} else if cfg!(target_os="macos") {"playground_store: Cannot protect scratch source. Unlock the default macOS Keychain and review Rocket's access, then retry. Your code is retained."} else {"playground_store: Cannot protect scratch source with the key in your desktop keyring. Your code is retained."}))?;
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|_| invalid("playground_store: Invalid clock."))?
            .as_nanos();
        let temp = directory.join(format!("playground-{}-{stamp}.tmp", std::process::id()));
        let result = (|| {
            let mut f = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temp)
                .map_err(|_| invalid("playground_store: Cannot prepare saved code."))?;
            f.write_all(&raw)
                .and_then(|()| f.sync_all())
                .map_err(|_| invalid("playground_store: Cannot flush saved code."))?;
            drop(f);
            cancel.check()?;
            if read(directory)?.as_deref() != expected {
                return Err(invalid(
                    "playground_changed: Scratch source changed while saving. Your code is retained.",
                ));
            }
            fs::rename(&temp, directory.join(FILE)).map_err(|_| {
                invalid("playground_store: Cannot publish saved code. Your code is retained.")
            })?;
            Ok(raw)
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temp);
        }
        result
    })();
    drop(handle);
    #[cfg(not(windows))]
    {
        let _ = fs::remove_file(lock);
    }
    result
}
#[cfg(all(test, windows))]
mod tests {
    use super::*;
    #[test]
    fn protected_round_trip_external_change_and_cancel() {
        let p = std::env::temp_dir().join(format!(
            "rocket-playground-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&p).unwrap();
        let token = CancellationToken::new();
        let source = "-- 私のコード 🚀\nreturn function(ctx) return ui.text('hello') end";
        assert_eq!(load(&p).unwrap().0, STARTER);
        let first = save(&p, source, None, &token).unwrap();
        assert_eq!(load(&p).unwrap().0, source);
        assert!(!String::from_utf8_lossy(&first).contains("return function"));
        let second = save(&p, STARTER, Some(&first), &token).unwrap();
        assert!(save(&p, "external overwrite", Some(&first), &token).is_err());
        token.cancel();
        assert!(matches!(
            save(&p, source, Some(&second), &token),
            Err(rocket_git::Error::Cancelled)
        ));
        assert_eq!(load(&p).unwrap().0, STARTER);
        fs::remove_file(p.join(FILE)).unwrap();
        fs::remove_file(p.join("playground.lock")).unwrap();
        fs::remove_dir(p).unwrap();
    }
}
