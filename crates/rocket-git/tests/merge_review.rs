mod common;
use common::Fixture;
use rocket_git::*;
use std::fs;

fn token() -> CancellationToken {
    CancellationToken::new()
}
fn base(format: &str) -> Fixture {
    let f = Fixture::with_format(format);
    f.write("共同.rs", "base\n");
    f.write(".gitignore", "ignored.txt\n");
    f.commit("PRIVATE_BASE");
    f
}
fn topic(f: &Fixture) -> ObjectId {
    f.run(&["switch", "-c", "feature/合并"]);
    f.write("新 文件.rs", "incoming\n");
    f.commit("PRIVATE_INCOMING");
    let id = f.repo().resolve_commit("HEAD", &token()).unwrap();
    f.run(&["switch", "main"]);
    id
}
fn assert_failure<T: std::fmt::Debug>(result: Result<T>, expected: MergeFailure) {
    assert!(
        matches!(&result, Err(Error::Merge(f)) if *f == expected),
        "{result:?}"
    );
}
fn successful(report: MergeWriteReport) {
    report.write.result.unwrap();
    report.write.status.unwrap();
    assert!(report.state.unwrap().is_idle());
    report.branches.unwrap();
    assert!(report.failure.is_none());
}

#[test]
fn committed_merge_markers_refuse_duplicate_commit_and_abort_then_clear_without_changes() {
    for format in ["sha1", "sha256"] {
        let f = base(format);
        let incoming = topic(&f);
        let repo = f.repo();
        let preview = repo
            .preview_merge("feature/合并", MergeMode::NoFastForward, &token())
            .unwrap();
        successful(repo.apply_merge(&preview, &token()).unwrap());
        let head = repo.head(&token()).unwrap();
        // State observed when a post-merge hook is interrupted: the merge commit
        // exists, while MERGE_HEAD/MERGE_MSG have not yet been removed.
        fs::write(
            f.repository.join(".git/MERGE_HEAD"),
            format!("{incoming}\n"),
        )
        .unwrap();
        fs::write(f.repository.join(".git/MERGE_MSG"), "pending cleanup\n").unwrap();
        // A replacement can hide parents in rev-list; recovery must inspect the
        // real object rather than mistake it for an uncommitted merge.
        f.run(&["replace", "HEAD", "ORIG_HEAD"]);
        assert!(repo.operation_state(&token()).unwrap().merge_committed);
        f.write("共同.rs", "staged\n");
        f.run(&["add", "--", "共同.rs"]);
        f.write("共同.rs", "unstaged\n");
        f.write("untracked 文", "keep");
        let index = fs::read(f.repository.join(".git/index")).unwrap();
        assert_failure(
            repo.continue_merge("duplicate", &token()),
            MergeFailure::AlreadyCommitted,
        );
        assert_failure(repo.abort_merge(&token()), MergeFailure::AlreadyCommitted);
        let cancelled = token();
        cancelled.cancel();
        assert!(matches!(
            repo.finish_committed_merge(&cancelled),
            Err(Error::Cancelled)
        ));
        assert!(f.repository.join(".git/MERGE_HEAD").exists());
        successful(repo.finish_committed_merge(&token()).unwrap());
        assert_eq!(repo.head(&token()).unwrap(), head);
        assert_eq!(fs::read(f.repository.join(".git/index")).unwrap(), index);
        assert_eq!(
            fs::read_to_string(f.repository.join("共同.rs")).unwrap(),
            "unstaged\n"
        );
        assert_eq!(
            fs::read_to_string(f.repository.join("untracked 文")).unwrap(),
            "keep"
        );
        assert!(!f.repository.join(".git/MERGE_HEAD").exists());
    }
}

#[test]
fn read_only_unicode_reviews_and_fast_forward_support_both_object_formats() {
    for format in ["sha1", "sha256"] {
        let f = base(format);
        let target = topic(&f);
        f.write("untracked 文.txt", "keep this");
        f.write("ignored.txt", "keep ignored");
        let repo = f.repo();
        let before = repo.status(&token()).unwrap();
        let index = fs::read(f.repository.join(".git/index")).unwrap();
        let config = fs::read(f.repository.join(".git/config")).unwrap();
        let p = repo
            .preview_merge(
                "refs/heads/feature/合并",
                MergeMode::FastForwardOnly,
                &token(),
            )
            .unwrap();
        assert_eq!(p.target().id, target);
        assert_eq!(p.relation(), MergeRelation::FastForward);
        assert_eq!((p.current_only(), p.incoming()), (0, 1));
        assert!(p.can_apply());
        assert_eq!(repo.status(&token()).unwrap(), before);
        assert_eq!(fs::read(f.repository.join(".git/index")).unwrap(), index);
        assert_eq!(fs::read(f.repository.join(".git/config")).unwrap(), config);
        successful(repo.apply_merge(&p, &token()).unwrap());
        assert_eq!(repo.resolve_commit("HEAD", &token()).unwrap(), target);
        assert_eq!(
            fs::read(f.repository.join("untracked 文.txt")).unwrap(),
            b"keep this"
        );
        assert_eq!(
            fs::read(f.repository.join("ignored.txt")).unwrap(),
            b"keep ignored"
        );
        let p = repo
            .preview_merge("feature/合并", MergeMode::NoFastForward, &token())
            .unwrap();
        assert_eq!(p.relation(), MergeRelation::UpToDate);
        successful(repo.apply_merge(&p, &token()).unwrap());
        assert_eq!(repo.resolve_commit("HEAD", &token()).unwrap(), target);
    }
}

