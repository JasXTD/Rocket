mod common;
use common::{Fixture, isolated};
use rocket_git::*;
use std::fs;
const FILE: &str = "src/検索/結果 e\u{301} 🚀.rs";
fn id(f: &Fixture, revision: &str) -> ObjectId {
    f.repo()
        .resolve_commit(revision, &CancellationToken::new())
        .unwrap()
}
fn text(f: &Fixture, args: &[&str]) -> String {
    String::from_utf8(f.run(args)).unwrap().trim().into()
}
fn setup(format: &str) -> Fixture {
    let f = Fixture::with_format(format);
    f.write(FILE, "base\n");
    f.write(".gitignore", "ignored/\n");
    f.commit("PRIVATE_BASE 中文");
    f.write(FILE, "second\n");
    f.write("second", "second\n");
    f.commit("PRIVATE_SECOND 🚀");
    f
}
fn preview(f: &Fixture, mode: ResetMode) -> RecoveryPreview {
    f.repo()
        .preview_recovery(
            &id(f, "HEAD~"),
            RecoveryAction::Reset(mode),
            &CancellationToken::new(),
        )
        .unwrap()
}
fn apply(f: &Fixture, p: &RecoveryPreview) -> RecoveryWriteReport {
    f.repo()
        .apply_recovery(p, &CancellationToken::new())
        .unwrap()
}
fn ok(r: RecoveryWriteReport) {
    assert!(r.write.result.is_ok(), "{r:?}");
    assert!(r.verification.is_ok(), "{r:?}");
    assert!(r.branches.is_ok());
    assert!(r.write.status.is_ok());
    assert_eq!(r.failure, None);
}
fn refused<T: std::fmt::Debug>(r: Result<T>, want: RecoveryFailure) {
    assert!(matches!(&r,Err(Error::Recovery(f)) if *f==want), "{r:?}");
}
#[test]
fn reflog_window_and_branch_recovery_pin_commit_sha1_sha256() {
    isolated(
        "ROCKET_RECOVERY_CASE",
        "reflog_window_and_branch_recovery_pin_commit_sha1_sha256",
        || {
            for format in ["sha1", "sha256"] {
                let f = setup(format);
                let repo = f.repo();
                let log = repo.reflog(1, &CancellationToken::new()).unwrap();
                assert!(log.more);
                assert_eq!(log.entries.len(), 1);
                assert_eq!(log.entries[0].id, id(&f, "HEAD"));
                assert!(
                    String::from_utf8_lossy(&log.entries[0].subject).contains("PRIVATE_SECOND 🚀")
                );
                let selected = id(&f, "HEAD~");
                let p = repo
                    .preview_recovery(
                        &selected,
                        RecoveryAction::Branch("recovery/検索".into()),
                        &CancellationToken::new(),
                    )
                    .unwrap();
                let head = id(&f, "HEAD");
                // Move only reflog positions, leaving the reviewed branch/index/files intact.
                f.run(&[
                    "update-ref",
                    "-m",
                    "PRIVATE_REFLOG_SHIFT",
                    "HEAD",
                    head.as_str(),
                    head.as_str(),
                ]);
                ok(apply(&f, &p));
                assert_eq!(id(&f, "recovery/検索"), selected);
                assert_eq!(id(&f, "HEAD"), head);
                refused(
                    repo.preview_recovery(
                        &selected,
                        RecoveryAction::Branch("recovery/検索".into()),
                        &CancellationToken::new(),
                    ),
                    RecoveryFailure::BranchExists,
                );
                assert!(
                    repo.preview_recovery(
                        &selected,
                        RecoveryAction::Branch("--force".into()),
                        &CancellationToken::new()
                    )
                    .is_err()
                );
                let c = CancellationToken::new();
                c.cancel();
                assert!(matches!(repo.reflog(20, &c), Err(Error::Cancelled)));
                assert!(repo.reflog(2001, &CancellationToken::new()).is_err());
            }
            let f = Fixture::new();
            assert!(
                f.repo()
                    .reflog(200, &CancellationToken::new())
                    .unwrap()
                    .entries
                    .is_empty()
            );
        },
    );
}
#[test]
fn soft_keeps_staged_only_content_and_working_files() {
    isolated(
        "ROCKET_RECOVERY_CASE",
        "soft_keeps_staged_only_content_and_working_files",
        || {
            let f = setup("sha1");
            f.write(FILE, "PRIVATE_STAGED_ONLY\n");
            f.run(&["add", "--", FILE]);
            f.write(FILE, "PRIVATE_WORKING_ONLY\n");
            f.write("untracked", "PRIVATE_UNTRACKED\n");
            f.write("ignored/cache", "PRIVATE_IGNORED\n");
            let head = id(&f, "HEAD");
            let index = f.run(&["ls-files", "--stage", "-z"]);
            let config = fs::read(f.repository.join(".git/config")).unwrap();
            let p = preview(&f, ResetMode::Soft);
            ok(apply(&f, &p));
            assert_eq!(id(&f, "HEAD"), p.target().id);
            assert_eq!(f.run(&["ls-files", "--stage", "-z"]), index);
            assert_eq!(
                fs::read(f.repository.join(FILE)).unwrap(),
                b"PRIVATE_WORKING_ONLY\n"
            );
            assert_eq!(id(&f, &p.recovery_reference().unwrap()), head);
            assert_eq!(fs::read(f.repository.join(".git/config")).unwrap(), config);
            assert_eq!(
                fs::read(f.repository.join("untracked")).unwrap(),
                b"PRIVATE_UNTRACKED\n"
            );
            assert_eq!(
                fs::read(f.repository.join("ignored/cache")).unwrap(),
                b"PRIVATE_IGNORED\n"
            );
        },
    );
}
#[test]
fn mixed_keeps_files_and_hard_clean_only_preserves_unrelated_files() {
    isolated(
        "ROCKET_RECOVERY_CASE",
        "mixed_keeps_files_and_hard_clean_only_preserves_unrelated_files",
        || {
            for mode in [ResetMode::Mixed, ResetMode::Hard] {
                let f = setup("sha1");
                let old = id(&f, "HEAD");
                f.write("untracked", "PRIVATE_UNTRACKED\n");
                f.write("ignored/cache", "PRIVATE_IGNORED\n");
                if mode == ResetMode::Mixed {
                    f.write(FILE, "PRIVATE_UNSTAGED\n");
                }
                let p = preview(&f, mode);
                ok(apply(&f, &p));
                assert_eq!(id(&f, "HEAD"), p.target().id);
                assert_eq!(id(&f, &p.recovery_reference().unwrap()), old);
                assert!(f.run(&["diff", "--cached", "--name-only"]).is_empty());
                assert_eq!(
                    fs::read(f.repository.join(FILE)).unwrap(),
                    if mode == ResetMode::Mixed {
                        b"PRIVATE_UNSTAGED\n".to_vec()
                    } else {
                        b"base\n".to_vec()
                    }
                );
                assert_eq!(
                    f.repository.join("second").exists(),
                    mode == ResetMode::Mixed
                );
                assert_eq!(
                    fs::read(f.repository.join("untracked")).unwrap(),
                    b"PRIVATE_UNTRACKED\n"
                );
                assert_eq!(
                    fs::read(f.repository.join("ignored/cache")).unwrap(),
                    b"PRIVATE_IGNORED\n"
                );
            }
        },
    );
}
#[test]
fn unsafe_reset_preflights_preserve_index_and_files() {
    isolated(
        "ROCKET_RECOVERY_CASE",
        "unsafe_reset_preflights_preserve_index_and_files",
        || {
            let f = setup("sha1");
            let target = id(&f, "HEAD~");
            f.write(FILE, "PRIVATE_STAGED\n");
            f.run(&["add", "--", FILE]);
            let index = fs::read(f.repository.join(".git/index")).unwrap();
            for mode in [ResetMode::Mixed, ResetMode::Hard] {
                refused(
                    f.repo().preview_recovery(
                        &target,
                        RecoveryAction::Reset(mode),
                        &CancellationToken::new(),
                    ),
                    RecoveryFailure::StagedChanges,
                );
            }
            assert_eq!(fs::read(f.repository.join(".git/index")).unwrap(), index);
            f.run(&["restore", "--staged", "--", FILE]);
            refused(
                f.repo().preview_recovery(
                    &target,
                    RecoveryAction::Reset(ResetMode::Hard),
                    &CancellationToken::new(),
                ),
                RecoveryFailure::DirtyWorktree,
            );
            f.run(&["restore", "--", FILE]);
            for flag in ["--assume-unchanged", "--skip-worktree"] {
                let hidden = setup("sha1");
                hidden.run(&["update-index", flag, "--", FILE]);
                hidden.write(FILE, "PRIVATE_HIDDEN\n");
                refused(
                    hidden.repo().preview_recovery(
                        &id(&hidden, "HEAD~"),
                        RecoveryAction::Reset(ResetMode::Hard),
                        &CancellationToken::new(),
                    ),
                    RecoveryFailure::UnsupportedIndex,
                );
                assert_eq!(
                    fs::read(hidden.repository.join(FILE)).unwrap(),
                    b"PRIVATE_HIDDEN\n"
                );
            }
            f.run(&["switch", "--detach"]);
            refused(
                f.repo().preview_recovery(
                    &target,
                    RecoveryAction::Reset(ResetMode::Soft),
                    &CancellationToken::new(),
                ),
                RecoveryFailure::BranchRequired,
            );
        },
    );
}
#[test]
fn hard_reset_rejects_untracked_and_ignored_target_collisions() {
    isolated(
        "ROCKET_RECOVERY_CASE",
        "hard_reset_rejects_untracked_and_ignored_target_collisions",
        || {
            for ignored in [false, true] {
                let f = setup("sha1");
                let name = if ignored {
                    "ignored/keep"
                } else {
                    "untracked/keep"
                };
                f.write(name, "committed\n");
                f.run(&["add", "-f", "--", name]);
                f.run(&["commit", "-m", "collision target"]);
                let target = id(&f, "HEAD");
                f.run(&["reset", "--hard", "HEAD~"]);
                f.write(name, "PRIVATE_KEEP\n");
                let head = id(&f, "HEAD");
                refused(
                    f.repo().preview_recovery(
                        &target,
                        RecoveryAction::Reset(ResetMode::Hard),
                        &CancellationToken::new(),
                    ),
                    RecoveryFailure::LocalFileCollision,
                );
                assert_eq!(id(&f, "HEAD"), head);
                assert_eq!(
                    fs::read(f.repository.join(name)).unwrap(),
                    b"PRIVATE_KEEP\n"
                );
            }
        },
    );
}
#[test]
fn stale_cancelled_cross_repo_and_active_operation_are_refused() {
    isolated(
        "ROCKET_RECOVERY_CASE",
        "stale_cancelled_cross_repo_and_active_operation_are_refused",
        || {
            let f = setup("sha1");
            let p = preview(&f, ResetMode::Soft);
            f.write(FILE, "PRIVATE_EXTERNAL\n");
            refused(
                f.repo().apply_recovery(&p, &CancellationToken::new()),
                RecoveryFailure::StalePreview,
            );
            assert_eq!(
                fs::read(f.repository.join(FILE)).unwrap(),
                b"PRIVATE_EXTERNAL\n"
            );
            let p = preview(&f, ResetMode::Soft);
            f.write(FILE, "PRIVATE_SAME_STATUS\n");
            refused(
                f.repo().apply_recovery(&p, &CancellationToken::new()),
                RecoveryFailure::StalePreview,
            );
            let p = preview(&f, ResetMode::Soft);
            let c = CancellationToken::new();
            c.cancel();
            assert!(matches!(
                f.repo().apply_recovery(&p, &c),
                Err(Error::Cancelled)
            ));
            assert!(
                !f.repository
                    .join(".git/refs/rocket/recovery/reset")
                    .exists()
            );
            let other = setup("sha1");
            refused(
                other.repo().apply_recovery(&p, &CancellationToken::new()),
                RecoveryFailure::StalePreview,
            );
            fs::create_dir(f.repository.join(".git/rebase-merge")).unwrap();
            refused(
                f.repo().preview_recovery(
                    &id(&f, "HEAD~"),
                    RecoveryAction::Reset(ResetMode::Soft),
                    &CancellationToken::new(),
                ),
                RecoveryFailure::ActiveOperation,
            );
        },
    );
}
#[test]
fn locks_and_conflicting_recovery_refs_are_not_removed() {
    isolated(
        "ROCKET_RECOVERY_CASE",
        "locks_and_conflicting_recovery_refs_are_not_removed",
        || {
            let f = setup("sha1");
            let p = preview(&f, ResetMode::Hard);
            let original = id(&f, "HEAD");
            fs::write(
                f.repository.join(".git/index.lock"),
                b"PRIVATE_OTHER_CLIENT",
            )
            .unwrap();
            let report = apply(&f, &p);
            assert_eq!(report.failure, Some(RecoveryFailure::IndexLocked));
            assert!(report.write.result.is_err());
            assert!(report.write.status.is_ok());
            assert_eq!(id(&f, "HEAD"), original);
            assert_eq!(
                fs::read(f.repository.join(".git/index.lock")).unwrap(),
                b"PRIVATE_OTHER_CLIENT"
            );
            let other = setup("sha1");
            let p = preview(&other, ResetMode::Soft);
            other.run(&[
                "update-ref",
                &p.recovery_reference().unwrap(),
                p.target().id.as_str(),
            ]);
            refused(
                other.repo().apply_recovery(&p, &CancellationToken::new()),
                RecoveryFailure::RecoveryReferenceConflict,
            );
            assert_eq!(
                text(&other, &["log", "-1", "--format=%s"]),
                "PRIVATE_SECOND 🚀"
            );
        },
    );
}
#[test]
fn linked_worktree_reflog_and_reset_use_its_own_head() {
    isolated(
        "ROCKET_RECOVERY_CASE",
        "linked_worktree_reflog_and_reset_use_its_own_head",
        || {
            let f = setup("sha1");
            let original = id(&f, "HEAD");
            let linked = f.root.join("linked 文");
            f.run(&[
                "worktree",
                "add",
                "-b",
                "feature/linked",
                linked.to_str().unwrap(),
            ]);
            let r = f.git.open(&linked, &CancellationToken::new()).unwrap();
            assert_eq!(
                r.reflog(20, &CancellationToken::new()).unwrap().entries[0].id,
                original
            );
            let p = r
                .preview_recovery(
                    &id(&f, "HEAD~"),
                    RecoveryAction::Reset(ResetMode::Hard),
                    &CancellationToken::new(),
                )
                .unwrap();
            ok(r.apply_recovery(&p, &CancellationToken::new()).unwrap());
            assert_eq!(id(&f, "HEAD"), original);
            assert_eq!(fs::read(f.repository.join(FILE)).unwrap(), b"second\n");
            assert_eq!(fs::read(linked.join(FILE)).unwrap(), b"base\n");
        },
    );
}

