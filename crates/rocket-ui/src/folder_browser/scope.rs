//! Scope checks are repeated on the worker immediately before filesystem reads.
use super::linked;
use rocket_git::{Error, Result};
use std::{
    fs::{self, File, OpenOptions},
    io::Read,
    path::{Component, Path, PathBuf},
};

fn refused() -> Error {
    Error::InvalidInput(
        "This test window can only browse normal folders inside its test repositories folder."
            .into(),
    )
}

fn parts(path: &Path, root: &Path) -> Option<(PathBuf, PathBuf)> {
    let root = root.canonicalize().ok()?;
    let plain = PathBuf::from(crate::views::display_path(path));
    let base = PathBuf::from(crate::views::display_path(&root));
    let relative = plain.strip_prefix(&base).ok()?.to_path_buf();
    if relative
        .components()
        .any(|c| !matches!(c, Component::Normal(_)))
    {
        return None;
    }
    Some((root, relative))
}

/// Missing descendants are allowed for New/Clone destination previews, but every
/// existing ancestor must be normal and resolve inside the canonical root.
pub(super) fn inside(path: &Path, root: &Path) -> bool {
    let Some((root, relative)) = parts(path, root) else {
        return false;
    };
    let mut current = root.clone();
    let mut components = relative.components();
    loop {
        match fs::symlink_metadata(&current) {
            Ok(meta) if !linked(&meta) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return true,
            _ => return false,
        }
        if !current.canonicalize().is_ok_and(|p| p.starts_with(&root)) {
            return false;
        }
        let Some(next) = components.next() else {
            return true;
        };
        if !current.is_dir() {
            return false;
        }
        current.push(next);
    }
}

/// Hold existing directories against rename/reparse replacement while listing or
/// reading metadata. Each component is opened without following a reparse point.
pub(crate) fn pin_directory(path: &Path, root: &Path) -> Result<Vec<File>> {
    let (root, relative) = parts(path, root).ok_or_else(refused)?;
    let mut current = root.clone();
    let mut paths = vec![current.clone()];
    for component in relative.components() {
        current.push(component);
        paths.push(current.clone());
    }
    let mut guards = Vec::new();
    for path in paths {
        let meta = fs::symlink_metadata(&path).map_err(|_| refused())?;
        if !meta.is_dir() || linked(&meta) {
            return Err(refused());
        }
        let mut options = OpenOptions::new();
        options.read(true);
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            options.share_mode(3).custom_flags(0x02200000); // no delete sharing; open reparse point + directory
        }
        let file = options.open(&path).map_err(|_| refused())?;
        let meta = file.metadata().map_err(|_| refused())?;
        if !meta.is_dir()
            || linked(&meta)
            || !path.canonicalize().is_ok_and(|p| p.starts_with(&root))
        {
            return Err(refused());
        }
        guards.push(file);
    }
    Ok(guards)
}

pub(crate) fn read_small(path: &Path, root: &Path, limit: u64) -> Option<String> {
    let _guards = pin_directory(path.parent()?, root).ok()?;
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.share_mode(3).custom_flags(0x00200000);
    }
    let file = options.open(path).ok()?;
    let meta = file.metadata().ok()?;
    if !meta.is_file() || linked(&meta) || meta.len() > limit {
        return None;
    }
    let mut text = String::new();
    file.take(limit).read_to_string(&mut text).ok()?;
    Some(text)
}
