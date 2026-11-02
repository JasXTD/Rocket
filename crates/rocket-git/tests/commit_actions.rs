mod common;
use common::{Fixture, isolated};
use rocket_git::*;
use std::{
    fs,
    time::{Duration, Instant},
};

fn request(operation: CommitOperation, target: Option<ObjectId>) -> CommitActionRequest {
    CommitActionRequest {
        operation,
        targets: target.into_iter().collect(),
        mainline: None,
        recovery: None,
    }
}
fn recover(operation: CommitOperation, recovery: CommitRecovery) -> CommitActionRequest {
    CommitActionRequest {
        recovery: Some(recovery),
        ..request(operation, None)
    }
}
fn oid(f: &Fixture, revision: &str) -> ObjectId {
    f.repo()
        .resolve_commit(revision, &CancellationToken::new())
        .unwrap()
}
fn text(f: &Fixture, args: &[&str]) -> String {
    String::from_utf8(f.run(args)).unwrap().trim_end().into()
}
fn preview(f: &Fixture, request: CommitActionRequest) -> CommitPreview {
    f.repo()
        .preview_commit_action(request, &CancellationToken::new())
        .unwrap()
}
fn apply(f: &Fixture, p: &CommitPreview, message: Option<&str>) -> CommitWriteReport {
    f.repo()
        .apply_commit_action(p, message, &CancellationToken::new())
        .unwrap()
}
fn ok(report: CommitWriteReport) {
    assert!(report.write.result.is_ok(), "{report:?}");
    assert!(report.state.unwrap().is_idle());
    assert!(report.branches.is_ok());
}
fn refused<T: std::fmt::Debug>(result: Result<T>, failure: CommitFailure) {
    assert!(
        matches!(result, Err(Error::CommitAction(actual)) if actual == failure),
        "{result:?}"
    );
}

#[test]
fn amend_preserves_unstaged_files_author_and_recovery_ref() {
    isolated(
        "ROCKET_COMMIT_ACTION_CASE",
        "amend_preserves_unstaged_files_author_and_recovery_ref",
        || {
            let f = Fixture::new();
            f.write("file.txt", "old\n");
            f.commit("original 中文");
            let old = oid(&f, "HEAD");
            let author = text(&f, &["show", "-s", "--format=%an <%ae> %at"]);
            f.write("file.txt", "staged\n");
            f.run(&["add", "file.txt"]);
            f.write("file.txt", "unstaged\n");
            f.write("private.txt", "keep\n");
            let config = fs::read(f.repository.join(".git/config")).unwrap();
            let p = preview(&f, request(CommitOperation::Amend, None));
            assert!(String::from_utf8_lossy(p.patch()).contains("+staged"));
            ok(apply(&f, &p, Some("revised 中文\n\n# literal description")));
            assert_ne!(oid(&f, "HEAD"), old);
            assert_eq!(oid(&f, &p.recovery_reference().unwrap()), old);
            assert_eq!(text(&f, &["show", "HEAD:file.txt"]), "staged");
            assert_eq!(text(&f, &["show", "-s", "--format=%an <%ae> %at"]), author);
            assert_eq!(
                fs::read(f.repository.join("file.txt")).unwrap(),
                b"unstaged\n"
            );
            assert_eq!(
                fs::read(f.repository.join("private.txt")).unwrap(),
                b"keep\n"
            );
            assert_eq!(fs::read(f.repository.join(".git/config")).unwrap(), config);
            let p = preview(&f, request(CommitOperation::Amend, None));
            ok(apply(&f, &p, Some("message only")));
            assert_eq!(text(&f, &["show", "HEAD:file.txt"]), "staged");
            let p = preview(&f, request(CommitOperation::Amend, None));
            let current = oid(&f, "HEAD");
            let reference = p.recovery_reference().unwrap();
            f.run(&["update-ref", &reference, old.as_str()]);
            refused(
                f.repo().apply_commit_action(
                    &p,
                    Some("must not replace backup"),
                    &CancellationToken::new(),
                ),
                CommitFailure::RecoveryReferenceConflict,
            );
            assert_eq!(oid(&f, "HEAD"), current);
            assert_eq!(oid(&f, &reference), old);
        },
    );
}

#[test]
fn stale_or_wrong_preview_cannot_amend_a_different_index_or_branch() {
    isolated(
        "ROCKET_COMMIT_ACTION_CASE",
        "stale_or_wrong_preview_cannot_amend_a_different_index_or_branch",
        || {
            let f = Fixture::new();
            f.write("file.txt", "base\n");
            f.commit("base");
            let p = preview(&f, request(CommitOperation::Amend, None));
            f.write("file.txt", "new\n");
            f.run(&["add", "file.txt"]);
            refused(
                f.repo()
                    .apply_commit_action(&p, Some("stale"), &CancellationToken::new()),
                CommitFailure::StalePreview,
            );
            let p = preview(&f, request(CommitOperation::Amend, None));
            f.run(&["switch", "-c", "other"]);
            refused(
                f.repo()
                    .apply_commit_action(&p, Some("wrong branch"), &CancellationToken::new()),
                CommitFailure::StalePreview,
            );
            let other = Fixture::new();
            other.write("file", "new");
            other.commit("other");
            refused(
                other
                    .repo()
                    .apply_commit_action(&p, Some("wrong repo"), &CancellationToken::new()),
                CommitFailure::WrongRepository,
            );
            assert_eq!(text(&f, &["log", "-1", "--format=%s"]), "base");
        },
    );
}

