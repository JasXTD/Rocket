mod common;
use common::{Fixture, isolated};
use rocket_git::*;
use std::fs;
#[test]
fn set_url_stages_only_metadata_keeps_edits_and_rejects_stale_review() {
    isolated("ROCKET_SUBMODULE_ACTION_CASE",
        "set_url_stages_only_metadata_keeps_edits_and_rejects_stale_review", || {
        let f = parent(); let s = source(); let replacement = source();
        apply(&f, &add(&f, &s));
        f.run(&["commit", "-m", "add"]);
        f.write("parent.txt", "staged parent\n"); f.run(&["add", "parent.txt"]);
        f.write("parent.txt", "unstaged parent\n");
        f.write(&format!("{NAME}/file 文.txt"), "dirty child\n");
        let staged = f.run(&["show", ":parent.txt"]);
        let child = f.repository.join(NAME);
        let head = f.run(&["rev-parse", "HEAD"]);
        let child_head = f.run_at(&child, &["rev-parse", "HEAD"]);
        let a = SubmoduleAction::SetUrl { path: path(NAME), source: replacement.repository.to_str().unwrap().into() };
        let r = f.repo(); let c = CancellationToken::new();
        let p = r.preview_submodule_action(&a, &c).unwrap();
        let before = snapshot(&f);
        let stopped = CancellationToken::new(); stopped.cancel();
        assert!(matches!(r.apply_submodule_action(&p, &stopped), Err(Error::Cancelled)));
        assert_eq!(snapshot(&f), before);
        f.run(&["config", "rocket.external", "keep"]);
        let external = snapshot(&f);
        assert!(matches!(r.apply_submodule_action(&p, &c), Err(Error::Submodule(SubmoduleFailure::StalePreview))));
        assert_eq!(snapshot(&f), external);
        let report = apply(&f, &a);
        assert!(report.recovery_directory.unwrap().join("gitmodules.before").is_file());
        assert_eq!(f.run(&["rev-parse", "HEAD"]), head);
        assert_eq!(f.run_at(&child, &["rev-parse", "HEAD"]), child_head);
        assert_eq!(f.run(&["show", ":parent.txt"]), staged);
        assert_eq!(fs::read_to_string(child.join("file 文.txt")).unwrap(), "dirty child\n");
        assert_eq!(fs::read_to_string(f.repository.join("parent.txt")).unwrap(), "unstaged parent\n");
        assert_eq!(String::from_utf8(f.run_at(&child, &["config", "remote.origin.url"])).unwrap().trim(), replacement.repository.to_str().unwrap());
        assert!(String::from_utf8(f.run(&["show", ":.gitmodules"])).unwrap().contains("url"));
        // Do not overwrite an existing, separately edited .gitmodules.
        f.write(".gitmodules", "# external draft\n");
        let edited = snapshot(&f);
        assert!(r.preview_submodule_action(&a, &c).is_err());
        assert_eq!(snapshot(&f), edited);
    });
}

