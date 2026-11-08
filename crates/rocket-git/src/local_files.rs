//! Filesystem primitives for repository creation and transactional config writes.
use crate::{Error, Result};
#[cfg(windows)]
use std::fs::OpenOptions;
use std::{
    fs::{self, File},
    io::Read,
    path::Path,
};

#[cfg(target_os = "macos")]
use darwin as atomic;
#[cfg(target_os = "linux")]
use linux as atomic;

pub(crate) fn redirected(meta: &fs::Metadata) -> bool {
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

pub(crate) fn directory(path: &Path) -> Result<()> {
    let meta = fs::symlink_metadata(path).map_err(|e| Error::io("inspect repository folder", e))?;
    if !meta.is_dir() || redirected(&meta) {
        return Err(Error::InvalidInput(
            "Choose a normal folder, not a file, symbolic link or junction".into(),
        ));
    }
    Ok(())
}

pub(crate) fn read_config(path: &Path) -> Result<Vec<u8>> {
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.custom_flags(0x00200000); // FILE_FLAG_OPEN_REPARSE_POINT
    }
    let file = options
        .open(path)
        .map_err(|e| Error::io("read repository config", e))?;
    let meta = file
        .metadata()
        .map_err(|e| Error::io("inspect repository config", e))?;
    if !meta.is_file()
        || redirected(&meta)
        || fs::symlink_metadata(path).is_ok_and(|m| redirected(&m))
    {
        return Err(Error::Unsupported(
            "Repository config must be a regular file; edit redirected configuration with Git"
                .into(),
        ));
    }
    let mut bytes = Vec::new();
    file.take(2 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| Error::io("read repository config", e))?;
    if bytes.len() > 2 * 1024 * 1024 {
        return Err(Error::Unsupported(
            "Repository config exceeds the 2 MiB editor limit; edit it with Git".into(),
        ));
    }
    Ok(bytes)
}

/// Drive letters Windows reports as present, read from the drive table without
/// touching any media, so an empty card reader or optical drive never asks for a disk.
pub fn drive_letters() -> Vec<char> {
    #[cfg(windows)]
    {
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn GetLogicalDrives() -> u32;
        }
        // SAFETY: GetLogicalDrives takes no arguments and returns a bit mask by value.
        let mask = unsafe { GetLogicalDrives() };
        (0..26u8)
            .filter(|bit| mask & (1 << bit) != 0)
            .map(|bit| char::from(b'A' + bit))
            .collect()
    }
    #[cfg(not(windows))]
    {
        Vec::new()
    }
}

#[cfg(windows)]
pub(crate) fn move_path(source: &Path, destination: &Path, replace: bool) -> Result<()> {
    use std::os::windows::ffi::OsStrExt;
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn MoveFileExW(existing: *const u16, new: *const u16, flags: u32) -> i32;
    }
    let wide = |path: &Path| -> Result<Vec<u16>> {
        let mut value: Vec<_> = path.as_os_str().encode_wide().collect();
        if value.contains(&0) {
            return Err(Error::InvalidInput("A path cannot contain NUL".into()));
        }
        value.push(0);
        Ok(value)
    };
    let from = wide(source)?;
    let to = wide(destination)?;
    // SAFETY: live NUL-terminated buffers. No cross-volume copying. Creation uses
    // flags zero, so even a destination appearing concurrently is never replaced.
    let flags = if replace { 1 | 8 } else { 0 };
    if unsafe { MoveFileExW(from.as_ptr(), to.as_ptr(), flags) } == 0 {
        return Err(Error::io(
            "publish repository metadata",
            std::io::Error::last_os_error(),
        ));
    }
    Ok(())
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
pub(crate) fn move_path(source: &Path, destination: &Path, replace: bool) -> Result<()> {
    if replace {
        return fs::rename(source, destination)
            .map_err(|e| Error::io("publish repository metadata", e));
    }
    move_new(source, destination).map_err(|e| no_replace_error(e, "publish repository metadata"))
}