#[test]
fn divergence_requires_explicit_merge_mode_and_creates_both_parents() {
    let f = base("sha1");
    let incoming = topic(&f);
    f.write("local.txt", "local");
    f.commit("PRIVATE_LOCAL");
    let repo = f.repo();
    let original = repo.resolve_commit("HEAD", &token()).unwrap();
    let p = repo
        .preview_merge("feature/合并", MergeMode::FastForwardOnly, &token())
        .unwrap();
    assert_eq!(p.relation(), MergeRelation::Diverged);
    assert_eq!((p.current_only(), p.incoming()), (1, 1));
    assert!(!p.can_apply());
    assert_failure(repo.apply_merge(&p, &token()), MergeFailure::NotFastForward);
    assert_eq!(repo.resolve_commit("HEAD", &token()).unwrap(), original);
    let p = repo
        .preview_merge("feature/合并", MergeMode::AllowFastForward, &token())
        .unwrap();
    successful(repo.apply_merge(&p, &token()).unwrap());
    let c = repo
        .history(HistoryStart::Head, 1, &token())
        .unwrap()
        .commits
        .remove(0);
    assert_eq!(c.parents, vec![original, incoming]);
}

#[test]
fn no_ff_creates_merge_commit_even_with_linear_incoming_history() {
    let f = base("sha1");
    let incoming = topic(&f);
    let repo = f.repo();
    let original = repo.resolve_commit("HEAD", &token()).unwrap();
    let p = repo
        .preview_merge("feature/合并", MergeMode::NoFastForward, &token())
        .unwrap();
    successful(repo.apply_merge(&p, &token()).unwrap());
    assert_eq!(
        repo.history(HistoryStart::Head, 1, &token())
            .unwrap()
            .commits[0]
            .parents,
        vec![original, incoming]
    );
}

#[test]
fn moved_target_changed_checkout_and_other_repository_cannot_reuse_review() {
    let f = base("sha1");
    topic(&f);
    let repo = f.repo();
    let original = repo.resolve_commit("HEAD", &token()).unwrap();
    let p = repo
        .preview_merge("feature/合并", MergeMode::FastForwardOnly, &token())
        .unwrap();
    let other = base("sha1");
    assert_failure(
        other.repo().apply_merge(&p, &token()),
        MergeFailure::StalePreview,
    );
    f.run(&["update-ref", "refs/heads/feature/合并", original.as_str()]);
    assert_failure(repo.apply_merge(&p, &token()), MergeFailure::StalePreview);
    f.run(&[
        "update-ref",
        "refs/heads/feature/合并",
        p.target().id.as_str(),
    ]);
    f.run(&["switch", "-c", "same-commit-different-branch"]);
    assert_failure(repo.apply_merge(&p, &token()), MergeFailure::StalePreview);
    assert_eq!(repo.resolve_commit("HEAD", &token()).unwrap(), original);
}