#[test]
fn parent_push_blocks_unpublished_submodule_without_pushing_child() {
    isolated("ROCKET_SUBMODULE_ACTION_CASE",
        "parent_push_blocks_unpublished_submodule_without_pushing_child", || {
        let f = parent(); let s = source(); apply(&f, &add(&f, &s));
        f.run(&["commit", "-m", "add child"]);
        let bare = f.root.join("parent-remote.git");
        f.run(&["init", "--bare", bare.to_str().unwrap()]);
        f.run(&["remote", "add", "origin", bare.to_str().unwrap()]);
        let c = CancellationToken::new();
        let ok = f.repo().push("origin", "main", "main", PushOptions::default(), &c).unwrap();
        assert!(ok.result.is_ok(), "{:?}", ok.result);
        let remote_before = f.run_at(&bare, &["rev-parse", "refs/heads/main"]);
        let child = f.repository.join(NAME);
        let source_before = s.run(&["rev-parse", "HEAD"]);
        f.run_at(&child, &["config", "user.name", "Fixture"]);
        f.run_at(&child, &["config", "user.email", "fixture@example.invalid"]);
        f.write(&format!("{NAME}/file 文.txt"), "unpublished child\n");
        f.run_at(&child, &["add", "."]);
        f.run_at(&child, &["-c", "commit.gpgsign=false", "commit", "-m", "unpublished"]);
        f.run(&["add", NAME]); f.run(&["commit", "-m", "reference unpublished child"]);
        let before = snapshot(&f);
        let rejected = f.repo().push("origin", "main", "main", PushOptions::default(), &c).unwrap();
        assert_eq!(rejected.result.as_ref().unwrap_err().push_failure(), Some(PushFailure::UnpublishedSubmodule));
        assert_eq!(f.run_at(&bare, &["rev-parse", "refs/heads/main"]), remote_before);
        assert_eq!(s.run(&["rev-parse", "HEAD"]), source_before);
        assert_eq!(snapshot(&f), before);
        let repo = f.repo();
        let rewritten = String::from_utf8(f.run(&["commit-tree", "HEAD^{tree}", "-p", "HEAD~2", "-m", "rewritten child reference"])).unwrap();
        f.run(&["branch", "rewritten", rewritten.trim()]);
        let lease = repo.preview_force_push(b"origin", b"rewritten", b"main", &c).unwrap();
        let rejected = repo.apply_force_push(&lease, &c).unwrap();
        assert_eq!(rejected.result.as_ref().unwrap_err().push_failure(), Some(PushFailure::UnpublishedSubmodule));
        let atomic = repo.preview_atomic_push(b"origin", &[AtomicPushSelection {
            local_branch: b"main".to_vec(), remote_branch: b"main".to_vec(),
        }], &c).unwrap();
        let rejected = repo.apply_atomic_push(&atomic, &c).unwrap();
        assert_eq!(rejected.result.as_ref().unwrap_err().push_failure(), Some(PushFailure::UnpublishedSubmodule));
        assert_eq!(f.run_at(&bare, &["rev-parse", "refs/heads/main"]), remote_before);
        assert_eq!(s.run(&["rev-parse", "HEAD"]), source_before);
        assert_eq!(snapshot(&f), before);
        f.run_at(&child, &["push", "origin", "HEAD:refs/heads/published"]);
        let ok = f.repo().push("origin", "main", "main", PushOptions::default(), &c).unwrap();
        assert!(ok.result.is_ok(), "{:?}", ok.result);
    });
}
const NAME: &str = "vendor/描画 🚀 é";
fn path(s: &str) -> RepoPath {
    RepoPath::new(s.as_bytes().to_vec()).unwrap()
}
fn source() -> Fixture {
    let f = Fixture::new();
    f.write("file 文.txt", "child\n");
    f.commit("child");
    f
}
fn parent() -> Fixture {
    let f = Fixture::new();
    f.write("parent.txt", "base\n");
    f.commit("parent");
    f
}
fn add(f: &Fixture, src: &Fixture) -> SubmoduleAction {
    SubmoduleAction::Add {
        parent: f.repo().info().worktree.clone().unwrap(),
        path: path(NAME),
        source: src.repository.to_str().unwrap().into(),
    }
}
fn apply(f: &Fixture, a: &SubmoduleAction) -> SubmoduleActionReport {
    let r = f.repo();
    let c = CancellationToken::new();
    let p = r.preview_submodule_action(a, &c).unwrap();
    let result = r.apply_submodule_action(&p, &c).unwrap();
    assert!(result.result.is_ok(), "{:?}", result.result);
    assert!(result.verification.is_ok(), "{:?}", result.verification);
    result
}
fn snapshot(f: &Fixture) -> (Vec<u8>, Vec<u8>, Option<Vec<u8>>, Vec<u8>) {
    (
        fs::read(f.repository.join(".git/index")).unwrap(),
        fs::read(f.repository.join(".git/config")).unwrap(),
        fs::read(f.repository.join(".gitmodules")).ok(),
        f.run(&["rev-parse", "HEAD"]),
    )
}
#[test]
fn add_remove_unicode_preserves_unrelated_staging_and_retains_checkout() {
    isolated(
        "ROCKET_SUBMODULE_ACTION_CASE",
        "add_remove_unicode_preserves_unrelated_staging_and_retains_checkout",
        || {
            let f = parent();
            let s = source();
            f.write("parent.txt", "staged\n");
            f.run(&["add", "parent.txt"]);
            f.write("parent.txt", "unstaged\n");
            let staged = f.run(&["show", ":parent.txt"]);
            f.run(&["update-index", "--split-index"]);
            let head = f.run(&["rev-parse", "HEAD"]);
            let added = apply(&f, &add(&f, &s));
            assert!(
                added
                    .recovery_directory
                    .unwrap()
                    .join("shared-index.before")
                    .is_file()
            );
            assert_eq!(f.run(&["show", ":parent.txt"]), staged);
            assert_eq!(f.run(&["rev-parse", "HEAD"]), head);
            assert_eq!(
                fs::read_to_string(f.repository.join("parent.txt")).unwrap(),
                "unstaged\n"
            );
            assert!(
                f.run(&["ls-files", "--stage", "--", NAME])
                    .starts_with(b"160000")
            );
            assert_eq!(
                fs::read_to_string(f.repository.join(NAME).join("file 文.txt")).unwrap(),
                "child\n"
            );
            f.run(&["commit", "-m", "add submodule"]);
            let child = f.repository.join(NAME);
            f.run_at(&child, &["config", "user.name", "Fixture"]);
            f.run_at(&child, &["config", "user.email", "fixture@example.invalid"]);
            f.run_at(&child, &["commit", "--allow-empty", "-m", "local-only tip"]);
            let tip = f.run_at(&child, &["rev-parse", "HEAD"]);
            let result = apply(&f, &SubmoduleAction::Remove(path(NAME)));
            let saved = result.recovery_directory.unwrap().join("checkout");
            assert!(!child.exists());
            assert_eq!(f.run_at(&saved, &["rev-parse", "HEAD"]), tip);
            assert!(f.run(&["ls-files", "--stage", "--", NAME]).is_empty());
            assert_eq!(
                fs::read_to_string(f.repository.join("parent.txt")).unwrap(),
                "unstaged\n"
            );
        },
    );
}
#[test]
fn occupied_dirty_and_metadata_edits_are_refused() {
    isolated(
        "ROCKET_SUBMODULE_ACTION_CASE",
        "occupied_dirty_and_metadata_edits_are_refused",
        || {
            let f = parent();
            let s = source();
            let c = CancellationToken::new();
            f.write(&format!("{NAME}/keep"), "keep");
            let before = snapshot(&f);
            assert!(f.repo().preview_submodule_action(&add(&f, &s), &c).is_err());
            assert_eq!(snapshot(&f), before);
            fs::rename(f.repository.join(NAME), f.root.join("occupied retained")).unwrap();
            apply(&f, &add(&f, &s));
            assert!(matches!(
                f.repo()
                    .preview_submodule_action(&SubmoduleAction::Remove(path(NAME)), &c),
                Err(Error::Submodule(SubmoduleFailure::MetadataEdits))
            ));
            f.run(&["commit", "-m", "add"]);
            f.write(&format!("{NAME}/file 文.txt"), "external change");
            let before = snapshot(&f);
            assert!(matches!(
                f.repo()
                    .preview_submodule_action(&SubmoduleAction::Remove(path(NAME)), &c),
                Err(Error::Submodule(SubmoduleFailure::Dirty))
            ));
            assert_eq!(snapshot(&f), before);
            f.write(".gitmodules", "external metadata\n");
            let before = snapshot(&f);
            assert!(f.repo().preview_submodule_action(&add(&f, &s), &c).is_err());
            assert_eq!(snapshot(&f), before);
        },
    );
}
#[test]
fn stale_preview_cancel_and_foreign_owner_do_not_write() {
    isolated(
        "ROCKET_SUBMODULE_ACTION_CASE",
        "stale_preview_cancel_and_foreign_owner_do_not_write",
        || {
            let f = parent();
            let s = source();
            let c = CancellationToken::new();
            let r = f.repo();
            let p = r.preview_submodule_action(&add(&f, &s), &c).unwrap();
            let before = snapshot(&f);
            let stopped = CancellationToken::new();
            stopped.cancel();
            assert!(matches!(
                r.apply_submodule_action(&p, &stopped),
                Err(Error::Cancelled)
            ));
            assert_eq!(snapshot(&f), before);
            assert!(matches!(
                parent().repo().apply_submodule_action(&p, &c),
                Err(Error::Submodule(SubmoduleFailure::StalePreview))
            ));
            f.write("new", "new staged file");
            f.run(&["add", "new"]);
            let before = snapshot(&f);
            assert!(matches!(
                r.apply_submodule_action(&p, &c),
                Err(Error::Submodule(SubmoduleFailure::StalePreview))
            ));
            assert_eq!(snapshot(&f), before);
            assert!(!f.repository.join(NAME).exists());
        },
    );
}
#[test]
fn sync_preserves_dirty_files_index_and_push_urls() {
    isolated(
        "ROCKET_SUBMODULE_ACTION_CASE",
        "sync_preserves_dirty_files_index_and_push_urls",
        || {
            let f = parent();
            let s = source();
            let replacement = source();
            apply(&f, &add(&f, &s));
            f.run(&["commit", "-m", "add"]);
            let child = f.repository.join(NAME);
            f.write(&format!("{NAME}/file 文.txt"), "keep dirty\n");
            f.run_at(
                &child,
                &[
                    "config",
                    "remote.origin.pushurl",
                    "https://example.invalid/do-not-change",
                ],
            );
            f.run(&[
                "config",
                "--file",
                ".gitmodules",
                &format!("submodule.{NAME}.url"),
                replacement.repository.to_str().unwrap(),
            ]);
            let index = fs::read(f.repository.join(".git/index")).unwrap();
            let modules = fs::read(f.repository.join(".gitmodules")).unwrap();
            apply(&f, &SubmoduleAction::Sync(path(NAME)));
            assert_eq!(fs::read(f.repository.join(".git/index")).unwrap(), index);
            assert_eq!(fs::read(f.repository.join(".gitmodules")).unwrap(), modules);
            assert_eq!(
                fs::read_to_string(child.join("file 文.txt")).unwrap(),
                "keep dirty\n"
            );
            assert_eq!(
                String::from_utf8(f.run_at(&child, &["config", "remote.origin.url"]))
                    .unwrap()
                    .trim(),
                replacement.repository.to_str().unwrap()
            );
            assert_eq!(
                f.run_at(&child, &["config", "remote.origin.pushurl"]),
                b"https://example.invalid/do-not-change\n"
            );
        },
    );
}
#[test]
fn nested_add_and_remove_target_only_immediate_parent() {
    isolated(
        "ROCKET_SUBMODULE_ACTION_CASE",
        "nested_add_and_remove_target_only_immediate_parent",
        || {
            let f = parent();
            let s = source();
            let nested = source();
            apply(&f, &add(&f, &s));
            f.run(&["commit", "-m", "add"]);
            let r = f.repo();
            let c = CancellationToken::new();
            let child = r.submodules(&c).unwrap()[0].directory.clone();
            let before = snapshot(&f);
            let p = r
                .preview_submodule_action(
                    &SubmoduleAction::Add {
                        parent: child.clone(),
                        path: path("deps/文字 é"),
                        source: nested.repository.to_str().unwrap().into(),
                    },
                    &c,
                )
                .unwrap();
            let report = r.apply_submodule_action(&p, &c).unwrap();
            assert!(report.result.is_ok(), "{:?}", report.result);
            assert!(report.verification.is_ok());
            assert_eq!(snapshot(&f), before);
            let entries = r.submodules(&c).unwrap();
            assert_eq!(entries.len(), 2);
            assert_eq!(entries[1].depth, 1);
            assert!(matches!(
                r.preview_submodule_action(&SubmoduleAction::Remove(path(NAME)), &c),
                Err(Error::Submodule(SubmoduleFailure::Dirty))
            ));
            f.run_at(
                &child,
                &[
                    "-c",
                    "user.name=Fixture",
                    "-c",
                    "user.email=fixture@example.invalid",
                    "commit",
                    "-m",
                    "nested",
                ],
            );
            let p = r
                .preview_submodule_action(&SubmoduleAction::Remove(entries[1].path.clone()), &c)
                .unwrap();
            let report = r.apply_submodule_action(&p, &c).unwrap();
            assert!(report.result.is_ok(), "{:?}", report.result);
            assert!(report.verification.is_ok());
            assert_eq!(snapshot(&f), before);
            assert!(
                report
                    .recovery_directory
                    .unwrap()
                    .join("checkout/file 文.txt")
                    .exists()
            );
        },
    );
}
#[test]
fn add_to_unborn_repository_and_sha256() {
    isolated(
        "ROCKET_SUBMODULE_ACTION_CASE",
        "add_to_unborn_repository_and_sha256",
        || {
            for format in ["sha1", "sha256"] {
                let f = Fixture::with_format(format);
                let s = Fixture::with_format(format);
                s.write("file", "child");
                s.commit("child");
                apply(&f, &add(&f, &s));
                assert!(matches!(
                    f.repo().head(&CancellationToken::new()).unwrap(),
                    Head::Unborn { .. }
                ));
                assert!(
                    f.run(&["ls-files", "--stage", "--", NAME])
                        .starts_with(b"160000")
                );
            }
        },
    );
}