#[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
pub(crate) fn move_path(source: &Path, destination: &Path, replace: bool) -> Result<()> {
    if !replace {
        return Err(Error::Unsupported(
            "Non-replacing publication is unavailable on this platform".into(),
        ));
    }
    fs::rename(source, destination).map_err(|e| Error::io("publish repository metadata", e))
}

/// Move `source` to `destination` only while nothing is there. The check and the
/// move are one step (Linux NOREPLACE / Darwin EXCL), so an entry that appears
/// concurrently is never replaced. Both paths must be on one filesystem.
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub(crate) fn move_new(source: &Path, destination: &Path) -> std::io::Result<()> {
    atomic::rename(source, destination, atomic::RENAME_NOREPLACE)
}

/// The error for a failed non-replacing move. A filesystem without the flag (EINVAL
/// or ENOSYS, as on some network and FUSE mounts) is refused: Rocket never retries
/// with a move that could replace an existing file.
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub(crate) fn no_replace_error(error: std::io::Error, action: &'static str) -> Error {
    if atomic::unsupported(&error) {
        #[cfg(target_os = "linux")]
        {
            Error::Unsupported(
                "This folder's filesystem cannot move files without the risk of replacing \
             existing ones (renameat2 RENAME_NOREPLACE), so Rocket did not move anything. \
             Use a folder on a local filesystem such as ext4, XFS or Btrfs"
                    .into(),
            )
        }
        #[cfg(target_os = "macos")]
        {
            Error::Unsupported(
                "This filesystem cannot publish without replacement (renamex_np RENAME_EXCL). \
             Nothing was replaced; choose a local volume supporting exclusive rename, such as APFS"
                    .into(),
            )
        }
    } else {
        Error::io(action, error)
    }
}