#[test]
fn hard_reset_refuses_gitlinks_in_current_or_target_tree() {
    isolated(
        "ROCKET_RECOVERY_CASE",
        "hard_reset_refuses_gitlinks_in_current_or_target_tree",
        || {
            let f = setup("sha1");
            let original = id(&f, "HEAD");
            f.run(&[
                "update-index",
                "--add",
                "--cacheinfo",
                &format!("160000,{},module", original.as_str()),
            ]);
            f.run(&["commit", "-m", "PRIVATE_GITLINK"]);
            fs::create_dir(f.repository.join("module")).unwrap();
            let with_module = id(&f, "HEAD");
            assert!(f.run(&["status", "--porcelain"]).is_empty());
            refused(
                f.repo().preview_recovery(
                    &original,
                    RecoveryAction::Reset(ResetMode::Hard),
                    &CancellationToken::new(),
                ),
                RecoveryFailure::UnsupportedIndex,
            );
            f.run(&["reset", "--hard", original.as_str()]);
            refused(
                f.repo().preview_recovery(
                    &with_module,
                    RecoveryAction::Reset(ResetMode::Hard),
                    &CancellationToken::new(),
                ),
                RecoveryFailure::UnsupportedIndex,
            );
            assert_eq!(id(&f, "HEAD"), original);
        },
    );
}

