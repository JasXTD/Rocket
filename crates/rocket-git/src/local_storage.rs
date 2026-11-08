//! Rocket's per-user folder outside Windows, where it stays %LOCALAPPDATA%\Rocket.
//! This follows the XDG Base Directory specification: state belongs in
//! $XDG_STATE_HOME, by default ~/.local/state, and a relative value is ignored.
use std::{
    ffi::OsString,
    io,
    path::{Path, PathBuf},
};

/// `$XDG_STATE_HOME/rocket`, by default `~/.local/state/rocket`, for logs, settings,
/// crash reports and previews. Missing folders are created for this user only (0700).
/// An unavailable or unsafe location is an error, never a temporary-folder fallback.
pub fn directory() -> io::Result<PathBuf> {
    let directory =
        resolve(std::env::var_os("XDG_STATE_HOME"), std::env::home_dir()).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                "Rocket needs an absolute HOME or XDG_STATE_HOME for private storage.",
            )
        })?;
    create_private(&directory)?;
    Ok(directory)
}

fn resolve(state_home: Option<OsString>, home: Option<PathBuf>) -> Option<PathBuf> {
    state_home
        .map(PathBuf::from)
        .filter(|state| state.is_absolute())
        .or_else(|| {
            home.filter(|home| home.is_absolute())
                .map(|home| home.join(".local").join("state"))
        })
        .map(|state| state.join("rocket"))
}

/// Create private storage, refusing unsafe existing folders without changing them.
/// On Unix, every ancestor must be a real folder owned by this user or root and
/// protected against replacement by other users. The final folder must belong to
/// this user and have no group/other permissions. Missing folders use mode 0700.
pub fn create_private(directory: &Path) -> io::Result<()> {
    #[cfg(unix)]
    return unix::directory(directory, unix::Mode::Private);
    #[cfg(not(unix))]
    std::fs::create_dir_all(directory)
}

/// As above, but fail if the final folder already exists (for new update jobs).
pub fn create_private_new(directory: &Path) -> io::Result<()> {
    #[cfg(unix)]
    return unix::directory(directory, unix::Mode::New);
    #[cfg(not(unix))]
    {
        if let Some(parent) = directory.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::create_dir(directory)
    }
}

/// Check an existing private folder and its ancestors without creating anything.
pub fn check_private(directory: &Path) -> io::Result<()> {
    #[cfg(unix)]
    return unix::directory(directory, unix::Mode::Check);
    #[cfg(not(unix))]
    if std::fs::metadata(directory)?.is_dir() {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::NotADirectory,
            "Storage must be a directory",
        ))
    }
}

/// Nested settings/log/cache folders may inherit privacy from an owned 0700 parent.
/// This preserves earlier profiles' 0755 subfolders inside a private Rocket root.
/// New folders are still 0700; untrusted/writable ancestors are always refused.
pub fn create_protected(directory: &Path) -> io::Result<()> {
    #[cfg(unix)]
    return unix::directory(directory, unix::Mode::Protected);
    #[cfg(not(unix))]
    std::fs::create_dir_all(directory)
}

#[cfg(unix)]
mod unix {
    use super::*;
    use std::{
        fs,
        os::unix::fs::{DirBuilderExt, MetadataExt},
        path::Component,
    };

    pub(super) enum Mode {
        Private,
        New,
        Check,
        Protected,
    }

