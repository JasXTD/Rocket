use super::*;
use std::{
    fs::{File, OpenOptions},
    io::{Read, Write},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};
static NEXT: AtomicU64 = AtomicU64::new(0);
pub(super) fn target(repo: &Repository, path: &RepoPath) -> Result<PathBuf> {
    let s = std::str::from_utf8(path.as_bytes()).map_err(|_| Error::Lfs(LfsFailure::Layout))?;
    if s.chars().any(char::is_control)
        || s.contains(['\\', ':', '*', '?', '"', '<', '>', '|'])
        || s.split('/').any(|p| {
            let stem = p.split('.').next().unwrap_or("").to_ascii_uppercase();
            p.eq_ignore_ascii_case(".git")
                || p.ends_with(['.', ' '])
                || matches!(
                    stem.as_str(),
                    "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
                )
                || ["COM", "LPT"].iter().any(|prefix| {
                    stem.strip_prefix(prefix).is_some_and(|n| {
                        matches!(
                            n,
                            "0" | "1"
                                | "2"
                                | "3"
                                | "4"
                                | "5"
                                | "6"
                                | "7"
                                | "8"
                                | "9"
                                | "¹"
                                | "²"
                                | "³"
                        )
                    })
                })
        })
    {
        return fail(LfsFailure::Layout);
    }
    let root = repo
        .info()
        .worktree
        .as_ref()
        .ok_or(Error::WorktreeRequired)?;
    let full = root.join(path.argument()?);
    let parent = full.parent().ok_or(Error::Lfs(LfsFailure::Layout))?;
    let _guards = local_files::pin_directories(root, parent)?;
    let mut ancestor = parent;
    while ancestor != root {
        if ancestor
            .join(".git")
            .try_exists()
            .map_err(|e| Error::io("inspect LFS nested boundary", e))?
        {
            return fail(LfsFailure::Layout);
        }
        ancestor = ancestor.parent().ok_or(Error::Lfs(LfsFailure::Layout))?;
    }
    Ok(full)
}
pub(super) fn open(path: &Path, delete: bool) -> Result<Option<File>> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options
            .share_mode(if delete { 1 | 4 } else { 1 })
            .custom_flags(0x00200000);
    }
    #[cfg(not(windows))]
    let _ = delete;
    let file = match options.open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(Error::io("open LFS file", e)),
    };
    let m = file
        .metadata()
        .map_err(|e| Error::io("inspect LFS file", e))?;
    if !m.is_file() || local_files::redirected(&m) {
        return fail(LfsFailure::Layout);
    }
    Ok(Some(file))
}
fn pointer(file: &mut File) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    file.take(1025)
        .read_to_end(&mut bytes)
        .map_err(|e| Error::io("read LFS pointer", e))?;
    Ok(bytes)
}
pub(super) fn pointer_bytes(repo: &Repository, path: &RepoPath) -> Result<Option<Vec<u8>>> {
    let p = target(repo, path)?;
    let _dirs = local_files::pin_directories(repo.cwd(), p.parent().unwrap())?;
    let Some(mut f) = open(&p, false)? else {
        return Ok(None);
    };
    crate::review::verify_preview_handle(&f, repo.cwd(), &p)?;
    Ok(Some(pointer(&mut f)?))
}
fn object(repo: &Repository, p: &LfsPointer) -> PathBuf {
    repo.info()
        .common_dir
        .join("lfs/objects")
        .join(&p.oid[..2])
        .join(&p.oid[2..4])
        .join(&p.oid)
}
pub(super) fn check_existing_store(repo: &Repository) -> Result<()> {
    let root = &repo.info().common_dir;
    for relative in [
        "lfs",
        "lfs/objects",
        "lfs/incomplete",
        "lfs/tmp",
        "lfs/logs",
    ] {
        let path = root.join(relative);
        let _g = existing_directories(root, &path)?;
    }
    Ok(())
}
// Inspect one component at a time, without following a missing or redirected
// parent. Retain handles while a consumer accesses the cache below it.
fn existing_directories(root: &Path, parent: &Path) -> Result<Option<Vec<File>>> {
    let relative = parent
        .strip_prefix(root)
        .map_err(|_| Error::Lfs(LfsFailure::Layout))?;
    let mut current = root.to_path_buf();
    let mut guards = local_files::pin_directories(root, root)?;
    for part in relative.components() {
        current.push(part);
        match fs::symlink_metadata(&current) {
            Ok(m) if m.is_dir() && !local_files::redirected(&m) => (),
            Ok(_) => return fail(LfsFailure::Layout),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(Error::io("inspect LFS storage", e)),
        }
        guards.extend(local_files::pin_directories(root, &current)?);
    }
    Ok(Some(guards))
}
pub(super) fn cached(repo: &Repository, p: &LfsPointer) -> Result<bool> {
    check_existing_store(repo)?;
    let path = object(repo, p);
    let Some(_guards) = existing_directories(&repo.info().common_dir, path.parent().unwrap())?
    else {
        return Ok(false);
    };
    Ok(open(&path, false)?.is_some_and(|f| f.metadata().is_ok_and(|m| m.len() == p.size)))
}
pub(super) fn pin_source(
    repo: &Repository,
    entries: &[LfsEntry],
    c: &CancellationToken,
) -> Result<Vec<File>> {
    let mut guards = Vec::new();
    for entry in entries {
        c.check()?;
        let path = object(repo, &entry.object);
        guards.extend(
            existing_directories(&repo.info().common_dir, path.parent().unwrap())?
                .ok_or(Error::Lfs(LfsFailure::MissingObject))?,
        );
        guards.push(open(&path, false)?.ok_or(Error::Lfs(LfsFailure::MissingObject))?);
    }
    Ok(guards)
}
pub(super) fn prepare_store(repo: &Repository, entries: &[LfsEntry]) -> Result<Vec<File>> {
    check_existing_store(repo)?;
    let root = &repo.info().common_dir;
    let mut guards = Vec::new();
    let mut directories = vec![
        root.join("lfs"),
        root.join("lfs/objects"),
        root.join("lfs/incomplete"),
        root.join("lfs/tmp"),
        root.join("lfs/logs"),
    ];
    for e in entries {
        let p = object(repo, &e.object);
        directories.push(p.parent().unwrap().parent().unwrap().to_owned());
        directories.push(p.parent().unwrap().to_owned());
    }
    for path in directories {
        guards.extend(local_files::pin_directories(root, path.parent().unwrap())?);
        match fs::create_dir(&path) {
            Ok(()) => (),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => (),
            Err(e) => return Err(Error::io("prepare LFS cache", e)),
        }
        guards.extend(local_files::pin_directories(root, &path)?);
    }
    Ok(guards)
}
pub(super) fn verify_cached(
    repo: &Repository,
    exe: &Path,
    p: &LfsPointer,
    c: &CancellationToken,
) -> Result<()> {
    if p.size > LFS_FILE_LIMIT {
        return fail(LfsFailure::Limit);
    }
    if !cached(repo, p)? {
        return fail(LfsFailure::MissingObject);
    }
    let path = object(repo, p);
    let _dirs = local_files::pin_directories(&repo.info().common_dir, path.parent().unwrap())?;
    let _f = open(&path, false)?.ok_or(Error::Lfs(LfsFailure::MissingObject))?;
    if helper::hash(repo, exe, &path, c)? != *p {
        return fail(LfsFailure::InvalidObject);
    }
    Ok(())
}
/// Read an integrity-checked local object while its handle prevents replacement
/// or in-place writes. Never run smudge/clean filters or fetch missing objects.
pub(super) fn conflict_content(
    repo: &Repository,
    exe: &Path,
    p: &LfsPointer,
    c: &CancellationToken,
) -> Result<Vec<u8>> {
    if p.size > crate::CONFLICT_FILE_LIMIT as u64 {
        return fail(LfsFailure::Limit);
    }
    if p.size == 0 && p.oid == "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855" {
        c.check()?;
        return Ok(Vec::new());
    }
    check_existing_store(repo)?;
    let path = object(repo, p);
    let _dirs = existing_directories(&repo.info().common_dir, path.parent().unwrap())?
        .ok_or(Error::Lfs(LfsFailure::MissingObject))?;
    let mut file = open(&path, false)?.ok_or(Error::Lfs(LfsFailure::MissingObject))?;
    if file.metadata().map_err(|e| Error::io("inspect conflict LFS object", e))?.len() != p.size {
        return fail(LfsFailure::InvalidObject);
    }
    if helper::hash(repo, exe, &path, c)? != *p {
        return fail(LfsFailure::InvalidObject);
    }
    let mut bytes = Vec::new();
    Read::by_ref(&mut file)
        .take(p.size + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| Error::io("read conflict LFS object", e))?;
    if bytes.len() as u64 != p.size {
        return fail(LfsFailure::InvalidObject);
    }
    c.check()?;
    Ok(bytes)
}
pub(super) fn validate_existing_object(
    repo: &Repository,
    exe: &Path,
    p: &LfsPointer,
    c: &CancellationToken,
) -> Result<Option<File>> {
    let path = object(repo, p);
    let _dirs = local_files::pin_directories(&repo.info().common_dir, path.parent().unwrap())?;
    let file = open(&path, false)?;
    if file.is_some() && helper::hash(repo, exe, &path, c)? != *p {
        return fail(LfsFailure::InvalidObject);
    }
    Ok(file)
}
pub(super) fn verify_working(
    repo: &Repository,
    exe: &Path,
    e: &LfsEntry,
    c: &CancellationToken,
) -> Result<()> {
    let path = target(repo, &e.path)?;
    let _dirs = local_files::pin_directories(repo.cwd(), path.parent().unwrap())?;
    let _f = open(&path, false)?.ok_or(Error::Lfs(LfsFailure::Verification))?;
    if helper::hash(repo, exe, &path, c)? != e.object {
        return fail(LfsFailure::Verification);
    }
    Ok(())
}
fn save(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut f = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|e| Error::io("reserve LFS recovery file", e))?;
    f.write_all(bytes)
        .and_then(|_| f.sync_all())
        .map_err(|e| Error::io("save LFS recovery file", e))
}
pub(super) fn checkout(
    repo: &Repository,
    p: &LfsPreview,
    c: &CancellationToken,
    recovery: &mut Option<PathBuf>,
    completed: &mut usize,
) -> Result<()> {
    let e = &p.entries[0];
    let path = target(repo, &e.path)?;
    let _parents = local_files::pin_directories(repo.cwd(), path.parent().unwrap())?;
    let mut original = open(&path, true)?.ok_or(Error::Lfs(LfsFailure::Modified))?;
    crate::review::verify_preview_handle(&original, repo.cwd(), &path)?;
    let permissions = original
        .metadata()
        .map_err(|e| Error::io("inspect LFS pointer", e))?
        .permissions();
    if permissions.readonly() || pointer(&mut original)? != e.bytes {
        return fail(LfsFailure::Stale);
    }
    let base = repo.info().git_dir.join("rocket-lfs-checkouts");
    let mut guards = local_files::pin_directories(&repo.info().git_dir, &repo.info().git_dir)?;
    match fs::create_dir(&base) {
        Ok(()) => (),
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => (),
        Err(e) => return Err(Error::io("create LFS recovery root", e)),
    }
    guards.extend(local_files::pin_directories(&repo.info().git_dir, &base)?);
    let id = format!(
        "{}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| Error::Lfs(LfsFailure::Verification))?
            .as_nanos(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    );
    let workspace = base.join(id);
    fs::create_dir(&workspace).map_err(|e| Error::io("create LFS recovery workspace", e))?;
    guards.extend(local_files::pin_directories(
        &repo.info().git_dir,
        &workspace,
    )?);
    *recovery = Some(workspace.clone());
    save(&workspace.join("before.pointer"), &e.bytes)?;
    save(&workspace.join("RECOVERY.md"),format!("Rocket LFS checkout recovery\n\nWorking file: {}\nObject: {}\nSize: {}\n\nbefore.pointer is the original pointer. original.pointer, if present, is the file moved from the working tree. prepared.blob, if present, contains prepared content. Inspect the working file and Git status before restoring anything. A completed or cancelled write is never automatically rolled back. Staging was not changed.\n",path.display(),e.object.oid,e.object.size).as_bytes())?;
    let cached = object(repo, &e.object);
    guards.extend(local_files::pin_directories(
        &repo.info().common_dir,
        cached.parent().unwrap(),
    )?);
    let mut source = open(&cached, false)?.ok_or(Error::Lfs(LfsFailure::MissingObject))?;
    let prepared = workspace.join("prepared.blob");
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&prepared)
        .map_err(|e| Error::io("prepare LFS checkout", e))?;
    let mut buffer = [0; 65536];
    let mut total = 0u64;
    loop {
        c.check()?;
        let n = source
            .read(&mut buffer)
            .map_err(|e| Error::io("read LFS cache", e))?;
        if n == 0 {
            break;
        }
        total += n as u64;
        if total > e.object.size || total > LFS_FILE_LIMIT {
            return fail(LfsFailure::InvalidObject);
        }
        output
            .write_all(&buffer[..n])
            .map_err(|e| Error::io("prepare LFS checkout", e))?;
    }
    output
        .sync_all()
        .map_err(|e| Error::io("flush LFS checkout", e))?;
    drop(output);
    if total != e.object.size || helper::hash(repo, &p.helper, &prepared, c)? != e.object {
        return fail(LfsFailure::InvalidObject);
    }
    fs::set_permissions(&prepared, permissions)
        .map_err(|e| Error::io("set LFS checkout permissions", e))?;
    repo.lfs_recheck(p, c)?;
    local_files::move_path(&path, &workspace.join("original.pointer"), false)?;
    *completed = 1;
    // Inspect the file that actually moved, then publish without replacing a new
    // arrival. Cancellation here retains both originals and prepared contents.
    if fs::read(workspace.join("original.pointer"))
        .map_err(|e| Error::io("verify retained LFS pointer", e))?
        != e.bytes
    {
        return fail(LfsFailure::Stale);
    }
    c.check()?;
    local_files::move_path(&prepared, &path, false)?;
    *completed = 2;
    Ok(())
}