#[test]
fn edits_locks_hidden_index_entries_and_cancellation_preserve_work() {
    let f = base("sha1");
    topic(&f);
    let repo = f.repo();
    let p = repo
        .preview_merge("feature/合并", MergeMode::FastForwardOnly, &token())
        .unwrap();
    let before = repo.head(&token()).unwrap();
    f.write("共同.rs", "external edit");
    assert_failure(repo.apply_merge(&p, &token()), MergeFailure::DirtyWorktree);
    f.run(&["add", "."]);
    assert_failure(repo.apply_merge(&p, &token()), MergeFailure::DirtyWorktree);
    assert_eq!(
        fs::read(f.repository.join("共同.rs")).unwrap(),
        b"external edit"
    );
    f.run(&["restore", "--staged", "--worktree", "共同.rs"]);
    for flag in ["--assume-unchanged", "--skip-worktree"] {
        f.run(&["update-index", flag, "共同.rs"]);
        assert_failure(
            repo.preview_merge("feature/合并", MergeMode::FastForwardOnly, &token()),
            MergeFailure::UnsupportedIndex,
        );
        f.run(&["update-index", &flag.replace("--", "--no-"), "共同.rs"]);
    }
    fs::write(f.repository.join(".git/index.lock"), "owned elsewhere").unwrap();
    assert_failure(repo.apply_merge(&p, &token()), MergeFailure::IndexLocked);
    assert_eq!(
        fs::read(f.repository.join(".git/index.lock")).unwrap(),
        b"owned elsewhere"
    );
    fs::remove_file(f.repository.join(".git/index.lock")).unwrap();
    let cancel = token();
    cancel.cancel();
    assert!(matches!(
        repo.preview_merge("feature/合并", MergeMode::FastForwardOnly, &cancel),
        Err(Error::Cancelled)
    ));
    assert!(matches!(
        repo.apply_merge(&p, &cancel),
        Err(Error::Cancelled)
    ));
    assert_eq!(repo.head(&token()).unwrap(), before);
}

#[test]
fn conflicts_are_distinct_and_continue_or_abort_use_existing_recovery() {
    for finish in [false, true] {
        let f = base("sha1");
        f.run(&["switch", "-c", "topic"]);
        f.write("共同.rs", "theirs\n");
        f.commit("incoming");
        f.run(&["switch", "main"]);
        f.write("共同.rs", "ours\n");
        f.commit("local");
        let repo = f.repo();
        let original = repo.resolve_commit("HEAD", &token()).unwrap();
        let p = repo
            .preview_merge("topic", MergeMode::AllowFastForward, &token())
            .unwrap();
        let report = repo.apply_merge(&p, &token()).unwrap();
        assert!(report.write.result.is_err());
        assert_eq!(report.failure, Some(MergeFailure::Conflicts));
        assert_eq!(
            report.state.unwrap().merge_heads,
            vec![p.target().id.clone()]
        );
        assert_failure(
            repo.preview_merge("topic", MergeMode::AllowFastForward, &token()),
            MergeFailure::ActiveOperation,
        );
        if finish {
            f.write("共同.rs", "resolved\n");
            repo.stage_all(&token()).unwrap().result.unwrap();
            successful(repo.continue_merge("Resolved merge", &token()).unwrap());
            assert_eq!(
                repo.history(HistoryStart::Head, 1, &token())
                    .unwrap()
                    .commits[0]
                    .parents,
                vec![original, p.target().id.clone()]
            );
        } else {
            successful(repo.abort_merge(&token()).unwrap());
            assert_eq!(repo.resolve_commit("HEAD", &token()).unwrap(), original);
            assert_eq!(fs::read(f.repository.join("共同.rs")).unwrap(), b"ours\n");
        }
    }
}

#[test]
fn untracked_and_ignored_collisions_are_never_overwritten() {
    for ignored in [false, true] {
        let f = base("sha1");
        topic(&f);
        if ignored {
            f.write(".git/info/exclude", "新 文件.rs\n");
        }
        f.write("新 文件.rs", "irreplaceable local data");
        let repo = f.repo();
        let original = repo.head(&token()).unwrap();
        let p = repo
            .preview_merge("feature/合并", MergeMode::FastForwardOnly, &token())
            .unwrap();
        let result = repo.apply_merge(&p, &token()).unwrap();
        assert!(result.write.result.is_err());
        assert_eq!(result.failure, Some(MergeFailure::CommandFailed));
        assert_eq!(repo.head(&token()).unwrap(), original);
        assert_eq!(
            fs::read(f.repository.join("新 文件.rs")).unwrap(),
            b"irreplaceable local data"
        );
    }
}

#[test]
fn absent_target_unborn_and_unrelated_history_produce_clear_refusals() {
    let f = Fixture::new();
    assert_failure(
        f.repo()
            .preview_merge("missing", MergeMode::FastForwardOnly, &token()),
        MergeFailure::UnbornHead,
    );
    f.write("base", "base");
    f.commit("base");
    let repo = f.repo();
    assert_failure(
        repo.preview_merge("missing", MergeMode::FastForwardOnly, &token()),
        MergeFailure::TargetUnavailable,
    );
    f.run(&["switch", "--orphan", "orphan"]);
    f.write("other", "other");
    f.commit("unrelated");
    f.run(&["switch", "main"]);
    assert_failure(
        repo.preview_merge("orphan", MergeMode::AllowFastForward, &token()),
        MergeFailure::UnrelatedHistories,
    );
}
