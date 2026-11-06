//! Rocket's per-user folder outside Windows, where it stays %LOCALAPPDATA%\Rocket.
//! This follows the XDG Base Directory specification: state belongs in
//! $XDG_STATE_HOME, by default ~/.local/state, and a relative value is ignored.
use std::{
    ffi::OsString,
    path::{Path, PathBuf},
};

/// `$XDG_STATE_HOME/rocket`, by default `~/.local/state/rocket`, for logs, settings,
/// crash reports and previews; None without an absolute home folder. A missing folder
/// is created for this user only (0700), as the specification asks.
pub fn directory() -> Option<PathBuf> {
    let directory = resolve(std::env::var_os("XDG_STATE_HOME"), std::env::home_dir())?;
    create_private(&directory);
    Some(directory)
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

/// Creates the folder and any missing parents with mode 0700; an existing folder keeps
/// its permissions. A failure is reported where Rocket first writes inside it.
fn create_private(directory: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        let _ = std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(directory);
    }
    #[cfg(not(unix))]
    let _ = std::fs::create_dir_all(directory);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_absolute_xdg_state_home_wins_over_the_home_default() {
        let home = PathBuf::from("/home/ada");
        let default = Some(home.join(".local/state/rocket"));
        assert_eq!(resolve(Some("/state".into()), Some(home.clone())), Some(PathBuf::from("/state/rocket")));
        for ignored in ["", "relative/state"] {
            assert_eq!(resolve(Some(ignored.into()), Some(home.clone())), default, "{ignored:?}");
        }
        assert_eq!(resolve(None, Some(home)), default);
        assert_eq!(resolve(None, Some("relative".into())), None);
        assert_eq!(resolve(None, None), None);
    }

    #[cfg(unix)]
    #[test]
    fn missing_folders_are_private_and_existing_ones_keep_their_mode() {
        use std::os::unix::fs::PermissionsExt;
        let mode = |path: &Path| std::fs::metadata(path).unwrap().permissions().mode() & 0o777;
        let root = std::env::temp_dir().join(format!("rocket-local-storage-{}", std::process::id()));
        let directory = root.join("state").join("rocket");
        create_private(&directory);
        for folder in [&root, &root.join("state"), &directory] {
            assert_eq!(mode(folder), 0o700, "{}", folder.display());
        }
        std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o750)).unwrap();
        create_private(&directory);
        assert_eq!(mode(&directory), 0o750);
        std::fs::remove_dir_all(root).unwrap();
    }
}