#[test]
fn cherry_pick_and_revert_one_commit_preserve_unrelated_local_files() {
    isolated(
        "ROCKET_COMMIT_ACTION_CASE",
        "cherry_pick_and_revert_one_commit_preserve_unrelated_local_files",
        || {
            for format in ["sha1", "sha256"] {
                let f = Fixture::with_format(format);
                f.write("base", "base");
                f.commit("base");
                f.run(&["switch", "-c", "source"]);
                f.write("picked-日本語.txt", "picked\n");
                f.commit("picked 中文");
                let source = oid(&f, "HEAD");
                f.run(&["switch", "main"]);
                f.write("main.txt", "main");
                f.commit("main change");
                let old = oid(&f, "HEAD");
                f.write("private.txt", "keep");
                f.write("ignored.txt", "keep ignored");
                fs::create_dir_all(f.repository.join(".git/info")).unwrap();
                fs::write(f.repository.join(".git/info/exclude"), b"ignored.txt\n").unwrap();
                let p = preview(&f, request(CommitOperation::CherryPick, Some(source)));
                ok(apply(&f, &p, None));
                let picked = oid(&f, "HEAD");
                assert_eq!(oid(&f, "HEAD^"), old);
                let p = preview(&f, request(CommitOperation::Revert, Some(picked)));
                assert!(String::from_utf8_lossy(p.patch()).contains("-picked"));
                ok(apply(&f, &p, None));
                assert_eq!(
                    text(&f, &["rev-parse", "HEAD^{tree}"]),
                    text(&f, &["rev-parse", &format!("{old}^{{tree}}")])
                );
                assert_eq!(fs::read(f.repository.join("private.txt")).unwrap(), b"keep");
                assert_eq!(
                    fs::read(f.repository.join("ignored.txt")).unwrap(),
                    b"keep ignored"
                );
            }
        },
    );
}

fn conflicting(operation: CommitOperation) -> (Fixture, ObjectId) {
    let f = Fixture::new();
    f.write("file.txt", "base\n");
    f.commit("base");
    if operation == CommitOperation::CherryPick {
        f.run(&["switch", "-c", "source"]);
        f.write("file.txt", "source\n");
        f.commit("source");
        let source = oid(&f, "HEAD");
        f.run(&["switch", "main"]);
        f.write("file.txt", "main\n");
        f.commit("main");
        (f, source)
    } else {
        f.write("file.txt", "source\n");
        f.commit("source");
        let source = oid(&f, "HEAD");
        f.write("file.txt", "main\n");
        f.commit("main");
        (f, source)
    }
}
#[test]
fn conflicts_allow_reviewed_abort_skip_and_continue_with_stale_resolution_guard() {
    isolated(
        "ROCKET_COMMIT_ACTION_CASE",
        "conflicts_allow_reviewed_abort_skip_and_continue_with_stale_resolution_guard",
        || {
            for operation in [CommitOperation::CherryPick, CommitOperation::Revert] {
                let (f, target) = conflicting(operation);
                let old = oid(&f, "HEAD");
                for recovery in [
                    CommitRecovery::Abort,
                    CommitRecovery::Skip,
                    CommitRecovery::Continue,
                ] {
                    let p = preview(&f, request(operation, Some(target.clone())));
                    let r = apply(&f, &p, None);
                    assert!(r.write.result.is_err());
                    assert_eq!(r.failure, Some(CommitFailure::ConflictsRemain));
                    assert!(!r.state.unwrap().sequencer);
                    refused(
                        f.repo().preview_commit_action(
                            recover(operation, CommitRecovery::Continue),
                            &CancellationToken::new(),
                        ),
                        CommitFailure::ConflictsRemain,
                    );
                    if recovery == CommitRecovery::Continue {
                        f.write("file.txt", "resolved 中文\n");
                        f.repo()
                            .mark_resolved(
                                &RepoPath::new(b"file.txt").unwrap(),
                                &CancellationToken::new(),
                            )
                            .unwrap()
                            .result
                            .unwrap();
                    }
                    let p = preview(&f, recover(operation, recovery));
                    if recovery == CommitRecovery::Abort {
                        let lock = f.repository.join(".git/index.lock");
                        fs::write(&lock, b"existing lock").unwrap();
                        let blocked = apply(&f, &p, None);
                        assert_eq!(blocked.failure, Some(CommitFailure::IndexLocked));
                        assert_eq!(fs::read(&lock).unwrap(), b"existing lock");
                        assert!(!blocked.state.unwrap().is_idle());
                        fs::remove_file(&lock).unwrap();
                        f.write("file.txt", "new resolution after preview\n");
                        refused(
                            f.repo()
                                .apply_commit_action(&p, None, &CancellationToken::new()),
                            CommitFailure::StalePreview,
                        );
                    }
                    let p = preview(&f, recover(operation, recovery));
                    ok(apply(&f, &p, None));
                    if recovery != CommitRecovery::Continue {
                        assert_eq!(oid(&f, "HEAD"), old);
                        assert_eq!(fs::read(f.repository.join("file.txt")).unwrap(), b"main\n");
                    } else {
                        assert_eq!(
                            fs::read(f.repository.join("file.txt")).unwrap(),
                            "resolved 中文\n".as_bytes()
                        );
                    }
                }
            }
        },
    );
}

