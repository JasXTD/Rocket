//! Discovery is read-only. A preset never installs or executes a candidate.
use std::path::PathBuf;

pub(super) fn copilot() -> Option<PathBuf> {
    let mut directories = std::env::var_os("PATH")
        .map(|value| std::env::split_paths(&value).collect::<Vec<_>>())
        .unwrap_or_default();
    if let Some(home) = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }) {
        directories.push(PathBuf::from(home).join(".local/bin"));
    }
    #[cfg(windows)]
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        directories.push(PathBuf::from(local).join("Microsoft/WinGet/Links"));
    }
    find(directories)
}

fn find(directories: impl IntoIterator<Item = PathBuf>) -> Option<PathBuf> {
    let name = if cfg!(windows) {
        "copilot.exe"
    } else {
        "copilot"
    };
    directories
        .into_iter()
        .filter(|p| p.is_absolute())
        .find_map(|dir| {
            let candidate = dir.join(name);
            // Reject Windows network/device paths before filesystem discovery.
            if !rocket_git::valid_custom_program_path(&candidate) {
                return None;
            }
            // Resolve installed links (WinGet/Homebrew) to the file the preview
            // will hash and hold. Never accept a relative PATH entry or .cmd shell.
            let path = candidate.canonicalize().ok()?;
            if !rocket_git::valid_custom_program_path(&path) {
                return None;
            }
            let meta = path.metadata().ok()?;
            if !meta.is_file() {
                return None;
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                if meta.permissions().mode() & 0o111 == 0 {
                    return None;
                }
            }
            Some(path)
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn copilot_discovery_skips_relative_missing_and_non_executable_candidates() {
        let f = crate::test_common::Fixture::new();
        let name = if cfg!(windows) {
            "copilot.exe"
        } else {
            "copilot"
        };
        let invalid = f.root.join("invalid");
        std::fs::create_dir(&invalid).unwrap();
        std::fs::create_dir(invalid.join(name)).unwrap();
        let valid = f.root.join("CLI 資料");
        std::fs::create_dir(&valid).unwrap();
        let executable = valid.join(name);
        std::fs::write(&executable, b"synthetic executable fixture").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert!(find([valid.clone()]).is_none());
            std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        assert!(find([PathBuf::new(), PathBuf::from("relative"), invalid.clone()]).is_none());
        #[cfg(windows)]
        assert!(find([PathBuf::from(r"\\unreachable.invalid\share")]).is_none());
        assert_eq!(
            find([f.root.join("missing"), invalid, valid]),
            Some(executable.canonicalize().unwrap())
        );
    }
    #[cfg(unix)]
    #[test]
    fn copilot_discovery_returns_the_reviewable_native_link_target() {
        use std::os::unix::{fs::PermissionsExt, fs::symlink};
        let f = crate::test_common::Fixture::new();
        let executable = f.root.join("native-copilot");
        std::fs::write(&executable, b"synthetic").unwrap();
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
        symlink(&executable, f.root.join("copilot")).unwrap();
        assert_eq!(
            find([f.root.clone()]),
            Some(executable.canonicalize().unwrap())
        );
    }
}
