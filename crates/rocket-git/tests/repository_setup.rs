#![cfg(any(windows, target_os = "linux"))]
mod common;
use common::Fixture;
use rocket_git::*;
use std::{fs, io::Write};
fn token() -> CancellationToken {
    CancellationToken::new()
}
fn isolated(name: &str, f: fn()) {
    common::isolated("ROCKET_SETUP_CASE", name, f);
}
fn identity(name: &str, email: &str) -> LocalIdentity {
    LocalIdentity {
        name: Some(name.into()),
        email: Some(email.into()),
    }
}

#[test]
fn init_new_empty_and_populated_folders_preserves_bytes_and_creates_only_metadata() {
    isolated(
        "init_new_empty_and_populated_folders_preserves_bytes_and_creates_only_metadata",
        || {
            let f = Fixture::new();
            let root = f.root.canonicalize().unwrap();
            for (leaf, existing) in [("new 文 folder", false), ("empty", true), ("project", true)]
            {
                let path = root.join(leaf);
                if existing {
                    fs::create_dir(&path).unwrap();
                }
                if leaf == "project" {
                    fs::create_dir(path.join("src")).unwrap();
                    fs::write(path.join("src/data.bin"), [0, 255, 42, 10]).unwrap();
                    fs::write(path.join(".gitignore"), "ignored\n").unwrap();
                    fs::write(path.join("ignored"), "keep ignored").unwrap();
                    fs::write(path.join("README.md"), "do not replace").unwrap();
                }
                let report = f
                    .git
                    .init_repository(&path, "custom/start", &token())
                    .unwrap();
                assert!(report.preparation.is_ok(), "{:?}", report.preparation);
                assert!(
                    matches!(report.publication, Some(Ok(()))),
                    "{:?}",
                    report.publication
                );
                assert!(matches!(report.cleanup, Some(Ok(()))));
                let repo = report.repository.unwrap().unwrap();
                assert!(
                    matches!(repo.head(&token()).unwrap(), Head::Unborn{branch} if branch==b"custom/start")
                );
                assert!(!path.join(".git/index").exists());
                assert!(!path.join(".git/hooks").exists());
                assert_eq!(
                    repo.identity(&token()).unwrap().local,
                    LocalIdentity::default()
                );
                if leaf == "project" {
                    assert_eq!(
                        fs::read(path.join("src/data.bin")).unwrap(),
                        [0, 255, 42, 10]
                    );
                    assert_eq!(
                        fs::read_to_string(path.join("README.md")).unwrap(),
                        "do not replace"
                    );
                    assert_eq!(
                        fs::read_to_string(path.join("ignored")).unwrap(),
                        "keep ignored"
                    );
                    assert_eq!(repo.status(&token()).unwrap().entries.len(), 3);
                }
            }
        },
    );
}

#[test]
fn init_refuses_existing_metadata_nested_repositories_files_and_invalid_inputs() {
    isolated(
        "init_refuses_existing_metadata_nested_repositories_files_and_invalid_inputs",
        || {
            let f = Fixture::new();
            let root = f.root.canonicalize().unwrap();
            let repository = f.repository.canonicalize().unwrap();
            let original = fs::read(f.repository.join(".git/config")).unwrap();
            assert!(
                f.git
                    .init_repository(&repository, "main", &token())
                    .is_err()
            );
            assert!(
                f.git
                    .init_repository(repository.join("nested"), "main", &token())
                    .is_err()
            );
            let invalid = root.join("invalid");
            fs::create_dir(&invalid).unwrap();
            fs::write(invalid.join(".git"), "keep invalid marker").unwrap();
            assert!(f.git.init_repository(&invalid, "main", &token()).is_err());
            assert_eq!(
                fs::read_to_string(invalid.join(".git")).unwrap(),
                "keep invalid marker"
            );
            let file = root.join("file");
            fs::write(&file, "keep file").unwrap();
            assert!(f.git.init_repository(&file, "main", &token()).is_err());
            for branch in ["", "--bad", "../bad", "main:bad", "bad\nname"] {
                assert!(
                    f.git
                        .init_repository(root.join("untouched"), branch, &token())
                        .is_err()
                );
            }
            for path in [
                root.join("NUL"),
                root.join("project."),
                root.join("missing/child"),
            ] {
                assert!(f.git.init_repository(path, "main", &token()).is_err());
            }
            assert!(!f.root.join("untouched").exists());
            assert_eq!(
                fs::read(f.repository.join(".git/config")).unwrap(),
                original
            );
            let cancel = token();
            cancel.cancel();
            assert!(matches!(
                f.git
                    .init_repository(root.join("cancelled"), "main", &cancel),
                Err(Error::Cancelled)
            ));
            assert!(!f.root.join("cancelled").exists());
        },
    );
}