/// Replace the file at `target` with `replacement`, keeping the file it displaces at
/// `backup`. All three are on one volume, and `backup` must not exist. A failure can
/// leave a partial result: never delete the recovery folder, and never assume that an
/// error means `target` is unchanged.
#[cfg(windows)]
pub(crate) fn replace_file(
    target: &Path,
    replacement: &Path,
    backup: &Path,
) -> std::io::Result<()> {
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
    let wide = |path: &Path| {
        path.as_os_str()
            .encode_wide()
            .chain(Some(0))
            .collect::<Vec<_>>()
    };
    let (target, replacement, backup) = (wide(target), wide(replacement), wide(backup));
    // SAFETY: live NUL-terminated paths, reserved pointers null; flags zero retain
    // ACL/stream checks, and the actual displaced file is kept at `backup`.
    let ok = unsafe {
        ReplaceFileW(
            target.as_ptr(),
            replacement.as_ptr(),
            backup.as_ptr(),
            0,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    if ok == 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

/// Swap `replacement` and `target` in one step (Linux EXCHANGE / Darwin SWAP), so `target`
/// always names a whole file, then move the displaced file to `backup` without
/// replacing anything. The displaced file is the one actually at `target` at the swap,
/// even if another program renamed something there after the last check.
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub(crate) fn replace_file(
    target: &Path,
    replacement: &Path,
    backup: &Path,
) -> std::io::Result<()> {
    atomic::rename(replacement, target, atomic::RENAME_EXCHANGE).map_err(|error| {
        if atomic::unsupported(&error) {
            std::io::Error::new(
                std::io::ErrorKind::Unsupported,
                "This filesystem cannot atomically swap files; the file was not changed. \
                 Choose a local filesystem supporting atomic exchange (Linux renameat2 \
                 RENAME_EXCHANGE or macOS renamex_np RENAME_SWAP)",
            )
        } else {
            error
        }
    })?;
    atomic::rename(replacement, backup, atomic::RENAME_NOREPLACE).map_err(|error| {
        std::io::Error::new(
            error.kind(),
            format!(
                "the new file is in place, but the file it replaced is still in the \
                 recovery folder as \"replacement\" ({error})"
            ),
        )
    })
}

/// Unix: give a file about to be published the permission bits Git expects. Keep the
/// bits of the file it replaces (`original`), or of `prepared` as created for a new
/// file; with a known Git mode, set or clear the executable bits to match it, the way
/// checkout derives them from the read bits. Windows has no executable bit.
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub(crate) fn set_published_mode(
    prepared: &Path,
    original: Option<&File>,
    executable: Option<bool>,
) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let base = match original {
        Some(file) => file.metadata()?,
        None => fs::symlink_metadata(prepared)?,
    };
    let mode = published_mode(base.permissions().mode(), executable);
    fs::set_permissions(prepared, fs::Permissions::from_mode(mode))
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn published_mode(base: u32, executable: Option<bool>) -> u32 {
    let base = base & 0o777;
    match executable {
        None => base,
        Some(false) => base & !0o111,
        Some(true) => base | ((base & 0o444) >> 2),
    }
}

/// Unix: whether `file`, opened from `path`, is the entry at `path` itself and not a
/// file reached through a symbolic link. Windows opens reparse points themselves.
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub(crate) fn opened_without_link(file: &File, path: &Path) -> std::io::Result<bool> {
    use std::os::unix::fs::MetadataExt;
    let opened = file.metadata()?;
    let entry = fs::symlink_metadata(path)?;
    Ok(!entry.file_type().is_symlink()
        && entry.dev() == opened.dev()
        && entry.ino() == opened.ino())
}

/// Query the *opened* vnode, not a pathname that could now refer to another file.
#[cfg(target_os = "macos")]
pub(crate) fn opened_path(file: &File) -> std::io::Result<std::path::PathBuf> {
    use std::{
        ffi::OsString,
        io,
        os::{fd::AsRawFd, unix::ffi::OsStringExt},
    };
    unsafe extern "C" {
        fn fcntl(fd: std::ffi::c_int, command: std::ffi::c_int, ...) -> std::ffi::c_int;
    }
    let mut path = [0u8; 1024]; // Darwin MAXPATHLEN; F_GETPATH = 50 (sys/fcntl.h).
    // SAFETY: live descriptor, correctly sized writable F_GETPATH buffer.
    if unsafe { fcntl(file.as_raw_fd(), 50, path.as_mut_ptr()) } == -1 {
        return Err(io::Error::last_os_error());
    }
    let end = path
        .iter()
        .position(|b| *b == 0)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "Unterminated file path"))?;
    let path = std::path::PathBuf::from(OsString::from_vec(path[..end].to_vec()));
    if !path.is_absolute() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Opened file has no absolute path",
        ));
    }
    Ok(path)
}

/// Prevent leaf-link traversal at open time; callers still verify parent directories
/// and the opened file's identity. std retains its own close-on-exec behavior.
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub(crate) fn no_follow(options: &mut fs::OpenOptions) {
    use std::os::unix::fs::OpenOptionsExt;
    #[cfg(target_os = "linux")]
    options.custom_flags(0x20000); // O_NOFOLLOW, Linux
    #[cfg(target_os = "macos")]
    options.custom_flags(0x100); // O_NOFOLLOW, Darwin (not Linux's O_NOCTTY!)
}