#[test]
fn empty_cherry_pick_stops_for_explicit_skip() {
    isolated(
        "ROCKET_COMMIT_ACTION_CASE",
        "empty_cherry_pick_stops_for_explicit_skip",
        || {
            let f = Fixture::new();
            f.write("base", "base");
            f.commit("base");
            f.write("file", "already here");
            f.commit("change");
            let old = oid(&f, "HEAD");
            let r = apply(
                &f,
                &preview(&f, request(CommitOperation::CherryPick, Some(old.clone()))),
                None,
            );
            assert_eq!(r.failure, Some(CommitFailure::EmptyChange));
            refused(
                f.repo().preview_commit_action(
                    recover(CommitOperation::CherryPick, CommitRecovery::Continue),
                    &CancellationToken::new(),
                ),
                CommitFailure::EmptyChange,
            );
            ok(apply(
                &f,
                &preview(
                    &f,
                    recover(CommitOperation::CherryPick, CommitRecovery::Skip),
                ),
                None,
            ));
            assert_eq!(oid(&f, "HEAD"), old);
        },
    );
}

#[test]
fn dirty_tracked_and_untracked_or_ignored_collisions_are_rejected() {
    isolated(
        "ROCKET_COMMIT_ACTION_CASE",
        "dirty_tracked_and_untracked_or_ignored_collisions_are_rejected",
        || {
            let f = Fixture::new();
            f.write("base", "base");
            f.commit("base");
            f.run(&["switch", "-c", "source"]);
            f.write("private/data.txt", "incoming");
            f.commit("source");
            let source = oid(&f, "HEAD");
            f.run(&["switch", "main"]);
            f.write("base", "dirty");
            refused(
                f.repo().preview_commit_action(
                    request(CommitOperation::CherryPick, Some(source.clone())),
                    &CancellationToken::new(),
                ),
                CommitFailure::DirtyWorktree,
            );
            f.write("base", "base");
            f.write("private/data.txt", "keep");
            refused(
                f.repo().preview_commit_action(
                    request(CommitOperation::CherryPick, Some(source.clone())),
                    &CancellationToken::new(),
                ),
                CommitFailure::LocalFileCollision,
            );
            fs::create_dir_all(f.repository.join(".git/info")).unwrap();
            fs::write(f.repository.join(".git/info/exclude"), b"private/\n").unwrap();
            refused(
                f.repo().preview_commit_action(
                    request(CommitOperation::CherryPick, Some(source)),
                    &CancellationToken::new(),
                ),
                CommitFailure::LocalFileCollision,
            );
            assert_eq!(
                fs::read(f.repository.join("private/data.txt")).unwrap(),
                b"keep"
            );
        },
    );
}

#[test]
fn merge_commit_requires_explicit_valid_mainline() {
    isolated(
        "ROCKET_COMMIT_ACTION_CASE",
        "merge_commit_requires_explicit_valid_mainline",
        || {
            let f = Fixture::new();
            f.write("base", "base");
            f.commit("base");
            let base = oid(&f, "HEAD");
            f.run(&["switch", "-c", "source"]);
            f.write("side", "side");
            f.commit("side");
            f.run(&["switch", "main"]);
            f.write("main", "main");
            f.commit("main");
            f.run(&["merge", "--no-edit", "source"]);
            let merged = oid(&f, "HEAD");
            refused(
                f.repo().preview_commit_action(
                    request(CommitOperation::Revert, Some(merged.clone())),
                    &CancellationToken::new(),
                ),
                CommitFailure::MainlineRequired,
            );
            let mut req = request(CommitOperation::Revert, Some(merged));
            req.mainline = Some(3);
            refused(
                f.repo()
                    .preview_commit_action(req.clone(), &CancellationToken::new()),
                CommitFailure::InvalidMainline,
            );
            req.mainline = Some(1);
            let p = preview(&f, req);
            let sides: Vec<_> = p.parent_commits().iter().map(|c| c.subject_lossy().into_owned()).collect();
            assert_eq!(sides, ["main", "side"]);
            ok(apply(&f, &p, None));
            assert!(!f.repository.join("side").exists());
            assert!(f.repository.join("main").exists());
            // Root commits are also valid replay sources, with no fabricated empty-tree hash.
            let p = preview(&f, request(CommitOperation::Revert, Some(base)));
            assert!(String::from_utf8_lossy(p.patch()).contains("-base"));
        },
    );
}

