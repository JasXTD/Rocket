//! Filesystem primitives for repository creation and transactional config writes.
use crate::{Error, Result};
#[cfg(windows)]
use std::fs::OpenOptions;
use std::{
    fs::{self, File},
    io::Read,
    path::Path,
};

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

#[cfg(not(windows))]
pub(crate) fn move_path(source: &Path, destination: &Path, replace: bool) -> Result<()> {
    if !replace {
        return Err(Error::Unsupported(
            "Non-replacing repository publication currently requires Windows".into(),
        ));
    }
    fs::rename(source, destination).map_err(|e| Error::io("publish repository metadata", e))
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
