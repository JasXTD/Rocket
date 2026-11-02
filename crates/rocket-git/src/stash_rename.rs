//! Rename one file-backed stash reflog entry, without replacing its commit,
//! changing its position, or touching working files. Reftable is refused.
use crate::{runner::args, *};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};
#[derive(Clone, Debug)]
pub struct StashRenamePreview {
    entry: StashEntry,
    bytes: Vec<u8>,
    name: String,
}
impl StashRenamePreview {
    pub fn entry(&self) -> &StashEntry {
        &self.entry
    }
    pub fn name(&self) -> &str {
        &self.name
    }
}
fn invalid(s: &str) -> Error {
    Error::InvalidInput(s.into())
}
fn read(path: &Path) -> Result<Vec<u8>> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.custom_flags(0x00200000);
    }
    let file = options
        .open(path)
        .map_err(|e| Error::io("read stash reflog", e))?;
    let metadata = file
        .metadata()
        .map_err(|e| Error::io("inspect stash reflog", e))?;
    if !metadata.is_file() || crate::local_files::redirected(&metadata) {
        return Err(invalid(
            "stash_rename_layout: the stash reflog must be a regular file.",
        ));
    }
    let mut bytes = Vec::new();
    file.take(2 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| Error::io("read stash reflog", e))?;
    if bytes.len() > 2 * 1024 * 1024 {
        return Err(invalid("stash_rename_limit: stash reflog exceeds 2 MiB."));
    }
    Ok(bytes)
}
struct Lock {
    path: PathBuf,
    file: Option<File>,
    owned: bool,
}
impl Lock {
    fn new(path: PathBuf) -> Result<Self> {
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(|e| Error::io("acquire stash metadata lock", e))?;
        Ok(Self {
            path,
            file: Some(file),
            owned: true,
        })
    }
}
impl Drop for Lock {
    fn drop(&mut self) {
        self.file.take();
        if self.owned {
            let _ = fs::remove_file(&self.path);
        }
    }
}
impl Repository {
    fn stash_log_path(&self, c: &CancellationToken) -> Result<PathBuf> {
        let output = self
            .runner
            .run(
                self.cwd(),
                &args(&["rev-parse", "--show-ref-format"]),
                vec![],
                c,
            )?
            .success("stash-rename")?;
        if parse::line(&output) != b"files" {
            return Err(invalid(
                "stash_rename_format: renaming currently requires file-backed refs; reftable stashes are left unchanged.",
            ));
        }
        Ok(self.info().common_dir.join("logs/refs/stash"))
    }
    pub fn preview_stash_rename(
        &self,
        entry: &StashEntry,
        name: &str,
        c: &CancellationToken,
    ) -> Result<StashRenamePreview> {
        self.runner
            .step("stash-rename-preview", "review_stash_rename", || {
                if name.trim().is_empty() || name.len() > 4096 || name.chars().any(char::is_control)
                {
                    return Err(invalid(
                        "Choose a nonblank stash name up to 4096 bytes without control characters.",
                    ));
                }
                if entry.repository != self.info().common_dir
                    || self.stashes(c)?.get(entry.index) != Some(entry)
                {
                    return Err(invalid(
                        "stash_rename_stale: the stash list changed; select it again.",
                    ));
                }
                let path = self.stash_log_path(c)?;
                let _guards = crate::local_files::pin_directories(
                    &self.info().common_dir,
                    path.parent().unwrap(),
                )?;
                let bytes = read(&path)?;
                renamed(&bytes, entry, name)?;
                Ok(StashRenamePreview {
                    entry: entry.clone(),
                    bytes,
                    name: name.into(),
                })
            })
    }
    pub fn rename_stash(&self, p: &StashRenamePreview, c: &CancellationToken) -> Result<PathBuf> {
        let _permit = self.acquire_write(c)?;
        self.runner.step("stash-rename", "rename_saved_stash", || {
            if p.entry.repository != self.info().common_dir {
                return Err(invalid("stash_rename_stale: wrong repository."));
            }
            let path = self.stash_log_path(c)?;
            let root = &self.info().common_dir;
            let _guards = crate::local_files::pin_directories(root, path.parent().unwrap())?;
            let _refdirs = crate::local_files::pin_directories(root, &root.join("refs"))?;
            let _ref_lock = Lock::new(root.join("refs/stash.lock"))?;
            let mut log_lock = Lock::new(path.with_extension("lock"))?;
            if self.stashes(c)?.get(p.entry.index) != Some(&p.entry) || read(&path)? != p.bytes {
                return Err(invalid(
                    "stash_rename_stale: the stash reflog changed; review again.",
                ));
            }
            let bytes = renamed(&p.bytes, &p.entry, &p.name)?;
            let stamp = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_err(|_| invalid("Invalid system clock"))?
                .as_nanos();
            let backup = root.join(format!("rocket-stash-reflog-{stamp}.backup"));
            let mut backup_file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&backup)
                .map_err(|e| Error::io("create stash reflog recovery copy", e))?;
            backup_file
                .write_all(&p.bytes)
                .and_then(|_| backup_file.sync_all())
                .map_err(|e| Error::io("save stash reflog recovery copy", e))?;
            let file = log_lock.file.as_mut().unwrap();
            file.write_all(&bytes)
                .and_then(|_| file.sync_all())
                .map_err(|e| Error::io("prepare stash rename", e))?;
            c.check()?;
            if read(&path)? != p.bytes {
                return Err(invalid(
                    "stash_rename_stale: external edit detected; recovery copy retained.",
                ));
            }
            log_lock.file.take();
            crate::local_files::move_path(&log_lock.path, &path, true)?;
            // Publishing consumed this lock path. A new Git process may now own it.
            log_lock.owned = false;
            Ok(backup)
        })
    }
}
fn renamed(bytes: &[u8], entry: &StashEntry, name: &str) -> Result<Vec<u8>> {
    let mut rows = bytes
        .split_inclusive(|b| *b == b'\n')
        .map(<[u8]>::to_vec)
        .collect::<Vec<_>>();
    let index = rows
        .len()
        .checked_sub(entry.index + 1)
        .ok_or_else(|| invalid("stash_rename_stale: missing stash reflog entry."))?;
    let row = &rows[index];
    let tab = row
        .iter()
        .position(|b| *b == b'\t')
        .ok_or_else(|| invalid("stash_rename_format: malformed reflog."))?;
    let id = row[..tab]
        .split(|b| *b == b' ')
        .nth(1)
        .ok_or_else(|| invalid("stash_rename_format: malformed reflog object."))?;
    if id != entry.id.as_str().as_bytes() {
        return Err(invalid("stash_rename_stale: the selected stash moved."));
    }
    rows[index] = [&row[..=tab], name.as_bytes(), b"\n"].concat();
    Ok(rows.concat())
}