#[test]
fn amend_failure_lock_and_cancellation_preserve_history_and_expose_recovery() {
    isolated(
        "ROCKET_COMMIT_ACTION_CASE",
        "amend_failure_lock_and_cancellation_preserve_history_and_expose_recovery",
        || {
            let f = Fixture::new();
            f.write("file", "base");
            f.commit("base");
            let old = oid(&f, "HEAD");
            let hook = f.root.join("no-hooks/pre-commit");
            fs::write(&hook, b"#!/bin/sh\necho PRIVATE_HOOK_PAYLOAD >&2\nexit 1\n").unwrap();
            let p = preview(&f, request(CommitOperation::Amend, None));
            let r = apply(&f, &p, Some("PRIVATE_MESSAGE"));
            assert!(r.write.result.is_err());
            assert_eq!(r.failure, Some(CommitFailure::CommandFailed));
            assert_eq!(oid(&f, "HEAD"), old);
            assert_eq!(oid(&f, &r.recovery_reference.unwrap()), old);
            fs::write(&hook, b"#!/bin/sh\nexit 0\n").unwrap();
            fs::write(f.repository.join(".git/index.lock"), b"owned elsewhere").unwrap();
            let p = preview(&f, request(CommitOperation::Amend, None));
            let r = apply(&f, &p, Some("locked"));
            assert_eq!(r.failure, Some(CommitFailure::IndexLocked));
            assert_eq!(
                fs::read(f.repository.join(".git/index.lock")).unwrap(),
                b"owned elsewhere"
            );
            fs::remove_file(f.repository.join(".git/index.lock")).unwrap();
            let marker = f.root.join("hook-started");
            let script = format!(
                "#!/bin/sh\necho ready > '{}'\nsleep 30\n",
                marker.to_string_lossy().replace('\\', "/")
            );
            fs::write(&hook, script).unwrap();
            let p = preview(&f, request(CommitOperation::Amend, None));
            let repo = f.repo();
            let token = CancellationToken::new();
            let child = token.clone();
            let task =
                std::thread::spawn(move || repo.apply_commit_action(&p, Some("cancelled"), &child));
            let until = Instant::now() + Duration::from_secs(10);
            while !marker.exists() {
                assert!(Instant::now() < until);
                std::thread::sleep(Duration::from_millis(20));
            }
            token.cancel();
            let r = task.join().unwrap().unwrap();
            assert!(matches!(r.write.result, Err(Error::Cancelled)));
            assert!(r.write.status.is_ok());
            assert!(r.state.unwrap().is_idle());
            assert_eq!(oid(&f, "HEAD"), old);
        },
    );
}

#[test]
fn recovery_checks_operation_metadata_and_ignored_restore_collisions() {
    isolated(
        "ROCKET_COMMIT_ACTION_CASE",
        "recovery_checks_operation_metadata_and_ignored_restore_collisions",
        || {
            let f = Fixture::new();
            f.write("file.txt", "base\n");
            f.write("protected-É.txt", "original\n");
            f.commit("base");
            let base = oid(&f, "HEAD");
            f.run(&["switch", "-c", "source"]);
            f.write("file.txt", "source\n");
            f.run(&["rm", "protected-É.txt"]);
            f.commit("source");
            let target = oid(&f, "HEAD");
            f.run(&["switch", "main"]);
            f.write("file.txt", "main\n");
            f.commit("main");
            let r = apply(
                &f,
                &preview(
                    &f,
                    request(CommitOperation::CherryPick, Some(target.clone())),
                ),
                None,
            );
            assert_eq!(r.failure, Some(CommitFailure::ConflictsRemain));
            fs::create_dir_all(f.repository.join(".git/info")).unwrap();
            fs::write(f.repository.join(".git/info/exclude"), "protected-*\n").unwrap();
            f.write("protected-É.txt", "keep replacement\n");
            refused(
                f.repo().preview_commit_action(
                    recover(CommitOperation::CherryPick, CommitRecovery::Abort),
                    &CancellationToken::new(),
                ),
                CommitFailure::LocalFileCollision,
            );
            assert_eq!(
                fs::read(f.repository.join("protected-É.txt")).unwrap(),
                b"keep replacement\n"
            );
            fs::remove_file(f.repository.join("protected-É.txt")).unwrap();
            let p = preview(
                &f,
                recover(CommitOperation::CherryPick, CommitRecovery::Abort),
            );
            let message = fs::read(f.repository.join(".git/MERGE_MSG")).unwrap();
            fs::write(f.repository.join(".git/MERGE_MSG"), b"changed externally").unwrap();
            refused(
                f.repo()
                    .apply_commit_action(&p, None, &CancellationToken::new()),
                CommitFailure::StalePreview,
            );
            fs::write(f.repository.join(".git/MERGE_MSG"), message).unwrap();
            f.run(&["update-ref", "CHERRY_PICK_HEAD", base.as_str()]);
            refused(
                f.repo()
                    .apply_commit_action(&p, None, &CancellationToken::new()),
                CommitFailure::StalePreview,
            );
            f.run(&["update-ref", "CHERRY_PICK_HEAD", target.as_str()]);
            fs::create_dir(f.repository.join(".git/sequencer")).unwrap();
            refused(
                f.repo().preview_commit_action(
                    recover(CommitOperation::CherryPick, CommitRecovery::Abort),
                    &CancellationToken::new(),
                ),
                CommitFailure::UnsupportedSequence,
            );
            fs::remove_dir(f.repository.join(".git/sequencer")).unwrap();
            ok(apply(
                &f,
                &preview(
                    &f,
                    recover(CommitOperation::CherryPick, CommitRecovery::Abort),
                ),
                None,
            ));
            assert_eq!(
                fs::read(f.repository.join("protected-É.txt")).unwrap(),
                b"original\n"
            );
        },
    );
}

