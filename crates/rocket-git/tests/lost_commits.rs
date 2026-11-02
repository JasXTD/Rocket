mod common;
use common::{Fixture, isolated};
use rocket_git::*;
use std::{fs, sync::mpsc::sync_channel};

const FILE: &str = "src/検索/結果 🚀 e\u{301}.rs";
fn id(f: &Fixture, rev: &str) -> ObjectId {
    f.repo()
        .resolve_commit(rev, &CancellationToken::new())
        .unwrap()
}
fn lost(f: &Fixture) -> (ObjectId, ObjectId, ObjectId) {
    f.write(FILE, "base\n");
    f.write(".gitignore", "ignored/\n");
    f.commit("PRIVATE_BASE");
    let base = id(f, "HEAD");
    f.run(&["switch", "-c", "lost/検索"]);
    f.write(FILE, "base\nlost parent\n");
    f.commit("PRIVATE_LOST_PARENT");
    let parent = id(f, "HEAD");
    f.write(FILE, "base\nlost parent\nlost tip\n");
    f.commit("PRIVATE_LOST_TIP");
    let tip = id(f, "HEAD");
    f.run(&["switch", "main"]);
    f.run(&["branch", "-D", "lost/検索"]);
    // Disposable fixture only. Production scans never expire reflogs or prune objects.
    f.run(&["reflog", "expire", "--expire=now", "--all"]);
    (base, parent, tip)
}
fn bytes(f: &Fixture, p: &str) -> Vec<u8> {
    fs::read(f.repository.join(p)).unwrap()
}

#[test]
fn scans_ancestors_and_recovers_unicode_with_dirty_files_sha1_sha256() {
    isolated(
        "ROCKET_LOST_CASE",
        "scans_ancestors_and_recovers_unicode_with_dirty_files_sha1_sha256",
        || {
            for format in ["sha1", "sha256"] {
                let f = Fixture::with_format(format);
                let (base, parent, tip) = lost(&f);
                f.write(FILE, "PRIVATE_STAGED\n");
                f.run(&["add", "--", FILE]);
                f.write(FILE, "PRIVATE_UNSTAGED\n");
                f.write("untracked 文", "PRIVATE_UNTRACKED");
                f.write("ignored/local", "PRIVATE_IGNORED");
                let before = [
                    bytes(&f, ".git/index"),
                    bytes(&f, ".git/config"),
                    bytes(&f, FILE),
                ];
                let repo = f.repo();
                let c = CancellationToken::new();
                let (tx, rx) = sync_channel(8);
                let scan = repo.scan_lost_commits(&c, Some(&tx)).unwrap();
                assert_eq!(scan.commits().len(), 2);
                assert!(scan.commits().iter().any(|x| x.id == parent));
                assert!(scan.commits().iter().any(|x| x.id == tip));
                assert!(scan.page(1).unwrap().more);
                assert!(matches!(
                    rx.try_iter().last(),
                    Some(LostCommitProgress::Complete(2))
                ));
                assert_eq!(
                    repo.reflog_scoped(ReflogScope::Unreachable, 200, &c)
                        .unwrap()
                        .entries
                        .len(),
                    2
                );
                assert!(
                    repo.preview_lost_recovery(&scan, &tip, "recovery/invalid name", &c)
                        .is_err()
                );
                let p = repo
                    .preview_lost_recovery(&scan, &tip, "recovery/検索-🚀", &c)
                    .unwrap();
                assert!(p.is_lost_commit());
                assert_eq!(p.target().parents, vec![parent.clone()]);
                let patch = String::from_utf8_lossy(p.patch());
                assert!(
                    patch.contains("検索"),
                    "Unicode paths must remain readable: {patch}"
                );
                assert!(patch.contains("+lost tip"), "{patch}");
                assert!(
                    !patch.contains("+lost parent"),
                    "must preview this commit, not HEAD: {patch}"
                );
                let r = repo.apply_recovery(&p, &c).unwrap();
                assert!(r.write.result.is_ok() && r.verification.is_ok(), "{r:?}");
                assert_eq!(id(&f, "recovery/検索-🚀"), tip);
                assert_eq!(id(&f, "HEAD"), base);
                assert_eq!(
                    [
                        bytes(&f, ".git/index"),
                        bytes(&f, ".git/config"),
                        bytes(&f, FILE)
                    ],
                    before
                );
                assert_eq!(bytes(&f, "untracked 文"), b"PRIVATE_UNTRACKED");
                assert_eq!(bytes(&f, "ignored/local"), b"PRIVATE_IGNORED");
                assert!(!f.repository.join(".git/lost-found").exists());
                assert!(
                    repo.scan_lost_commits(&c, None)
                        .unwrap()
                        .commits()
                        .is_empty()
                );
                assert!(matches!(
                    repo.preview_lost_recovery(&scan, &tip, "recovery/検索-🚀", &c),
                    Err(Error::Recovery(RecoveryFailure::BranchExists))
                ));
            }
        },
    );
}

