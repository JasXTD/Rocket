mod common;
use common::{Fixture, isolated};
use rocket_git::*;
use std::fs;
fn token() -> CancellationToken {
    CancellationToken::new()
}
fn fixture(format: &str) -> (Fixture, Vec<ObjectId>) {
    let f = Fixture::with_format(format);
    f.write(".gitignore", "build/\nignored.txt\n");
    let mut ids = Vec::new();
    for n in 0..7 {
        f.write("路径 文件.rs", format!("version {n}\n"));
        f.commit("PRIVATE_BISECT");
        ids.push(f.repo().resolve_commit("HEAD", &token()).unwrap());
    }
    (f, ids)
}
fn review(repo: &Repository, action: BisectAction) -> BisectPreview {
    repo.preview_bisect(action, "", "", &token()).unwrap()
}
fn apply(repo: &Repository, p: BisectPreview) -> Option<BisectSession> {
    let report = repo.apply_bisect(&p, &token()).unwrap();
    report.result.unwrap();
    report.status.unwrap();
    report.state.unwrap();
    report.session.unwrap()
}
fn start(repo: &Repository, ids: &[ObjectId]) -> BisectSession {
    let p = repo
        .preview_bisect(
            BisectAction::Start,
            ids[0].as_str(),
            ids.last().unwrap().as_str(),
            &token(),
        )
        .unwrap();
    apply(repo, p).unwrap()
}
fn refused<T: std::fmt::Debug>(r: Result<T>, failure: BisectFailure) {
    assert!(matches!(&r,Err(Error::Bisect(f)) if *f==failure), "{r:?}");
}
fn wait_for_hook(task: &mut WriteTask<BisectWriteReport>, path: &std::path::Path) {
    let started = std::time::Instant::now();
    while !path.exists() {
        if let Some(result) = task.try_finish() {
            panic!("Bisect finished before reaching the checkout hook: {result:?}");
        }
        // This includes discovery, review validation and the switch command.
        // Match the native acceptance runner's whole-operation deadline; it is
        // not the latency allowed to stop an already running hook.
        assert!(
            started.elapsed().as_secs() < 90,
            "Bisect did not reach its checkout hook"
        );
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
}

#[test]
fn good_bad_find_exact_commit_and_restore_sha1_sha256() {
    isolated(
        "ROCKET_BISECT_TEST",
        "good_bad_find_exact_commit_and_restore_sha1_sha256",
        || {
            for format in ["sha1", "sha256"] {
                let (f, ids) = fixture(format);
                let repo = f.repo();
                let original = repo.head(&token()).unwrap();
                f.write("scratch 文", "keep untracked");
                f.write("build/output", "keep ignored");
                let mut s = start(&repo, &ids);
                let recovery = s.recovery_reference.clone();
                for _ in 0..8 {
                    if s.phase == BisectPhase::Found {
                        break;
                    }
                    assert!(s.can_mark(), "{s:?}");
                    let n = ids
                        .iter()
                        .position(|id| Some(id) == s.target.as_ref())
                        .unwrap();
                    s = apply(
                        &repo,
                        review(
                            &repo,
                            if n < 4 {
                                BisectAction::Good
                            } else {
                                BisectAction::Bad
                            },
                        ),
                    )
                    .unwrap();
                }
                assert_eq!(s.phase, BisectPhase::Found);
                assert_eq!(s.target, Some(ids[4].clone()));
                assert!(repo.operation_state(&token()).unwrap().bisect);
                assert!(!s.decisions.is_empty());
                assert!(apply(&repo, review(&repo, BisectAction::Restore)).is_none());
                assert_eq!(repo.head(&token()).unwrap(), original);
                assert!(repo.operation_state(&token()).unwrap().is_idle());
                assert_eq!(
                    fs::read_to_string(f.repository.join("scratch 文")).unwrap(),
                    "keep untracked"
                );
                assert_eq!(
                    fs::read_to_string(f.repository.join("build/output")).unwrap(),
                    "keep ignored"
                );
                assert!(repo.resolve_commit(&recovery, &token()).is_err());
            }
        },
    );
}
#[test]
fn skipped_candidates_are_inconclusive_and_restore_detached() {
    isolated(
        "ROCKET_BISECT_TEST",
        "skipped_candidates_are_inconclusive_and_restore_detached",
        || {
            let (f, ids) = fixture("sha1");
            f.run(&["switch", "--detach", ids[6].as_str()]);
            let repo = f.repo();
            let original = repo.head(&token()).unwrap();
            let mut s = start(&repo, &ids);
            for _ in 0..8 {
                if s.phase == BisectPhase::Inconclusive {
                    break;
                }
                s = apply(&repo, review(&repo, BisectAction::Skip)).unwrap();
            }
            assert_eq!(s.phase, BisectPhase::Inconclusive);
            assert!(s.candidates.len() > 1);
            refused(
                repo.preview_bisect(BisectAction::Good, "", "", &token()),
                BisectFailure::NotReady,
            );
            apply(&repo, review(&repo, BisectAction::Restore));
            assert_eq!(repo.head(&token()).unwrap(), original);
        },
    );
}
#[test]
fn protects_staged_unstaged_and_hidden_edits_and_stale_reviews() {
    isolated(
        "ROCKET_BISECT_TEST",
        "protects_staged_unstaged_and_hidden_edits_and_stale_reviews",
        || {
            let (f, ids) = fixture("sha1");
            let repo = f.repo();
            let p = repo
                .preview_bisect(
                    BisectAction::Start,
                    ids[0].as_str(),
                    ids[6].as_str(),
                    &token(),
                )
                .unwrap();
            f.write("路径 文件.rs", "local");
            f.run(&["add", "."]);
            f.write("路径 文件.rs", "unstaged");
            let index = fs::read(f.repository.join(".git/index")).unwrap();
            refused(
                repo.apply_bisect(&p, &token()),
                BisectFailure::DirtyWorktree,
            );
            assert_eq!(fs::read(f.repository.join(".git/index")).unwrap(), index);
            assert!(!f.repository.join(".git/rocket-bisect").exists());
            f.run(&["restore", "--staged", "--worktree", "--", "路径 文件.rs"]);
            f.run(&["update-index", "--assume-unchanged", "--", "路径 文件.rs"]);
            refused(
                repo.preview_bisect(
                    BisectAction::Start,
                    ids[0].as_str(),
                    ids[6].as_str(),
                    &token(),
                ),
                BisectFailure::UnsupportedIndex,
            );
            f.run(&[
                "update-index",
                "--no-assume-unchanged",
                "--",
                "路径 文件.rs",
            ]);
            start(&repo, &ids);
            let p = review(&repo, BisectAction::Good);
            apply(&repo, review(&repo, BisectAction::Skip));
            refused(repo.apply_bisect(&p, &token()), BisectFailure::StalePreview);
            f.write("路径 文件.rs", "local during bisect");
            refused(
                repo.preview_bisect(BisectAction::Restore, "", "", &token()),
                BisectFailure::DirtyWorktree,
            );
            assert_eq!(
                fs::read_to_string(f.repository.join("路径 文件.rs")).unwrap(),
                "local during bisect"
            );
            assert!(repo.operation_state(&token()).unwrap().bisect);
        },
    );
}
#[test]
fn checkout_collision_records_once_then_resume_protects_ignored_file() {
    isolated(
        "ROCKET_BISECT_TEST",
        "checkout_collision_records_once_then_resume_protects_ignored_file",
        || {
            let (f, ids) = fixture("sha1");
            let repo = f.repo();
            f.run(&["switch", "--detach", ids[3].as_str()]);
            f.write("ignored.txt", "historical");
            f.run(&["add", "-f", "ignored.txt"]);
            f.run(&["commit", "-m", "historical ignored"]);
            let bad = repo.resolve_commit("HEAD", &token()).unwrap();
            f.run(&["switch", "main"]);
            f.write("ignored.txt", "PRIVATE_LOCAL");
            // Adjacent commits force the found result to this colliding tree.
            let p = repo
                .preview_bisect(BisectAction::Start, ids[3].as_str(), bad.as_str(), &token())
                .unwrap();
            let r = repo.apply_bisect(&p, &token()).unwrap();
            refused(r.result, BisectFailure::CheckoutBlocked);
            assert_eq!(
                r.session.unwrap().unwrap().phase,
                BisectPhase::CheckoutPending
            );
            assert_eq!(
                fs::read_to_string(f.repository.join("ignored.txt")).unwrap(),
                "PRIVATE_LOCAL"
            );
            // Moving the user's file is explicit test setup, never an app fallback.
            fs::create_dir_all(f.repository.join("build")).unwrap();
            fs::rename(
                f.repository.join("ignored.txt"),
                f.repository.join("build/kept.txt"),
            )
            .unwrap();
            let s = apply(&repo, review(&repo, BisectAction::Resume)).unwrap();
            assert_eq!(s.phase, BisectPhase::Found);
            assert_eq!(s.target, Some(bad));
            apply(&repo, review(&repo, BisectAction::Restore));
            assert_eq!(
                fs::read_to_string(f.repository.join("build/kept.txt")).unwrap(),
                "PRIVATE_LOCAL"
            );
        },
    );
}
#[test]
fn moved_original_branch_is_never_reset() {
    isolated(
        "ROCKET_BISECT_TEST",
        "moved_original_branch_is_never_reset",
        || {
            let (f, ids) = fixture("sha1");
            let repo = f.repo();
            let s = start(&repo, &ids);
            f.run(&["update-ref", "refs/heads/main", ids[0].as_str()]);
            refused(
                repo.preview_bisect(BisectAction::Restore, "", "", &token()),
                BisectFailure::OriginalBranchMoved,
            );
            assert_eq!(repo.resolve_commit("main", &token()).unwrap(), ids[0]);
            assert_eq!(
                repo.resolve_commit(&s.recovery_reference, &token())
                    .unwrap(),
                ids[6]
            );
            // Return the test ref to its original value and recover normally.
            f.run(&["update-ref", "refs/heads/main", ids[6].as_str()]);
            // A replaced recovery ref must never cause cleanup to delete the
            // branch to which an external symbolic ref now points.
            f.run(&["update-ref", "-d", &s.recovery_reference]);
            f.run(&["symbolic-ref", &s.recovery_reference, "refs/heads/main"]);
            let r = repo
                .apply_bisect(&review(&repo, BisectAction::Restore), &token())
                .unwrap();
            refused(r.result, BisectFailure::JournalChanged);
            assert_eq!(repo.resolve_commit("main", &token()).unwrap(), ids[6]);
            assert!(f.repository.join(".git/rocket-bisect").exists());
            f.run(&["symbolic-ref", "--delete", &s.recovery_reference]);
            f.run(&["update-ref", &s.recovery_reference, ids[6].as_str()]);
            apply(&repo, review(&repo, BisectAction::Restore));
        },
    );
}
#[test]
fn rejects_foreign_sessions_invalid_ranges_and_existing_locks() {
    isolated(
        "ROCKET_BISECT_TEST",
        "rejects_foreign_sessions_invalid_ranges_and_existing_locks",
        || {
            let (f, ids) = fixture("sha1");
            let repo = f.repo();
            refused(
                repo.preview_bisect(
                    BisectAction::Start,
                    "PRIVATE_MISSING",
                    ids[6].as_str(),
                    &token(),
                ),
                BisectFailure::RevisionUnavailable,
            );
            f.run(&["tag", "known-good", ids[0].as_str()]);
            let p = repo
                .preview_bisect(BisectAction::Start, "known-good", ids[6].as_str(), &token())
                .unwrap();
            f.run(&["update-ref", "refs/tags/known-good", ids[1].as_str()]);
            refused(repo.apply_bisect(&p, &token()), BisectFailure::StalePreview);
            assert!(!f.repository.join(".git/rocket-bisect").exists());
            refused(
                repo.preview_bisect(
                    BisectAction::Start,
                    ids[6].as_str(),
                    ids[0].as_str(),
                    &token(),
                ),
                BisectFailure::InvalidRange,
            );
            fs::write(
                f.repository.join(".git/index.lock"),
                "owned by another tool",
            )
            .unwrap();
            refused(
                repo.preview_bisect(
                    BisectAction::Start,
                    ids[0].as_str(),
                    ids[6].as_str(),
                    &token(),
                ),
                BisectFailure::IndexLocked,
            );
            assert_eq!(
                fs::read_to_string(f.repository.join(".git/index.lock")).unwrap(),
                "owned by another tool"
            );
            fs::remove_file(f.repository.join(".git/index.lock")).unwrap();
            f.run(&["bisect", "start", ids[6].as_str(), ids[0].as_str()]);
            refused(repo.bisect_session(&token()), BisectFailure::ForeignSession);
            refused(
                repo.preview_bisect(BisectAction::Restore, "", "", &token()),
                BisectFailure::ForeignSession,
            );
            f.run(&["bisect", "reset"]);
            let p = repo
                .preview_bisect(
                    BisectAction::Start,
                    ids[0].as_str(),
                    ids[6].as_str(),
                    &token(),
                )
                .unwrap();
            let c = token();
            c.cancel();
            assert!(matches!(repo.apply_bisect(&p, &c), Err(Error::Cancelled)));
            assert!(!f.repository.join(".git/rocket-bisect").exists());
        },
    );
}

#[test]
fn cancelled_checkout_reconciles_and_original_restore_can_be_retried() {
    isolated(
        "ROCKET_BISECT_TEST",
        "cancelled_checkout_reconciles_and_original_restore_can_be_retried",
        || {
            let (f, ids) = fixture("sha1");
            let repo = f.repo();
            let original = repo.head(&token()).unwrap();
            let hooks = f.root.join("slow-hooks");
            fs::create_dir(&hooks).unwrap();
            let hook = hooks.join("post-checkout");
            fs::write(
                &hook,
                "#!/bin/sh\nprintf ready > .git/bisect-ready\nsleep 30\n",
            )
            .unwrap();
            f.run(&["config", "core.hooksPath", hooks.to_str().unwrap()]);
            let p = repo
                .preview_bisect(
                    BisectAction::Start,
                    ids[0].as_str(),
                    ids[6].as_str(),
                    &token(),
                )
                .unwrap();
            let writer = repo.clone();
            let mut task = WriteTask::spawn(move |c| writer.apply_bisect(&p, &c)).unwrap();
            let ready = f.repository.join(".git/bisect-ready");
            wait_for_hook(&mut task, &ready);
            task.cancel();
            let report = task.wait().unwrap();
            assert!(matches!(report.result, Err(Error::Cancelled)));
            assert_eq!(report.session.unwrap().unwrap().phase, BisectPhase::Testing);
            fs::remove_file(&hook).unwrap();
            refused(
                repo.commit("not during bisect", &token()),
                BisectFailure::ActiveOperation,
            );
            // Interrupt restoration after Git has already switched back. Reopen the
            // repository and retry cleanup without losing the durable return record.
            fs::remove_file(&ready).unwrap();
            fs::write(
                &hook,
                "#!/bin/sh\nprintf ready > .git/bisect-ready\nsleep 30\n",
            )
            .unwrap();
            let p = review(&repo, BisectAction::Restore);
            let writer = repo.clone();
            let mut task = WriteTask::spawn(move |c| writer.apply_bisect(&p, &c)).unwrap();
            wait_for_hook(&mut task, &ready);
            task.cancel();
            let report = task.wait().unwrap();
            assert!(matches!(report.result, Err(Error::Cancelled)));
            assert_eq!(repo.head(&token()).unwrap(), original);
            assert!(f.repository.join(".git/rocket-bisect").exists());
            fs::remove_file(hook).unwrap();
            let reopened = f.repo();
            assert!(apply(&reopened, review(&reopened, BisectAction::Restore)).is_none());
            assert_eq!(reopened.head(&token()).unwrap(), original);
            fs::write(hooks.join("post-checkout"), "#!/bin/sh\nexit 7\n").unwrap();
            let p = reopened
                .preview_bisect(
                    BisectAction::Start,
                    ids[0].as_str(),
                    ids[6].as_str(),
                    &token(),
                )
                .unwrap();
            let r = reopened.apply_bisect(&p, &token()).unwrap();
            refused(r.result, BisectFailure::CommandFailed);
            assert_eq!(r.session.unwrap().unwrap().phase, BisectPhase::Testing);
            fs::remove_file(hooks.join("post-checkout")).unwrap();
            apply(&reopened, review(&reopened, BisectAction::Restore));
            // A successful hook may itself move HEAD. This is a post-write
            // failure, not a stale preview claiming that nothing was applied.
            fs::write(
                hooks.join("post-checkout"),
                format!("#!/bin/sh\ngit update-ref --no-deref HEAD {}\n", ids[6]),
            )
            .unwrap();
            let p = reopened
                .preview_bisect(
                    BisectAction::Start,
                    ids[0].as_str(),
                    ids[6].as_str(),
                    &token(),
                )
                .unwrap();
            let r = reopened.apply_bisect(&p, &token()).unwrap();
            refused(r.result, BisectFailure::CommandFailed);
            assert_eq!(reopened.resolve_commit("HEAD", &token()).unwrap(), ids[6]);
            assert_eq!(
                r.session.unwrap().unwrap().phase,
                BisectPhase::CheckoutPending
            );
            fs::remove_file(hooks.join("post-checkout")).unwrap();
            // Explicit fixture repair of the index/files left by its own hook.
            f.run(&["restore", "--staged", "--worktree", "--", "路径 文件.rs"]);
            apply(&reopened, review(&reopened, BisectAction::Restore));
        },
    );
}
#[test]
fn linked_worktree_bisect_is_local_and_new_detached_commits_are_protected() {
    isolated(
        "ROCKET_BISECT_TEST",
        "linked_worktree_bisect_is_local_and_new_detached_commits_are_protected",
        || {
            let (f, ids) = fixture("sha1");
            let repo = f.repo();
            let linked = f.root.join("linked 中文");
            f.run(&[
                "worktree",
                "add",
                "-b",
                "feature/検索",
                linked.to_str().unwrap(),
            ]);
            let other = f.git.open(&linked, &token()).unwrap();
            let original = other.head(&token()).unwrap();
            start(&other, &ids);
            assert!(!repo.operation_state(&token()).unwrap().bisect);
            assert!(other.operation_state(&token()).unwrap().bisect);
            assert!(!f.repository.join(".git/rocket-bisect").exists());
            apply(&other, review(&other, BisectAction::Restore));
            assert_eq!(other.head(&token()).unwrap(), original);
            let s = start(&repo, &ids);
            f.write("new work", "save me");
            f.commit("new detached work");
            refused(
                repo.preview_bisect(BisectAction::Restore, "", "", &token()),
                BisectFailure::UnrecordedCheckout,
            );
            f.run(&["branch", "saved-work"]);
            f.run(&["switch", "--detach", s.target.unwrap().as_str()]);
            apply(&repo, review(&repo, BisectAction::Restore));
            assert!(repo.resolve_commit("saved-work", &token()).is_ok());
        },
    );
}

#[test]
fn interrupted_setup_without_engine_or_recovery_ref_can_restore() {
    isolated(
        "ROCKET_BISECT_TEST",
        "interrupted_setup_without_engine_or_recovery_ref_can_restore",
        || {
            let (f, ids) = fixture("sha1");
            let repo = f.repo();
            let original = repo.head(&token()).unwrap();
            // Durable format at the point between journal sync and ref creation.
            let journal = format!(
                "ROCKET-BISECT-1\0main\0{}\0{}\0{}\0refs/rocket/bisect/1-2",
                ids[6], ids[0], ids[6]
            );
            fs::write(f.repository.join(".git/rocket-bisect"), journal).unwrap();
            assert_eq!(
                repo.bisect_session(&token()).unwrap().unwrap().phase,
                BisectPhase::RestoreOnly
            );
            assert!(apply(&repo, review(&repo, BisectAction::Restore)).is_none());
            assert_eq!(repo.head(&token()).unwrap(), original);
        },
    );
}

#[test]
fn exact_operation_refs_and_reftable_bisect_state_are_observed() {
    isolated(
        "ROCKET_BISECT_TEST",
        "exact_operation_refs_and_reftable_bisect_state_are_observed",
        || {
            let (f, ids) = fixture("sha1");
            let repo = f.repo();
            let refs = ["CHERRY_PICK_HEAD", "REVERT_HEAD", "MERGE_AUTOSTASH"];
            for name in refs {
                f.run(&["branch", name, ids[6].as_str()]);
            }
            for storage in ["files", "reftable"] {
                if storage == "reftable" {
                    f.run(&["refs", "migrate", "--ref-format=reftable"]);
                }
                // Similarly named branches must not be mistaken for operation refs.
                repo.preview_bisect(
                    BisectAction::Start,
                    ids[0].as_str(),
                    ids[6].as_str(),
                    &token(),
                )
                .unwrap();
                for name in refs {
                    f.run(&["update-ref", name, ids[6].as_str()]);
                    refused(
                        repo.preview_bisect(
                            BisectAction::Start,
                            ids[0].as_str(),
                            ids[6].as_str(),
                            &token(),
                        ),
                        BisectFailure::ActiveOperation,
                    );
                    f.run(&["update-ref", "-d", name, ids[6].as_str()]);
                }
                let original = repo.head(&token()).unwrap();
                let s = start(&repo, &ids);
                assert_eq!(s.phase, BisectPhase::Testing);
                assert_eq!(
                    Some(repo.resolve_commit("HEAD", &token()).unwrap()),
                    s.target
                );
                apply(&repo, review(&repo, BisectAction::Good));
                assert!(apply(&repo, review(&repo, BisectAction::Restore)).is_none());
                assert_eq!(repo.head(&token()).unwrap(), original);
            }
        },
    );
}