/// glibc's `renameat2` (2.28 and later), declared here: rocket-git has no dependencies.
#[cfg(target_os = "linux")]
mod linux {
    use std::{
        ffi::{CString, c_char, c_int, c_uint},
        io,
        os::unix::ffi::OsStrExt,
        path::Path,
    };
    const AT_FDCWD: c_int = -100;
    pub(super) const RENAME_NOREPLACE: c_uint = 1;
    pub(super) const RENAME_EXCHANGE: c_uint = 2;
    unsafe extern "C" {
        fn renameat2(
            old_directory: c_int,
            old_path: *const c_char,
            new_directory: c_int,
            new_path: *const c_char,
            flags: c_uint,
        ) -> c_int;
    }
    fn text(path: &Path) -> io::Result<CString> {
        CString::new(path.as_os_str().as_bytes())
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "A path cannot contain NUL"))
    }
    pub(super) fn rename(source: &Path, destination: &Path, flags: c_uint) -> io::Result<()> {
        let (from, to) = (text(source)?, text(destination)?);
        // SAFETY: both are live NUL-terminated strings for the duration of the call.
        // Rocket passes absolute paths, so AT_FDCWD never changes their meaning.
        if unsafe { renameat2(AT_FDCWD, from.as_ptr(), AT_FDCWD, to.as_ptr(), flags) } == 0 {
            Ok(())
        } else {
            Err(io::Error::last_os_error())
        }
    }
    /// EINVAL (a filesystem without the flag) or ENOSYS (a kernel without renameat2),
    /// as reported by the system, not a path Rocket could not pass to it.
    pub(super) fn unsupported(error: &io::Error) -> bool {
        error.raw_os_error().is_some()
            && matches!(
                error.kind(),
                io::ErrorKind::InvalidInput | io::ErrorKind::Unsupported
            )
    }
}

// ABI and flags: Apple xnu bsd/sys/stdio.h and renamex_np(2). Darwin's EXCL is
// not Linux's NOREPLACE value. Unsupported filesystems must fail closed.
#[cfg(target_os = "macos")]
mod darwin {
    use std::{
        ffi::{CString, c_char, c_int, c_uint},
        io,
        os::unix::ffi::OsStrExt,
        path::Path,
    };
    pub(super) const RENAME_NOREPLACE: c_uint = 0x4; // RENAME_EXCL
    pub(super) const RENAME_EXCHANGE: c_uint = 0x2; // RENAME_SWAP
    unsafe extern "C" {
        fn renamex_np(from: *const c_char, to: *const c_char, flags: c_uint) -> c_int;
    }
    pub(super) fn rename(source: &Path, destination: &Path, flags: c_uint) -> io::Result<()> {
        let text = |p: &Path| {
            CString::new(p.as_os_str().as_bytes()).map_err(|_| {
                io::Error::new(io::ErrorKind::InvalidInput, "A path cannot contain NUL")
            })
        };
        let (from, to) = (text(source)?, text(destination)?);
        // SAFETY: owned NUL-terminated byte paths live throughout the system call.
        if unsafe { renamex_np(from.as_ptr(), to.as_ptr(), flags) } == 0 {
            Ok(())
        } else {
            Err(io::Error::last_os_error())
        }
    }
    pub(super) fn unsupported(error: &io::Error) -> bool {
        error.raw_os_error().is_some()
            && matches!(
                error.kind(),
                io::ErrorKind::InvalidInput | io::ErrorKind::Unsupported
            )
    }
}

pub(crate) fn pin_directories(root: &Path, parent: &Path) -> Result<Vec<File>> {
    let relative = parent.strip_prefix(root).map_err(|_| {
        Error::Unsupported(
            "File operations require existing normal folders, without links or junctions".into(),
        )
    })?;
    let mut current = root.to_path_buf();
    #[cfg(windows)]
    let mut out = Vec::new();
    // Other platforms have no handle that pins a folder against rename.
    #[cfg(not(windows))]
    let out = Vec::new();
    let mut paths = vec![current.clone()];
    for component in relative.components() {
        if !matches!(component, std::path::Component::Normal(_)) {
            return Err(Error::Unsupported(
                "File operations require existing normal folders, without links or junctions"
                    .into(),
            ));
        }
        current.push(component);
        paths.push(current.clone());
    }
    for path in paths {
        directory(&path)?;
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            let file = OpenOptions::new()
                .read(true)
                .share_mode(3)
                .custom_flags(0x02200000)
                .open(&path)
                .map_err(|e| Error::io("pin file operation directory", e))?;
            let meta = file
                .metadata()
                .map_err(|e| Error::io("inspect file operation directory", e))?;
            if !meta.is_dir() || redirected(&meta) {
                return Err(Error::Unsupported(
                    "File operations require existing normal folders, without links or junctions"
                        .into(),
                ));
            }
            out.push(file);
        }
    }
    Ok(out)
}