    pub(super) fn directory(directory: &Path, mode: Mode) -> io::Result<()> {
        if !directory.is_absolute()
            || directory
                .components()
                .any(|c| matches!(c, Component::ParentDir))
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Rocket storage must use an absolute path without parent traversal.",
            ));
        }
        unsafe extern "C" {
            fn geteuid() -> u32;
        }
        // SAFETY: geteuid takes no arguments and cannot fail.
        let user = unsafe { geteuid() };
        let ancestors: Vec<_> = directory.ancestors().collect();
        let mut private_parent = false;
        // Check from the root down, before reading or creating the next component.
        // Other users cannot replace a checked component: its parent is not writable,
        // or is a trusted sticky directory (e.g. /tmp) and the entry is ours/root's.
        // Root and processes with our own UID remain outside this trust boundary.
        for part in ancestors.into_iter().rev() {
            let leaf = part == directory;
            let metadata = match fs::symlink_metadata(part) {
                Ok(_) if leaf && matches!(mode, Mode::New) => {
                    return Err(io::Error::new(
                        io::ErrorKind::AlreadyExists,
                        "Storage already exists",
                    ));
                }
                Ok(metadata) => metadata,
                Err(e) if e.kind() == io::ErrorKind::NotFound && !matches!(mode, Mode::Check) => {
                    match fs::DirBuilder::new().mode(0o700).create(part) {
                        Ok(()) => {}
                        Err(e)
                            if e.kind() == io::ErrorKind::AlreadyExists
                                && !(leaf && matches!(mode, Mode::New)) => {}
                        Err(e) => return Err(e),
                    }
                    fs::symlink_metadata(part)?
                }
                Err(e) => return Err(e),
            };
            let owned = metadata.uid() == user;
            let private = owned && metadata.mode() & 0o077 == 0;
            let trusted = (owned || metadata.uid() == 0)
                && (metadata.mode() & 0o022 == 0 || (!leaf && metadata.mode() & 0o1000 != 0));
            let protected = private || (private_parent && matches!(mode, Mode::Protected));
            if !metadata.is_dir() || !trusted || (leaf && (!owned || !protected)) {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    format!(
                        "Unsafe Rocket storage folder: {}. Use a real folder owned by you with private permissions (0700), under parents other users cannot replace. Existing files and permissions were preserved.",
                        part.display()
                    ),
                ));
            }
            private_parent |= private;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_absolute_xdg_state_home_wins_over_the_home_default() {
        let home = PathBuf::from("/home/ada");
        let default = Some(home.join(".local/state/rocket"));
        assert_eq!(
            resolve(Some("/state".into()), Some(home.clone())),
            Some(PathBuf::from("/state/rocket"))
        );
        for ignored in ["", "relative/state"] {
            assert_eq!(
                resolve(Some(ignored.into()), Some(home.clone())),
                default,
                "{ignored:?}"
            );
        }
        assert_eq!(resolve(None, Some(home)), default);
        assert_eq!(resolve(None, Some("relative".into())), None);
        assert_eq!(resolve(None, None), None);
    }

    #[cfg(unix)]
    #[test]
    fn missing_folders_are_private_and_unsafe_existing_ones_are_preserved() {
        use std::os::unix::fs::PermissionsExt;
        let mode = |path: &Path| std::fs::metadata(path).unwrap().permissions().mode() & 0o777;
        let root =
            std::env::temp_dir().join(format!("rocket-local-storage-{}", std::process::id()));
        let directory = root.join("state").join("rocket");
        create_private(&directory).unwrap();
        for folder in [&root, &root.join("state"), &directory] {
            assert_eq!(mode(folder), 0o700, "{}", folder.display());
        }
        std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o750)).unwrap();
        assert_eq!(
            create_private(&directory).unwrap_err().kind(),
            io::ErrorKind::PermissionDenied
        );
        assert_eq!(mode(&directory), 0o750);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    mod permissions {
        use super::*;
        use std::{
            fs,
            os::unix::fs::{PermissionsExt, symlink},
            time::{SystemTime, UNIX_EPOCH},
        };

        struct Fixture(PathBuf);
        impl Fixture {
            fn new() -> Self {
                let path = std::env::temp_dir().join(format!(
                    "rocket-private-storage-{}-{}",
                    std::process::id(),
                    SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap()
                        .as_nanos()
                ));
                create_private_new(&path).unwrap();
                Self(path)
            }
        }
        impl Drop for Fixture {
            fn drop(&mut self) {
                fs::remove_dir_all(&self.0).unwrap();
            }
        }
        fn chmod(path: &Path, mode: u32) {
            fs::set_permissions(path, fs::Permissions::from_mode(mode)).unwrap();
        }

        #[test]
        fn shared_state_modes_are_refused_without_touching_data_or_permissions() {
            let root = Fixture::new();
            let directory = root.0.join("rocket");
            create_private(&directory).unwrap();
            fs::write(directory.join("draft"), b"private draft").unwrap();
            for mode in [0o750, 0o755, 0o770, 0o777] {
                chmod(&directory, mode);
                assert_eq!(
                    create_private(&directory).unwrap_err().kind(),
                    io::ErrorKind::PermissionDenied
                );
                assert_eq!(
                    check_private(&directory).unwrap_err().kind(),
                    io::ErrorKind::PermissionDenied
                );
                assert_eq!(
                    fs::metadata(&directory).unwrap().permissions().mode() & 0o777,
                    mode
                );
                assert_eq!(fs::read(directory.join("draft")).unwrap(), b"private draft");
            }
        }

        #[test]
        fn writable_ancestors_are_refused_before_creating_children() {
            let root = Fixture::new();
            let parent = root.0.join("shared");
            create_private(&parent).unwrap();
            let private = parent.join("rocket");
            create_private(&private).unwrap();
            for mode in [0o770, 0o777] {
                chmod(&parent, mode);
                assert_eq!(
                    check_private(&private).unwrap_err().kind(),
                    io::ErrorKind::PermissionDenied
                );
                assert_eq!(
                    create_private(&parent.join("new/rocket"))
                        .unwrap_err()
                        .kind(),
                    io::ErrorKind::PermissionDenied
                );
                assert!(!parent.join("new").exists());
            }
            // A trusted sticky parent protects entries owned by this user (as /tmp does).
            chmod(&parent, 0o1777);
            check_private(&private).unwrap();
            create_private_new(&parent.join("new")).unwrap();
        }

        #[test]
        fn links_files_and_missing_storage_fail_without_following_or_creating() {
            let root = Fixture::new();
            let outside = root.0.join("outside");
            create_private(&outside).unwrap();
            let link = root.0.join("link");
            symlink(&outside, &link).unwrap();
            assert!(create_private(&link).is_err());
            assert!(create_private(&link.join("rocket")).is_err());
            assert!(!outside.join("rocket").exists());
            let file = root.0.join("file");
            fs::write(&file, b"preserve").unwrap();
            assert!(create_private(&file.join("rocket")).is_err());
            assert_eq!(fs::read(file).unwrap(), b"preserve");
            let missing = root.0.join("missing");
            assert_eq!(
                check_private(&missing).unwrap_err().kind(),
                io::ErrorKind::NotFound
            );
            assert!(!missing.exists());
            assert_eq!(
                check_private(Path::new("/")).unwrap_err().kind(),
                io::ErrorKind::PermissionDenied
            );
        }

        #[test]
        fn new_jobs_are_exclusive_and_legacy_children_inherit_private_storage() {
            let root = Fixture::new();
            let state = root.0.join("state");
            fs::create_dir(&state).unwrap();
            chmod(&state, 0o755);
            create_protected(&state).unwrap();
            assert_eq!(
                fs::metadata(&state).unwrap().permissions().mode() & 0o777,
                0o755
            );
            let job = state.join("updates/job");
            create_private_new(&job).unwrap();
            assert_eq!(
                create_private_new(&job).unwrap_err().kind(),
                io::ErrorKind::AlreadyExists
            );
            check_private(&job).unwrap();
            assert_eq!(
                fs::metadata(&job).unwrap().permissions().mode() & 0o777,
                0o700
            );
        }

        #[test]
        fn creation_errors_reach_the_caller() {
            let root = Fixture::new();
            chmod(&root.0, 0o500);
            let result = create_private(&root.0.join("blocked"));
            chmod(&root.0, 0o700);
            assert_eq!(result.unwrap_err().kind(), io::ErrorKind::PermissionDenied);
            assert!(!root.0.join("blocked").exists());
        }
    }
}
