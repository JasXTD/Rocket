//! Bounded, cancellable discovery. Never walks Git metadata or directory links.
use crate::{
    folder_browser::scope::pin_directory,
    repository_library::{MAX_REPOSITORIES, same_path},
};
use rocket_git::{CancellationToken, Error, GitClient, Result};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicUsize, Ordering},
};

#[derive(Default)]
pub struct Progress {
    pub folders: AtomicUsize,
    pub found: AtomicUsize,
    pub checked: AtomicUsize,
}
#[derive(Clone, Debug)]
pub struct Found {
    pub path: PathBuf,
    pub kind: &'static str,
}
#[derive(Default)]
pub struct Report {
    pub root: PathBuf,
    pub repositories: Vec<Found>,
    pub skipped: usize,
    pub invalid: usize,
    pub limited: bool,
    pub issues: Vec<Issue>,
}
pub struct Issue {
    pub path: PathBuf,
    pub reason: &'static str,
}
impl Report {
    fn issue(&mut self, path: &Path, reason: &'static str) {
        // Retain useful explanations without keeping every inaccessible path in memory.
        if self.issues.len() < 20 {
            self.issues.push(Issue {
                path: path.to_path_buf(),
                reason,
            });
        }
    }
}
fn linked(m: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        m.file_attributes() & 0x400 != 0
    }
    #[cfg(not(windows))]
    {
        m.file_type().is_symlink()
    }
}
fn normal(path: &Path, directory: bool) -> bool {
    fs::symlink_metadata(path)
        .is_ok_and(|m| !linked(&m) && if directory { m.is_dir() } else { m.is_file() })
}
/// Use this for the picked root too: resolving a junction first would hide it.
pub fn check_root(path: &Path, scope: Option<&Path>) -> Result<PathBuf> {
    if !path.is_absolute() || !normal(path, true) {
        return Err(Error::InvalidInput(
            "Choose an existing, absolute folder, not a folder link.".into(),
        ));
    }
    let root = path.canonicalize().map_err(|source| Error::Io {
        action: "resolve scan folder",
        source,
    })?;
    // With an automation boundary, check the original spelling and all ancestors.
    if let Some(scope) = scope {
        let _pins = pin_directory(path, scope)?;
    }
    Ok(root)
}
/// Git is authoritative. Reject discovery that falls back to a parent repository.
pub fn verify(
    client: &GitClient,
    path: &Path,
    scope: Option<&Path>,
    cancel: &CancellationToken,
) -> Result<Found> {
    cancel.check()?;
    let root = check_root(path, scope)?;
    let _pins = pin_directory(&root, scope.unwrap_or(&root))?;
    let dotgit = root.join(".git");
    let ordinary = normal(&dotgit, true)
        && normal(&dotgit.join("HEAD"), false)
        && (normal(&dotgit.join("objects"), true) || normal(&dotgit.join("commondir"), false));
    let bare = normal(&root.join("HEAD"), false)
        && normal(&root.join("objects"), true)
        && normal(&root.join("refs"), true);
    if !ordinary && !bare && !normal(&dotgit, false) {
        // In particular, don't let a removed .git make Git search a parent checkout.
        return Err(Error::InvalidInput(
            "Repository metadata is missing or invalid. Scan again before adding it.".into(),
        ));
    }
    // Validate Git-file/commondir targets before Git can read them in an MCP window.
    // Linked worktrees may share metadata, but it must stay inside the fixture root.
    let mut metadata_pins = Vec::new();
    if let Some(scope) = scope {
        let read = |path: &Path| {
            crate::folder_browser::scope::read_small(path, scope, 4096).ok_or_else(|| {
                Error::InvalidInput(
                    "Repository metadata is unreadable or outside the scan boundary.".into(),
                )
            })
        };
        let metadata = if normal(&dotgit, false) {
            let text = read(&dotgit)?;
            let target = text
                .trim_end_matches(['\r', '\n'])
                .strip_prefix("gitdir: ")
                .ok_or_else(|| {
                    Error::InvalidInput("Invalid repository Git directory file.".into())
                })?;
            root.join(target)
                .canonicalize()
                .map_err(|source| Error::Io {
                    action: "resolve scan Git directory",
                    source,
                })?
        } else if normal(&dotgit, true) {
            dotgit
        } else {
            root.clone()
        };
        metadata_pins.extend(pin_directory(&metadata, scope)?);
        if metadata.join("commondir").exists() {
            let text = read(&metadata.join("commondir"))?;
            let common = metadata
                .join(text.trim_end_matches(['\r', '\n']))
                .canonicalize()
                .map_err(|source| Error::Io {
                    action: "resolve scan common directory",
                    source,
                })?;
            metadata_pins.extend(pin_directory(&common, scope)?);
        }
    }
    let repo = client.open(&root, cancel)?;
    let info = repo.info();
    let actual = info.worktree.as_ref().unwrap_or(&info.git_dir);
    if !same_path(&root, actual) {
        return Err(Error::InvalidInput(
            "A discovered repository moved or is no longer at this location. Scan again.".into(),
        ));
    }
    if let Some(scope) = scope {
        let _git = pin_directory(&info.git_dir, scope)?;
        let _common = pin_directory(&info.common_dir, scope)?;
    }
    cancel.check()?;
    Ok(Found {
        path: root,
        kind: if info.worktree.is_none() {
            "Bare repository"
        } else if !same_path(&info.git_dir, &info.common_dir) {
            "Linked worktree"
        } else {
            "Repository"
        },
    })
}