#[test]
fn identity_save_is_local_preserves_state_and_controls_new_commit_identity() {
    isolated(
        "identity_save_is_local_preserves_state_and_controls_new_commit_identity",
        || {
            let f = Fixture::new();
            f.write("file", "base");
            f.commit("base");
            f.write("file", "staged");
            f.run(&["add", "file"]);
            f.write("file", "working");
            f.write("untracked", "keep");
            let repo = f.repo();
            let before = repo.identity(&token()).unwrap();
            let index = fs::read(f.repository.join(".git/index")).unwrap();
            let head = repo.head(&token()).unwrap();
            let global = std::env::var_os("GIT_CONFIG_GLOBAL").unwrap();
            let original_global = fs::read(&global).unwrap();
            let report = repo
                .set_identity(
                    &before,
                    &identity("PRIVATE Jane 文", "PRIVATE+jane@example.invalid"),
                    &token(),
                )
                .unwrap();
            assert!(report.result.is_ok(), "{:?}", report.result);
            assert_eq!(
                report.identity.unwrap().effective,
                identity("PRIVATE Jane 文", "PRIVATE+jane@example.invalid")
            );
            assert_eq!(fs::read(&global).unwrap(), original_global);
            assert_eq!(fs::read(f.repository.join(".git/index")).unwrap(), index);
            assert_eq!(repo.head(&token()).unwrap(), head);
            assert_eq!(
                fs::read_to_string(f.repository.join("file")).unwrap(),
                "working"
            );
            assert_eq!(
                fs::read_to_string(f.repository.join("untracked")).unwrap(),
                "keep"
            );
            assert_eq!(f.run(&["config", "core.autocrlf"]), b"false\n");
            assert_eq!(f.run(&["config", "commit.gpgsign"]), b"false\n");
            assert!(
                repo.commit("local identity proof", &token())
                    .unwrap()
                    .result
                    .is_ok()
            );
            assert_eq!(
                f.run(&["log", "-1", "--format=%an <%ae>"]),
                "PRIVATE Jane 文 <PRIVATE+jane@example.invalid>\n".as_bytes()
            );
        },
    );
}

#[test]
fn identity_clear_inherits_without_touching_global_or_include_files() {
    isolated(
        "identity_clear_inherits_without_touching_global_or_include_files",
        || {
            let f = Fixture::new();
            let repo = f.repo();
            let global = std::env::var_os("GIT_CONFIG_GLOBAL").unwrap();
            fs::write(
                &global,
                "[user]\n name = Inherited Name\n email = inherited@example.invalid\n",
            )
            .unwrap();
            let before_global = fs::read(&global).unwrap();
            let before = repo.identity(&token()).unwrap();
            let report = repo
                .set_identity(&before, &LocalIdentity::default(), &token())
                .unwrap();
            assert!(report.result.is_ok());
            let cleared = report.identity.unwrap();
            assert_eq!(cleared.local, LocalIdentity::default());
            assert_eq!(
                cleared.effective,
                identity("Inherited Name", "inherited@example.invalid")
            );
            assert_eq!(fs::read(&global).unwrap(), before_global);
            let included = f.root.join("included.config");
            fs::write(
                &included,
                "[user]\n name = Included Name\n email = included@example.invalid\n",
            )
            .unwrap();
            f.run(&["config", "include.path", included.to_str().unwrap()]);
            let report = repo
                .set_identity(
                    &repo.identity(&token()).unwrap(),
                    &identity("Local", "local@example.invalid"),
                    &token(),
                )
                .unwrap();
            assert!(report.result.is_ok());
            assert_eq!(
                fs::read_to_string(included).unwrap(),
                "[user]\n name = Included Name\n email = included@example.invalid\n"
            );
            assert_eq!(fs::read(&global).unwrap(), before_global);
        },
    );
}

