mod common;
use common::{Fixture, isolated};
use rocket_git::*;
use std::{fs, path::Path};
fn token() -> CancellationToken {
    CancellationToken::new()
}
fn seed() -> Fixture {
    let f = Fixture::new();
    f.write("hello.txt", "base\n");
    f.write(".gitignore", "ignored/\n");
    f.commit("baseline");
    f
}
fn create(f: &Fixture, name: &str) -> WorktreePreview {
    f.repo()
        .preview_create_worktree(
            &f.root.canonicalize().unwrap().join(name),
            WorktreeCheckout::NewBranch(format!("topic/{}", name.replace(' ', "-"))),
            "HEAD",
            &token(),
        )
        .unwrap()
}
fn apply(f: &Fixture, p: &WorktreePreview) {
    let r = f.repo().apply_worktree(p, &token()).unwrap();
    assert!(r.command.is_ok(), "{:?}", r);
    assert!(r.verification.is_ok(), "{:?}", r);
    assert!(r.retained_workspace.is_none(), "{:?}", r);
}
fn entry(f: &Fixture, path: &Path) -> WorktreeEntry {
    let path = path.canonicalize().unwrap();
    f.repo()
        .worktrees(&token())
        .unwrap()
        .into_iter()
        .find(|e| e.path == path)
        .unwrap()
}
fn failure<T: std::fmt::Debug>(r: Result<T>, want: WorktreeFailure) {
    assert!(
        matches!(&r,Err(Error::Worktree(f))if *f==want),
        "expected {want:?}, got {r:?}"
    )
}