#[test]
fn empty_root_amend_and_identity_and_signing_errors_are_explicit() {
    isolated(
        "ROCKET_COMMIT_ACTION_CASE",
        "empty_root_amend_and_identity_and_signing_errors_are_explicit",
        || {
            let f = Fixture::new();
            refused(
                f.repo().preview_commit_action(
                    request(CommitOperation::Amend, None),
                    &CancellationToken::new(),
                ),
                CommitFailure::MissingHead,
            );
            f.run(&["commit", "--allow-empty", "-m", "empty root"]);
            let p = preview(&f, request(CommitOperation::Amend, None));
            ok(apply(&f, &p, Some("new empty root message")));
            let old = oid(&f, "HEAD");
            f.run(&["config", "user.useConfigOnly", "true"]);
            f.run(&["config", "--unset", "user.email"]);
            let r = apply(
                &f,
                &preview(&f, request(CommitOperation::Amend, None)),
                Some("identity rejected"),
            );
            assert_eq!(r.failure, Some(CommitFailure::IdentityMissing));
            assert_eq!(oid(&f, "HEAD"), old);
            f.run(&["config", "user.email", "rocket@example.invalid"]);
            f.run(&["config", "commit.gpgsign", "true"]);
            f.run(&[
                "config",
                "gpg.program",
                f.root.join("missing-signer.exe").to_str().unwrap(),
            ]);
            let r = apply(
                &f,
                &preview(&f, request(CommitOperation::Amend, None)),
                Some("signing rejected"),
            );
            assert_eq!(r.failure, Some(CommitFailure::SigningFailed));
            assert_eq!(oid(&f, "HEAD"), old);
        },
    );
}

#[test]
fn cancellation_after_amend_commits_reconciles_changed_head_without_retry() {
    isolated(
        "ROCKET_COMMIT_ACTION_CASE",
        "cancellation_after_amend_commits_reconciles_changed_head_without_retry",
        || {
            let f = Fixture::new();
            f.write("file", "base");
            f.commit("base");
            let old = oid(&f, "HEAD");
            let marker = f.root.join("post-commit-started");
            fs::write(
                f.root.join("no-hooks/post-commit"),
                format!(
                    "#!/bin/sh\necho ready > '{}'\nsleep 30\n",
                    marker.to_string_lossy().replace('\\', "/")
                ),
            )
            .unwrap();
            let p = preview(&f, request(CommitOperation::Amend, None));
            let repo = f.repo();
            let cancel = CancellationToken::new();
            let child = cancel.clone();
            let task = std::thread::spawn(move || {
                repo.apply_commit_action(&p, Some("actually committed"), &child)
            });
            let until = Instant::now() + Duration::from_secs(10);
            while !marker.exists() {
                assert!(Instant::now() < until);
                std::thread::sleep(Duration::from_millis(20));
            }
            cancel.cancel();
            let r = task.join().unwrap().unwrap();
            assert!(matches!(r.write.result, Err(Error::Cancelled)));
            let observed = r.write.status.unwrap().head;
            assert!(matches!(observed,Head::Branch{target,..} if target!=old));
            assert_eq!(
                text(&f, &["log", "-1", "--format=%B"]),
                "actually committed"
            );
            assert_eq!(oid(&f, &r.recovery_reference.unwrap()), old);
            assert!(r.state.unwrap().is_idle());
        },
    );
}

fn many(operation: CommitOperation, targets: Vec<ObjectId>) -> CommitActionRequest {
    CommitActionRequest {
        operation,
        targets,
        mainline: None,
        recovery: None,
    }
}
/// main, plus `topic` with three commits changing a.txt, b.txt and c.txt.
fn sequence_fixture(format: &str) -> (Fixture, [ObjectId; 3]) {
    let f = Fixture::with_format(format);
    for name in ["a.txt", "b.txt", "c.txt"] {
        f.write(name, "base\n");
    }
    f.commit("base");
    f.run(&["switch", "-q", "-c", "topic"]);
    let mut ids = Vec::new();
    for name in ["a", "b", "c"] {
        f.write(&format!("{name}.txt"), format!("{name} from topic 中文\n"));
        f.commit(&format!("change {name}"));
        ids.push(oid(&f, "HEAD"));
    }
    f.run(&["switch", "-q", "main"]);
    f.write("main.txt", "main\n");
    f.commit("main change");
    (f, ids.try_into().unwrap())
}
fn journal_exists(f: &Fixture) -> bool {
    f.repository.join(".git/rocket-commit-sequence").exists()
}

