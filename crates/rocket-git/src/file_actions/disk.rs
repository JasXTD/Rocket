use super::*;
use crate::local_files;
use std::{
    fs::{self, File, OpenOptions},
    io::Read,
    path::Path,
};
// Used by the Windows and Linux publication; elsewhere publish reports Unsupported.
#[cfg(any(windows, target_os = "linux"))]
use std::{
    io::Write,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};
const FOLDER: &str = "rocket-file-backups";
#[cfg(any(windows, target_os = "linux"))]
static NEXT: AtomicU64 = AtomicU64::new(0);
pub(super) fn validate_path(path: &RepoPath) -> Result<()> {
    for part in path.as_bytes().split(|b| *b == b'/') {
        if part.eq_ignore_ascii_case(b".git") || part.ends_with(b".") || part.ends_with(b" ") {
            return fail(FileEditFailure::Unsupported);
        }
        #[cfg(windows)]
        {
            let base = part
                .split(|b| *b == b'.')
                .next()
                .unwrap()
                .to_ascii_uppercase();
            if matches!(base.as_slice(), b"CON" | b"PRN" | b"AUX" | b"NUL")
                || (base.len() == 4
                    && (base.starts_with(b"COM") || base.starts_with(b"LPT"))
                    && base[3].is_ascii_digit())
            {
                return fail(FileEditFailure::Unsupported);
            }
        }
    }
    Ok(())
}
fn open(root: &Path, path: &Path) -> Result<Option<File>> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.share_mode(1 | 4).custom_flags(0x00200000);
    }
    let file = match options.open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(Error::io("open reviewed file", e)),
    };
    let meta = file
        .metadata()
        .map_err(|e| Error::io("inspect reviewed file", e))?;
    if !meta.is_file() || local_files::redirected(&meta) {
        return fail(FileEditFailure::Unsupported);
    }
    // Opening follows a symbolic link here; refuse one as Windows refuses a reparse point.
    #[cfg(target_os = "linux")]
    if !local_files::opened_without_link(&file, path)
        .map_err(|e| Error::io("inspect reviewed file", e))?
    {
        return fail(FileEditFailure::Unsupported);
    }
    crate::review::verify_preview_handle(&file, root, path)?;
    Ok(Some(file))
}
fn read(root: &Path, path: &Path, cancel: &CancellationToken) -> Result<Option<Vec<u8>>> {
    let Some(mut file) = open(root, path)? else {
        return Ok(None);
    };
    let mut out = Vec::new();
    let mut buffer = [0; 65536];
    loop {
        cancel.check()?;
        let size = file
            .read(&mut buffer)
            .map_err(|e| Error::io("read reviewed file", e))?;
        if size == 0 {
            break;
        }
        out.extend_from_slice(&buffer[..size]);
        if out.len() > FILE_EDIT_LIMIT {
            return fail(FileEditFailure::Unsupported);
        }
    }
    Ok(Some(out))
}
fn target(repo: &Repository, path: &RepoPath) -> Result<PathBuf> {
    validate_path(path)?;
    let root = repo
        .info()
        .worktree
        .as_ref()
        .ok_or(Error::WorktreeRequired)?;
    let full = root.join(path.argument()?);
    let parent = full.parent().unwrap();
    let _guards = local_files::pin_directories(root, parent)?;
    let real = parent
        .canonicalize()
        .map_err(|e| Error::io("resolve file parent", e))?;
    if real.starts_with(&repo.info().git_dir) || real.starts_with(&repo.info().common_dir) {
        return fail(FileEditFailure::Unsupported);
    }
    let mut ancestor = parent;
    while ancestor != root {
        match fs::symlink_metadata(ancestor.join(".git")) {
            Ok(_) => return fail(FileEditFailure::Unsupported),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
            Err(e) => return Err(Error::io("inspect nested repository boundary", e)),
        }
        ancestor = ancestor
            .parent()
            .ok_or(Error::FileEdit(FileEditFailure::Unsupported))?;
    }
    Ok(full)
}
pub(super) fn read_working(
    repo: &Repository,
    path: &RepoPath,
    cancel: &CancellationToken,
) -> Result<Option<Vec<u8>>> {
    let full = target(repo, path)?;
    let root = repo.info().worktree.as_ref().unwrap();
    let _guard = local_files::pin_directories(root, full.parent().unwrap())?;
    read(root, &full, cancel)
}
#[cfg(any(windows, target_os = "linux"))]
fn write(directory: &Path, name: &str, bytes: &[u8], cancel: &CancellationToken) -> Result<()> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(directory.join(name))
        .map_err(|e| Error::io("reserve file recovery copy", e))?;
    for chunk in bytes.chunks(65536) {
        cancel.check()?;
        file.write_all(chunk)
            .map_err(|e| Error::io("write file recovery copy", e))?
    }
    file.sync_all()
        .map_err(|e| Error::io("flush file recovery copy", e))
}
fn backup_path(repo: &Repository, id: &str) -> Result<PathBuf> {
    if id.is_empty() || id.len() > 100 || !id.bytes().all(|b| b.is_ascii_digit() || b == b'-') {
        return fail(FileEditFailure::Unsupported);
    }
    Ok(repo.info().git_dir.join(FOLDER).join(id))
}
fn metadata(repo: &Repository, id: &str, cancel: &CancellationToken) -> Result<FileBackup> {
    let dir = backup_path(repo, id)?;
    let _guard = local_files::pin_directories(&repo.info().git_dir, &dir)?;
    let data = read(&repo.info().git_dir, &dir.join("record"), cancel)?
        .ok_or_else(|| Error::parse("file-backups", "missing recovery record"))?;
    let rest = data
        .strip_prefix(b"ROCKET-FILE-BACKUP-1\n")
        .ok_or_else(|| Error::parse("file-backups", "invalid recovery record"))?;
    if rest.len() < 2 || rest.len() > 32768 || rest[0] > 1 {
        return fail(FileEditFailure::Unsupported);
    }
    let path = RepoPath::new(rest[1..].to_vec())?;
    validate_path(&path)?;
    Ok(FileBackup {
        repository: repo.info().git_dir.clone(),
        id: id.into(),
        path,
        present: rest[0] == 1,
    })
}
pub(super) fn list(repo: &Repository, cancel: &CancellationToken) -> Result<Vec<FileBackup>> {
    let root = repo.info().git_dir.join(FOLDER);
    let _guard = local_files::pin_directories(&repo.info().git_dir, &repo.info().git_dir)?;
    if !root
        .try_exists()
        .map_err(|e| Error::io("inspect file backups", e))?
    {
        return Ok(Vec::new());
    }
    let _guard = local_files::pin_directories(&repo.info().git_dir, &root)?;
    let mut out = Vec::new();
    for (scanned, entry) in fs::read_dir(&root)
        .map_err(|e| Error::io("list file backups", e))?
        .enumerate()
    {
        cancel.check()?;
        let entry = entry.map_err(|e| Error::io("read file backup entry", e))?;
        if scanned >= 1000 {
            return Err(Error::Unsupported("More than 1,000 recovery copies. Inspect the retained copies in this worktree's Git metadata before continuing.".into()));
        }
        let id = entry
            .file_name()
            .into_string()
            .map_err(|_| Error::parse("file-backups", "invalid backup name"))?;
        // A cancelled preparation may not yet have a complete record. It remains
        // on disk, but is never offered as a valid recovery copy.
        if !entry
            .path()
            .join("ready")
            .try_exists()
            .map_err(|e| Error::io("read backup readiness", e))?
        {
            continue;
        }
        out.push(metadata(repo, &id, cancel)?);
    }
    out.sort_by(|a, b| b.id.cmp(&a.id));
    Ok(out)
}
pub(super) fn backup_contents(
    repo: &Repository,
    b: &FileBackup,
    cancel: &CancellationToken,
) -> Result<Option<Vec<u8>>> {
    let current = metadata(repo, &b.id, cancel)?;
    if current.path != b.path || current.present != b.present {
        return fail(FileEditFailure::Stale);
    }
    if !b.present {
        return Ok(None);
    }
    let dir = backup_path(repo, &b.id)?;
    let _guard = local_files::pin_directories(&repo.info().git_dir, &dir)?;
    // Prefer the exact file displaced at publication, including a late external
    // rename. The observed copy remains available when preparation was cancelled.
    for name in ["replaced-original", "observed-original"] {
        if let Some(bytes) = read(&repo.info().git_dir, &dir.join(name), cancel)? {
            return Ok(Some(bytes));
        }
    }
    Err(Error::parse("file-backups", "recovery content is missing"))
}
#[cfg(any(windows, target_os = "linux"))]
pub(super) fn publish(
    repo: &Repository,
    p: &FileEditPreview,
    cancel: &CancellationToken,
    applied: &mut bool,
    recovery: &mut Option<PathBuf>,
) -> Result<()> {
    let full = target(repo, &p.path)?;
    let root = repo.info().worktree.as_ref().unwrap();
    let _working_guard = local_files::pin_directories(root, full.parent().unwrap())?;
    let opened = open(root, &full)?;
    if let Some(f) = &opened
        && f.metadata()
            .map_err(|e| Error::io("inspect file permissions", e))?
            .permissions()
            .readonly()
    {
        return Err(Error::InvalidInput(
            "This file is read-only. Change its permissions explicitly before replacing it.".into(),
        ));
    }
    let git = &repo.info().git_dir;
    let _git_guard = local_files::pin_directories(git, git)?;
    let parent = git.join(FOLDER);
    match fs::create_dir(&parent) {
        Ok(()) => (),
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => (),
        Err(e) => return Err(Error::io("create file recovery folder", e)),
    }
    let _parent_guard = local_files::pin_directories(git, &parent)?;
    let id = format!(
        "{:020}-{}-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| Error::InvalidInput("System clock is before Unix epoch".into()))?
            .as_nanos(),
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    );
    let directory = parent.join(&id);
    fs::create_dir(&directory).map_err(|e| Error::io("reserve file recovery folder", e))?;
    let _backup_guard = local_files::pin_directories(git, &directory)?;
    *recovery = Some(directory.clone());
    repo.runner.step(p.source.label(), "prepare-backup", || {
        if let Some(bytes) = &p.before {
            write(&directory, "observed-original", bytes, cancel)?
        }
        if let Some(bytes) = &p.after {
            write(&directory, "result", bytes, cancel)?;
            write(&directory, "replacement", bytes, cancel)?
        }
        let mut record = b"ROCKET-FILE-BACKUP-1\n".to_vec();
        record.push(u8::from(p.before.is_some()));
        record.extend_from_slice(p.path.as_bytes());
        write(&directory, "record", &record, cancel)?;
        write(&directory, "ready", b"1", cancel)
    })?;
    repo.runner
        .step(p.source.label(), "revalidate-before-publish", || {
            repo.recheck_file_edit(p, cancel)
        })?;
    cancel.check()?;
    repo.runner.step(p.source.label(), "publish-file", || {
        let replacement = directory.join("replacement");
        // Linux files carry the executable bit Git tracks: keep the file's own
        // permissions, or apply the restored entry's mode.
        #[cfg(target_os = "linux")]
        if p.after.is_some() {
            local_files::set_published_mode(&replacement, opened.as_ref(), p.executable)
                .map_err(|e| Error::io("prepare file permissions", e))?;
        }
        match (&p.before, &p.after) {
            // Validated paths; the actual displaced data is retained.
            (Some(_), Some(_)) => local_files::replace_file(
                &full,
                &replacement,
                &directory.join("replaced-original"),
            )
            .map_err(|e| {
                Error::io(
                    "replace file; inspect retained recovery copies before retrying",
                    e,
                )
            })?,
            (Some(_), None) => {
                local_files::move_path(&full, &directory.join("replaced-original"), false)?
            }
            (None, Some(_)) => local_files::move_path(&replacement, &full, false)?,
            (None, None) => return fail(FileEditFailure::NoChange),
        }
        *applied = true;
        Ok(())
    })?;
    // A successful replacement is never reported as cancelled. Verify with a
    // fresh token and preserve any actual displaced file if a rename raced us.
    let fresh = CancellationToken::new();
    if p.before.is_some() && read(git, &directory.join("replaced-original"), &fresh)? != p.before {
        return fail(FileEditFailure::Verification);
    }
    Ok(())
}
#[cfg(not(any(windows, target_os = "linux")))]
pub(super) fn publish(
    _: &Repository,
    _: &FileEditPreview,
    _: &CancellationToken,
    _: &mut bool,
    _: &mut Option<PathBuf>,
) -> Result<()> {
    Err(Error::Unsupported(
        "Checked whole-file replacement currently requires Windows".into(),
    ))
}
