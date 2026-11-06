use super::*;
use crate::local_files;
use std::{
    fs::{File, OpenOptions},
    io::{Read, Seek, SeekFrom},
};
// Used by the Windows publication; elsewhere publish_change reports Unsupported.
#[cfg(windows)]
use std::{
    fs,
    io::Write,
    path::Path,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};
#[cfg(windows)]
static NEXT: AtomicU64 = AtomicU64::new(0);
fn io(action: &'static str, e: std::io::Error) -> Error {
    Error::io(action, e)
}

/// The largest file a read accepts, and the failure reported for an oversized
/// or special file. Text editing and whole-version resolution differ in both.
#[derive(Clone, Copy)]
pub(super) struct Bounds {
    pub limit: usize,
    pub unsupported: ConflictFailure,
}
pub(super) const TEXT: Bounds = Bounds {
    limit: CONFLICT_TEXT_LIMIT,
    unsupported: ConflictFailure::UnsupportedFile,
};

pub(super) struct Opened {
    pub file: File,
    _parents: Vec<File>,
}
// Pin every directory beneath the canonical root against rename on Windows.
// The final file allows replacement, but denies in-place writers while read.
pub(super) fn open(repo: &Repository, path: &RepoPath) -> Result<Opened> {
    open_mode(repo, path, true, TEXT)
}
pub(super) fn for_staging(repo: &Repository, path: &RepoPath) -> Result<Opened> {
    open_mode(repo, path, false, TEXT)
}
fn open_mode(
    repo: &Repository,
    path: &RepoPath,
    allow_replace: bool,
    bounds: Bounds,
) -> Result<Opened> {
    let unsupported = || Error::Conflict(bounds.unsupported);
    let root = repo
        .info()
        .worktree
        .as_ref()
        .ok_or(Error::WorktreeRequired)?;
    let full = root.join(path.argument()?);
    let parents = local_files::pin_directories(root, full.parent().ok_or_else(unsupported)?)?;
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options
            .share_mode(if allow_replace { 1 | 4 } else { 1 })
            .custom_flags(0x00200000);
    }
    #[cfg(not(windows))]
    let _ = allow_replace; // Only Windows share modes deny writers while a file is read.
    let file = options
        .open(&full)
        .map_err(|e| io("open conflict working file", e))?;
    let meta = file
        .metadata()
        .map_err(|e| io("inspect conflict file", e))?;
    if !meta.is_file() || local_files::redirected(&meta) {
        return Err(unsupported());
    }
    crate::review::verify_preview_handle(&file, root, &full)?;
    Ok(Opened {
        file,
        _parents: parents,
    })
}
pub(super) fn bytes(file: &File) -> Result<Vec<u8>> {
    limited(file, TEXT)
}
fn limited(file: &File, bounds: Bounds) -> Result<Vec<u8>> {
    let mut file = file;
    file.seek(SeekFrom::Start(0))
        .map_err(|e| io("seek conflict file", e))?;
    let mut bytes = Vec::new();
    file.take((bounds.limit + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|e| io("read conflict file", e))?;
    if bytes.len() > bounds.limit {
        return Err(Error::Conflict(bounds.unsupported));
    }
    Ok(bytes)
}
pub(super) fn read(repo: &Repository, path: &RepoPath) -> Result<Vec<u8>> {
    bytes(&open(repo, path)?.file)
}
fn read_bounded(repo: &Repository, path: &RepoPath, bounds: Bounds) -> Result<Vec<u8>> {
    limited(&open_mode(repo, path, true, bounds)?.file, bounds)
}
/// The working bytes, or None when the path (or its folder) does not exist.
pub(super) fn read_optional(
    repo: &Repository,
    path: &RepoPath,
    bounds: Bounds,
) -> Result<Option<Vec<u8>>> {
    match read_bounded(repo, path, bounds) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(Error::Io { source, .. }) if source.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

/// One checked working-file change and how its failures are reported.
#[cfg_attr(not(windows), expect(dead_code, reason = "only the Windows publication reads a change"))]
pub(super) struct Change<'a> {
    pub path: &'a RepoPath,
    /// The reviewed bytes, or None when the path was absent.
    pub observed: Option<&'a [u8]>,
    /// The intended bytes, or None to move the file into recovery.
    pub result: Option<&'a [u8]>,
    pub bounds: Bounds,
    /// Reported when the file no longer matches `observed` before publication.
    pub stale: ConflictFailure,
    /// Reported when the displaced or published file differs afterwards.
    pub unverified: ConflictFailure,
    pub readonly: &'static str,
}

/// The text editor's save: replace the reviewed file with `result`.
pub(super) fn publish(
    repo: &Repository,
    preview: &ConflictPreview,
    result: &[u8],
    cancel: &CancellationToken,
    saved: &mut bool,
    recovery: &mut Option<PathBuf>,
) -> Result<()> {
    let change = Change {
        path: &preview.path,
        observed: Some(&preview.original),
        result: Some(result),
        bounds: TEXT,
        stale: ConflictFailure::FileChanged,
        unverified: ConflictFailure::FileChanged,
        readonly: "The working file is read-only. Change its permissions explicitly before saving; your draft is retained.",
    };
    let recheck = || {
        if repo.conflict_context(&preview.path, cancel)? != preview.context {
            return Err(Error::Conflict(ConflictFailure::ContextChanged));
        }
        Ok(())
    };
    publish_change(repo, &change, recheck, cancel, saved, recovery)
}

#[cfg(windows)]
pub(super) fn publish_change(
    repo: &Repository,
    change: &Change<'_>,
    recheck: impl Fn() -> Result<()>,
    cancel: &CancellationToken,
    saved: &mut bool,
    recovery: &mut Option<PathBuf>,
) -> Result<()> {
    use std::os::windows::ffi::OsStrExt;
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn ReplaceFileW(
            replaced: *const u16,
            replacement: *const u16,
            backup: *const u16,
            flags: u32,
            exclude: *mut std::ffi::c_void,
            reserved: *mut std::ffi::c_void,
        ) -> i32;
    }
    let stale = || Error::Conflict(change.stale);
    let unverified = || Error::Conflict(change.unverified);
    cancel.check()?;
    let root = repo
        .info()
        .worktree
        .as_ref()
        .ok_or(Error::WorktreeRequired)?;
    let target = root.join(change.path.argument()?);
    // An existing file stays open against in-place writers until publication;
    // a file to be created has its folders pinned instead.
    let (_opened, _parents) = match change.observed {
        Some(observed) => {
            let opened = open_mode(repo, change.path, true, change.bounds)?;
            if limited(&opened.file, change.bounds)? != observed {
                return Err(stale());
            }
            if opened
                .file
                .metadata()
                .map_err(|e| io("inspect conflict permissions", e))?
                .permissions()
                .readonly()
            {
                return Err(Error::InvalidInput(change.readonly.into()));
            }
            (Some(opened), Vec::new())
        }
        None => {
            if read_optional(repo, change.path, change.bounds)?.is_some() {
                return Err(stale());
            }
            let parent = target
                .parent()
                .ok_or(Error::Conflict(change.bounds.unsupported))?;
            (None, local_files::pin_directories(root, parent)?)
        }
    };
    if change.observed.is_none() && change.result.is_none() {
        return Ok(());
    }
    let git_dir = &repo.info().git_dir;
    let _git_guard = local_files::pin_directories(git_dir, git_dir)?;
    let parent = git_dir.join("rocket-conflict-recovery");
    match fs::create_dir(&parent) {
        Ok(()) => (),
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => (),
        Err(e) => return Err(io("create conflict recovery directory", e)),
    }
    let _parent_guard = local_files::pin_directories(git_dir, &parent)?;
    let unique = format!(
        "{}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| Error::InvalidInput("System clock is before the Unix epoch".into()))?
            .as_nanos(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    );
    let directory = parent.join(unique);
    fs::create_dir(&directory).map_err(|e| io("reserve conflict recovery directory", e))?;
    let _recovery_guard = local_files::pin_directories(git_dir, &directory)?;
    *recovery = Some(directory.clone());
    let write = |name: &str, content: &[u8]| -> Result<()> {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(directory.join(name))
            .map_err(|e| io("prepare conflict recovery copy", e))?;
        file.write_all(content)
            .and_then(|()| file.sync_all())
            .map_err(|e| io("flush conflict recovery copy", e))
    };
    if let Some(observed) = change.observed {
        write("observed-original", observed)?;
    }
    if let Some(result) = change.result {
        write("result", result)?;
        write("replacement", result)?;
    }
    // No cancellation is reported after successful publication as though nothing
    // happened. The caller reconciles staging and operation state independently.
    cancel.check()?;
    recheck()?;
    match change.observed {
        Some(observed) if read_bounded(repo, change.path, change.bounds)? != observed => {
            return Err(stale());
        }
        None if read_optional(repo, change.path, change.bounds)?.is_some() => {
            return Err(stale());
        }
        _ => (),
    }
    cancel.check()?;
    let backup = directory.join("replaced-original");
    match (change.observed, change.result) {
        (Some(_), Some(_)) => {
            let wide = |path: &Path| {
                path.as_os_str()
                    .encode_wide()
                    .chain(Some(0))
                    .collect::<Vec<_>>()
            };
            let target = wide(&target);
            let replacement = wide(&directory.join("replacement"));
            let backup_wide = wide(&backup);
            // SAFETY: live NUL-terminated paths, reserved pointers null; flags zero retain
            // ACL/stream checks. All artifacts are uniquely reserved on the Git volume.
            // ReplaceFile can partially fail: never delete its recovery directory or
            // assume an error means the original path is intact.
            let ok = unsafe {
                ReplaceFileW(
                    target.as_ptr(),
                    replacement.as_ptr(),
                    backup_wide.as_ptr(),
                    0,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                )
            };
            if ok == 0 {
                return Err(io(
                    "replace conflict file; inspect retained recovery copies before retrying",
                    std::io::Error::last_os_error(),
                ));
            }
        }
        // Removal keeps the displaced file itself; creation never replaces a file
        // that appeared after the last check (MoveFileEx without REPLACE_EXISTING).
        (Some(_), None) => local_files::move_path(&target, &backup, false).map_err(|e| {
            moved(
                e,
                "move conflict file into recovery; inspect the path and retained copies before retrying",
            )
        })?,
        (None, Some(_)) => {
            local_files::move_path(&directory.join("replacement"), &target, false).map_err(
                |e| match e {
                    Error::Io { source, .. } if matches!(source.raw_os_error(), Some(80 | 183)) => {
                        stale()
                    }
                    other => moved(
                        other,
                        "create conflict file; inspect retained recovery copies before retrying",
                    ),
                },
            )?
        }
        (None, None) => unreachable!("no-op changes return before reserving recovery"),
    }
    *saved = true;
    // A rename from another application can race the last path check. Keep the
    // actual displaced file, detect that race, and do not stage or auto-rollback.
    if let Some(observed) = change.observed {
        let backup_file = {
            use std::os::windows::fs::OpenOptionsExt;
            OpenOptions::new()
                .read(true)
                .share_mode(1)
                .custom_flags(0x00200000)
                .open(&backup)
                .map_err(|e| io("verify replaced conflict file", e))?
        };
        let meta = backup_file
            .metadata()
            .map_err(|e| io("inspect conflict recovery copy", e))?;
        if !meta.is_file() || local_files::redirected(&meta) {
            return Err(Error::Conflict(change.bounds.unsupported));
        }
        crate::review::verify_preview_handle(&backup_file, git_dir, &backup)?;
        if limited(&backup_file, change.bounds)? != observed {
            return Err(unverified());
        }
    }
    match change.result {
        Some(result) if read_bounded(repo, change.path, change.bounds)? != result => {
            Err(unverified())
        }
        None if fs::symlink_metadata(target).is_ok() => Err(unverified()),
        _ => Ok(()),
    }
}
#[cfg(windows)]
fn moved(error: Error, action: &'static str) -> Error {
    match error {
        Error::Io { source, .. } => io(action, source),
        other => other,
    }
}
#[cfg(not(windows))]
pub(super) fn publish_change(
    _repo: &Repository,
    _change: &Change<'_>,
    _recheck: impl Fn() -> Result<()>,
    _cancel: &CancellationToken,
    _saved: &mut bool,
    _recovery: &mut Option<PathBuf>,
) -> Result<()> {
    Err(Error::Unsupported(
        "Checked conflict-file publication currently requires Windows".into(),
    ))
}
