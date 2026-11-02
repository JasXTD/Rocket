mod common;
use common::Fixture;
use rocket_git::*;
use std::fs;

fn recorded(f: &Fixture, root: bool) -> UndoCommit {
    if !root {
        f.write("資料.txt", "base\n");
        f.commit("baseline");
    }
    f.write("資料.txt", "committed\n");
    f.run(&["add", "--all"]);
    let report = f
        .repo()
        .commit_with_undo("ordinary commit", &CancellationToken::new())
        .unwrap();
    report.write.result.unwrap();
    report.undo.expect("ordinary commit Undo")
}
#[test]
fn undo_normal_and_first_commit_preserves_index_working_and_untracked_bytes() {
    common::isolated(
        "ROCKET_UNDO_TEST",
        "undo_normal_and_first_commit_preserves_index_working_and_untracked_bytes",
        || {
            for (root, format) in [(false, "sha1"), (true, "sha1"), (false, "sha256")] {
                let f = Fixture::with_format(format);
                let record = recorded(&f, root);
                let c = CancellationToken::new();
                f.write("資料.txt", "unstaged external bytes\n");
                f.write("untracked 🚀", "keep");
                let index = fs::read(f.repository.join(".git/index")).unwrap();
                let preview = f.repo().preview_undo_commit(&record, &c).unwrap();
                let report = f.repo().apply_undo_commit(&preview, &c).unwrap();
                report.result.unwrap();
                report.verified.unwrap();
                assert_eq!(fs::read(f.repository.join(".git/index")).unwrap(), index);
                assert_eq!(
                    fs::read(f.repository.join("資料.txt")).unwrap(),
                    b"unstaged external bytes\n"
                );
                assert_eq!(
                    fs::read(f.repository.join("untracked 🚀")).unwrap(),
                    b"keep"
                );
                assert_eq!(
                    f.repo()
                        .resolve_commit(preview.recovery_reference(), &c)
                        .unwrap(),
                    *record.commit()
                );
                if root {
                    assert!(matches!(
                        f.repo().status(&c).unwrap().head,
                        Head::Unborn { .. }
                    ));
                } else {
                    assert_eq!(
                        f.repo().resolve_commit("HEAD", &c).unwrap(),
                        *record.previous().unwrap()
                    );
                }
            }
        },
    );
}
#[test]
fn stale_working_file_staging_head_and_other_checkout_are_refused() {
    common::isolated(
        "ROCKET_UNDO_TEST",
        "stale_working_file_staging_head_and_other_checkout_are_refused",
        || {
            let f = Fixture::new();
            let record = recorded(&f, false);
            let c = CancellationToken::new();
            f.write("資料.txt", "first external\n");
            let preview = f.repo().preview_undo_commit(&record, &c).unwrap();
            f.write("資料.txt", "other external\n");
            assert!(
                f.repo()
                    .apply_undo_commit(&preview, &c)
                    .unwrap_err()
                    .to_string()
                    .contains("undo_stale_preview")
            );
            f.run(&["add", "--all"]);
            assert!(
                f.repo()
                    .preview_undo_commit(&record, &c)
                    .unwrap_err()
                    .to_string()
                    .contains("undo_index_changed")
            );
            f.run(&["reset", "--mixed", "HEAD"]);
            f.run(&["switch", "-c", "other"]);
            assert!(
                f.repo()
                    .preview_undo_commit(&record, &c)
                    .unwrap_err()
                    .to_string()
                    .contains("undo_head_changed")
            );
            let other = Fixture::new();
            assert!(
                other
                    .repo()
                    .preview_undo_commit(&record, &c)
                    .unwrap_err()
                    .to_string()
                    .contains("undo_wrong_repository")
            );
            assert_eq!(
                fs::read(f.repository.join("資料.txt")).unwrap(),
                b"other external\n"
            );
        },
    );
}
#[test]
fn known_published_commit_active_operation_locks_and_precancel_are_refused() {
    common::isolated(
        "ROCKET_UNDO_TEST",
        "known_published_commit_active_operation_locks_and_precancel_are_refused",
        || {
            let f = Fixture::new();
            let record = recorded(&f, false);
            let c = CancellationToken::new();
            f.run(&[
                "update-ref",
                "refs/remotes/origin/main",
                record.commit().as_str(),
            ]);
            assert!(
                f.repo()
                    .preview_undo_commit(&record, &c)
                    .unwrap_err()
                    .to_string()
                    .contains("undo_shared_commit")
            );
            f.run(&["update-ref", "-d", "refs/remotes/origin/main"]);
            let p = f.repo().preview_undo_commit(&record, &c).unwrap();
            for lock in ["index.lock", "HEAD.lock"] {
                let path = f.repository.join(".git").join(lock);
                fs::write(&path, "other writer").unwrap();
                assert!(
                    f.repo()
                        .apply_undo_commit(&p, &c)
                        .unwrap_err()
                        .to_string()
                        .contains("undo_locked")
                );
                assert_eq!(fs::read(&path).unwrap(), b"other writer");
                fs::remove_file(path).unwrap();
            }
            let stopped = CancellationToken::new();
            stopped.cancel();
            assert!(matches!(
                f.repo().apply_undo_commit(&p, &stopped),
                Err(Error::Cancelled)
            ));
            fs::write(
                f.repository.join(".git/MERGE_HEAD"),
                record.commit().as_str(),
            )
            .unwrap();
            assert!(
                f.repo()
                    .preview_undo_commit(&record, &c)
                    .unwrap_err()
                    .to_string()
                    .contains("undo_active_operation")
            );
            assert_eq!(
                f.repo().resolve_commit("HEAD", &c).unwrap(),
                *record.commit()
            );
        },
    );
}

