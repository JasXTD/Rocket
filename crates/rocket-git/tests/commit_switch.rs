//! Composed workflows, isolated local remotes and synthetic user data only.
#![cfg(windows)]
mod common;
use common::{Fixture, isolated};
use rocket_git::*;
use std::{
    fs,
    path::PathBuf,
    time::{Duration, Instant},
};
fn c() -> CancellationToken {
    CancellationToken::new()
}
fn case(name: &str, f: fn()) {
    isolated("ROCKET_COMMIT_SWITCH_CASE", name, f)
}
fn base() -> Fixture {
    let f = Fixture::new();
    f.write("資料.txt", "base\n");
    f.commit("base");
    f
}
fn id(f: &Fixture, r: &str) -> String {
    String::from_utf8(f.run(&["rev-parse", r]))
        .unwrap()
        .trim()
        .into()
}
fn remote(f: &Fixture) -> PathBuf {
    let p = f.root.join("remote.git");
    fs::create_dir(&p).unwrap();
    f.run_at(&p, &["init", "--bare"]);
    f.run(&["remote", "add", "origin", p.to_str().unwrap()]);
    p
}
fn staged(f: &Fixture) {
    f.write("資料.txt", "staged\n");
    f.run(&["add", "資料.txt"]);
    f.write("資料.txt", "unstaged\n");
    f.write("未追蹤.txt", "keep\n");
}
fn switch_review(f: &Fixture) -> SavedWorkPreview {
    f.repo()
        .preview_saved_work(SavedWorkAction::Switch("topic".into()), &c())
        .unwrap()
}
fn hook(f: &Fixture, name: &str, body: &str) {
    fs::write(
        f.root.join("no-hooks").join(name),
        format!("#!/bin/sh\n{body}\n"),
    )
    .unwrap();
}
fn cancel_at(marker: PathBuf, token: CancellationToken) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let start = Instant::now();
        while !marker.exists() {
            assert!(
                start.elapsed() < Duration::from_secs(20),
                "hook marker timeout"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        token.cancel();
    })
}
#[test]
fn commit_push_success_preserves_working_changes_and_supports_first_commit() {
    case(
        "commit_push_success_preserves_working_changes_and_supports_first_commit",
        || {
            for first in [false, true] {
                let f = if first { Fixture::new() } else { base() };
                let remote = remote(&f);
                staged(&f);
                let repo = f.repo();
                let p = repo
                    .preview_commit_push("Synthetic 文", b"origin", b"topic", &c())
                    .unwrap();
                assert_eq!(p.staged_files(), 1);
                let r = repo.apply_commit_push(&p, &c()).unwrap();
                r.result.unwrap();
                assert_eq!(r.commit.as_ref().unwrap().as_str(), id(&f, "HEAD"));
                let push = r.push.unwrap();
                push.result.unwrap();
                assert_eq!(push.remote_outcome, PushRemoteOutcome::Confirmed);
                assert_eq!(
                    f.run_at(&remote, &["rev-parse", "refs/heads/topic"]),
                    f.run(&["rev-parse", "HEAD"])
                );
                assert_eq!(f.run(&["show", "HEAD:資料.txt"]), b"staged\n");
                assert_eq!(
                    fs::read(f.repository.join("資料.txt")).unwrap(),
                    b"unstaged\n"
                );
                assert_eq!(
                    fs::read(f.repository.join("未追蹤.txt")).unwrap(),
                    b"keep\n"
                );
                assert!(f.run(&["diff", "--cached", "--name-only"]).is_empty());
            }
        },
    )
}
#[test]
fn commit_push_rejects_stale_index_destination_empty_and_precancel() {
    case(
        "commit_push_rejects_stale_index_destination_empty_and_precancel",
        || {
            let f = base();
            remote(&f);
            let repo = f.repo();
            assert!(
                repo.preview_commit_push("empty", b"origin", b"main", &c())
                    .is_err()
            );
            staged(&f);
            let p = repo
                .preview_commit_push("review", b"origin", b"main", &c())
                .unwrap();
            let before = id(&f, "HEAD");
            let stop = c();
            stop.cancel();
            assert!(matches!(
                repo.apply_commit_push(&p, &stop),
                Err(Error::Cancelled)
            ));
            f.write("資料.txt", "external\n");
            f.run(&["add", "資料.txt"]);
            assert!(repo.apply_commit_push(&p, &c()).is_err());
            assert_eq!(id(&f, "HEAD"), before);
            let p = repo
                .preview_commit_push("review", b"origin", b"main", &c())
                .unwrap();
            f.run(&["config", "remote.origin.pushurl", "missing-local-remote"]);
            assert!(repo.apply_commit_push(&p, &c()).is_err());
            assert_eq!(id(&f, "HEAD"), before);
        },
    )
}
#[test]
fn commit_push_rejection_keeps_commit_and_hook_failure_never_pushes() {
    case(
        "commit_push_rejection_keeps_commit_and_hook_failure_never_pushes",
        || {
            let f = base();
            let remote = remote(&f);
            f.run(&["push", "origin", "main"]);
            let before = id(&f, "HEAD");
            fs::create_dir_all(remote.join("hooks")).unwrap();
            fs::write(
                remote.join("hooks/pre-receive"),
                "#!/bin/sh\necho 'synthetic policy rejection' >&2\nexit 1\n",
            )
            .unwrap();
            staged(&f);
            let repo = f.repo();
            let p = repo
                .preview_commit_push("retained", b"origin", b"main", &c())
                .unwrap();
            let r = repo.apply_commit_push(&p, &c()).unwrap();
            assert!(r.result.is_err());
            assert!(r.commit.is_some());
            assert_eq!(r.push.unwrap().remote_outcome, PushRemoteOutcome::Rejected);
            assert_ne!(id(&f, "HEAD"), before);
            assert_eq!(
                String::from_utf8(f.run_at(&remote, &["rev-parse", "main"]))
                    .unwrap()
                    .trim(),
                before
            );
            f.run(&["add", "資料.txt"]);
            hook(&f, "pre-commit", "exit 1");
            let before = id(&f, "HEAD");
            let p = repo
                .preview_commit_push("blocked", b"origin", b"main", &c())
                .unwrap();
            let r = repo.apply_commit_push(&p, &c()).unwrap();
            assert!(r.result.is_err());
            assert!(r.commit.is_none() && r.push.is_none());
            assert_eq!(id(&f, "HEAD"), before);
        },
    )
}
#[test]
fn commit_push_hook_changed_commit_is_retained_without_push() {
    case(
        "commit_push_hook_changed_commit_is_retained_without_push",
        || {
            let f = base();
            remote(&f);
            staged(&f);
            hook(
                &f,
                "pre-commit",
                "printf 'hook change\\n' > hook.txt\ngit add hook.txt",
            );
            let repo = f.repo();
            let p = repo
                .preview_commit_push("review", b"origin", b"main", &c())
                .unwrap();
            let r = repo.apply_commit_push(&p, &c()).unwrap();
            assert!(r.result.is_err());
            assert!(r.commit.is_some() && r.push.is_none());
            assert_eq!(f.run(&["show", "HEAD:hook.txt"]), b"hook change\n");
        },
    )
}
#[test]
fn commit_push_cancel_after_local_commit_keeps_it_without_push() {
    case(
        "commit_push_cancel_after_local_commit_keeps_it_without_push",
        || {
            let f = base();
            remote(&f);
            staged(&f);
            hook(
                &f,
                "post-commit",
                "touch .git/commit-marker\nwhile :; do sleep 1; done",
            );
            let repo = f.repo();
            let p = repo
                .preview_commit_push("retained on cancel", b"origin", b"main", &c())
                .unwrap();
            let stop = c();
            let t = cancel_at(f.repository.join(".git/commit-marker"), stop.clone());
            let r = repo.apply_commit_push(&p, &stop).unwrap();
            t.join().unwrap();
            assert!(r.result.is_err());
            assert!(r.commit.is_some() && r.push.is_none());
            assert_eq!(id(&f, "HEAD"), r.commit.unwrap().as_str());
        },
    )
}
#[test]
fn saved_switch_restores_staging_unicode_and_keeps_recovery() {
    case(
        "saved_switch_restores_staging_unicode_and_keeps_recovery",
        || {
            let f = base();
            f.run(&["branch", "topic"]);
            staged(&f);
            let index = f.run(&["ls-files", "--stage", "-z"]);
            let p = switch_review(&f);
            let r = f.repo().apply_saved_work(&p, &c()).unwrap();
            r.result.unwrap();
            assert_eq!(r.phase, "reapplied");
            assert!(r.stash.is_some() && r.journal.exists());
            assert_eq!(f.run(&["branch", "--show-current"]), b"topic\n");
            assert_eq!(index, f.run(&["ls-files", "--stage", "-z"]));
            assert_eq!(
                fs::read(f.repository.join("資料.txt")).unwrap(),
                b"unstaged\n"
            );
            assert_eq!(
                fs::read(f.repository.join("未追蹤.txt")).unwrap(),
                b"keep\n"
            );
            assert_eq!(f.repo().stashes(&c()).unwrap()[0].id, r.stash.unwrap());
        },
    )
}
#[test]
fn saved_switch_stale_and_precancel_do_not_save_or_switch() {
    case(
        "saved_switch_stale_and_precancel_do_not_save_or_switch",
        || {
            let f = base();
            f.run(&["branch", "topic"]);
            staged(&f);
            let p = switch_review(&f);
            let stop = c();
            stop.cancel();
            assert!(matches!(
                f.repo().apply_saved_work(&p, &stop),
                Err(Error::Cancelled)
            ));
            f.write("資料.txt", "external\n");
            assert!(f.repo().apply_saved_work(&p, &c()).is_err());
            assert!(f.repo().stashes(&c()).unwrap().is_empty());
            assert_eq!(f.run(&["branch", "--show-current"]), b"main\n");
            assert_eq!(
                fs::read(f.repository.join("資料.txt")).unwrap(),
                b"external\n"
            );
        },
    )
}
#[test]
fn saved_switch_collision_and_restore_conflict_retain_stash() {
    case(
        "saved_switch_collision_and_restore_conflict_retain_stash",
        || {
            for collision in [true, false] {
                let f = base();
                f.run(&["switch", "-c", "topic"]);
                f.write(
                    if collision {
                        "untracked.txt"
                    } else {
                        "資料.txt"
                    },
                    "target\n",
                );
                f.commit("target");
                f.run(&["switch", "main"]);
                staged(&f);
                if collision {
                    f.write("untracked.txt", "protected\n");
                }
                let p = switch_review(&f);
                let r = f.repo().apply_saved_work(&p, &c()).unwrap();
                assert!(r.result.is_err());
                assert!(r.stash.is_some() && r.journal.exists());
                assert_eq!(f.repo().stashes(&c()).unwrap()[0].id, r.stash.unwrap());
                if collision {
                    assert_eq!(
                        fs::read(f.repository.join("untracked.txt")).unwrap(),
                        b"protected\n"
                    );
                    assert_eq!(f.run(&["branch", "--show-current"]), b"main\n");
                } else {
                    assert_eq!(f.run(&["branch", "--show-current"]), b"topic\n");
                    assert_eq!(r.phase, "reapplying");
                }
            }
        },
    )
}
#[test]
fn saved_switch_cancel_after_checkout_keeps_stash_without_restoring() {
    case(
        "saved_switch_cancel_after_checkout_keeps_stash_without_restoring",
        || {
            let f = base();
            f.run(&["branch", "topic"]);
            staged(&f);
            hook(
                &f,
                "post-checkout",
                "touch .git/switch-marker\nwhile :; do sleep 1; done",
            );
            let p = switch_review(&f);
            let stop = c();
            let t = cancel_at(f.repository.join(".git/switch-marker"), stop.clone());
            let r = f.repo().apply_saved_work(&p, &stop).unwrap();
            t.join().unwrap();
            assert!(r.result.is_err());
            assert!(r.stash.is_some());
            assert_eq!(f.run(&["branch", "--show-current"]), b"topic\n");
            assert_eq!(fs::read(f.repository.join("資料.txt")).unwrap(), b"base\n");
            assert_eq!(
                fs::read(f.repository.join("未追蹤.txt")).unwrap(),
                b"keep\n"
            );
        },
    )
}

