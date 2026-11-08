//! Filesystem-only discovery: never execute Apple's installation launcher.
use crate::{Error, Result};
use std::{
    ffi::OsStr,
    fs,
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
};

#[cfg(target_os = "macos")]
const LAUNCHER: &str = "/usr/bin/git";
const FALLBACKS: &[&str] = &[
    "/opt/homebrew/bin/git",
    "/usr/local/bin/git",
    "/Library/Developer/CommandLineTools/usr/bin/git",
    "/Applications/Xcode.app/Contents/Developer/usr/bin/git",
];

#[cfg(target_os = "macos")]
pub(super) fn find(paths: &OsStr, developer: Option<&OsStr>) -> Result<PathBuf> {
    find_in(candidates(paths, developer), Path::new(LAUNCHER))
}

fn candidates(paths: &OsStr, developer: Option<&OsStr>) -> Vec<PathBuf> {
    let mut out: Vec<_> = std::env::split_paths(paths)
        .filter(|p| p.is_absolute())
        .map(|p| p.join("git"))
        .collect();
    // Finder commonly has a minimal PATH. Respect an explicitly selected developer
    // directory without executing xcrun/git stubs or changing xcode-select settings.
    if let Some(directory) = developer.map(Path::new).filter(|p| p.is_absolute()) {
        let directory = if directory.extension() == Some(OsStr::new("app")) {
            directory.join("Contents/Developer")
        } else {
            directory.to_path_buf()
        };
        out.push(directory.join("usr/bin/git"));
    }
    out.extend(FALLBACKS.iter().map(PathBuf::from));
    out
}

fn find_in(candidates: Vec<PathBuf>, launcher: &Path) -> Result<PathBuf> {
    for path in candidates {
        let Ok(path) = fs::canonicalize(path) else {
            continue;
        };
        if check(&path, launcher).is_ok() {
            return Ok(path);
        }
    }
    Err(Error::InvalidInput(
        "No installed Git executable was found. Select Git in Preferences, or install Git / Xcode Command Line Tools yourself. Rocket does not run Apple's installation launcher.".into(),
    ))
}

#[cfg(target_os = "macos")]
pub(crate) fn check_explicit(path: &Path) -> Result<()> {
    check(path, Path::new(LAUNCHER))
}

fn check(path: &Path, launcher: &Path) -> Result<()> {
    let metadata = fs::metadata(path).map_err(|e| Error::io("inspect Git executable", e))?;
    let is_launcher = path == launcher
        || fs::metadata(launcher)
            .is_ok_and(|stub| stub.dev() == metadata.dev() && stub.ino() == metadata.ino());
    if is_launcher {
        return Err(Error::InvalidInput(
            "Select the actual Git executable in Homebrew or an installed developer-tools folder, not /usr/bin/git. Rocket will not launch Apple's installation prompt.".into(),
        ));
    }
    if !metadata.is_file() || metadata.mode() & 0o111 == 0 {
        return Err(Error::InvalidInput("Git must be an executable file".into()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        os::unix::fs::{PermissionsExt, symlink},
        time::{SystemTime, UNIX_EPOCH},
    };

    #[cfg(target_os = "macos")]
    #[test]
    fn explicit_apple_launcher_is_refused_before_the_version_probe() {
        let result = crate::GitClient::new(LAUNCHER, crate::CommandLimits::default());
        assert!(
            matches!(result, Err(Error::InvalidInput(message)) if message.contains("installation prompt"))
        );
        let cancel = crate::CancellationToken::new();
        cancel.cancel();
        assert!(matches!(
            crate::GitClient::discover_with_options(
                Some(Path::new(LAUNCHER)),
                crate::CommandLimits::default(),
                None,
                &cancel
            ),
            Err(Error::Cancelled)
        ));
    }

    #[test]
    fn finder_paths_and_explicit_developer_directory_have_stable_priority() {
        let paths = candidates(
            OsStr::new("relative:/custom/bin::/usr/bin"),
            Some(OsStr::new("/Apps/Xcode Test.app")),
        );
        assert_eq!(
            &paths[..3],
            &[
                PathBuf::from("/custom/bin/git"),
                PathBuf::from("/usr/bin/git"),
                PathBuf::from("/Apps/Xcode Test.app/Contents/Developer/usr/bin/git"),
            ]
        );
        assert!(paths.contains(&PathBuf::from("/opt/homebrew/bin/git")));
        assert!(paths.contains(&PathBuf::from(
            "/Library/Developer/CommandLineTools/usr/bin/git"
        )));
        assert_eq!(
            candidates(OsStr::new(""), Some(OsStr::new("relative"))),
            FALLBACKS.iter().map(PathBuf::from).collect::<Vec<_>>()
        );
    }

    #[test]
    fn discovery_skips_launchers_aliases_and_non_executables_without_running_them() {
        let root = std::env::temp_dir().join(format!(
            "rocket-mac-git-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&root).unwrap();
        let stub = root.join("launcher");
        let plain = root.join("not executable");
        let git = root.join("真实 git");
        for path in [&stub, &plain, &git] {
            fs::write(path, b"not executable code\n").unwrap();
        }
        fs::set_permissions(&stub, fs::Permissions::from_mode(0o755)).unwrap();
        fs::set_permissions(&git, fs::Permissions::from_mode(0o755)).unwrap();
        fs::set_permissions(&plain, fs::Permissions::from_mode(0o644)).unwrap();
        let alias = root.join("alias");
        let hardlink = root.join("hardlink");
        symlink(&stub, &alias).unwrap();
        fs::hard_link(&stub, &hardlink).unwrap();
        assert!(
            find_in(
                vec![stub.clone(), alias.clone(), hardlink.clone(), plain.clone()],
                &stub
            )
            .is_err()
        );
        assert_eq!(
            find_in(
                vec![stub.clone(), alias, hardlink, plain, git.clone()],
                &stub
            )
            .unwrap(),
            git.canonicalize().unwrap()
        );
        assert!(
            check(&stub, &stub)
                .unwrap_err()
                .to_string()
                .contains("installation prompt")
        );
        fs::remove_dir_all(root).unwrap();
    }
}