#[test]
fn identity_stale_invalid_locked_and_cancelled_writes_preserve_config() {
    isolated(
        "identity_stale_invalid_locked_and_cancelled_writes_preserve_config",
        || {
            let f = Fixture::new();
            let repo = f.repo();
            let config = f.repository.join(".git/config");
            let before = repo.identity(&token()).unwrap();
            f.run(&["config", "custom.setting", "external edit"]);
            let original = fs::read(&config).unwrap();
            let stale = repo
                .set_identity(&before, &identity("New", "new@example.invalid"), &token())
                .unwrap();
            assert!(stale.result.is_err());
            assert_eq!(fs::read(&config).unwrap(), original);
            let before = repo.identity(&token()).unwrap();
            for bad in [
                identity("bad\nname", "valid@example.invalid"),
                identity("Name", "invalid"),
                identity("Name", " space@example.invalid"),
                identity("<bad>", "valid@example.invalid"),
            ] {
                assert!(repo.set_identity(&before, &bad, &token()).is_err());
                assert_eq!(fs::read(&config).unwrap(), original);
            }
            let lock = f.repository.join(".git/config.lock");
            fs::write(&lock, "owned by someone else").unwrap();
            let report = repo
                .set_identity(&before, &identity("New", "new@example.invalid"), &token())
                .unwrap();
            assert!(report.result.is_err());
            assert_eq!(fs::read_to_string(&lock).unwrap(), "owned by someone else");
            fs::remove_file(&lock).unwrap();
            let nested_lock = f.repository.join(".git/config.lock.lock");
            fs::write(&nested_lock, "another lock").unwrap();
            let report = repo
                .set_identity(&before, &identity("New", "new@example.invalid"), &token())
                .unwrap();
            assert!(report.result.is_err());
            assert_eq!(fs::read(&config).unwrap(), original);
            assert_eq!(fs::read_to_string(&nested_lock).unwrap(), "another lock");
            assert!(!lock.exists());
            fs::remove_file(nested_lock).unwrap();
            let cancel = token();
            cancel.cancel();
            assert!(matches!(
                repo.set_identity(&before, &identity("New", "new@example.invalid"), &cancel),
                Err(Error::Cancelled)
            ));
            assert_eq!(fs::read(&config).unwrap(), original);
            fs::OpenOptions::new()
                .append(true)
                .open(&config)
                .unwrap()
                .write_all(b"[invalid\n")
                .unwrap();
            assert!(repo.identity(&token()).is_err());
        },
    );
}

#[test]
fn identity_is_shared_across_linked_worktrees_and_reports_worktree_overrides() {
    isolated(
        "identity_is_shared_across_linked_worktrees_and_reports_worktree_overrides",
        || {
            let f = Fixture::new();
            f.write("file", "base");
            f.commit("base");
            let linked = f.root.join("linked");
            f.run(&["worktree", "add", "-b", "linked", linked.to_str().unwrap()]);
            f.run(&["config", "extensions.worktreeConfig", "true"]);
            let status = f
                .command(
                    &linked,
                    &["config", "--worktree", "user.name", "Worktree override"],
                )
                .output()
                .unwrap();
            assert!(status.status.success());
            let repo = f.git.open(&linked, &token()).unwrap();
            let before = repo.identity(&token()).unwrap();
            let report = repo
                .set_identity(
                    &before,
                    &identity("Shared name", "shared@example.invalid"),
                    &token(),
                )
                .unwrap();
            assert!(report.result.is_ok());
            let after = report.identity.unwrap();
            assert_eq!(after.local.name.as_deref(), Some("Shared name"));
            assert_eq!(after.effective.name.as_deref(), Some("Worktree override"));
            assert_eq!(
                f.repo().identity(&token()).unwrap().local,
                identity("Shared name", "shared@example.invalid")
            );
        },
    );
}