#[test]
fn stale_review_collision_foreign_scan_and_cancel_never_write() {
    isolated(
        "ROCKET_LOST_CASE",
        "stale_review_collision_foreign_scan_and_cancel_never_write",
        || {
            let f = Fixture::new();
            let (_, _, tip) = lost(&f);
            let repo = f.repo();
            let c = CancellationToken::new();
            let scan = repo.scan_lost_commits(&c, None).unwrap();
            f.write(FILE, "local one");
            let p = repo
                .preview_lost_recovery(&scan, &tip, "recovery/local", &c)
                .unwrap();
            f.write(FILE, "local two");
            assert!(matches!(
                repo.apply_recovery(&p, &c),
                Err(Error::Recovery(RecoveryFailure::StalePreview))
            ));
            assert_eq!(bytes(&f, FILE), b"local two");
            let p = repo
                .preview_lost_recovery(&scan, &tip, "recovery/collision", &c)
                .unwrap();
            f.run(&["branch", "recovery/collision", "HEAD"]);
            assert!(matches!(
                repo.apply_recovery(&p, &c),
                Err(Error::Recovery(RecoveryFailure::BranchExists))
            ));
            assert_ne!(id(&f, "recovery/collision"), tip);
            let foreign = Fixture::new();
            foreign.write("x", "x");
            foreign.commit("foreign");
            assert!(matches!(
                foreign
                    .repo()
                    .preview_lost_recovery(&scan, &tip, "recovery/foreign", &c),
                Err(Error::Recovery(RecoveryFailure::StalePreview))
            ));
            let p = repo
                .preview_lost_recovery(&scan, &tip, "recovery/cancel", &c)
                .unwrap();
            c.cancel();
            assert!(matches!(repo.apply_recovery(&p, &c), Err(Error::Cancelled)));
            assert!(matches!(
                repo.scan_lost_commits(&c, None),
                Err(Error::Cancelled)
            ));
            assert_eq!(
                f.run(&[
                    "for-each-ref",
                    "--format=%(refname)",
                    "refs/heads/recovery/local",
                    "refs/heads/recovery/cancel"
                ]),
                b""
            );
        },
    );
}

#[test]
fn unavailable_objects_and_corrupt_scan_are_not_success_or_empty() {
    isolated(
        "ROCKET_LOST_CASE",
        "unavailable_objects_and_corrupt_scan_are_not_success_or_empty",
        || {
            let f = Fixture::new();
            let (_, _, tip) = lost(&f);
            let repo = f.repo();
            let c = CancellationToken::new();
            let scan = repo.scan_lost_commits(&c, None).unwrap();
            let p = repo
                .preview_lost_recovery(&scan, &tip, "recovery/missing", &c)
                .unwrap();
            // Prune only this owned fixture to simulate objects disappearing externally.
            f.run(&["prune", "--expire=now"]);
            assert!(matches!(
                repo.apply_recovery(&p, &c),
                Err(Error::Recovery(RecoveryFailure::LostCommitUnavailable))
            ));
            assert!(matches!(
                repo.preview_lost_recovery(&scan, &tip, "recovery/missing", &c),
                Err(Error::Recovery(RecoveryFailure::LostCommitUnavailable))
            ));
            let corrupt = f.repository.join(".git/objects/aa");
            fs::create_dir_all(&corrupt).unwrap();
            fs::write(corrupt.join("1".repeat(38)), b"corrupt").unwrap();
            assert!(matches!(
                repo.scan_lost_commits(&c, None),
                Err(Error::Recovery(RecoveryFailure::LostScanFailed))
            ));
            assert!(
                !f.repository
                    .join(".git/refs/heads/recovery/missing")
                    .exists()
            );
        },
    );
}