#[test]
fn sequences_apply_the_reviewed_order_and_revert_newest_first_sha1_sha256() {
    isolated(
        "ROCKET_COMMIT_ACTION_CASE",
        "sequences_apply_the_reviewed_order_and_revert_newest_first_sha1_sha256",
        || {
            for format in ["sha1", "sha256"] {
                let (f, [a, b, c]) = sequence_fixture(format);
                f.write("private.txt", "keep\n");
                let p = preview(
                    &f,
                    many(
                        CommitOperation::CherryPick,
                        vec![c.clone(), a.clone(), b.clone()],
                    ),
                );
                assert!(p.is_sequence());
                assert_eq!(
                    p.commits().iter().map(|c| c.id.clone()).collect::<Vec<_>>(),
                    [c.clone(), a.clone(), b.clone()]
                );
                let summary = String::from_utf8_lossy(p.patch()).into_owned();
                for name in ["a.txt", "b.txt", "c.txt"] {
                    assert!(summary.contains(name), "{summary}");
                }
                let r = apply(&f, &p, None);
                assert!(r.journal_cleanup.is_ok());
                ok(r);
                assert_eq!(
                    text(&f, &["log", "-3", "--format=%s"]),
                    "change b\nchange a\nchange c"
                );
                assert!(!journal_exists(&f));
                assert_eq!(
                    fs::read(f.repository.join("private.txt")).unwrap(),
                    b"keep\n"
                );
                f.run(&["switch", "-q", "topic"]);
                let p = preview(
                    &f,
                    many(CommitOperation::Revert, vec![c.clone(), b.clone()]),
                );
                ok(apply(&f, &p, None));
                assert_eq!(
                    text(&f, &["log", "-2", "--format=%s"]),
                    "Revert \"change b\"\nRevert \"change c\""
                );
                assert_eq!(text(&f, &["show", "HEAD:c.txt"]), "base");
                assert_eq!(text(&f, &["show", "HEAD:a.txt"]), "a from topic 中文");
            }
        },
    );
}

#[test]
fn sequence_conflicts_continue_skip_and_abort_follow_the_reviewed_journal() {
    isolated(
        "ROCKET_COMMIT_ACTION_CASE",
        "sequence_conflicts_continue_skip_and_abort_follow_the_reviewed_journal",
        || {
            for recovery in [
                CommitRecovery::Skip,
                CommitRecovery::Continue,
                CommitRecovery::Abort,
            ] {
                let (f, [a, b, c]) = sequence_fixture("sha1");
                f.write("b.txt", "main changes b\n");
                f.commit("main changes b");
                let original = oid(&f, "HEAD");
                let p = preview(
                    &f,
                    many(
                        CommitOperation::CherryPick,
                        vec![a.clone(), b.clone(), c.clone()],
                    ),
                );
                let r = apply(&f, &p, None);
                assert_eq!(r.failure, Some(CommitFailure::ConflictsRemain), "{r:?}");
                let state = r.state.unwrap();
                assert!(state.sequencer && state.cherry_pick);
                assert!(journal_exists(&f));
                refused(
                    f.repo().preview_commit_action(
                        recover(CommitOperation::CherryPick, CommitRecovery::Continue),
                        &CancellationToken::new(),
                    ),
                    CommitFailure::ConflictsRemain,
                );
                if recovery == CommitRecovery::Continue {
                    f.write("b.txt", "resolved\n");
                    f.run(&["add", "--", "b.txt"]);
                }
                let p = preview(&f, recover(CommitOperation::CherryPick, recovery));
                assert!(p.is_sequence());
                assert_eq!(p.remaining(), [b.clone(), c.clone()]);
                assert_eq!(p.source().id, b);
                assert_eq!(p.applied().len(), 1);
                assert_eq!(p.applied()[0].parents, std::slice::from_ref(&original));
                let picked_a = p.applied()[0].id.clone();
                let reference = p.recovery_reference();
                assert_eq!(reference.is_some(), recovery == CommitRecovery::Abort);
                let r = apply(&f, &p, None);
                assert!(r.journal_cleanup.is_ok());
                ok(r);
                assert!(!journal_exists(&f));
                match recovery {
                    CommitRecovery::Skip => assert_eq!(
                        text(&f, &["log", "-3", "--format=%s"]),
                        "change c\nchange a\nmain changes b"
                    ),
                    CommitRecovery::Continue => {
                        assert_eq!(
                            text(&f, &["log", "-4", "--format=%s"]),
                            "change c\nchange b\nchange a\nmain changes b"
                        );
                        assert_eq!(text(&f, &["show", "HEAD:b.txt"]), "resolved");
                    }
                    CommitRecovery::Abort => {
                        assert_eq!(oid(&f, "HEAD"), original);
                        // The commit the sequence had already created stays recoverable.
                        assert_eq!(oid(&f, &reference.unwrap()), picked_a);
                    }
                }
            }
        },
    );
}