#[test]
fn cancellation_between_staged_identity_changes_never_publishes_half_an_identity() {
    isolated(
        "cancellation_between_staged_identity_changes_never_publishes_half_an_identity",
        || {
            use std::{
                sync::{
                    Arc,
                    atomic::{AtomicBool, Ordering},
                },
                thread,
                time::{Duration, Instant},
            };
            let f = Fixture::new();
            let repo = f.repo();
            let before = repo.identity(&token()).unwrap();
            let config = f.repository.join(".git/config");
            let original = fs::read(&config).unwrap();
            let lock = config.with_extension("lock");
            let cancel = token();
            let watcher_cancel = cancel.clone();
            let done = Arc::new(AtomicBool::new(false));
            let watcher_done = done.clone();
            let observer = thread::spawn(move || {
                let deadline = Instant::now() + Duration::from_secs(40);
                while !watcher_done.load(Ordering::Acquire) && Instant::now() < deadline {
                    if fs::read_to_string(&lock)
                        .is_ok_and(|s| s.contains("PRIVATE staged identity"))
                    {
                        watcher_cancel.cancel();
                        return true;
                    }
                    thread::sleep(Duration::from_millis(1));
                }
                false
            });
            let report = repo
                .set_identity(
                    &before,
                    &identity("PRIVATE staged identity", "changed@example.invalid"),
                    &cancel,
                )
                .unwrap();
            done.store(true, Ordering::Release);
            assert!(
                observer.join().unwrap(),
                "did not observe the first prepared key"
            );
            assert!(matches!(report.result, Err(Error::Cancelled)));
            assert_eq!(fs::read(&config).unwrap(), original);
            assert_eq!(report.identity.unwrap(), before);
            assert!(!config.with_extension("lock").exists());
        },
    );
}

#[test]
fn init_cancellation_keeps_existing_files_and_retains_unpublished_workspace() {
    isolated(
        "init_cancellation_keeps_existing_files_and_retains_unpublished_workspace",
        || {
            use std::{
                thread,
                time::{Duration, Instant},
            };
            let f = Fixture::new();
            let root = f.root.canonicalize().unwrap();
            let project = root.join("cancel-project");
            fs::create_dir(&project).unwrap();
            fs::write(project.join("file"), "keep").unwrap();
            let cancel = token();
            let observer_cancel = cancel.clone();
            let observe = root.clone();
            let observer = thread::spawn(move || {
                let deadline = Instant::now() + Duration::from_secs(30);
                while Instant::now() < deadline {
                    if fs::read_dir(&observe)
                        .unwrap()
                        .filter_map(|e| e.ok())
                        .any(|e| e.file_name().to_string_lossy().starts_with(".rocket-init-"))
                    {
                        observer_cancel.cancel();
                        return true;
                    }
                    thread::sleep(Duration::from_millis(1));
                }
                false
            });
            let report = f.git.init_repository(&project, "main", &cancel).unwrap();
            assert!(observer.join().unwrap());
            assert!(report.preparation.is_err());
            assert!(report.publication.is_none());
            assert!(report.staging_directory.exists());
            assert!(!project.join(".git").exists());
            assert_eq!(fs::read_to_string(project.join("file")).unwrap(), "keep");
        },
    );
}