#[cfg(all(test, windows))]
mod drive_tests {
    #[test]
    fn drive_letters_include_the_system_drive() {
        let system = std::env::var("SystemDrive").unwrap_or_else(|_| "C:".into());
        let letter = system.chars().next().unwrap().to_ascii_uppercase();
        assert!(super::drive_letters().contains(&letter));
    }
}

#[cfg(all(test, any(target_os = "linux", target_os = "macos")))]
mod unix_tests {
    use super::*;
    use std::{io::ErrorKind, os::unix::fs::PermissionsExt, path::PathBuf};

    struct Scratch(PathBuf);
    impl Scratch {
        fn new(name: &str) -> Self {
            let path = std::env::temp_dir()
                .join(format!("rocket-local-files-{name}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&path);
            fs::create_dir(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn non_replacing_moves_publish_new_paths_and_never_replace() {
        let s = Scratch::new("move");
        let (a, b) = (s.0.join("a"), s.0.join("b"));
        fs::create_dir(&a).unwrap();
        fs::write(a.join("file"), "a").unwrap();
        move_path(&a, &b, false).unwrap();
        assert!(!a.exists());
        assert_eq!(fs::read_to_string(b.join("file")).unwrap(), "a");
        // A folder or file already at the destination stays, and so does the source.
        fs::write(&a, "source").unwrap();
        let error = move_path(&a, &b, false).unwrap_err();
        assert!(
            matches!(&error, Error::Io { source, .. } if source.kind() == ErrorKind::AlreadyExists),
            "{error}"
        );
        assert_eq!(fs::read_to_string(&a).unwrap(), "source");
        assert_eq!(fs::read_to_string(b.join("file")).unwrap(), "a");
        let taken = s.0.join("taken");
        fs::write(&taken, "existing").unwrap();
        assert!(move_path(&a, &taken, false).is_err());
        assert_eq!(fs::read_to_string(&taken).unwrap(), "existing");
        // Replacing moves stay ordinary renames.
        move_path(&a, &taken, true).unwrap();
        assert_eq!(fs::read_to_string(&taken).unwrap(), "source");
    }

    #[test]
    fn filesystems_without_the_flag_are_refused_without_a_fallback() {
        let refused = no_replace_error(
            std::io::Error::from_raw_os_error(22), // EINVAL
            "publish repository metadata",
        );
        let flag = if cfg!(target_os = "macos") {
            "RENAME_EXCL"
        } else {
            "RENAME_NOREPLACE"
        };
        assert!(
            matches!(&refused, Error::Unsupported(text) if text.contains(flag)),
            "{refused}"
        );
        #[cfg(all(
            target_os = "linux",
            any(target_arch = "x86_64", target_arch = "aarch64")
        ))]
        assert!(matches!(
            no_replace_error(std::io::Error::from_raw_os_error(38), "move"), // ENOSYS
            Error::Unsupported(_)
        ));
        // Other failures keep their own error, and a path the system never saw
        // (here, one with NUL) is not a filesystem refusal.
        assert!(matches!(
            no_replace_error(std::io::Error::from_raw_os_error(13), "move"), // EACCES
            Error::Io { .. }
        ));
        let nul = move_path(
            Path::new("/nonexistent/a\0b"),
            Path::new("/nonexistent/c"),
            false,
        );
        assert!(matches!(nul, Err(Error::Io { .. })), "{nul:?}");
    }

    #[test]
    fn replacement_swaps_in_one_step_and_keeps_the_displaced_file() {
        let s = Scratch::new("replace");
        let target = s.0.join("target");
        let (replacement, backup) = (s.0.join("replacement"), s.0.join("backup"));
        fs::write(&target, "old").unwrap();
        fs::set_permissions(&target, fs::Permissions::from_mode(0o750)).unwrap();
        fs::write(&replacement, "new").unwrap();
        let original = File::open(&target).unwrap();
        set_published_mode(&replacement, Some(&original), None).unwrap();
        replace_file(&target, &replacement, &backup).unwrap();
        assert_eq!(fs::read_to_string(&target).unwrap(), "new");
        assert_eq!(fs::read_to_string(&backup).unwrap(), "old");
        assert!(!replacement.exists());
        assert_eq!(
            fs::metadata(&target).unwrap().permissions().mode() & 0o777,
            0o750
        );
        // A taken backup name is never replaced. The new file is already in place,
        // and the file it displaced stays as "replacement".
        fs::write(&replacement, "newer").unwrap();
        let error = replace_file(&target, &replacement, &backup).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::AlreadyExists, "{error}");
        assert_eq!(fs::read_to_string(&target).unwrap(), "newer");
        assert_eq!(fs::read_to_string(&replacement).unwrap(), "new");
        assert_eq!(fs::read_to_string(&backup).unwrap(), "old");
        // Without a file to replace, nothing moves.
        let missing = replace_file(&s.0.join("missing"), &replacement, &s.0.join("other"));
        assert_eq!(missing.unwrap_err().kind(), ErrorKind::NotFound);
        assert_eq!(fs::read_to_string(&replacement).unwrap(), "new");
    }

