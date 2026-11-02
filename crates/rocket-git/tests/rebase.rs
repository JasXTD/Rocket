mod common;
use common::{Fixture, isolated};
use rocket_git::*;
use std::{
    fs,
    time::{Duration, Instant},
};
const FILE: &str = "src/検索/結果 🐦 e\u{301}.rs";
#[test]
fn hidden_index_edits_are_refused_without_clearing_flags() {
    isolated(
        "ROCKET_REBASE_CASE",
        "hidden_index_edits_are_refused_without_clearing_flags",
        || {
            for flag in ["--assume-unchanged", "--skip-worktree"] {
                let f = fixture("sha1", false);
                f.run(&["update-index", flag, "--", FILE]);
                f.write(FILE, "PRIVATE_HIDDEN_EDIT\n");
                assert!(f.run(&["status", "--porcelain"]).is_empty());
                let index = f.run(&["ls-files", "-v", "--", FILE]);
                refusal(
                    f.repo()
                        .preview_rebase(RebaseAction::Start, "main", &CancellationToken::new()),
                    RebaseFailure::UnsupportedIndex,
                );
                assert_eq!(
                    fs::read(f.repository.join(FILE)).unwrap(),
                    b"PRIVATE_HIDDEN_EDIT\n"
                );
                assert_eq!(f.run(&["ls-files", "-v", "--", FILE]), index);
            }
        },
    );
}
fn text(f: &Fixture, args: &[&str]) -> String {
    String::from_utf8(f.run(args)).unwrap().trim().to_owned()
}
fn oid(f: &Fixture, revision: &str) -> String {
    text(f, &["rev-parse", revision])
}
fn preview(f: &Fixture, action: RebaseAction) -> RebasePreview {
    f.repo()
        .preview_rebase(
            action,
            if action == RebaseAction::Start {
                "main"
            } else {
                ""
            },
            &CancellationToken::new(),
        )
        .unwrap()
}
fn apply(f: &Fixture, p: &RebasePreview) -> RebaseWriteReport {
    f.repo().apply_rebase(p, &CancellationToken::new()).unwrap()
}
fn ok(r: RebaseWriteReport) {
    assert!(r.write.result.is_ok(), "{r:?}");
    assert!(r.state.unwrap().is_idle());
    assert!(r.branches.is_ok())
}
fn refusal<T: std::fmt::Debug>(r: Result<T>, code: RebaseFailure) {
    assert!(matches!(r,Err(Error::Rebase(f))if f==code), "{r:?}")
}
fn fixture(format: &str, conflict: bool) -> Fixture {
    let f = Fixture::with_format(format);
    f.write(FILE, "base\n");
    f.write(".gitignore", "ignored/\n");
    f.commit("base");
    f.run(&["switch", "-c", "feature/検索"]);
    f.write(FILE, "feature\n");
    f.commit("PRIVATE_COMMIT feature 中文");
    f.write("second", "second\n");
    f.commit("PRIVATE_COMMIT second");
    f.run(&["switch", "main"]);
    if conflict {
        f.write(FILE, "main\n")
    } else {
        f.write("upstream", "main\n")
    }
    f.commit("main advances");
    f.run(&["switch", "feature/検索"]);
    f
}
#[test]
fn linear_sha1_sha256_preserves_other_refs_config_and_empty_commits() {
    isolated(
        "ROCKET_REBASE_CASE",
        "linear_sha1_sha256_preserves_other_refs_config_and_empty_commits",
        || {
            for format in ["sha1", "sha256"] {
                let f = fixture(format, false);
                f.run(&["commit", "--allow-empty", "-m", "intentional empty"]);
                let original = oid(&f, "HEAD");
                f.run(&["branch", "other"]);
                for (k, v) in [
                    ("rebase.updateRefs", "true"),
                    ("rebase.autoStash", "true"),
                    ("rebase.autosquash", "true"),
                    (
                        "rebase.instructionFormat",
                        "%s%nexec touch PRIVATE_UNEXPECTED_EXEC",
                    ),
                    ("rebase.rebaseMerges", "true"),
                ] {
                    f.run(&["config", k, v]);
                }
                f.write("private", "untracked\n");
                f.write("ignored/cache", "ignored\n");
                let config = fs::read(f.repository.join(".git/config")).unwrap();
                let p = preview(&f, RebaseAction::Start);
                assert_eq!(p.commits().len(), 3);
                assert!(!p.fast_forward());
                ok(apply(&f, &p));
                assert!(!f.repository.join("PRIVATE_UNEXPECTED_EXEC").exists());
                assert_eq!(oid(&f, "other"), original);
                assert_eq!(oid(&f, &p.recovery_reference()), original);
                assert_eq!(text(&f, &["log", "-1", "--format=%s"]), "intentional empty");
                assert_eq!(text(&f, &["rev-list", "--count", "main..HEAD"]), "3");
                assert_eq!(fs::read(f.repository.join(FILE)).unwrap(), b"feature\n");
                assert_eq!(
                    fs::read(f.repository.join("private")).unwrap(),
                    b"untracked\n"
                );
                assert_eq!(
                    fs::read(f.repository.join("ignored/cache")).unwrap(),
                    b"ignored\n"
                );
                assert_eq!(fs::read(f.repository.join(".git/config")).unwrap(), config);
                let no_op = preview(&f, RebaseAction::Start);
                assert!(no_op.up_to_date());
                let head = oid(&f, "HEAD");
                ok(apply(&f, &no_op));
                assert_eq!(oid(&f, "HEAD"), head);
            }
        },
    );
}
#[test]
fn conflict_continue_skip_abort_and_stale_recovery() {
    isolated(
        "ROCKET_REBASE_CASE",
        "conflict_continue_skip_abort_and_stale_recovery",
        || {
            for action in [
                RebaseAction::Continue,
                RebaseAction::Skip,
                RebaseAction::Abort,
            ] {
                let f = fixture("sha1", true);
                let original = oid(&f, "HEAD");
                let r = apply(&f, &preview(&f, RebaseAction::Start));
                assert_eq!(r.failure, Some(RebaseFailure::ConflictsRemain));
                assert!(r.state.unwrap().rebase);
                refusal(
                    f.repo()
                        .preview_rebase(RebaseAction::Continue, "", &CancellationToken::new()),
                    RebaseFailure::ConflictsRemain,
                );
                f.write("private", "keep\n");
                f.write(FILE, "resolved 中文\n");
                if action == RebaseAction::Continue {
                    f.run(&["add", "--", FILE]);
                }
                let p = preview(&f, action);
                assert!(!p.patch().is_empty());
                assert!(p.patch().windows(FILE.len()).any(|w| w == FILE.as_bytes()));
                f.write(FILE, "external\n");
                refusal(
                    f.repo().apply_rebase(&p, &CancellationToken::new()),
                    if action == RebaseAction::Continue {
                        RebaseFailure::DirtyWorktree
                    } else {
                        RebaseFailure::StalePreview
                    },
                );
                f.write(FILE, "resolved 中文\n");
                let p = preview(&f, action);
                ok(apply(&f, &p));
                assert_eq!(fs::read(f.repository.join("private")).unwrap(), b"keep\n");
                assert_eq!(text(&f, &["branch", "--show-current"]), "feature/検索");
                match action {
                    RebaseAction::Abort => assert_eq!(oid(&f, "HEAD"), original),
                    RebaseAction::Skip => {
                        assert_eq!(fs::read(f.repository.join(FILE)).unwrap(), b"main\n")
                    }
                    _ => assert_eq!(
                        fs::read(f.repository.join(FILE)).unwrap(),
                        "resolved 中文\n".as_bytes()
                    ),
                }
            }
        },
    );
}
#[test]
fn skip_can_pause_again_and_continue_the_remaining_original_plan() {
    isolated(
        "ROCKET_REBASE_CASE",
        "skip_can_pause_again_and_continue_the_remaining_original_plan",
        || {
            let f = fixture("sha1", true);
            f.run(&["switch", "main"]);
            f.write("second", "upstream second\n");
            f.commit("second conflict");
            f.run(&["switch", "feature/検索"]);
            let original = oid(&f, "HEAD");
            let first = apply(&f, &preview(&f, RebaseAction::Start));
            assert_eq!(first.failure, Some(RebaseFailure::ConflictsRemain));
            let second = apply(&f, &preview(&f, RebaseAction::Skip));
            assert_eq!(second.failure, Some(RebaseFailure::ConflictsRemain));
            assert!(second.state.unwrap().rebase);
            f.write("second", "resolved second\n");
            f.run(&["add", "--", "second"]);
            let continuation = preview(&f, RebaseAction::Continue);
            ok(apply(&f, &continuation));
            assert_eq!(fs::read(f.repository.join(FILE)).unwrap(), b"main\n");
            assert_eq!(
                fs::read(f.repository.join("second")).unwrap(),
                b"resolved second\n"
            );
            assert_eq!(text(&f, &["rev-list", "--count", "main..HEAD"]), "1");
            assert_eq!(oid(&f, &continuation.recovery_reference()), original);
        },
    );
}
#[test]
fn stale_destination_branch_index_and_foreign_preview_are_refused() {
    isolated(
        "ROCKET_REBASE_CASE",
        "stale_destination_branch_index_and_foreign_preview_are_refused",
        || {
            let f = fixture("sha1", false);
            let p = preview(&f, RebaseAction::Start);
            let other = fixture("sha1", false);
            refusal(
                other.repo().apply_rebase(&p, &CancellationToken::new()),
                RebaseFailure::WrongRepository,
            );
            f.run(&["update-ref", "refs/heads/main", "HEAD"]);
            refusal(
                f.repo().apply_rebase(&p, &CancellationToken::new()),
                RebaseFailure::StalePreview,
            );
            let p = preview(&f, RebaseAction::Start);
            f.run(&["switch", "-c", "elsewhere"]);
            refusal(
                f.repo().apply_rebase(&p, &CancellationToken::new()),
                RebaseFailure::StalePreview,
            );
            f.write(FILE, "dirty\n");
            refusal(
                f.repo()
                    .preview_rebase(RebaseAction::Start, "main", &CancellationToken::new()),
                RebaseFailure::DirtyWorktree,
            );
            f.run(&["add", "--", FILE]);
            refusal(
                f.repo().apply_rebase(&p, &CancellationToken::new()),
                RebaseFailure::DirtyWorktree,
            );
        },
    );
}
#[test]
fn empty_replay_stops_and_unsafe_recovery_metadata_is_refused() {
    isolated(
        "ROCKET_REBASE_CASE",
        "empty_replay_stops_and_unsafe_recovery_metadata_is_refused",
        || {
            let f = fixture("sha1", true);
            f.run(&["switch", "main"]);
            f.write(FILE, "feature\n");
            f.commit("same change");
            f.run(&["switch", "feature/検索"]);
            let r = apply(&f, &preview(&f, RebaseAction::Start));
            assert_eq!(r.failure, Some(RebaseFailure::EmptyChange), "{r:?}");
            refusal(
                f.repo()
                    .preview_rebase(RebaseAction::Continue, "", &CancellationToken::new()),
                RebaseFailure::EmptyChange,
            );
            let todo = f.repository.join(".git/rebase-merge/git-rebase-todo");
            let saved = fs::read(&todo).unwrap();
            fs::write(&todo, b"exec echo PRIVATE_COMMAND\n").unwrap();
            refusal(
                f.repo()
                    .preview_rebase(RebaseAction::Skip, "", &CancellationToken::new()),
                RebaseFailure::UnsupportedSequence,
            );
            fs::write(
                &todo,
                format!("pick {} unexpected replacement\n", oid(&f, "main")),
            )
            .unwrap();
            refusal(
                f.repo()
                    .preview_rebase(RebaseAction::Skip, "", &CancellationToken::new()),
                RebaseFailure::UnsupportedSequence,
            );
            fs::write(&todo, &saved).unwrap();
            let done = f.repository.join(".git/rebase-merge/done");
            let saved_done = fs::read(&done).unwrap();
            fs::write(&done, &saved).unwrap();
            fs::write(&todo, &saved_done).unwrap();
            refusal(
                f.repo()
                    .preview_rebase(RebaseAction::Abort, "", &CancellationToken::new()),
                RebaseFailure::UnsupportedSequence,
            );
            fs::write(&done, saved_done).unwrap();
            fs::write(&todo, saved).unwrap();
            let p = preview(&f, RebaseAction::Skip);
            f.run(&["update-ref", "refs/heads/feature/検索", "main"]);
            refusal(
                f.repo().apply_rebase(&p, &CancellationToken::new()),
                RebaseFailure::StalePreview,
            );
            f.run(&[
                "update-ref",
                "refs/heads/feature/検索",
                p.original().as_str(),
            ]);
            ok(apply(&f, &preview(&f, RebaseAction::Skip)));
            assert_eq!(text(&f, &["rev-list", "--count", "main..HEAD"]), "1");
        },
    );
}
#[test]
fn fast_forward_merge_and_unrelated_history_have_explicit_previews() {
    isolated(
        "ROCKET_REBASE_CASE",
        "fast_forward_merge_and_unrelated_history_have_explicit_previews",
        || {
            let f = fixture("sha1", false);
            f.run(&["switch", "-c", "behind", "main~"]);
            let p = preview(&f, RebaseAction::Start);
            assert!(p.fast_forward());
            ok(apply(&f, &p));
            assert_eq!(oid(&f, "HEAD"), oid(&f, "main"));
            f.run(&["switch", "feature/検索"]);
            f.run(&["merge", "--no-edit", "main"]);
            f.run(&["switch", "main"]);
            f.write("next", "next");
            f.commit("next");
            f.run(&["switch", "feature/検索"]);
            refusal(
                f.repo()
                    .preview_rebase(RebaseAction::Start, "main", &CancellationToken::new()),
                RebaseFailure::MergeCommits,
            );
            f.run(&["switch", "--orphan", "orphan"]);
            f.write("root", "root");
            f.commit("other root");
            refusal(
                f.repo()
                    .preview_rebase(RebaseAction::Start, "main", &CancellationToken::new()),
                RebaseFailure::UnrelatedHistory,
            );
        },
    );
}
#[test]
fn late_cancellation_reports_moved_branch_and_refuses_automatic_recovery() {
    isolated(
        "ROCKET_REBASE_CASE",
        "late_cancellation_reports_moved_branch_and_refuses_automatic_recovery",
        || {
            let f = fixture("sha1", false);
            let original = oid(&f, "HEAD");
            fs::write(
                f.root.join("no-hooks/post-rewrite"),
                b"#!/bin/sh\necho ready > late-ready\nsleep 30\n",
            )
            .unwrap();
            let repo = f.repo();
            let p = preview(&f, RebaseAction::Start);
            let recovery = p.recovery_reference();
            let task = WriteTask::spawn(move |cancel| repo.apply_rebase(&p, &cancel)).unwrap();
            let start = Instant::now();
            while !f.repository.join("late-ready").exists() {
                assert!(start.elapsed() < Duration::from_secs(20));
                std::thread::sleep(Duration::from_millis(30));
            }
            let applied = oid(&f, "HEAD");
            assert_ne!(applied, original);
            task.cancel();
            let r = task.wait().unwrap();
            assert!(matches!(r.write.result, Err(Error::Cancelled)), "{r:?}");
            assert_eq!(r.failure, Some(RebaseFailure::RecoveryBranchMoved));
            let state = r.state.unwrap();
            assert!(state.rebase);
            assert_eq!(state.step.unwrap().kind, StepKind::BranchMoved);
            for action in [
                RebaseAction::Continue,
                RebaseAction::Skip,
                RebaseAction::Abort,
            ] {
                refusal(
                    f.repo()
                        .preview_rebase(action, "", &CancellationToken::new()),
                    RebaseFailure::RecoveryBranchMoved,
                );
            }
            assert_eq!(oid(&f, "HEAD"), applied);
            assert_eq!(oid(&f, &recovery), original);
        },
    );
}
/// The step says what the review will refuse, whichever stop the rebase is at and
/// however Git stores the branch.
#[test]
fn a_branch_moved_during_a_stop_is_named_in_the_step() {
    isolated(
        "ROCKET_REBASE_CASE",
        "a_branch_moved_during_a_stop_is_named_in_the_step",
        || {
            let f = fixture("sha1", true);
            let original = oid(&f, "HEAD");
            let step = |f: &Fixture| {
                let state = f.repo().operation_state(&CancellationToken::new()).unwrap();
                state.step.unwrap().kind
            };
            let r = apply(&f, &preview(&f, RebaseAction::Start));
            assert_eq!(r.failure, Some(RebaseFailure::ConflictsRemain));
            assert_eq!(step(&f), StepKind::Commit, "the branch is where it was");
            f.run(&[
                "update-ref",
                "refs/heads/feature/検索",
                &format!("{original}~1"),
            ]);
            f.run(&["pack-refs", "--all"]);
            assert_eq!(step(&f), StepKind::BranchMoved);
            refusal(
                f.repo()
                    .preview_rebase(RebaseAction::Abort, "", &CancellationToken::new()),
                RebaseFailure::RecoveryBranchMoved,
            );
        },
    );
}
#[test]
fn ignored_collision_locks_hooks_and_cancel_preserve_data() {
    isolated(
        "ROCKET_REBASE_CASE",
        "ignored_collision_locks_hooks_and_cancel_preserve_data",
        || {
            let f = fixture("sha1", false);
            f.run(&["switch", "main"]);
            f.write("ignored/file", "tracked upstream");
            f.run(&["add", "--force", "ignored/file"]);
            f.run(&["commit", "-m", "tracked"]);
            f.run(&["switch", "feature/検索"]);
            f.write("ignored/file", "private data");
            refusal(
                f.repo()
                    .preview_rebase(RebaseAction::Start, "main", &CancellationToken::new()),
                RebaseFailure::LocalFileCollision,
            );
            assert_eq!(
                fs::read(f.repository.join("ignored/file")).unwrap(),
                b"private data"
            );
            fs::remove_file(f.repository.join("ignored/file")).unwrap();
            let p = preview(&f, RebaseAction::Start);
            let original = oid(&f, "HEAD");
            let lock = f.repository.join(".git/index.lock");
            fs::write(&lock, b"existing lock").unwrap();
            let r = apply(&f, &p);
            assert_eq!(r.failure, Some(RebaseFailure::IndexLocked));
            assert_eq!(fs::read(&lock).unwrap(), b"existing lock");
            fs::remove_file(lock).unwrap();
            let hook = f.root.join("no-hooks/pre-rebase");
            fs::write(&hook, b"#!/bin/sh\necho PRIVATE_HOOK_PAYLOAD >&2\nexit 1\n").unwrap();
            let r = apply(&f, &preview(&f, RebaseAction::Start));
            assert!(r.write.result.is_err());
            assert_eq!(oid(&f, "HEAD"), original);
            fs::write(&hook, b"#!/bin/sh\necho ready > hook-ready\nsleep 30\n").unwrap();
            let repo = f.repo();
            let p = preview(&f, RebaseAction::Start);
            let task = WriteTask::spawn(move |cancel| repo.apply_rebase(&p, &cancel)).unwrap();
            let start = Instant::now();
            while !f.repository.join("hook-ready").exists() {
                assert!(start.elapsed() < Duration::from_secs(15));
                std::thread::sleep(Duration::from_millis(30))
            }
            task.cancel();
            let r = task.wait().unwrap();
            assert!(matches!(r.write.result, Err(Error::Cancelled)), "{r:?}");
            assert!(r.state.is_ok());
            assert_eq!(oid(&f, "HEAD"), original);
        },
    );
}
