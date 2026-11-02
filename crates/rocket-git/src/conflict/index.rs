//! Specialized resolutions publish a prepared index while owning Git's live
//! index lock. The original index and stages remain in the recovery directory.
use super::*;
use crate::local_files;
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::Path,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};
static NEXT: AtomicU64 = AtomicU64::new(0);
struct Lock {
    path: PathBuf,
    file: Option<File>,
    owned: bool,
}
impl Drop for Lock {
    fn drop(&mut self) {
        drop(self.file.take());
        if self.owned {
            let _ = fs::remove_file(&self.path);
        }
    }
}
fn read(path: &Path) -> Result<Vec<u8>> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.custom_flags(0x00200000);
    }
    let f = options
        .open(path)
        .map_err(|e| Error::io("read conflict index", e))?;
    let meta = f
        .metadata()
        .map_err(|e| Error::io("inspect conflict index", e))?;
    if !meta.is_file() || local_files::redirected(&meta) {
        return Err(Error::Conflict(ConflictFailure::UnsupportedVersion));
    }
    let mut bytes = Vec::new();
    f.take(64 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| Error::io("read conflict index", e))?;
    if bytes.len() > 64 * 1024 * 1024 {
        return Err(Error::Unsupported(
            "The index exceeds the 64 MiB conflict recovery limit.".into(),
        ));
    }
    Ok(bytes)
}
fn write_new(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut f = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(path)
        .map_err(|e| Error::io("create conflict index recovery", e))?;
    f.write_all(bytes)
        .and_then(|()| f.sync_all())
        .map_err(|e| Error::io("flush conflict index recovery", e))
}
pub(super) fn check_layout(repo: &Repository, cancel: &CancellationToken) -> Result<()> {
    let split = repo.read_diff(
        "inspect conflict index layout",
        &args(&["rev-parse", "--shared-index-path"]),
        cancel,
    )?;
    if !crate::parse::line(&split).is_empty() {
        return Err(Error::Unsupported("Resolve this split index with Git, or explicitly disable split-index and review again. Nothing was written.".into()));
    }
    read(&repo.info().git_dir.join("index"))?;
    cancel.check()
}
pub(super) fn stage(
    repo: &Repository,
    stages: &[u8],
    input: Vec<u8>,
    recheck: impl Fn() -> Result<()>,
    cancel: &CancellationToken,
    staged: &mut bool,
    recovery: &mut Option<PathBuf>,
) -> Result<()> {
    cancel.check()?;
    let admin = &repo.info().git_dir;
    let _admin = local_files::pin_directories(admin, admin)?;
    let path = admin.join("index.lock");
    let mut options = OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.share_mode(1);
    }
    let file = options.open(&path).map_err(|e| {
        Error::io(
            "acquire conflict index lock (inspect existing locks; Rocket never removes them)",
            e,
        )
    })?;
    let mut lock = Lock {
        path,
        file: Some(file),
        owned: true,
    };
    recheck()?;
    let split = repo.read_diff(
        "inspect conflict index layout",
        &args(&["rev-parse", "--shared-index-path"]),
        cancel,
    )?;
    if !crate::parse::line(&split).is_empty() {
        return Err(Error::Unsupported("Resolve this split index with Git, or explicitly disable split-index and review again. The index was not changed.".into()));
    }
    let original = read(&admin.join("index"))?;
    if recovery.is_none() {
        let parent = admin.join("rocket-conflict-recovery");
        match fs::create_dir(&parent) {
            Ok(()) => (),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => (),
            Err(e) => return Err(Error::io("create conflict recovery", e)),
        }
        let _parent = local_files::pin_directories(admin, &parent)?;
        let id = format!(
            "index-{}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_err(|_| Error::InvalidInput("System clock is before Unix epoch".into()))?
                .as_nanos(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        );
        let directory = parent.join(id);
        fs::create_dir(&directory).map_err(|e| Error::io("reserve conflict recovery", e))?;
        *recovery = Some(directory);
    }
    let directory = recovery.as_ref().unwrap();
    let _recovery = local_files::pin_directories(admin, directory)?;
    write_new(&directory.join("index-before"), &original)?;
    write_new(&directory.join("conflict-stages"), stages)?;
    write_new(&directory.join("INDEX-RECOVERY.md"), b"Original index and unmerged stages before this resolution. Compare with current Git status before restoring; never overwrite newer staged work. A submodule resolution changes only the parent gitlink, never its checkout or nested files.\n")?;
    let prepared = directory.join("index-prepared");
    write_new(&prepared, &original)?;
    repo.runner
        .private_index(
            repo.cwd(),
            &prepared,
            &args(&["update-index", "-z", "--index-info"]),
            input,
            cancel,
        )?
        .success("prepare conflict index")?;
    let bytes = read(&prepared)?;
    recheck()?;
    if read(&admin.join("index"))? != original {
        return Err(Error::Conflict(ConflictFailure::ResolutionUnverified));
    }
    let file = lock.file.as_mut().unwrap();
    file.write_all(&bytes)
        .and_then(|()| file.sync_all())
        .map_err(|e| Error::io("flush conflict index", e))?;
    cancel.check()?;
    drop(lock.file.take());
    local_files::move_path(&lock.path, &admin.join("index"), true)?;
    lock.owned = false;
    *staged = true;
    Ok(())
}