#[test]
fn sequences_refuse_merges_duplicates_and_external_or_changed_state() {
    isolated(
        "ROCKET_COMMIT_ACTION_CASE",
        "sequences_refuse_merges_duplicates_and_external_or_changed_state",
        || {
            let (f, [a, b, c]) = sequence_fixture("sha1");
            let repo = f.repo();
            let cancel = CancellationToken::new();
            f.run(&["switch", "-q", "-c", "merged", "topic"]);
            f.run(&["merge", "-q", "--no-ff", "-m", "merge main", "main"]);
            let merge = oid(&f, "HEAD");
            f.run(&["switch", "-q", "main"]);
            refused(
                repo.preview_commit_action(
                    many(CommitOperation::CherryPick, vec![merge, a.clone()]),
                    &cancel,
                ),
                CommitFailure::SequenceMerge,
            );
            for invalid in [
                many(CommitOperation::CherryPick, vec![a.clone(), a.clone()]),
                many(CommitOperation::CherryPick, Vec::new()),
                CommitActionRequest {
                    targets: vec![a.clone()],
                    ..recover(CommitOperation::CherryPick, CommitRecovery::Abort)
                },
            ] {
                assert!(matches!(
                    repo.preview_commit_action(invalid, &cancel),
                    Err(Error::InvalidInput(_))
                ));
            }
            refused(
                repo.preview_commit_action(
                    CommitActionRequest {
                        mainline: Some(1),
                        ..many(CommitOperation::CherryPick, vec![a.clone(), b.clone()])
                    },
                    &cancel,
                ),
                CommitFailure::InvalidMainline,
            );
            // A sequence started outside Rocket has no review to resume.
            f.write("b.txt", "main changes b\n");
            f.commit("main changes b");
            let ids = [a.as_str(), b.as_str(), c.as_str()];
            let mut external = vec!["cherry-pick", "--no-edit", "--"];
            external.extend(ids);
            assert!(!f.output(&f.repository, &external).status.success());
            refused(
                repo.preview_commit_action(
                    recover(CommitOperation::CherryPick, CommitRecovery::Abort),
                    &cancel,
                ),
                CommitFailure::UnsupportedSequence,
            );
            f.run(&["cherry-pick", "--abort"]);
            // A Rocket sequence whose todo was changed is not guessed at.
            let p = preview(
                &f,
                many(
                    CommitOperation::CherryPick,
                    vec![a.clone(), b.clone(), c.clone()],
                ),
            );
            assert!(apply(&f, &p, None).state.unwrap().sequencer);
            let todo = f.repository.join(".git/sequencer/todo");
            let saved = fs::read(&todo).unwrap();
            fs::write(
                &todo,
                &saved[..saved.iter().position(|b| *b == b'\n').unwrap() + 1],
            )
            .unwrap();
            refused(
                repo.preview_commit_action(
                    recover(CommitOperation::CherryPick, CommitRecovery::Abort),
                    &cancel,
                ),
                CommitFailure::SequenceJournal,
            );
            fs::write(&todo, saved).unwrap();
            // After a manual commit nothing is stopped: Skip is refused, Continue resumes.
            f.write("b.txt", "resolved by hand\n");
            f.run(&["add", "--", "b.txt"]);
            f.run(&["commit", "-q", "--no-edit"]);
            refused(
                repo.preview_commit_action(
                    recover(CommitOperation::CherryPick, CommitRecovery::Skip),
                    &cancel,
                ),
                CommitFailure::NothingToSkip,
            );
            let p = preview(
                &f,
                recover(CommitOperation::CherryPick, CommitRecovery::Continue),
            );
            assert_eq!(p.remaining(), [b.clone(), c.clone()]);
            ok(apply(&f, &p, None));
            assert_eq!(
                text(&f, &["log", "-3", "--format=%s"]),
                "change c\nchange b\nchange a"
            );
            assert!(!journal_exists(&f));
        },
    );
}

/// What a review draws: each commit's own change, and where a stopped range stands.
#[test]
fn a_range_review_carries_each_change_and_a_stop_says_where_it_stands() {
    isolated(
        "ROCKET_COMMIT_ACTION_CASE",
        "a_range_review_carries_each_change_and_a_stop_says_where_it_stands",
        || {
            let (f, [a, b, c]) = sequence_fixture("sha1");
            f.run(&["switch", "-q", "topic"]);
            f.write("b.txt", "later change\n");
            f.commit("later change");
            let original = oid(&f, "HEAD");
            let p = preview(&f, many(CommitOperation::Revert, vec![c.clone(), b.clone(), a.clone()]));
            let change = |i: usize| String::from_utf8_lossy(&p.patches()[i]).into_owned();
            assert_eq!(p.patches().len(), 3);
            assert!(change(0).contains("-c from topic 中文") && change(0).contains("+base"), "undone, not replayed: {}", change(0));
            assert!(change(1).contains("b.txt") && !change(1).contains("c.txt"), "{}", change(1));
            assert!(p.reviewed().is_empty(), "nothing has stopped yet");
            let r = apply(&f, &p, None);
            assert_eq!(r.failure, Some(CommitFailure::ConflictsRemain), "{r:?}");
            let step = r.state.unwrap().step.unwrap();
            assert_eq!((step.at, step.total, step.kind, step.subject.as_str()), (2, 3, StepKind::Commit, "change b"));
            let p = preview(&f, recover(CommitOperation::Revert, CommitRecovery::Abort));
            assert_eq!(p.position(), (2, 3));
            let subjects: Vec<_> = p.reviewed().iter().map(|c| c.subject_lossy().into_owned()).collect();
            assert_eq!(subjects, ["change c", "change b", "change a"]);
            assert_eq!(p.stopped_commit().map(|c| c.id.clone()), Some(b));
            assert_eq!(p.original_commit().map(|c| c.id.clone()), Some(original));
        },
    );
}