#[test]
fn sha256_reset_keeps_existing_recovery_ref_and_refuses_symbolic_replacement() {
    isolated(
        "ROCKET_RECOVERY_CASE",
        "sha256_reset_keeps_existing_recovery_ref_and_refuses_symbolic_replacement",
        || {
            let f = setup("sha256");
            let original = id(&f, "HEAD");
            let p = preview(&f, ResetMode::Hard);
            let reference = p.recovery_reference().unwrap();
            f.run(&["update-ref", &reference, original.as_str()]);
            ok(apply(&f, &p));
            assert_eq!(id(&f, &reference), original);
            assert_eq!(id(&f, "HEAD"), p.target().id);
            f.run(&["reset", "--hard", original.as_str()]);
            f.run(&["symbolic-ref", &reference, "refs/heads/main"]);
            let p = preview(&f, ResetMode::Soft);
            refused(
                f.repo().apply_recovery(&p, &CancellationToken::new()),
                RecoveryFailure::RecoveryReferenceConflict,
            );
            assert_eq!(id(&f, "HEAD"), original);
            assert_eq!(text(&f, &["symbolic-ref", &reference]), "refs/heads/main");
        },
    );
}

fn scope(f: &Fixture, scope: ReflogScope, limit: usize) -> ReflogPage {
    f.repo()
        .reflog_scoped(scope, limit, &CancellationToken::new())
        .unwrap()
}

