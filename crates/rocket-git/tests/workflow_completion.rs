//! Focused 0.69 integration checks. Every mutation is in a disposable repository;
//! transports and user/system configuration are isolated by the child runner.
#![cfg(windows)]
mod common;
use common::{Fixture, isolated};
use rocket_git::*;
use std::{fs, path::PathBuf};
fn case(name: &str, f: fn()) {
    isolated("ROCKET_WORKFLOW_CASE", name, f)
}
fn c() -> CancellationToken {
    CancellationToken::new()
}
fn base() -> Fixture {
    let f = Fixture::new();
    f.write("文件.txt", "base\n");
    f.commit("synthetic base");
    f
}
fn oid(f: &Fixture, revision: &str) -> String {
    String::from_utf8(f.run(&["rev-parse", revision]))
        .unwrap()
        .trim()
        .into()
}
fn remote(f: &Fixture) -> PathBuf {
    let path = f.root.join("remote.git");
    fs::create_dir(&path).unwrap();
    f.run_at(&path, &["init", "--bare", "--initial-branch=main"]);
    f.run(&["remote", "add", "origin", path.to_str().unwrap()]);
    f.run(&["push", "--set-upstream", "origin", "main"]);
    path
}
fn topic(f: &Fixture) {
    f.run(&["switch", "-c", "topic"]);
    f.write("新增.txt", "incoming\n");
    f.commit("incoming");
    f.run(&["switch", "main"]);
}
fn index(f: &Fixture) -> Vec<u8> {
    f.run(&["ls-files", "--stage", "-z"])
}