pub fn scan(
    client: &GitClient,
    path: &Path,
    recursive: bool,
    scope: Option<&Path>,
    progress: &Progress,
    cancel: &CancellationToken,
) -> Result<Report> {
    scan_limited(
        client, path, recursive, scope, progress, cancel, 50_000, 500_000,
    )
}
#[allow(clippy::too_many_arguments)]
fn scan_limited(
    client: &GitClient,
    path: &Path,
    recursive: bool,
    scope: Option<&Path>,
    progress: &Progress,
    cancel: &CancellationToken,
    max_folders: usize,
    max_entries: usize,
) -> Result<Report> {
    cancel.check()?;
    let root = check_root(path, scope)?;
    let mut report = Report {
        root: root.clone(),
        ..Default::default()
    };
    let mut pending = vec![(root.clone(), 0usize)];
    let mut entries = 0;
    while let Some((folder, depth)) = pending.pop() {
        cancel.check()?;
        if progress.folders.load(Ordering::Relaxed) >= max_folders {
            report.limited = true;
            break;
        }
        let pins = pin_directory(&folder, &root);
        let _pins = match pins {
            Ok(p) => p,
            Err(_) if depth == 0 => {
                return Err(Error::InvalidInput("Cannot read the selected folder. Check its location and permissions, then try again.".into()));
            }
            Err(_) => {
                report.skipped += 1;
                report.issue(&folder, "Folder became unavailable or could not be read");
                continue;
            }
        };
        progress.folders.fetch_add(1, Ordering::Relaxed);
        let dotgit = folder.join(".git");
        let bare = normal(&folder.join("HEAD"), false)
            && normal(&folder.join("objects"), true)
            && normal(&folder.join("refs"), true);
        let candidate = normal(&dotgit, true) || normal(&dotgit, false) || bare;
        let mut verified_bare = false;
        if candidate {
            match verify(client, &folder, scope, cancel) {
                Ok(found) => {
                    verified_bare = found.kind == "Bare repository";
                    if !report
                        .repositories
                        .iter()
                        .any(|r| same_path(&r.path, &found.path))
                    {
                        report.repositories.push(found);
                        progress
                            .found
                            .store(report.repositories.len(), Ordering::Relaxed);
                        if report.repositories.len() >= MAX_REPOSITORIES {
                            report.limited = true;
                            break;
                        }
                    }
                }
                Err(Error::Cancelled) => return Err(Error::Cancelled),
                Err(_) => {
                    report.invalid += 1;
                    report.issue(&folder, "Not a readable Git repository");
                }
            }
        }
        // Even in shallow mode inspect immediate children of the selected parent.
        if verified_bare || (!recursive && depth >= 1) {
            continue;
        }
        if depth >= 64 {
            report.limited = true;
            report.issue(&folder, "Depth limit reached");
            continue;
        }
        let children = match fs::read_dir(&folder) {
            Ok(r) => r,
            Err(source) if depth == 0 => {
                return Err(Error::Io {
                    action: "read scan folder",
                    source,
                });
            }
            Err(_) => {
                report.skipped += 1;
                report.issue(&folder, "Could not read this folder");
                continue;
            }
        };
        for child in children {
            cancel.check()?;
            entries += 1;
            if entries > max_entries {
                report.limited = true;
                break;
            }
            let child = match child {
                Ok(c) => c,
                Err(_) => {
                    report.skipped += 1;
                    report.issue(&folder, "Could not read a directory entry");
                    continue;
                }
            };
            if child.file_name().eq_ignore_ascii_case(".git") {
                continue;
            }
            let meta = match child.metadata() {
                Ok(m) => m,
                Err(_) => {
                    report.skipped += 1;
                    report.issue(
                        &child.path(),
                        "Entry became unavailable or could not be read",
                    );
                    continue;
                }
            };
            if linked(&meta) {
                report.skipped += 1;
                report.issue(&child.path(), "Link skipped");
                continue;
            }
            if meta.is_dir() {
                if pending.len() + progress.folders.load(Ordering::Relaxed) >= max_folders {
                    report.limited = true;
                    continue;
                }
                pending.push((child.path(), depth + 1));
            }
        }
        if entries > max_entries {
            break;
        }
    }
    cancel.check()?;
    report.repositories.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_common::Fixture;

    #[test]
    fn skipped_repository_details_are_bounded_and_do_not_follow_git_files_outside_scope() {
        let f = Fixture::new();
        let root = f.root.canonicalize().unwrap();
        let selected = root.join("selected");
        fs::create_dir(&selected).unwrap();
        for i in 0..24 {
            fs::create_dir_all(selected.join(format!("invalid-{i}/.git"))).unwrap();
        }
        let escaped = selected.join("outside-metadata");
        fs::create_dir(&escaped).unwrap();
        fs::write(
            escaped.join(".git"),
            format!("gitdir: {}\n", f.repository.join(".git").display()),
        )
        .unwrap();
        let report = scan(
            &f.git,
            &selected,
            true,
            Some(&selected),
            &Progress::default(),
            &CancellationToken::new(),
        )
        .unwrap();
        assert!(report.repositories.is_empty());
        assert_eq!(report.invalid, 25);
        assert_eq!(report.issues.len(), 20);
        assert!(
            report
                .issues
                .iter()
                .all(|i| i.path.starts_with(&selected)
                    && i.reason == "Not a readable Git repository")
        );
        assert!(!report.limited);
    }

    #[cfg(windows)]
    #[test]
    fn unavailable_selected_folder_is_an_error_not_an_empty_success() {
        use std::os::windows::fs::OpenOptionsExt;
        let f = Fixture::new();
        let root = f.root.canonicalize().unwrap().join("locked");
        fs::create_dir(&root).unwrap();
        let lock = fs::OpenOptions::new()
            .read(true)
            .share_mode(0)
            .custom_flags(0x02000000)
            .open(&root)
            .unwrap();
        assert!(
            scan(
                &f.git,
                &root,
                true,
                None,
                &Progress::default(),
                &CancellationToken::new()
            )
            .is_err()
        );
        drop(lock);
        let report = scan(
            &f.git,
            &root,
            true,
            None,
            &Progress::default(),
            &CancellationToken::new(),
        )
        .unwrap();
        assert!(report.repositories.is_empty());
        assert_eq!(report.skipped, 0);
    }

    #[test]
    fn finds_unicode_nested_bare_and_linked_worktrees_without_git_writes() {
        let f = Fixture::new();
        f.write("keep.txt", "committed");
        f.commit("fixture");
        let linked = f.root.join("linked 日本語 🚀");
        f.run(&["worktree", "add", "-b", "linked", linked.to_str().unwrap()]);
        let nested = f.root.join("nested/deep repo");
        fs::create_dir_all(&nested).unwrap();
        f.run_at(&nested, &["init", "--initial-branch=main"]);
        let bare = f.root.join("bare.git");
        fs::create_dir(&bare).unwrap();
        f.run_at(&bare, &["init", "--bare"]);
        let invalid = f.root.join("invalid/.git");
        fs::create_dir_all(invalid).unwrap();
        // A repository hidden inside Git metadata must never appear.
        let hidden = f.repository.join(".git/hidden");
        fs::create_dir_all(&hidden).unwrap();
        f.run_at(&hidden, &["init"]);
        f.write("keep.txt", "staged");
        f.run(&["add", "keep.txt"]);
        f.write("keep.txt", "unstaged");
        f.write("untracked.txt", "keep");
        let index = fs::read(f.repository.join(".git/index")).unwrap();
        let head = f.run(&["rev-parse", "HEAD"]);
        let progress = Progress::default();
        let root = f.root.canonicalize().unwrap();
        let report = scan(
            &f.git,
            &root,
            true,
            Some(&root),
            &progress,
            &CancellationToken::new(),
        )
        .unwrap();
        assert_eq!(report.repositories.len(), 4);
        assert_eq!(report.invalid, 1);
        assert!(!report.limited);
        assert!(
            report
                .repositories
                .iter()
                .any(|r| r.kind == "Linked worktree"
                    && same_path(&r.path, &linked.canonicalize().unwrap()))
        );
        assert!(
            report
                .repositories
                .iter()
                .any(|r| r.kind == "Bare repository")
        );
        let shallow = scan(
            &f.git,
            &root,
            false,
            Some(&root),
            &Progress::default(),
            &CancellationToken::new(),
        )
        .unwrap();
        assert_eq!(shallow.repositories.len(), 3);
        assert_eq!(index, fs::read(f.repository.join(".git/index")).unwrap());
        assert_eq!(head, f.run(&["rev-parse", "HEAD"]));
        assert_eq!(
            fs::read(f.repository.join("keep.txt")).unwrap(),
            b"unstaged"
        );
        assert_eq!(
            fs::read(f.repository.join("untracked.txt")).unwrap(),
            b"keep"
        );
    }

    #[test]
    fn rejects_missing_scope_escape_and_parent_fallback_and_bounds_discovery() {
        let f = Fixture::new();
        let cancel = CancellationToken::new();
        assert!(
            scan(
                &f.git,
                &f.root.join("missing"),
                true,
                None,
                &Progress::default(),
                &cancel
            )
            .is_err()
        );
        assert!(
            scan(
                &f.git,
                &f.root,
                true,
                Some(&f.repository),
                &Progress::default(),
                &cancel
            )
            .is_err()
        );
        let sub = f.repository.join("ordinary");
        fs::create_dir(&sub).unwrap();
        assert!(verify(&f.git, &sub, None, &cancel).is_err());
        let report = scan_limited(
            &f.git,
            &f.root,
            true,
            None,
            &Progress::default(),
            &cancel,
            1,
            500,
        )
        .unwrap();
        assert!(report.limited);
        let report = scan_limited(
            &f.git,
            &f.root,
            true,
            None,
            &Progress::default(),
            &cancel,
            100,
            1,
        )
        .unwrap();
        assert!(report.limited);
        cancel.cancel();
        assert!(matches!(
            scan(&f.git, &f.root, true, None, &Progress::default(), &cancel),
            Err(Error::Cancelled)
        ));
    }
}
