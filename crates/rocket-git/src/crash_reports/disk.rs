use super::*;
use std::sync::atomic::AtomicU64;
static NEXT: AtomicU64 = AtomicU64::new(1);
pub(super) fn redirected(meta: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        meta.file_attributes() & 0x400 != 0
    }
    #[cfg(not(windows))]
    {
        meta.file_type().is_symlink()
    }
}
pub(super) fn check_directory(path: &Path) -> io::Result<()> {
    if !path.is_absolute() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Use an absolute report directory",
        ));
    }
    for parent in path.ancestors() {
        let meta = fs::symlink_metadata(parent)?;
        if redirected(&meta) || !meta.is_dir() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Report storage cannot use redirected directories",
            ));
        }
    }
    Ok(())
}
fn create_directory(path: &Path) -> io::Result<()> {
    if !path.is_absolute() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Use an absolute report directory",
        ));
    }
    if path.exists() {
        return check_directory(path);
    }
    let parent = path.parent().ok_or_else(invalid)?;
    create_directory(parent)?;
    match fs::create_dir(path) {
        Ok(()) => (),
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => (),
        Err(e) => return Err(e),
    }
    check_directory(path)
}
fn name(path: &Path) -> bool {
    let Some(name) = path
        .file_name()
        .and_then(|s| s.to_str())
        .and_then(|s| s.strip_prefix("crash-"))
        .and_then(|s| s.strip_suffix(".rcr"))
    else {
        return false;
    };
    let parts: Vec<_> = name.split('-').collect();
    parts.len() == 3
        && parts
            .iter()
            .all(|p| !p.is_empty() && p.len() <= 32 && p.bytes().all(|b| b.is_ascii_hexdigit()))
}
pub(super) fn entries(path: &Path) -> io::Result<Vec<PathBuf>> {
    let mut result = vec![];
    for (i, entry) in fs::read_dir(path)?.enumerate() {
        if i >= 256 {
            return Err(io::Error::other(
                "Too many entries in the crash report directory",
            ));
        }
        let entry = entry?;
        if name(&entry.path()) {
            result.push(entry.path());
        }
    }
    result.sort();
    Ok(result)
}
fn options() -> OpenOptions {
    let mut o = OpenOptions::new();
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        o.custom_flags(0x00200000);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        o.mode(0o600);
    }
    o
}
pub(super) fn read_file(path: &Path) -> io::Result<File> {
    let file = options().read(true).open(path)?;
    let meta = file.metadata()?;
    if !meta.is_file() || redirected(&meta) {
        return Err(invalid());
    }
    Ok(file)
}
/// Keep directory ancestry from being renamed while reading or exporting on Windows.
pub(super) fn pin_directory(path: &Path) -> io::Result<Vec<File>> {
    check_directory(path)?;
    let mut held = vec![];
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        for parent in path.ancestors().collect::<Vec<_>>().into_iter().rev() {
            let file = OpenOptions::new()
                .read(true)
                .custom_flags(0x02000000 | 0x00200000)
                .share_mode(3)
                .open(parent)?;
            let meta = file.metadata()?;
            if !meta.is_dir() || redirected(&meta) {
                return Err(invalid());
            }
            held.push(file);
        }
    }
    Ok(held)
}
pub(super) fn prepare(path: &Path) -> io::Result<(PathBuf, File)> {
    create_directory(path)?;
    let _held = pin_directory(path)?;
    let path = path.canonicalize()?;
    let lock = options()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path.join("store.lock"))?;
    if redirected(&lock.metadata()?) {
        return Err(invalid());
    }
    lock.try_lock()
        .map_err(|_| io::Error::new(io::ErrorKind::WouldBlock, "Crash storage is busy"))?;
    let mut entries = entries(&path)?;
    // Active Windows files deny deletion, so a live host never loses its sink.
    entries.retain(|p| {
        let empty =
            fs::symlink_metadata(p).is_ok_and(|m| m.is_file() && !redirected(&m) && m.len() == 0);
        !(empty && fs::remove_file(p).is_ok())
    });
    let mut remaining = entries.len();
    for file in entries {
        if remaining < MAX_FILES {
            break;
        }
        let safe = fs::symlink_metadata(&file).is_ok_and(|m| m.is_file() && !redirected(&m));
        if safe && fs::remove_file(file).is_ok() {
            remaining -= 1;
        }
    }
    if remaining >= MAX_FILES {
        return Err(io::Error::other("Crash report storage is full or in use"));
    }
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let name = format!(
        "crash-{stamp:032x}-{:08x}-{:08x}.rcr",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    );
    let mut o = options();
    o.create_new(true).append(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        o.share_mode(1);
    }
    let file = o.open(path.join(name))?;
    Ok((path, file))
}
pub(super) fn export(
    bytes: &[u8],
    destination: &Path,
    check: impl Fn() -> Result<()>,
) -> Result<()> {
    let parent = destination.parent().ok_or_else(|| io_error(invalid()))?;
    let _held = pin_directory(parent).map_err(io_error)?;
    if destination.file_name().is_none() {
        return Err(io_error(invalid()));
    }
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let temporary = parent.join(format!(
        ".rocket-report-{}-{stamp:x}-{}.tmp",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let mut options = options();
    options.create_new(true).write(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.share_mode(1);
    }
    let mut file = options.open(&temporary).map_err(io_error)?;
    let result = (|| {
        check()?;
        file.write_all(bytes).map_err(io_error)?;
        file.sync_all().map_err(io_error)?;
        check()?;
        // A new hard link is an atomic no-overwrite publication on Windows/NTFS.
        // Unsupported filesystems return an error rather than weaken that guarantee.
        fs::hard_link(&temporary, destination).map_err(io_error)
    })();
    drop(file);
    let cleanup = fs::remove_file(&temporary);
    result?;
    cleanup.map_err(|_|Error::Unsupported("The report was saved, but its temporary link could not be removed. Do not retry the same destination.".into()))
}