#[test]
fn locks_includes_and_source_rewrites_are_refused_without_mutation() {
    isolated(
        "ROCKET_SUBMODULE_ACTION_CASE",
        "locks_includes_and_source_rewrites_are_refused_without_mutation",
        || {
            let f = parent();
            let s = source();
            let c = CancellationToken::new();
            let a = add(&f, &s);
            fs::write(f.repository.join(".git/index.lock"), "external lock").unwrap();
            let before = snapshot(&f);
            assert!(matches!(
                f.repo().preview_submodule_action(&a, &c),
                Err(Error::Submodule(SubmoduleFailure::Busy))
            ));
            assert_eq!(snapshot(&f), before);
            assert_eq!(
                fs::read(f.repository.join(".git/index.lock")).unwrap(),
                b"external lock"
            );
            fs::remove_file(f.repository.join(".git/index.lock")).unwrap();
            let outside = source();
            f.run(&[
                "config",
                &format!("url.{}.insteadOf", outside.repository.to_str().unwrap()),
                s.repository.to_str().unwrap(),
            ]);
            assert!(matches!(
                f.repo().preview_submodule_action_local(&a, &s.root, &c),
                Err(Error::Submodule(SubmoduleFailure::UnsupportedSource))
            ));
            f.run(&[
                "config",
                "--remove-section",
                &format!("url.{}", outside.repository.to_str().unwrap()),
            ]);
            apply(&f, &a);
            f.run(&["commit", "-m", "add"]);
            let include = f.root.join("extra.config");
            fs::write(
                &include,
                format!("[submodule \"{NAME}\"]\nurl = https://example.invalid/outside\n"),
            )
            .unwrap();
            f.run(&["config", "include.path", include.to_str().unwrap()]);
            let before = snapshot(&f);
            let included = f
                .repo()
                .preview_submodule_action(&SubmoduleAction::Sync(path(NAME)), &c);
            assert!(
                matches!(
                    &included,
                    Err(Error::Submodule(SubmoduleFailure::ConfigScope))
                ),
                "{included:?}"
            );
            assert_eq!(snapshot(&f), before);
            f.run(&["config", "--unset", "include.path"]);
            f.run(&["config", "--remove-section", &format!("submodule.{NAME}")]);
            assert!(matches!(
                f.repo()
                    .preview_submodule_action(&SubmoduleAction::Sync(path(NAME)), &c),
                Err(Error::Submodule(SubmoduleFailure::RegistrationMissing))
            ));
        },
    );
}
#[test]
fn removal_preserves_gitfile_admin_and_refuses_ignored_and_hidden_files() {
    isolated(
        "ROCKET_SUBMODULE_ACTION_CASE",
        "removal_preserves_gitfile_admin_and_refuses_ignored_and_hidden_files",
        || {
            let f = parent();
            let s = source();
            s.write(".gitignore", "ignored/\n");
            s.commit("ignore");
            apply(&f, &add(&f, &s));
            f.run(&["commit", "-m", "add"]);
            let child = f.repository.join(NAME);
            let c = CancellationToken::new();
            let action = SubmoduleAction::Remove(path(NAME));
            f.run(&["submodule", "absorbgitdirs", "--", NAME]);
            f.write(&format!("{NAME}/ignored/keep"), "ignored data");
            assert!(matches!(
                f.repo().preview_submodule_action(&action, &c),
                Err(Error::Submodule(SubmoduleFailure::Dirty))
            ));
            fs::rename(child.join("ignored"), f.root.join("retained ignored")).unwrap();
            f.run_at(
                &child,
                &["update-index", "--assume-unchanged", "file 文.txt"],
            );
            assert!(matches!(
                f.repo().preview_submodule_action(&action, &c),
                Err(Error::Submodule(SubmoduleFailure::HiddenIndex))
            ));
            f.run_at(
                &child,
                &["update-index", "--no-assume-unchanged", "file 文.txt"],
            );
            let admin = String::from_utf8(f.run_at(&child, &["rev-parse", "--absolute-git-dir"]))
                .unwrap()
                .trim()
                .to_owned();
            let before_head = fs::read(std::path::Path::new(&admin).join("HEAD")).unwrap();
            let result = apply(&f, &action);
            assert!(!child.exists());
            assert_eq!(
                fs::read(std::path::Path::new(&admin).join("HEAD")).unwrap(),
                before_head
            );
            assert!(
                result
                    .recovery_directory
                    .unwrap()
                    .join("checkout/.git")
                    .is_file()
            );
        },
    );
}