#[test]
fn squash_and_no_commit_preserve_head_and_offer_correct_completion() {
    case(
        "squash_and_no_commit_preserve_head_and_offer_correct_completion",
        || {
            for mode in [MergeMode::Squash, MergeMode::NoCommit] {
                let f = base();
                topic(&f);
                let repo = f.repo();
                let before = repo.head(&c()).unwrap();
                let p = repo.preview_merge("topic", mode, &c()).unwrap();
                let r = repo.apply_merge(&p, &c()).unwrap();
                r.write.result.unwrap();
                assert_eq!(repo.head(&c()).unwrap(), before);
                assert_eq!(
                    r.state.unwrap().merge_heads.is_empty(),
                    mode == MergeMode::Squash
                );
                assert!(f.repository.join("新增.txt").exists());
                if mode == MergeMode::Squash {
                    repo.commit("squashed", &c()).unwrap().result.unwrap();
                    assert_eq!(
                        f.run(&["rev-list", "--parents", "-n", "1", "HEAD"])
                            .split(|b| *b == b' ')
                            .count(),
                        2
                    );
                } else {
                    repo.abort_merge(&c()).unwrap().write.result.unwrap();
                    assert_eq!(repo.head(&c()).unwrap(), before);
                    assert!(!f.repository.join("新增.txt").exists());
                }
            }
        },
    );
}
#[test]
fn merge_modes_refuse_dirty_stale_and_cancelled_reviews() {
    case(
        "merge_modes_refuse_dirty_stale_and_cancelled_reviews",
        || {
            let f = base();
            topic(&f);
            let repo = f.repo();
            for mode in [MergeMode::Squash, MergeMode::NoCommit] {
                let p = repo.preview_merge("topic", mode, &c()).unwrap();
                let stop = c();
                stop.cancel();
                assert!(matches!(repo.apply_merge(&p, &stop), Err(Error::Cancelled)));
                f.write("文件.txt", "external\n");
                assert!(repo.apply_merge(&p, &c()).is_err());
                assert!(repo.preview_merge("topic", mode, &c()).is_err());
                assert_eq!(
                    fs::read(f.repository.join("文件.txt")).unwrap(),
                    b"external\n"
                );
                f.write("文件.txt", "base\n");
            }
        },
    );
}
#[test]
fn squash_conflict_retains_changes_without_fabricating_merge_state() {
    case(
        "squash_conflict_retains_changes_without_fabricating_merge_state",
        || {
            let f = base();
            f.run(&["switch", "-c", "topic"]);
            f.write("文件.txt", "incoming\n");
            f.commit("incoming");
            f.run(&["switch", "main"]);
            f.write("文件.txt", "ours\n");
            f.commit("ours");
            let repo = f.repo();
            let before = repo.head(&c()).unwrap();
            let p = repo
                .preview_merge("topic", MergeMode::Squash, &c())
                .unwrap();
            let r = repo.apply_merge(&p, &c()).unwrap();
            assert!(r.write.result.is_err());
            assert_eq!(repo.head(&c()).unwrap(), before);
            assert!(r.state.unwrap().merge_heads.is_empty());
            assert!(
                repo.status(&c())
                    .unwrap()
                    .entries
                    .iter()
                    .any(|e| e.kind == EntryKind::Unmerged)
            );
        },
    );
}
#[test]
fn stash_rename_keeps_ids_order_worktree_and_recovery_copy() {
    case(
        "stash_rename_keeps_ids_order_worktree_and_recovery_copy",
        || {
            let f = base();
            let repo = f.repo();
            for text in ["one\n", "two\n"] {
                f.write("文件.txt", text);
                repo.push_stash(Some(text.trim()), false, &c())
                    .unwrap()
                    .write
                    .result
                    .unwrap();
            }
            let old = repo.stashes(&c()).unwrap();
            let original = fs::read(f.repository.join(".git/logs/refs/stash")).unwrap();
            f.write("文件.txt", "working\n");
            let before = index(&f);
            let p = repo
                .preview_stash_rename(&old[1], "保留 🚀 saved work", &c())
                .unwrap();
            let backup = repo.rename_stash(&p, &c()).unwrap();
            let new = repo.stashes(&c()).unwrap();
            assert_eq!(
                new.iter().map(|s| &s.id).collect::<Vec<_>>(),
                old.iter().map(|s| &s.id).collect::<Vec<_>>()
            );
            assert_eq!(new[1].subject, "保留 🚀 saved work".as_bytes());
            assert_eq!(fs::read(backup).unwrap(), original);
            assert_eq!(index(&f), before);
            assert_eq!(
                fs::read(f.repository.join("文件.txt")).unwrap(),
                b"working\n"
            );
            assert!(!f.repository.join(".git/refs/stash.lock").exists());
            assert!(!f.repository.join(".git/logs/refs/stash.lock").exists());
            assert!(repo.rename_stash(&p, &c()).is_err());
            let p = repo.preview_stash_rename(&new[0], "cancel", &c()).unwrap();
            let stop = c();
            stop.cancel();
            assert!(matches!(
                repo.rename_stash(&p, &stop),
                Err(Error::Cancelled)
            ));
            assert_eq!(repo.stashes(&c()).unwrap(), new);
        },
    );
}
#[test]
fn prune_only_reviewed_tracking_refs_and_retain_recovery_tips() {
    case(
        "prune_only_reviewed_tracking_refs_and_retain_recovery_tips",
        || {
            let f = base();
            let server = remote(&f);
            f.run(&["branch", "old"]);
            f.run(&["push", "origin", "old"]);
            f.run(&["branch", "--set-upstream-to=origin/old", "old"]);
            f.run_at(&server, &["update-ref", "-d", "refs/heads/old"]);
            f.write("文件.txt", "local\n");
            f.write("未跟踪.txt", "keep");
            let before = index(&f);
            let repo = f.repo();
            let p = repo.preview_remote_cleanup(b"origin", &c()).unwrap();
            assert_eq!(p.obsolete().len(), 1);
            assert_eq!(p.stale_local_branches(), [b"old".to_vec()]);
            let id = p.obsolete()[0].id.clone();
            let r = repo
                .prune_tracking_refs(&p, &[p.obsolete()[0].reference.clone()], &c())
                .unwrap();
            r.result.unwrap();
            assert_eq!(oid(&f, &format!("{}/0", r.recovery.unwrap())), id.as_str());
            assert_eq!(oid(&f, "old"), id.as_str());
            assert!(
                repo.preview_remote_cleanup(b"origin", &c())
                    .unwrap()
                    .obsolete()
                    .is_empty()
            );
            assert_eq!(index(&f), before);
            assert_eq!(fs::read(f.repository.join("文件.txt")).unwrap(), b"local\n");
        },
    );
}
#[test]
fn prune_rejects_reappearing_refs_custom_mapping_and_cancel() {
    case(
        "prune_rejects_reappearing_refs_custom_mapping_and_cancel",
        || {
            let f = base();
            let server = remote(&f);
            f.run(&["update-ref", "refs/remotes/origin/old", "HEAD"]);
            let repo = f.repo();
            let p = repo.preview_remote_cleanup(b"origin", &c()).unwrap();
            let names = vec![p.obsolete()[0].reference.clone()];
            let stop = c();
            stop.cancel();
            assert!(matches!(
                repo.prune_tracking_refs(&p, &names, &stop),
                Err(Error::Cancelled)
            ));
            f.run_at(&server, &["update-ref", "refs/heads/old", &oid(&f, "HEAD")]);
            assert!(repo.prune_tracking_refs(&p, &names, &c()).is_err());
            assert!(
                f.output(
                    &f.repository,
                    &["show-ref", "--verify", "refs/remotes/origin/old"]
                )
                .status
                .success()
            );
            f.run(&[
                "config",
                "remote.origin.fetch",
                "+refs/heads/main:refs/remotes/origin/main",
            ]);
            assert!(repo.preview_remote_cleanup(b"origin", &c()).is_err());
        },
    );
}
#[test]
fn remote_delete_uses_expected_tip_and_handles_default_rejection_and_cancel() {
    case(
        "remote_delete_uses_expected_tip_and_handles_default_rejection_and_cancel",
        || {
            let f = base();
            let server = remote(&f);
            f.run(&["push", "origin", "main:old"]);
            let repo = f.repo();
            assert!(
                repo.preview_remote_branch_delete(b"origin", b"main", &c())
                    .is_err()
            );
            let p = repo
                .preview_remote_branch_delete(b"origin", b"old", &c())
                .unwrap();
            let stop = c();
            stop.cancel();
            assert!(matches!(
                repo.delete_remote_branch(&p, &stop),
                Err(Error::Cancelled)
            ));
            f.write("added", "new");
            f.commit("advance");
            f.run(&["push", "origin", "main:old"]);
            assert!(repo.delete_remote_branch(&p, &c()).is_err());
            let p = repo
                .preview_remote_branch_delete(b"origin", b"old", &c())
                .unwrap();
            f.run_at(&server, &["config", "receive.denyDeletes", "true"]);
            let r = repo.delete_remote_branch(&p, &c()).unwrap();
            assert!(r.result.is_err());
            assert_eq!(r.remote_deleted, Some(false));
            f.run_at(&server, &["config", "receive.denyDeletes", "false"]);
            let r = repo.delete_remote_branch(&p, &c()).unwrap();
            r.result.unwrap();
            assert_eq!(r.remote_deleted, Some(true));
            assert_eq!(oid(&f, "HEAD"), oid(&f, "main"));
        },
    );
}
#[test]
fn remote_delete_refuses_unknown_default_branch() {
    case("remote_delete_refuses_unknown_default_branch", || {
        let f = base();
        let server = remote(&f);
        let head = oid(&f, "HEAD");
        f.run(&["push", "origin", "main:old"]);
        f.run_at(&server, &["update-ref", "--no-deref", "HEAD", &head]);
        let result = f
            .repo()
            .preview_remote_branch_delete(b"origin", b"old", &c());
        assert!(
            result.is_err(),
            "unidentified default must prevent deletion"
        );
        assert_eq!(
            String::from_utf8(f.run_at(&server, &["rev-parse", "refs/heads/old"]))
                .unwrap()
                .trim(),
            head
        );
    });
}
#[test]
fn saved_rebase_restores_staging_and_keeps_stash_and_untracked_files() {
    case(
        "saved_rebase_restores_staging_and_keeps_stash_and_untracked_files",
        || {
            let f = base();
            topic(&f);
            let repo = f.repo();
            f.write("文件.txt", "staged\n");
            f.run(&["add", "--", "文件.txt"]);
            f.write("文件.txt", "working\n");
            f.write("未跟踪.txt", "keep");
            let p = repo
                .preview_saved_work(SavedWorkAction::Rebase("topic".into()), &c())
                .unwrap();
            assert_eq!(p.changed_files(), 1);
            let r = repo.apply_saved_work(&p, &c()).unwrap();
            r.result.unwrap();
            assert_eq!(r.phase, "reapplied");
            assert_eq!(repo.stashes(&c()).unwrap()[0].id, r.stash.unwrap());
            assert!(r.journal.exists());
            assert_eq!(f.run(&["show", ":文件.txt"]), b"staged\n");
            assert_eq!(
                fs::read(f.repository.join("文件.txt")).unwrap(),
                b"working\n"
            );
            assert_eq!(fs::read(f.repository.join("未跟踪.txt")).unwrap(), b"keep");
            assert_eq!(oid(&f, "HEAD"), oid(&f, "topic"));
        },
    );
}
#[test]
fn saved_work_refuses_stale_files_targets_and_precancel_without_stashing() {
    case(
        "saved_work_refuses_stale_files_targets_and_precancel_without_stashing",
        || {
            let f = base();
            topic(&f);
            f.write("文件.txt", "draft\n");
            let repo = f.repo();
            let p = repo
                .preview_saved_work(SavedWorkAction::Rebase("topic".into()), &c())
                .unwrap();
            let stop = c();
            stop.cancel();
            assert!(matches!(
                repo.apply_saved_work(&p, &stop),
                Err(Error::Cancelled)
            ));
            f.write("文件.txt", "changed\n");
            assert!(repo.apply_saved_work(&p, &c()).is_err());
            assert!(repo.stashes(&c()).unwrap().is_empty());
            assert_eq!(
                fs::read(f.repository.join("文件.txt")).unwrap(),
                b"changed\n"
            );
            let p = repo
                .preview_saved_work(SavedWorkAction::Rebase("topic".into()), &c())
                .unwrap();
            f.run(&["branch", "-f", "topic", "main"]);
            assert!(repo.apply_saved_work(&p, &c()).is_err());
            assert!(repo.stashes(&c()).unwrap().is_empty());
        },
    );
}
#[test]
fn saved_work_conflicts_retain_recovery_stash() {
    case("saved_work_conflicts_retain_recovery_stash", || {
        let f = base();
        f.run(&["switch", "-c", "topic"]);
        f.write("文件.txt", "incoming\n");
        f.commit("incoming");
        f.run(&["switch", "main"]);
        f.write("文件.txt", "draft\n");
        let repo = f.repo();
        let p = repo
            .preview_saved_work(SavedWorkAction::Rebase("topic".into()), &c())
            .unwrap();
        let r = repo.apply_saved_work(&p, &c()).unwrap();
        assert!(r.result.is_err());
        assert_eq!(r.phase, "reapplying");
        assert_eq!(repo.stashes(&c()).unwrap()[0].id, r.stash.unwrap());
        assert_eq!(oid(&f, "HEAD"), oid(&f, "topic"));
        assert!(r.journal.exists());
    });
}
#[test]
fn saved_pull_fetches_pinned_upstream_and_rejects_excluded_upstream() {
    case(
        "saved_pull_fetches_pinned_upstream_and_rejects_excluded_upstream",
        || {
            let f = base();
            let server = remote(&f);
            topic(&f);
            f.run(&["push", "origin", "topic:main"]);
            f.write("文件.txt", "draft\n");
            let repo = f.repo();
            let p = repo
                .preview_saved_work(SavedWorkAction::Pull(PullStrategy::FastForward), &c())
                .unwrap();
            assert_eq!(p.target().as_str(), oid(&f, "topic"));
            repo.apply_saved_work(&p, &c()).unwrap().result.unwrap();
            assert_eq!(fs::read(f.repository.join("文件.txt")).unwrap(), b"draft\n");
            f.run(&["config", "--add", "remote.origin.fetch", "^refs/heads/main"]);
            assert!(
                repo.preview_saved_work(SavedWorkAction::Pull(PullStrategy::Rebase), &c())
                    .is_err()
            );
            assert!(server.exists());
        },
    );
}
#[test]
fn saved_integration_conflict_keeps_stash_and_can_abort_before_reapply() {
    case(
        "saved_integration_conflict_keeps_stash_and_can_abort_before_reapply",
        || {
            let f = base();
            f.write("draft.txt", "base\n");
            f.commit("draft base");
            f.run(&["switch", "-c", "topic"]);
            f.write("文件.txt", "incoming\n");
            f.commit("incoming");
            f.run(&["switch", "main"]);
            f.write("文件.txt", "ours\n");
            f.commit("ours");
            let original = oid(&f, "HEAD");
            f.write("draft.txt", "saved draft\n");
            let repo = f.repo();
            let p = repo
                .preview_saved_work(SavedWorkAction::Rebase("topic".into()), &c())
                .unwrap();
            let r = repo.apply_saved_work(&p, &c()).unwrap();
            assert!(r.result.is_err());
            assert_eq!(r.phase, "integrating");
            let id = r.stash.unwrap();
            assert!(repo.operation_state(&c()).unwrap().rebase);
            let abort = repo.preview_rebase(RebaseAction::Abort, "", &c()).unwrap();
            repo.apply_rebase(&abort, &c())
                .unwrap()
                .write
                .result
                .unwrap();
            assert_eq!(oid(&f, "HEAD"), original);
            repo.apply_stash(&id, true, &c())
                .unwrap()
                .write
                .result
                .unwrap();
            assert_eq!(
                fs::read(f.repository.join("draft.txt")).unwrap(),
                b"saved draft\n"
            );
            assert_eq!(repo.stashes(&c()).unwrap()[0].id, id);
        },
    );
}