#[test]
fn init_publication_refuses_destinations_or_git_metadata_created_concurrently() {
    isolated(
        "init_publication_refuses_destinations_or_git_metadata_created_concurrently",
        || {
            use std::{
                thread,
                time::{Duration, Instant},
            };
            let f = Fixture::new();
            let root = f.root.canonicalize().unwrap();
            for existing in [false, true] {
                let parent = root.join(if existing {
                    "existing-case"
                } else {
                    "new-case"
                });
                fs::create_dir(&parent).unwrap();
                let project = parent.join("project");
                if existing {
                    fs::create_dir(&project).unwrap();
                    fs::write(project.join("file"), "keep").unwrap();
                }
                let observed = parent.clone();
                let target = project.clone();
                let observer = thread::spawn(move || {
                    let deadline = Instant::now() + Duration::from_secs(30);
                    while Instant::now() < deadline {
                        if fs::read_dir(&observed)
                            .unwrap()
                            .filter_map(|e| e.ok())
                            .any(|e| e.file_name().to_string_lossy().starts_with(".rocket-init-"))
                        {
                            if existing {
                                fs::write(target.join(".git"), "created by someone else").unwrap();
                            } else {
                                fs::create_dir(&target).unwrap();
                                fs::write(target.join("someone-elses-file"), "preserve").unwrap();
                            }
                            return true;
                        }
                        thread::sleep(Duration::from_millis(1));
                    }
                    false
                });
                let report = f.git.init_repository(&project, "main", &token()).unwrap();
                assert!(observer.join().unwrap());
                assert!(report.preparation.is_ok());
                assert!(matches!(report.publication, Some(Err(_))));
                assert!(report.staging_directory.exists());
                if existing {
                    assert_eq!(
                        fs::read_to_string(project.join(".git")).unwrap(),
                        "created by someone else"
                    );
                    assert_eq!(fs::read_to_string(project.join("file")).unwrap(), "keep");
                } else {
                    assert_eq!(
                        fs::read_to_string(project.join("someone-elses-file")).unwrap(),
                        "preserve"
                    );
                    assert!(!project.join(".git").exists());
                }
            }
        },
    );
}

#[cfg(windows)]
#[test]
fn init_rejects_junction_destination_and_parent_without_touching_target() {
    isolated(
        "init_rejects_junction_destination_and_parent_without_touching_target",
        || {
            use std::{os::windows::process::CommandExt, process::Command};
            let f = Fixture::new();
            let root = f.root.canonicalize().unwrap();
            let target = root.join("junction-target");
            fs::create_dir(&target).unwrap();
            fs::write(target.join("file"), "preserve target").unwrap();
            let link = root.join("junction-link");
            let output = Command::new("cmd.exe")
                .args(["/d", "/c", "mklink", "/J"])
                .arg(&link)
                .arg(&target)
                .creation_flags(0x08000000)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            assert!(f.git.init_repository(&link, "main", &token()).is_err());
            assert!(
                f.git
                    .init_repository(link.join("child"), "main", &token())
                    .is_err()
            );
            assert!(!target.join(".git").exists());
            assert!(!target.join("child").exists());
            assert_eq!(
                fs::read_to_string(target.join("file")).unwrap(),
                "preserve target"
            );
            fs::remove_dir(&link).unwrap(); // Remove only the fixture junction itself, never recurse.
            assert!(target.is_dir());
        },
    );
}

/// Linux: a symbolic link is refused as a junction is, as the folder and as its parent.
#[cfg(target_os = "linux")]
#[test]
fn init_rejects_linked_destination_and_parent_without_touching_target() {
    isolated(
        "init_rejects_linked_destination_and_parent_without_touching_target",
        || {
            let f = Fixture::new();
            let root = f.root.canonicalize().unwrap();
            let target = root.join("link-target");
            fs::create_dir(&target).unwrap();
            fs::write(target.join("file"), "preserve target").unwrap();
            let link = root.join("link");
            std::os::unix::fs::symlink(&target, &link).unwrap();
            assert!(f.git.init_repository(&link, "main", &token()).is_err());
            assert!(
                f.git
                    .init_repository(link.join("child"), "main", &token())
                    .is_err()
            );
            assert!(!target.join(".git").exists());
            assert!(!target.join("child").exists());
            assert_eq!(
                fs::read_to_string(target.join("file")).unwrap(),
                "preserve target"
            );
            assert!(fs::symlink_metadata(&link).unwrap().file_type().is_symlink());
        },
    );
}