#[test]
fn commit_push_cancels_running_push_and_refuses_hook_destination_change() {
    case(
        "commit_push_cancels_running_push_and_refuses_hook_destination_change",
        || {
            for change in [false, true] {
                let f = base();
                let remote = remote(&f);
                f.run(&["push", "origin", "main"]);
                let before = id(&f, "HEAD");
                staged(&f);
                if change {
                    hook(
                        &f,
                        "post-commit",
                        "git config remote.origin.pushurl missing-local-remote",
                    );
                } else {
                    hook(
                        &f,
                        "pre-push",
                        "touch .git/push-marker\nwhile :; do sleep 1; done",
                    );
                }
                let repo = f.repo();
                let p = repo
                    .preview_commit_push("retained", b"origin", b"main", &c())
                    .unwrap();
                let stop = c();
                let waiter = (!change)
                    .then(|| cancel_at(f.repository.join(".git/push-marker"), stop.clone()));
                let r = repo.apply_commit_push(&p, &stop).unwrap();
                if let Some(t) = waiter {
                    t.join().unwrap();
                }
                assert!(r.commit.is_some() && r.reviewed_commit_confirmed);
                assert_ne!(id(&f, "HEAD"), before);
                if change {
                    assert!(r.result.is_err());
                    assert!(r.push.is_none());
                } else {
                    assert!(r.push.unwrap().result.is_err());
                }
                assert_eq!(
                    String::from_utf8(f.run_at(&remote, &["rev-parse", "main"]))
                        .unwrap()
                        .trim(),
                    before
                );
                assert_eq!(
                    fs::read(f.repository.join("資料.txt")).unwrap(),
                    b"unstaged\n"
                );
            }
        },
    )
}
#[test]
fn saved_switch_post_checkout_edit_and_moving_target_are_protected() {
    case(
        "saved_switch_post_checkout_edit_and_moving_target_are_protected",
        || {
            let f = base();
            f.run(&["branch", "topic"]);
            staged(&f);
            hook(
                &f,
                "post-checkout",
                "printf 'external after checkout\\n' > 資料.txt",
            );
            let p = switch_review(&f);
            let r = f.repo().apply_saved_work(&p, &c()).unwrap();
            assert!(r.result.is_err());
            assert!(r.stash.is_some());
            assert_eq!(r.phase, "ready-to-reapply");
            assert_eq!(
                fs::read(f.repository.join("資料.txt")).unwrap(),
                b"external after checkout\n"
            );
            let f = base();
            f.run(&["branch", "topic"]);
            staged(&f);
            let p = switch_review(&f);
            let tree = id(&f, "HEAD^{tree}");
            let other =
                String::from_utf8(f.run(&["commit-tree", &tree, "-m", "external tip"])).unwrap();
            f.run(&["update-ref", "refs/heads/topic", other.trim()]);
            assert!(f.repo().apply_saved_work(&p, &c()).is_err());
            assert!(f.repo().stashes(&c()).unwrap().is_empty());
            assert_eq!(
                fs::read(f.repository.join("資料.txt")).unwrap(),
                b"unstaged\n"
            );
        },
    )
}

#[test]
fn commit_push_keeps_secret_protection_enabled() {
    case("commit_push_keeps_secret_protection_enabled", || {
        let f = base();
        let remote = remote(&f);
        f.run(&["push", "origin", "main"]);
        let before = id(&f, "HEAD");
        f.write(".env", "SYNTHETIC_CONFIG=not-a-credential\n");
        f.run(&["add", ".env"]);
        let repo = f.repo();
        let p = repo
            .preview_commit_push("blocked synthetic environment", b"origin", b"main", &c())
            .unwrap();
        let r = repo.apply_commit_push(&p, &c()).unwrap();
        assert!(r.result.is_err());
        assert!(r.commit.is_none() && r.push.is_none());
        assert_eq!(id(&f, "HEAD"), before);
        assert_eq!(
            String::from_utf8(f.run_at(&remote, &["rev-parse", "main"]))
                .unwrap()
                .trim(),
            before
        );
        assert!(f.repository.join(".env").is_file());
        assert!(!f.run(&["diff", "--cached", "--name-only"]).is_empty());
    })
}
