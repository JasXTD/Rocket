use super::*;
use crate::local_files;
use std::{
    fs::{File, OpenOptions},
    io::{Read, Seek, SeekFrom},
};
// Used by the Windows and Linux publication; elsewhere publish_change reports Unsupported.
#[cfg(any(windows, target_os = "linux"))]
use std::{
    fs,
    io::Write,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};
#[cfg(any(windows, target_os = "linux"))]
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
    // Opening follows a symbolic link here; refuse one as Windows refuses a reparse point.
    #[cfg(target_os = "linux")]
    if !local_files::opened_without_link(&file, &full).map_err(|e| io("inspect conflict file", e))? {
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
#[cfg_attr(
    not(any(windows, target_os = "linux")),
    expect(dead_code, reason = "only the Windows and Linux publication read a change")
)]
pub(super) struct Change<'a> {
    pub path: &'a RepoPath,
    /// The reviewed bytes, or None when the path was absent.
    pub observed: Option<&'a [u8]>,
    /// The intended bytes, or None to move the file into recovery.
    pub result: Option<&'a [u8]>,
    /// The kept version's Git mode, when it decides the executable bit (Linux); None
    /// keeps the replaced file's permission bits.
    #[cfg_attr(windows, expect(dead_code, reason = "Windows files have no executable bit"))]
    pub executable: Option<bool>,
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
        executable: None,
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

#[cfg(any(windows, target_os = "linux"))]
pub(super) fn publish_change(
    repo: &Repository,
    change: &Change<'_>,
    recheck: impl Fn() -> Result<()>,
    cancel: &CancellationToken,
    saved: &mut bool,
    recovery: &mut Option<PathBuf>,
) -> Result<()> {
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
    let replacement = directory.join("replacement");
    // Linux files carry the executable bit Git tracks: keep the replaced file's
    // permissions, or apply the kept version's mode.
    #[cfg(target_os = "linux")]
    if change.result.is_some() {
        let original = _opened.as_ref().map(|opened| &opened.file);
        local_files::set_published_mode(&replacement, original, change.executable)
            .map_err(|e| io("prepare conflict file permissions", e))?;
    }
    match (change.observed, change.result) {
        // All artifacts are uniquely reserved on the Git volume. Replacement can
        // partially fail: never delete its recovery directory or assume an error
        // means the original path is intact.
        (Some(_), Some(_)) => local_files::replace_file(&target, &replacement, &backup)
            .map_err(|e| {
                io(
                    "replace conflict file; inspect retained recovery copies before retrying",
                    e,
                )
            })?,
        // Removal keeps the displaced file itself; creation never replaces a file
        // that appeared after the last check (a move that never replaces).
        (Some(_), None) => local_files::move_path(&target, &backup, false).map_err(|e| {
            moved(
                e,
                "move conflict file into recovery; inspect the path and retained copies before retrying",
            )
        })?,
        (None, Some(_)) => local_files::move_path(&replacement, &target, false).map_err(|e| {
            match e {
                Error::Io { source, .. } if source.kind() == std::io::ErrorKind::AlreadyExists => {
                    stale()
                }
                other => moved(
                    other,
                    "create conflict file; inspect retained recovery copies before retrying",
                ),
            }
        })?,
        (None, None) => unreachable!("no-op changes return before reserving recovery"),
    }
    *saved = true;
    // A rename from another application can race the last path check. Keep the
    // actual displaced file, detect that race, and do not stage or auto-rollback.
    if let Some(observed) = change.observed {
        let mut options = OpenOptions::new();
        options.read(true);
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            options.share_mode(1).custom_flags(0x00200000);
        }
        let backup_file = options
            .open(&backup)
            .map_err(|e| io("verify replaced conflict file", e))?;
        let meta = backup_file
            .metadata()
            .map_err(|e| io("inspect conflict recovery copy", e))?;
        if !meta.is_file() || local_files::redirected(&meta) {
            return Err(Error::Conflict(change.bounds.unsupported));
        }
        #[cfg(target_os = "linux")]
        if !local_files::opened_without_link(&backup_file, &backup)
            .map_err(|e| io("inspect conflict recovery copy", e))?
        {
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
/// Whether the working file's executable bit (the owner's, which Git reads) differs
/// from a kept version's mode, so keeping identical bytes still needs a publication.
/// Windows files have no executable bit.
pub(super) fn executable_differs(
    repo: &Repository,
    path: &RepoPath,
    executable: Option<bool>,
) -> Result<bool> {
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::fs::PermissionsExt;
        let (Some(executable), Some(root)) = (executable, repo.info().worktree.as_ref()) else {
            return Ok(false);
        };
        match fs::symlink_metadata(root.join(path.argument()?)) {
            Ok(meta) if meta.is_file() => {
                Ok((meta.permissions().mode() & 0o100 != 0) != executable)
            }
            Ok(_) => Ok(false),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(e) => Err(io("inspect conflict file permissions", e)),
        }
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (repo, path, executable);
        Ok(false)
    }
}
#[cfg(any(windows, target_os = "linux"))]
fn moved(error: Error, action: &'static str) -> Error {
    match error {
        Error::Io { source, .. } => io(action, source),
        other => other,
    }
}
#[cfg(not(any(windows, target_os = "linux")))]
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