fn hook(f: &Fixture, body: &str) {
    let path = f.root.join("no-hooks/reference-transaction");
    fs::write(&path, body).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
    }
}

#[test]
fn cancellation_before_and_after_ref_update_reconciles_and_keeps_recovery() {
    common::isolated(
        "ROCKET_UNDO_TEST",
        "cancellation_before_and_after_ref_update_reconciles_and_keeps_recovery",
        || {
            for phase in ["prepared", "committed"] {
                let f = Fixture::new();
                let record = recorded(&f, false);
                let c = CancellationToken::new();
                f.write("資料.txt", "keep unstaged\n");
                f.write("untracked", "keep untracked");
                let index = fs::read(f.repository.join(".git/index")).unwrap();
                let p = f.repo().preview_undo_commit(&record, &c).unwrap();
                hook(
                    &f,
                    &format!(
                        "#!/bin/sh\nlines=$(cat)\nif [ \"$1\" = \"{phase}\" ] && printf '%s' \"$lines\" | grep -q 'refs/heads/main$'; then echo ready > cancel-ready; sleep 30; fi\n"
                    ),
                );
                let repo = f.repo();
                let token = c.clone();
                let review = p.clone();
                let worker = std::thread::spawn(move || repo.apply_undo_commit(&review, &token));
                let start = std::time::Instant::now();
                while !f.repository.join("cancel-ready").exists() {
                    assert!(!worker.is_finished(), "Undo ended before the hook barrier");
                    assert!(start.elapsed().as_secs() < 20, "Hook barrier timed out");
                    std::thread::sleep(std::time::Duration::from_millis(20));
                }
                let start = std::time::Instant::now();
                c.cancel();
                let report = worker.join().unwrap().unwrap();
                assert!(start.elapsed().as_secs() < 10);
                assert!(matches!(report.result, Err(Error::Cancelled)), "{report:?}");
                assert_eq!(report.verified.is_ok(), phase == "committed", "{report:?}");
                let fresh = CancellationToken::new();
                assert_eq!(
                    f.repo().resolve_commit("HEAD", &fresh).unwrap(),
                    if phase == "prepared" {
                        record.commit().clone()
                    } else {
                        record.previous().unwrap().clone()
                    }
                );
                assert_eq!(
                    f.repo()
                        .resolve_commit(p.recovery_reference(), &fresh)
                        .unwrap(),
                    *record.commit()
                );
                assert_eq!(fs::read(f.repository.join(".git/index")).unwrap(), index);
                assert_eq!(
                    fs::read(f.repository.join("資料.txt")).unwrap(),
                    b"keep unstaged\n"
                );
                assert_eq!(
                    fs::read(f.repository.join("untracked")).unwrap(),
                    b"keep untracked"
                );
                assert!(!f.repository.join(".git/index.lock").exists());
            }
        },
    );
}

#[test]
fn stale_branch_recovery_collision_and_rejected_hook_do_not_overwrite_refs() {
    common::isolated(
        "ROCKET_UNDO_TEST",
        "stale_branch_recovery_collision_and_rejected_hook_do_not_overwrite_refs",
        || {
            let f = Fixture::new();
            let record = recorded(&f, false);
            let c = CancellationToken::new();
            let p = f.repo().preview_undo_commit(&record, &c).unwrap();
            f.run(&[
                "update-ref",
                "refs/heads/main",
                record.previous().unwrap().as_str(),
            ]);
            assert!(
                f.repo()
                    .apply_undo_commit(&p, &c)
                    .unwrap_err()
                    .to_string()
                    .contains("undo_head_changed")
            );
            f.run(&["update-ref", "refs/heads/main", record.commit().as_str()]);
            f.run(&[
                "update-ref",
                p.recovery_reference(),
                record.previous().unwrap().as_str(),
            ]);
            let report = f.repo().apply_undo_commit(&p, &c).unwrap();
            assert!(report.result.is_err());
            assert!(report.verified.is_err());
            assert_eq!(
                f.repo().resolve_commit(p.recovery_reference(), &c).unwrap(),
                *record.previous().unwrap()
            );
            let p = f.repo().preview_undo_commit(&record, &c).unwrap();
            hook(
                &f,
                "#!/bin/sh\nlines=$(cat)\nif [ \"$1\" = prepared ] && printf '%s' \"$lines\" | grep -q 'refs/heads/main$'; then exit 1; fi\n",
            );
            let report = f.repo().apply_undo_commit(&p, &c).unwrap();
            assert!(report.result.is_err());
            assert!(report.verified.is_err());
            assert_eq!(
                f.repo().resolve_commit("HEAD", &c).unwrap(),
                *record.commit()
            );
            assert_eq!(
                f.repo().resolve_commit(p.recovery_reference(), &c).unwrap(),
                *record.commit()
            );
        },
    );
}