#[test]
fn unborn_and_linked_checkout_refs_are_protected() {
    isolated(
        "ROCKET_LOST_CASE",
        "unborn_and_linked_checkout_refs_are_protected",
        || {
            let f = Fixture::new();
            let (_, _, tip) = lost(&f);
            let repo = f.repo();
            let c = CancellationToken::new();
            let linked = f.root.join("linked 文");
            f.run(&[
                "worktree",
                "add",
                "--detach",
                linked.to_str().unwrap(),
                tip.as_str(),
            ]);
            f.run(&["reflog", "expire", "--expire=now", "--all"]);
            assert!(
                repo.scan_lost_commits(&c, None)
                    .unwrap()
                    .commits()
                    .is_empty(),
                "linked worktree HEAD is reachable"
            );
            f.run(&["worktree", "remove", linked.to_str().unwrap()]);
            let scan = repo.scan_lost_commits(&c, None).unwrap();
            f.run(&["symbolic-ref", "HEAD", "refs/heads/unborn"]);
            assert!(matches!(
                repo.preview_lost_recovery(&scan, &tip, "unborn", &c),
                Err(Error::Recovery(RecoveryFailure::BranchExists))
            ));
            let p = repo
                .preview_lost_recovery(&scan, &tip, "recovery/unborn", &c)
                .unwrap();
            let r = repo.apply_recovery(&p, &c).unwrap();
            assert!(r.write.result.is_ok() && r.verification.is_ok(), "{r:?}");
            assert!(matches!(repo.status(&c).unwrap().head, Head::Unborn { .. }));
            f.run(&["replace", tip.as_str(), "main"]);
            assert!(matches!(
                repo.scan_lost_commits(&c, None),
                Err(Error::Recovery(RecoveryFailure::LostHistoryUnsupported))
            ));
        },
    );
}

#[test]
fn root_merge_and_reftable_previews_use_the_original_commit_graph() {
    isolated(
        "ROCKET_LOST_CASE",
        "root_merge_and_reftable_previews_use_the_original_commit_graph",
        || {
            let f = Fixture::new();
            let (base, _, tip) = lost(&f);
            let tree =
                String::from_utf8(f.run(&["rev-parse", &format!("{tip}^{{tree}}")])).unwrap();
            let root_text =
                String::from_utf8(f.run(&["commit-tree", tree.trim(), "-m", "PRIVATE_ROOT"]))
                    .unwrap();
            let root = id(&f, root_text.trim());
            let merge_text = String::from_utf8(f.run(&[
                "commit-tree",
                tree.trim(),
                "-p",
                base.as_str(),
                "-p",
                tip.as_str(),
                "-m",
                "PRIVATE_MERGE",
            ]))
            .unwrap();
            let merge = id(&f, merge_text.trim());
            f.run(&["refs", "migrate", "--ref-format=reftable"]);
            let repo = f.repo();
            let c = CancellationToken::new();
            let scan = repo.scan_lost_commits(&c, None).unwrap();
            assert_eq!(scan.commits().len(), 4);
            let p = repo
                .preview_lost_recovery(&scan, &root, "recovery/root", &c)
                .unwrap();
            assert!(p.target().parents.is_empty());
            assert!(String::from_utf8_lossy(p.patch()).contains("+base"));
            let p = repo
                .preview_lost_recovery(&scan, &merge, "recovery/merge", &c)
                .unwrap();
            assert_eq!(p.target().parents, vec![base.clone(), tip]);
            assert!(String::from_utf8_lossy(p.patch()).contains("+lost parent"));
            let r = repo.apply_recovery(&p, &c).unwrap();
            assert!(r.write.result.is_ok() && r.verification.is_ok(), "{r:?}");
            assert_eq!(id(&f, "recovery/merge"), merge);
            assert_eq!(id(&f, "HEAD"), base);
        },
    );
}