#[test]
fn a_lone_stop_names_its_commit_and_abort_leaves_the_branch_where_it_is() {
    isolated(
        "ROCKET_COMMIT_ACTION_CASE",
        "a_lone_stop_names_its_commit_and_abort_leaves_the_branch_where_it_is",
        || {
            for operation in [CommitOperation::Revert, CommitOperation::CherryPick] {
                let (f, target) = conflicting(operation);
                let head = oid(&f, "HEAD");
                let r = apply(&f, &preview(&f, request(operation, Some(target.clone()))), None);
                assert_eq!(r.failure, Some(CommitFailure::ConflictsRemain), "{r:?}");
                let step = r.state.unwrap().step.unwrap();
                assert_eq!((step.at, step.total, step.kind, step.subject.as_str()), (1, 1, StepKind::Commit, "source"));
                let p = preview(&f, recover(operation, CommitRecovery::Abort));
                assert!(!p.is_sequence());
                assert_eq!(p.stopped_commit().map(|c| c.id.clone()), Some(target));
                assert_eq!(p.original_commit().map(|c| c.id.clone()), Some(head));
            }
        },
    );
}

/// A commit made by hand mid-range finishes that commit: Git goes on after it, and
/// won't rewind a branch that moved without it.
#[test]
fn a_hand_commit_mid_range_counts_and_abort_leaves_it_in_place() {
    isolated(
        "ROCKET_COMMIT_ACTION_CASE",
        "a_hand_commit_mid_range_counts_and_abort_leaves_it_in_place",
        || {
            for abort in [false, true] {
                let (f, [a, b, c]) = sequence_fixture("sha1");
                f.run(&["switch", "-q", "topic"]);
                f.write("b.txt", "later change\n");
                f.commit("later change");
                let p = preview(&f, many(CommitOperation::Revert, vec![c, b, a]));
                let r = apply(&f, &p, None);
                assert_eq!(r.failure, Some(CommitFailure::ConflictsRemain), "{r:?}");
                f.write("b.txt", "settled by hand\n");
                f.commit("by hand");
                let hand = oid(&f, "HEAD");
                let state = f.repo().operation_state(&CancellationToken::new()).unwrap();
                assert!(state.sequencer && !state.revert, "{state:?}");
                assert_eq!(state.step.unwrap().kind, StepKind::Between);
                let recovery = if abort { CommitRecovery::Abort } else { CommitRecovery::Continue };
                let p = preview(&f, recover(CommitOperation::Revert, recovery));
                assert!(p.stopped_commit().is_none());
                assert_eq!(p.position(), (2, 3));
                assert!(!p.rewinds());
                if abort {
                    assert_eq!(p.original_commit().map(|c| c.id.clone()), Some(hand.clone()));
                    ok(apply(&f, &p, None));
                    assert_eq!(oid(&f, "HEAD"), hand, "Git doesn't move it, so the review mustn't say so");
                } else {
                    ok(apply(&f, &p, None));
                    assert_eq!(text(&f, &["log", "-2", "--format=%s"]), "Revert \"change a\"\nby hand");
                }
            }
        },
    );
}

/// Nothing to undo stops a range with no marker; Continue goes on without that commit.
#[test]
fn an_empty_revert_in_a_range_waits_and_continue_goes_on() {
    isolated(
        "ROCKET_COMMIT_ACTION_CASE",
        "an_empty_revert_in_a_range_waits_and_continue_goes_on",
        || {
            let (f, [a, b, _]) = sequence_fixture("sha1");
            f.run(&["switch", "-q", "topic"]);
            f.run(&["revert", "--no-edit", b.as_str()]);
            let p = preview(&f, many(CommitOperation::Revert, vec![b, a]));
            let r = apply(&f, &p, None);
            assert_eq!(r.failure, Some(CommitFailure::EmptyChange), "{r:?}");
            let state = r.state.unwrap();
            assert!(state.sequencer && !state.revert, "{state:?}");
            let p = preview(&f, recover(CommitOperation::Revert, CommitRecovery::Continue));
            assert!(p.stopped_commit().is_none() && p.rewinds());
            ok(apply(&f, &p, None));
            assert_eq!(text(&f, &["log", "-1", "--format=%s"]), "Revert \"change a\"");
        },
    );
}
