use super::*;
use crate::local_files;
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};
static NEXT: AtomicU64 = AtomicU64::new(0);
fn unique() -> String {
    format!(
        "rocket-patch-{}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    )
}
pub(super) fn pin_parent(path: &Path) -> Result<Vec<File>> {
    if !path.is_absolute()
        || path.components().any(|c| {
            matches!(
                c,
                std::path::Component::ParentDir | std::path::Component::CurDir
            )
        })
    {
        return failure(PatchFailure::UnsafePath);
    }
    let parent = path
        .parent()
        .ok_or(Error::Patch(PatchFailure::UnsafePath))?;
    let root = parent
        .ancestors()
        .last()
        .ok_or(Error::Patch(PatchFailure::UnsafePath))?;
    local_files::pin_directories(root, parent)
}
pub(super) fn read(path: &Path, cancel: &CancellationToken) -> Result<Vec<u8>> {
    read_bounded(path, PATCH_LIMIT, cancel)
}
pub(super) fn read_bounded(path: &Path, limit: usize, cancel: &CancellationToken) -> Result<Vec<u8>> {
    let _guard = pin_parent(path)?;
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.share_mode(1 | 4).custom_flags(0x00200000);
    }
    let mut file = options
        .open(path)
        .map_err(|e| Error::io("open patch file", e))?;
    let meta = file
        .metadata()
        .map_err(|e| Error::io("inspect patch file", e))?;
    if !meta.is_file() || local_files::redirected(&meta) || meta.len() > limit as u64 {
        return failure(PatchFailure::Unsupported);
    }
    crate::review::verify_preview_handle(&file, path.parent().unwrap(), path)?;
    let mut bytes = Vec::new();
    let mut chunk = [0; 65536];
    loop {
        cancel.check()?;
        let n = file
            .read(&mut chunk)
            .map_err(|e| Error::io("read patch file", e))?;
        if n == 0 {
            break;
        }
        bytes.extend_from_slice(&chunk[..n]);
        if bytes.len() > limit {
            return failure(PatchFailure::Unsupported);
        }
    }
    Ok(bytes)
}
pub(super) fn write_new(path: &Path, bytes: &[u8], cancel: &CancellationToken) -> Result<()> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|e| Error::io("reserve patch file", e))?;
    for chunk in bytes.chunks(65536) {
        cancel.check()?;
        file.write_all(chunk)
            .map_err(|e| Error::io("write patch file", e))?;
    }
    file.sync_all()
        .map_err(|e| Error::io("flush patch file", e))
}

pub(crate) struct Scratch {
    pub path: PathBuf,
    guards: Vec<File>,
    base: PathBuf,
}
impl Scratch {
    pub fn new() -> Result<Self> {
        let base = std::env::temp_dir()
            .canonicalize()
            .map_err(|e| Error::io("resolve patch preview folder", e))?;
        let path = base.join(unique());
        let mut guards = pin_parent(&path)?;
        fs::create_dir(&path).map_err(|e| Error::io("reserve patch preview folder", e))?;
        guards.extend(local_files::pin_directories(&base, &path)?);
        Ok(Self { path, guards, base })
    }
    pub fn cleanup(mut self) -> Result<()> {
        self.guards.clear();
        self.remove()
    }
    fn remove(&mut self) -> Result<()> {
        if self.path.as_os_str().is_empty() {
            return Ok(());
        }
        if self.path.parent() != Some(self.base.as_path())
            || self
                .path
                .canonicalize()
                .map_err(|e| Error::io("verify patch preview cleanup", e))?
                .parent()
                != Some(self.base.as_path())
        {
            return failure(PatchFailure::UnsafePath);
        }
        fs::remove_dir_all(&self.path)
            .map_err(|e| Error::io("remove owned patch preview folder", e))?;
        self.path = PathBuf::new();
        Ok(())
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        self.guards.clear();
        let _ = self.remove();
    }
}

pub(super) fn validate_destination(repo: &Repository, path: &Path) -> Result<()> {
    let _guard = pin_parent(path)?;
    let parent = path
        .parent()
        .unwrap()
        .canonicalize()
        .map_err(|e| Error::io("resolve patch output folder", e))?;
    if parent.starts_with(&repo.info().git_dir)
        || parent.starts_with(&repo.info().common_dir)
        || path.components().any(|c| {
            c.as_os_str()
                .eq_ignore_ascii_case(std::ffi::OsStr::new(".git"))
        })
    {
        return failure(PatchFailure::UnsafePath);
    }
    if path.file_name().is_none() {
        return failure(PatchFailure::UnsafePath);
    }
    match fs::symlink_metadata(path) {
        Ok(_) => failure(PatchFailure::OutputExists),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(Error::io("inspect patch destination", e)),
    }
}
pub(super) fn export(
    repo: &Repository,
    p: &PatchExportPreview,
    cancel: &CancellationToken,
) -> Result<()> {
    let _guard = pin_parent(&p.destination)?;
    validate_destination(repo, &p.destination)?;
    let temp = p.destination.parent().unwrap().join(unique() + ".part");
    let mut created = false;
    let result = (|| {
        // Reserve first so cleanup can never remove another process's existing file.
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)
            .map_err(|e| Error::io("reserve patch export", e))?;
        created = true;
        for chunk in p.bytes.chunks(65536) {
            cancel.check()?;
            file.write_all(chunk)
                .map_err(|e| Error::io("write patch export", e))?;
        }
        file.sync_all()
            .map_err(|e| Error::io("flush patch export", e))?;
        drop(file);
        repo.recheck_patch_export(p, cancel)?;
        cancel.check()?;
        // Non-replacing publication protects a destination created after review.
        local_files::move_path(&temp, &p.destination, false)
    })();
    if created && temp.exists() {
        fs::remove_file(&temp).map_err(|e| Error::io("clean incomplete patch export", e))?;
    }
    result
}