#[test]
fn create_open_remove_unicode_preserves_current_files_and_branch() {
    isolated(
        "ROCKET_WORKTREE_CASE",
        "create_open_remove_unicode_preserves_current_files_and_branch",
        || {
            let f = seed();
            f.write("hello.txt", "staged\n");
            f.run(&["add", "hello.txt"]);
            f.write("hello.txt", "working\n");
            f.write("local.txt", "private\n");
            let before = (
                f.run(&["rev-parse", "HEAD"]),
                f.run(&["ls-files", "--stage", "-z"]),
                fs::read(f.repository.join(".git/config")).unwrap(),
            );
            let p = create(&f, "検索 🚀");
            apply(&f, &p);
            let e = entry(&f, p.path());
            assert_eq!(f.repo().open_worktree(&e, &token()).unwrap(), p.path());
            assert_eq!(fs::read(p.path().join("hello.txt")).unwrap(), b"base\n");
            let removal = f.repo().preview_remove_worktree(&e, &token()).unwrap();
            let reference = removal.recovery_reference().unwrap();
            apply(&f, &removal);
            assert!(!p.path().exists());
            assert_eq!(
                f.run(&["rev-parse", &reference]).trim_ascii(),
                p.target().as_str().as_bytes()
            );
            assert_eq!(
                f.run(&["rev-parse", "refs/heads/topic/検索-🚀"])
                    .trim_ascii(),
                p.target().as_str().as_bytes()
            );
            assert_eq!(
                before,
                (
                    f.run(&["rev-parse", "HEAD"]),
                    f.run(&["ls-files", "--stage", "-z"]),
                    fs::read(f.repository.join(".git/config")).unwrap()
                )
            );
            assert_eq!(
                fs::read(f.repository.join("hello.txt")).unwrap(),
                b"working\n"
            );
            assert_eq!(
                fs::read(f.repository.join("local.txt")).unwrap(),
                b"private\n"
            );
        },
    );
}
#[test]
fn existing_paths_destinations_and_checked_out_branches_are_protected() {
    isolated(
        "ROCKET_WORKTREE_CASE",
        "existing_paths_destinations_and_checked_out_branches_are_protected",
        || {
            let f = seed();
            let p = create(&f, "planned");
            fs::create_dir(p.path()).unwrap();
            fs::write(p.path().join("keep"), "keep").unwrap();
            failure(
                f.repo().apply_worktree(&p, &token()),
                WorktreeFailure::DestinationExists,
            );
            assert_eq!(fs::read(p.path().join("keep")).unwrap(), b"keep");
            failure(
                f.repo().preview_create_worktree(
                    &f.repository.join("nested"),
                    WorktreeCheckout::NewBranch("topic/nested".into()),
                    "HEAD",
                    &token(),
                ),
                WorktreeFailure::InvalidDestination,
            );
            failure(
                f.repo().preview_create_worktree(
                    &f.root.canonicalize().unwrap().join("used"),
                    WorktreeCheckout::ExistingBranch("main".into()),
                    "HEAD",
                    &token(),
                ),
                WorktreeFailure::BranchInUse,
            );
            failure(
                f.repo().preview_create_worktree(
                    &f.root.canonicalize().unwrap().join("bad"),
                    WorktreeCheckout::NewBranch("bad name".into()),
                    "HEAD",
                    &token(),
                ),
                WorktreeFailure::InvalidBranch,
            );
            failure(
                f.repo().preview_create_worktree(
                    &f.root.canonicalize().unwrap().join("missing"),
                    WorktreeCheckout::ExistingBranch("missing".into()),
                    "HEAD",
                    &token(),
                ),
                WorktreeFailure::BranchUnavailable,
            );
            failure(
                f.repo().preview_create_worktree(
                    &f.root.canonicalize().unwrap().join("new"),
                    WorktreeCheckout::NewBranch("main".into()),
                    "HEAD",
                    &token(),
                ),
                WorktreeFailure::BranchExists,
            );
            let cancelled = token();
            cancelled.cancel();
            assert!(matches!(
                f.repo()
                    .apply_worktree(&create(&f, "cancelled"), &cancelled),
                Err(Error::Cancelled)
            ));
            assert!(!f.root.canonicalize().unwrap().join("cancelled").exists());
        },
    );
}
#[test]
fn new_targets_are_pinned_existing_branches_must_not_move() {
    isolated(
        "ROCKET_WORKTREE_CASE",
        "new_targets_are_pinned_existing_branches_must_not_move",
        || {
            let f = seed();
            f.run(&["branch", "existing"]);
            let new = create(&f, "pinned");
            let existing = f
                .repo()
                .preview_create_worktree(
                    &f.root.canonicalize().unwrap().join("existing"),
                    WorktreeCheckout::ExistingBranch("existing".into()),
                    "HEAD",
                    &token(),
                )
                .unwrap();
            f.write("second", "second");
            f.commit("second");
            f.run(&["branch", "-f", "existing", "HEAD"]);
            failure(
                f.repo().apply_worktree(&existing, &token()),
                WorktreeFailure::StalePreview,
            );
            apply(&f, &new);
            assert_eq!(entry(&f, new.path()).head.as_ref(), Some(new.target()));
            let existing = f
                .repo()
                .preview_create_worktree(
                    &f.root.canonicalize().unwrap().join("existing"),
                    WorktreeCheckout::ExistingBranch("existing".into()),
                    "ignored-input",
                    &token(),
                )
                .unwrap();
            apply(&f, &existing);
            let foreign = seed();
            failure(
                foreign.repo().apply_worktree(&new, &token()),
                WorktreeFailure::StalePreview,
            );
        },
    );
}
#[test]
fn removal_rechecks_external_edits_ignored_files_flags_and_locks() {
    isolated(
        "ROCKET_WORKTREE_CASE",
        "removal_rechecks_external_edits_ignored_files_flags_and_locks",
        || {
            let f = seed();
            let p = create(&f, "guarded");
            apply(&f, &p);
            let e = entry(&f, p.path());
            let review = f.repo().preview_remove_worktree(&e, &token()).unwrap();
            fs::write(p.path().join("untracked"), "do not delete").unwrap();
            failure(
                f.repo().apply_worktree(&review, &token()),
                WorktreeFailure::Dirty,
            );
            fs::remove_file(p.path().join("untracked")).unwrap();
            fs::create_dir(p.path().join("ignored")).unwrap();
            fs::write(p.path().join("ignored/local"), "ignored private").unwrap();
            failure(
                f.repo().preview_remove_worktree(&e, &token()),
                WorktreeFailure::Dirty,
            );
            fs::remove_file(p.path().join("ignored/local")).unwrap();
            fs::remove_dir(p.path().join("ignored")).unwrap();
            let root = p.path().to_str().unwrap().trim_start_matches(r"\\?\");
            f.run(&[
                "-C",
                root,
                "update-index",
                "--assume-unchanged",
                "hello.txt",
            ]);
            failure(
                f.repo().preview_remove_worktree(&e, &token()),
                WorktreeFailure::UnsupportedIndex,
            );
            f.run(&[
                "-C",
                root,
                "update-index",
                "--no-assume-unchanged",
                "hello.txt",
            ]);
            f.run(&["worktree", "lock", "--reason", "Keep this", root]);
            failure(
                f.repo().apply_worktree(&review, &token()),
                WorktreeFailure::StalePreview,
            );
            let locked = entry(&f, p.path());
            failure(
                f.repo().preview_remove_worktree(&locked, &token()),
                WorktreeFailure::Locked,
            );
            f.run(&["worktree", "unlock", root]);
            let linked = f.git.open(p.path(), &token()).unwrap();
            let lock = linked.info().git_dir.join("index.lock");
            fs::write(&lock, "OTHER_LOCK").unwrap();
            failure(
                f.repo().preview_remove_worktree(&e, &token()),
                WorktreeFailure::Locked,
            );
            assert_eq!(fs::read(&lock).unwrap(), b"OTHER_LOCK");
            fs::remove_file(lock).unwrap();
            fs::write(p.path().join("hello.txt"), "changed").unwrap();
            failure(
                f.repo().preview_remove_worktree(&e, &token()),
                WorktreeFailure::Dirty,
            );
            assert!(p.path().exists());
        },
    );
}
#[test]
fn main_current_missing_and_detached_worktrees_have_explicit_outcomes() {
    isolated(
        "ROCKET_WORKTREE_CASE",
        "main_current_missing_and_detached_worktrees_have_explicit_outcomes",
        || {
            let f = seed();
            let main = f.repo().worktrees(&token()).unwrap().remove(0);
            failure(
                f.repo().preview_remove_worktree(&main, &token()),
                WorktreeFailure::ProtectedWorktree,
            );
            let p = create(&f, "current");
            apply(&f, &p);
            let e = entry(&f, p.path());
            let linked = f.git.open(p.path(), &token()).unwrap();
            failure(
                linked.preview_remove_worktree(&e, &token()),
                WorktreeFailure::ProtectedWorktree,
            );
            let detached = f.root.canonicalize().unwrap().join("detached");
            f.run(&[
                "worktree",
                "add",
                "--detach",
                detached.to_str().unwrap().trim_start_matches(r"\\?\"),
                "HEAD",
            ]);
            let e = entry(&f, &detached);
            assert!(e.branch.is_none());
            apply(&f, &f.repo().preview_remove_worktree(&e, &token()).unwrap());
            let missing = f.root.canonicalize().unwrap().join("moved");
            fs::rename(p.path(), &missing).unwrap();
            let list = f.repo().worktrees(&token()).unwrap();
            let stale = list.into_iter().find(|e| !e.main).unwrap();
            assert!(stale.prunable.is_some());
            failure(
                f.repo().open_worktree(&stale, &token()),
                WorktreeFailure::Missing,
            );
            assert!(missing.join("hello.txt").exists());
        },
    );
}

#[test]
fn removal_refuses_active_operations_submodules_sparse_flags_and_recovery_collisions() {
    isolated(
        "ROCKET_WORKTREE_CASE",
        "removal_refuses_active_operations_submodules_sparse_flags_and_recovery_collisions",
        || {
            let f = seed();
            let p = create(&f, "restricted");
            apply(&f, &p);
            let e = entry(&f, p.path());
            let linked = f.git.open(p.path(), &token()).unwrap();
            let root = p.path().to_str().unwrap().trim_start_matches(r"\\?\");
            fs::write(
                linked.info().git_dir.join("CHERRY_PICK_HEAD"),
                p.target().as_str(),
            )
            .unwrap();
            failure(
                f.repo().preview_remove_worktree(&e, &token()),
                WorktreeFailure::ActiveOperation,
            );
            fs::remove_file(linked.info().git_dir.join("CHERRY_PICK_HEAD")).unwrap();
            f.run(&["-C", root, "update-index", "--skip-worktree", "hello.txt"]);
            fs::write(p.path().join("hello.txt"), "PRIVATE_HIDDEN_EDIT").unwrap();
            failure(
                f.repo().preview_remove_worktree(&e, &token()),
                WorktreeFailure::UnsupportedIndex,
            );
            assert_eq!(
                fs::read(p.path().join("hello.txt")).unwrap(),
                b"PRIVATE_HIDDEN_EDIT"
            );
            f.run(&[
                "-C",
                root,
                "update-index",
                "--no-skip-worktree",
                "hello.txt",
            ]);
            f.run(&["-C", root, "restore", "--", "hello.txt"]);
            f.run(&[
                "-C",
                root,
                "update-index",
                "--add",
                "--cacheinfo",
                &format!("160000,{},nested", p.target()),
            ]);
            failure(
                f.repo().preview_remove_worktree(&e, &token()),
                WorktreeFailure::UnsupportedIndex,
            );
            f.run(&["-C", root, "update-index", "--force-remove", "nested"]);
            let preview = f.repo().preview_remove_worktree(&e, &token()).unwrap();
            let recovery = preview.recovery_reference().unwrap();
            // A reference hook can introduce ignored data after the initial
            // clean check. The second check must preserve it before Git removes.
            let hook = f.root.join("no-hooks/reference-transaction");
            let quoted = root.replace('\\', "/").replace('\'', "'\\''");
            fs::write(&hook, format!("#!/bin/sh\nif [ \"$1\" = committed ]; then mkdir -p '{quoted}/ignored'; printf PRIVATE_AFTER_REVIEW > '{quoted}/ignored/after-review'; fi\n")).unwrap();
            let outcome = f.repo().apply_worktree(&preview, &token()).unwrap();
            assert_eq!(outcome.failure, Some(WorktreeFailure::Dirty));
            assert_eq!(
                fs::read(p.path().join("ignored/after-review")).unwrap(),
                b"PRIVATE_AFTER_REVIEW"
            );
            assert_eq!(
                outcome.recovery_reference.as_deref(),
                Some(recovery.as_str())
            );
            fs::remove_file(hook).unwrap();
            fs::remove_file(p.path().join("ignored/after-review")).unwrap();
            fs::remove_dir(p.path().join("ignored")).unwrap();
            f.write("second", "second");
            f.commit("other tip");
            f.run(&["update-ref", &recovery, "HEAD"]);
            let before = f.run(&["rev-parse", &recovery]);
            let outcome = f.repo().apply_worktree(&preview, &token()).unwrap();
            assert_eq!(outcome.failure, Some(WorktreeFailure::ReferenceConflict));
            assert!(outcome.recovery_reference.is_none());
            assert_eq!(f.run(&["rev-parse", &recovery]), before);
            assert!(p.path().join("hello.txt").exists());
        },
    );
}

#[test]
fn sha256_and_bare_repository_worktrees() {
    isolated(
        "ROCKET_WORKTREE_CASE",
        "sha256_and_bare_repository_worktrees",
        || {
            let f = Fixture::with_format("sha256");
            f.write("file", "base");
            f.commit("base");
            let p = create(&f, "sha256");
            apply(&f, &p);
            assert_eq!(p.target().as_str().len(), 64);
            let remove = f
                .repo()
                .preview_remove_worktree(&entry(&f, p.path()), &token())
                .unwrap();
            apply(&f, &remove);
            let bare = f.root.canonicalize().unwrap().join("bare.git");
            f.run(&[
                "clone",
                "--bare",
                ".",
                bare.to_str().unwrap().trim_start_matches(r"\\?\"),
            ]);
            let repo = f.git.open(&bare, &token()).unwrap();
            let list = repo.worktrees(&token()).unwrap();
            assert!(list[0].main && list[0].bare);
            let p = repo
                .preview_create_worktree(
                    &f.root.canonicalize().unwrap().join("bare-linked"),
                    WorktreeCheckout::ExistingBranch("main".into()),
                    "HEAD",
                    &token(),
                )
                .unwrap();
            let outcome = repo.apply_worktree(&p, &token()).unwrap();
            assert!(
                outcome.command.is_ok() && outcome.verification.is_ok(),
                "{outcome:?}"
            );
            let e = repo
                .worktrees(&token())
                .unwrap()
                .into_iter()
                .find(|e| !e.main)
                .unwrap();
            let p = repo.preview_remove_worktree(&e, &token()).unwrap();
            let outcome = repo.apply_worktree(&p, &token()).unwrap();
            assert!(
                outcome.command.is_ok() && outcome.verification.is_ok(),
                "{outcome:?}"
            );
        },
    );
}

#[test]
fn reftable_removal_matches_git_and_reports_incomplete_metadata_cleanup() {
    isolated(
        "ROCKET_WORKTREE_CASE",
        "reftable_removal_matches_git_and_reports_incomplete_metadata_cleanup",
        || {
            let f = seed();
            f.run(&["refs", "migrate", "--ref-format=reftable"]);
            let raw = f.root.canonicalize().unwrap().join("raw-git");
            let raw_arg = raw.to_str().unwrap().trim_start_matches(r"\\?\");
            f.run(&["worktree", "add", "-b", "raw-git", raw_arg]);
            let baseline = f.output(&f.repository, &["worktree", "remove", raw_arg]);
            let p = create(&f, "reftable");
            apply(&f, &p);
            let preview = f
                .repo()
                .preview_remove_worktree(&entry(&f, p.path()), &token())
                .unwrap();
            let result = f.repo().apply_worktree(&preview, &token()).unwrap();
            assert_eq!(
                result.command.is_ok(),
                baseline.status.success(),
                "{result:?}"
            );
            assert!(!p.path().exists());
            assert!(
                result
                    .observed
                    .as_ref()
                    .unwrap()
                    .iter()
                    .all(|e| e.path != p.path())
            );
            assert!(result.recovery_reference.is_some());
            if baseline.status.success() {
                assert!(result.verification.is_ok());
                assert!(result.retained_metadata.is_none());
            } else {
                // Git for Windows 2.55.0.windows.3 cannot finish reftable metadata
                // deletion. The direct Git baseline above must show that same error.
                assert!(String::from_utf8_lossy(&baseline.stderr).contains("failed to delete"));
                assert!(
                    matches!(&result.command, Err(Error::Git { stderr, .. }) if String::from_utf8_lossy(stderr).contains("failed to delete"))
                );
                assert!(result.verification.is_err());
                assert!(result.retained_metadata.as_ref().unwrap().is_dir());
                assert_eq!(result.failure, Some(WorktreeFailure::CommandFailed));
            }
        },
    );
}

#[test]
fn creation_preserves_other_worktree_relative_links() {
    isolated(
        "ROCKET_WORKTREE_CASE",
        "creation_preserves_other_worktree_relative_links",
        || {
            let f = seed();
            let neighbor = f.root.canonicalize().unwrap().join("neighbor");
            f.run(&[
                "worktree",
                "add",
                "--relative-paths",
                "-b",
                "neighbor",
                neighbor.to_str().unwrap().trim_start_matches(r"\\?\"),
            ]);
            let other = f.git.open(&neighbor, &token()).unwrap();
            let backlink = other.info().git_dir.join("gitdir");
            let before = (
                fs::read(neighbor.join(".git")).unwrap(),
                fs::read(&backlink).unwrap(),
                fs::read(f.repository.join(".git/config")).unwrap(),
            );
            let p = create(&f, "new-independent");
            apply(&f, &p);
            assert_eq!(
                before,
                (
                    fs::read(neighbor.join(".git")).unwrap(),
                    fs::read(&backlink).unwrap(),
                    fs::read(f.repository.join(".git/config")).unwrap()
                ),
                "Creating a worktree must not rewrite unrelated relative links"
            );
            fs::write(
                neighbor.join(".git"),
                b"gitdir: missing-unrelated-metadata\n",
            )
            .unwrap();
            let p = create(&f, "another-independent");
            apply(&f, &p);
            assert_eq!(
                fs::read(neighbor.join(".git")).unwrap(),
                b"gitdir: missing-unrelated-metadata\n",
                "Creation must not repair unrelated worktrees"
            );
        },
    );
}