#[test]
#[cfg(windows)]
fn interrupted_metadata_publication_reports_partial_steps_and_backups() {
    use std::os::windows::fs::OpenOptionsExt;
    isolated(
        "ROCKET_SUBMODULE_ACTION_CASE",
        "interrupted_metadata_publication_reports_partial_steps_and_backups",
        || {
            let f = parent();
            let s = source();
            let r = f.repo();
            let c = CancellationToken::new();
            let p = r.preview_submodule_action(&add(&f, &s), &c).unwrap();
            let before = snapshot(&f);
            // Model an editor holding config open. Reads and preparation work, but
            // Windows refuses replacement after the checkout/.gitmodules publish.
            let held = fs::OpenOptions::new()
                .read(true)
                .share_mode(1)
                .open(f.repository.join(".git/config"))
                .unwrap();
            let report = r.apply_submodule_action(&p, &c).unwrap();
            assert!(report.result.is_err());
            assert!(report.verification.is_err());
            assert_eq!(
                report.completed_steps,
                ["checkout_published", "gitmodules_published"]
            );
            assert_eq!(fs::read(f.repository.join(".git/index")).unwrap(), before.0);
            assert_eq!(
                fs::read(f.repository.join(".git/config")).unwrap(),
                before.1
            );
            assert!(f.repository.join(NAME).join("file 文.txt").exists());
            let refreshed = r.status(&CancellationToken::new()).unwrap();
            assert!(refreshed.entries.iter().any(|entry| {
                entry.path.as_bytes() == NAME.as_bytes() && entry.kind == EntryKind::Untracked
            }));
            let recovery = report.recovery_directory.unwrap();
            assert_eq!(fs::read(recovery.join("index.before")).unwrap(), before.0);
            assert_eq!(
                fs::read(recovery.join("parent-config.before")).unwrap(),
                before.1
            );
            for lock in [".git/index.lock", ".git/config.lock", ".gitmodules.lock"] {
                assert!(!f.repository.join(lock).exists());
            }
            drop(held);
        },
    );
}