    #[test]
    fn published_files_get_the_executable_bits_git_records() {
        assert_eq!(published_mode(0o100644, None), 0o644);
        assert_eq!(published_mode(0o4755, None), 0o755, "no set-user-ID");
        assert_eq!(published_mode(0o644, Some(true)), 0o755);
        assert_eq!(published_mode(0o600, Some(true)), 0o700);
        assert_eq!(published_mode(0o640, Some(true)), 0o750);
        assert_eq!(published_mode(0o755, Some(false)), 0o644);
        assert_eq!(published_mode(0o700, Some(false)), 0o600);
    }

    #[test]
    fn files_reached_through_links_are_not_the_entry_itself() {
        let s = Scratch::new("links");
        let file = s.0.join("file");
        fs::write(&file, "x").unwrap();
        let link = s.0.join("link");
        std::os::unix::fs::symlink(&file, &link).unwrap();
        assert!(opened_without_link(&File::open(&file).unwrap(), &file).unwrap());
        assert!(!opened_without_link(&File::open(&link).unwrap(), &link).unwrap());
        // A file put in its place after opening is not the opened one either.
        let opened = File::open(&file).unwrap();
        fs::rename(&file, s.0.join("moved")).unwrap();
        fs::write(&file, "y").unwrap();
        assert!(!opened_without_link(&opened, &file).unwrap());
    }

    #[test]
    fn no_follow_open_preserves_the_target_of_a_leaf_link() {
        let s = Scratch::new("nofollow");
        let target = s.0.join("target");
        fs::write(&target, "preserve me").unwrap();
        let alias = s.0.join("link");
        std::os::unix::fs::symlink(&target, &alias).unwrap();
        let mut options = fs::OpenOptions::new();
        options.read(true).write(true).create(true).truncate(false);
        no_follow(&mut options);
        assert!(options.open(&alias).is_err());
        assert!(options.open(&target).is_ok());
        assert_eq!(fs::read(&target).unwrap(), b"preserve me");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn opened_vnode_tracks_renames_and_detects_escape_from_the_review_root() {
        let s = Scratch::new("vnode");
        let root = s.0.join("working tree");
        fs::create_dir(&root).unwrap();
        let path = root.join("文件.txt");
        fs::write(&path, "original").unwrap();
        let file = File::open(&path).unwrap();
        crate::review::verify_preview_handle(&file, &root, &path).unwrap();
        let moved = s.0.join("moved 文件.txt");
        fs::rename(&path, &moved).unwrap();
        fs::write(&path, "different").unwrap();
        assert_eq!(opened_path(&file).unwrap(), moved.canonicalize().unwrap());
        assert!(crate::review::verify_preview_handle(&file, &root, &path).is_err());
    }
}