#[test]
fn deleted_branches_are_found_at_their_last_checked_out_tip_and_restored() {
    isolated(
        "ROCKET_RECOVERY_CASE",
        "deleted_branches_are_found_at_their_last_checked_out_tip_and_restored",
        || {
            for format in ["sha1", "sha256"] {
                let f = setup(format);
                f.run(&["switch", "-q", "-c", "feature/検索"]);
                f.write("feature", "PRIVATE_FEATURE\n");
                f.commit("PRIVATE_FEATURE_COMMIT");
                let lost = id(&f, "HEAD");
                f.run(&["switch", "-q", "main"]);
                f.run(&["branch", "-q", "-D", "feature/検索"]);
                // A deleted then recreated name, and a detached visit, are not candidates.
                f.run(&["switch", "-q", "-c", "kept"]);
                f.run(&["switch", "-q", "main"]);
                f.run(&["branch", "-q", "-D", "kept"]);
                f.run(&["branch", "-q", "kept"]);
                f.run(&["switch", "-q", "--detach", "HEAD~"]);
                f.run(&["switch", "-q", "main"]);
                let page = scope(&f, ReflogScope::DeletedBranches, 20);
                assert_eq!(page.entries.len(), 1, "{:?}", page.entries);
                let entry = &page.entries[0];
                assert_eq!(entry.id, lost);
                assert_eq!(entry.reference.as_deref(), Some("feature/検索".as_bytes()));
                assert!(String::from_utf8_lossy(&entry.selector).starts_with("HEAD@{20"));
                assert!(
                    String::from_utf8_lossy(&entry.subject)
                        .starts_with("checkout: moving from feature/検索 to main")
                );
                // Git's own all-refs history no longer has the deleted branch's log.
                assert!(
                    !scope(&f, ReflogScope::AllRefs, 200)
                        .entries
                        .iter()
                        .any(|e| String::from_utf8_lossy(&e.selector).contains("feature/検索"))
                );
                let p = f
                    .repo()
                    .preview_recovery(
                        &entry.id,
                        RecoveryAction::Branch("feature/検索".into()),
                        &CancellationToken::new(),
                    )
                    .unwrap();
                ok(apply(&f, &p));
                assert_eq!(id(&f, "feature/検索"), lost);
                assert!(
                    scope(&f, ReflogScope::DeletedBranches, 20)
                        .entries
                        .is_empty()
                );
            }
        },
    );
}

#[test]
fn retained_refs_and_unreachable_commits_are_recovery_scopes() {
    isolated(
        "ROCKET_RECOVERY_CASE",
        "retained_refs_and_unreachable_commits_are_recovery_scopes",
        || {
            let f = setup("sha1");
            // A Rocket reset retains the original tip under refs/rocket/.
            let original = id(&f, "HEAD");
            let p = preview(&f, ResetMode::Soft);
            ok(apply(&f, &p));
            f.run(&["tag", "-a", "-m", "PRIVATE_TAG", "annotated", "HEAD"]);
            let tag = id(&f, "annotated");
            let tag_object = text(&f, &["rev-parse", "annotated"]);
            f.run(&[
                "update-ref",
                &format!("refs/rocket/deleted-tags/{tag_object}"),
                &tag_object,
            ]);
            let tree = text(&f, &["rev-parse", "HEAD^{tree}"]);
            f.run(&["update-ref", "refs/rocket/deleted-tags/tree", &tree]);
            let retained = scope(&f, ReflogScope::Retained, 20);
            let ids: Vec<_> = retained.entries.iter().map(|e| e.id.clone()).collect();
            assert!(ids.contains(&original), "{:?}", retained.entries);
            assert!(ids.contains(&tag));
            assert_eq!(
                retained.entries.len(),
                2,
                "trees are not recoverable commits"
            );
            let reset_entry = retained.entries.iter().find(|e| e.id == original).unwrap();
            assert_eq!(
                reset_entry.reference.as_deref(),
                p.recovery_reference().as_deref().map(str::as_bytes)
            );
            assert_eq!(reset_entry.subject, "PRIVATE_SECOND 🚀".as_bytes());
            // A dropped stash is a dangling commit.
            f.write(FILE, "PRIVATE_STASHED\n");
            f.run(&["stash", "push", "-q", "-m", "PRIVATE_DROPPED_STASH"]);
            let stash = id(&f, "stash@{0}");
            f.run(&["stash", "drop", "-q"]);
            let lost = scope(&f, ReflogScope::Unreachable, 20);
            let entry = lost.entries.iter().find(|e| e.id == stash).unwrap();
            assert!(String::from_utf8_lossy(&entry.subject).contains("PRIVATE_DROPPED_STASH"));
            assert!(entry.reference.is_none() && entry.selector.is_empty());
            let p = f
                .repo()
                .preview_recovery(
                    &stash,
                    RecoveryAction::Branch("recovered-stash".into()),
                    &CancellationToken::new(),
                )
                .unwrap();
            ok(apply(&f, &p));
            assert_eq!(id(&f, "recovered-stash"), stash);
            assert!(
                !scope(&f, ReflogScope::Unreachable, 20)
                    .entries
                    .iter()
                    .any(|e| e.id == stash)
            );
            let c = CancellationToken::new();
            c.cancel();
            for s in [
                ReflogScope::DeletedBranches,
                ReflogScope::Retained,
                ReflogScope::Unreachable,
            ] {
                assert!(matches!(
                    f.repo().reflog_scoped(s, 20, &c),
                    Err(Error::Cancelled)
                ));
            }
            // Deleted-branch discovery reads the files backend only.
            let table = f.root.join("reftable");
            fs::create_dir(&table).unwrap();
            f.run_at(&table, &["init", "-q", "--ref-format=reftable", "."]);
            let r = f.git.open(&table, &CancellationToken::new()).unwrap();
            assert!(matches!(
                r.reflog_scoped(ReflogScope::DeletedBranches, 20, &CancellationToken::new()),
                Err(Error::Unsupported(_))
            ));
        },
    );
}
