#![cfg(any(windows, target_os = "linux"))]
mod common;
use common::{Fixture, isolated};
use rocket_git::*;
use std::fs;

const NAME: &str = "src/界面/问候 🚀 e\u{301}.rs";
fn conflict(format: &str) -> Fixture {
    let f = Fixture::with_format(format);
    f.write(NAME, "header\nbase\nfooter\n");
    f.commit("base");
    f.run(&["checkout", "-b", "other"]);
    f.write(NAME, "header\ntheirs 界\nfooter\n");
    f.commit("theirs");
    f.run(&["checkout", "main"]);
    f.write(NAME, "header\nours 🚀\nfooter\n");
    f.commit("ours");
    assert!(
        !f.output(&f.repository, &["merge", "other"])
            .status
            .success()
    );
    f
}
fn path() -> RepoPath {
    RepoPath::new(NAME.as_bytes().to_vec()).unwrap()
}

#[test]
fn unicode_save_then_stage_preserves_history_and_recovery() {
    isolated(
        "ROCKET_CONFLICT_CASE",
        "unicode_save_then_stage_preserves_history_and_recovery",
        || {
            for format in ["sha1", "sha256"] {
                let f = conflict(format);
                let repo = f.repo();
                let c = CancellationToken::new();
                let head = f.run(&["rev-parse", "HEAD"]);
                let original = fs::read(f.repository.join(NAME)).unwrap();
                let index = fs::read(f.repository.join(".git/index")).unwrap();
                let p = repo.edit_conflict(&path(), &c).unwrap();
                assert!(p.side(1).unwrap().contains("base"));
                assert!(p.side(2).unwrap().contains("ours 🚀"));
                assert!(p.side(3).unwrap().contains("theirs 界"));
                let text = "header\ncombined 世界 e\u{301} 🚀\nfooter\n";
                let saved = repo.save_conflict(&p, text, false, &c).unwrap();
                saved.result.unwrap();
                assert!(saved.saved && !saved.staged);
                assert_eq!(fs::read(f.repository.join(".git/index")).unwrap(), index);
                let recovery = saved.recovery_directory.unwrap();
                assert_eq!(
                    fs::read(recovery.join("replaced-original")).unwrap(),
                    original
                );
                assert_eq!(fs::read(recovery.join("result")).unwrap(), text.as_bytes());
                let p = repo.edit_conflict(&path(), &c).unwrap();
                let staged = repo.save_conflict(&p, text, true, &c).unwrap();
                staged.result.unwrap();
                assert!(staged.saved && staged.staged);
                assert!(
                    !staged
                        .status
                        .unwrap()
                        .entries
                        .iter()
                        .any(|e| e.kind == EntryKind::Unmerged)
                );
                assert!(!staged.state.unwrap().is_idle());
                assert_eq!(f.run(&["show", &format!(":{NAME}")]), text.as_bytes());
                assert_eq!(f.run(&["rev-parse", "HEAD"]), head);
            }
        },
    );
}

#[test]
fn external_edit_context_change_and_cancel_preserve_data() {
    isolated(
        "ROCKET_CONFLICT_CASE",
        "external_edit_context_change_and_cancel_preserve_data",
        || {
            let f = conflict("sha1");
            let repo = f.repo();
            let c = CancellationToken::new();
            let p = repo.edit_conflict(&path(), &c).unwrap();
            let index = fs::read(f.repository.join(".git/index")).unwrap();
            let cancelled = CancellationToken::new();
            cancelled.cancel();
            assert!(matches!(
                repo.save_conflict(&p, "draft\n", false, &cancelled),
                Err(Error::Cancelled)
            ));
            assert_eq!(
                fs::read(f.repository.join(NAME)).unwrap(),
                p.working().as_bytes()
            );
            f.write(NAME, "external editor 世界\n");
            let r = repo.save_conflict(&p, "draft\n", true, &c).unwrap();
            assert!(matches!(
                r.result,
                Err(Error::Conflict(ConflictFailure::FileChanged))
            ));
            assert!(!r.saved && r.recovery_directory.is_none());
            assert_eq!(
                fs::read(f.repository.join(NAME)).unwrap(),
                "external editor 世界\n".as_bytes()
            );
            assert_eq!(fs::read(f.repository.join(".git/index")).unwrap(), index);
            let p = repo.edit_conflict(&path(), &c).unwrap();
            f.run(&["add", "--", NAME]);
            let r = repo.save_conflict(&p, "draft\n", false, &c).unwrap();
            assert!(matches!(
                r.result,
                Err(Error::Conflict(ConflictFailure::NotConflicted))
            ));
            assert!(!r.saved);
            assert_eq!(
                fs::read(f.repository.join(NAME)).unwrap(),
                "external editor 世界\n".as_bytes()
            );
        },
    );
}

#[test]
fn crlf_bom_markers_and_locked_index_have_truthful_outcomes() {
    isolated(
        "ROCKET_CONFLICT_CASE",
        "crlf_bom_markers_and_locked_index_have_truthful_outcomes",
        || {
            let f = conflict("sha1");
            let c = CancellationToken::new();
            let repo = f.repo();
            let source = fs::read_to_string(f.repository.join(NAME))
                .unwrap()
                .replace('\n', "\r\n");
            f.write(NAME, format!("\u{feff}{source}"));
            let p = repo.edit_conflict(&path(), &c).unwrap();
            assert!(p.bom() && p.crlf());
            assert!(matches!(
                repo.save_conflict(&p, p.working(), true, &c),
                Err(Error::Conflict(ConflictFailure::MarkersRemain))
            ));
            f.write(".git/index.lock", "test-owned lock");
            let r = repo.save_conflict(&p, "resolved 界\n", true, &c).unwrap();
            assert!(r.saved && !r.staged && r.result.is_err());
            assert_eq!(
                fs::read(f.repository.join(NAME)).unwrap(),
                "\u{feff}resolved 界\r\n".as_bytes()
            );
            assert_eq!(
                fs::read(f.repository.join(".git/index.lock")).unwrap(),
                b"test-owned lock"
            );
            assert!(
                r.status
                    .unwrap()
                    .entries
                    .iter()
                    .any(|e| e.kind == EntryKind::Unmerged)
            );
            assert!(
                r.recovery_directory
                    .unwrap()
                    .join("replaced-original")
                    .is_file()
            );
        },
    );
}

#[test]
fn unsupported_and_foreign_previews_are_refused() {
    isolated(
        "ROCKET_CONFLICT_CASE",
        "unsupported_and_foreign_previews_are_refused",
        || {
            let f = conflict("sha1");
            let c = CancellationToken::new();
            let repo = f.repo();
            let p = repo.edit_conflict(&path(), &c).unwrap();
            let other = conflict("sha1");
            assert!(matches!(
                other.repo().save_conflict(&p, "draft", false, &c),
                Err(Error::Conflict(ConflictFailure::WrongRepository))
            ));
            for content in [
                vec![0, 1],
                vec![0xff],
                b"mixed\r\nline\n".to_vec(),
                vec![b'a'; CONFLICT_TEXT_LIMIT + 1],
            ] {
                f.write(NAME, &content);
                assert!(matches!(
                    repo.edit_conflict(&path(), &c),
                    Err(Error::Conflict(ConflictFailure::UnsupportedFile))
                ));
                assert_eq!(fs::read(f.repository.join(NAME)).unwrap(), content);
            }
        },
    );
}

#[test]
fn branch_identity_readonly_and_missing_file_prevent_overwrites() {
    isolated(
        "ROCKET_CONFLICT_CASE",
        "branch_identity_readonly_and_missing_file_prevent_overwrites",
        || {
            let f = conflict("sha1");
            let c = CancellationToken::new();
            let repo = f.repo();
            let p = repo.edit_conflict(&path(), &c).unwrap();
            let file = f.repository.join(NAME);
            let original = fs::read(&file).unwrap();
            let sibling = f.root.join("preserved-hardlink");
            fs::hard_link(&file, &sibling).unwrap();
            let original_permissions = fs::metadata(&file).unwrap().permissions();
            let mut permissions = original_permissions.clone();
            permissions.set_readonly(true);
            fs::set_permissions(&file, permissions).unwrap();
            let r = repo.save_conflict(&p, "resolved\n", false, &c).unwrap();
            fs::set_permissions(&file, original_permissions).unwrap();
            assert!(!r.saved && r.result.unwrap_err().to_string().contains("read-only"));
            assert_eq!(fs::read(&file).unwrap(), original);
            // Same object ID under a different symbolic HEAD must invalidate the binding.
            f.run(&["branch", "same-tip"]);
            f.run(&["symbolic-ref", "HEAD", "refs/heads/same-tip"]);
            let r = repo.save_conflict(&p, "resolved\n", false, &c).unwrap();
            assert!(
                !r.saved
                    && matches!(
                        r.result,
                        Err(Error::Conflict(ConflictFailure::ContextChanged))
                    )
            );
            assert_eq!(fs::read(&file).unwrap(), original);
            f.run(&["symbolic-ref", "HEAD", "refs/heads/main"]);
            fs::rename(&file, f.repository.join("retained-working")).unwrap();
            let r = repo.save_conflict(&p, "resolved\n", false, &c).unwrap();
            assert!(!r.saved && r.result.is_err());
            assert!(!file.exists());
            assert_eq!(
                fs::read(f.repository.join("retained-working")).unwrap(),
                original
            );
            fs::rename(f.repository.join("retained-working"), &file).unwrap();
            let r = repo.save_conflict(&p, "replacement\n", false, &c).unwrap();
            r.result.unwrap();
            assert!(r.saved);
            assert_eq!(fs::read(&sibling).unwrap(), original);
        },
    );
}

#[test]
fn add_add_without_base_and_unfinished_save_preserve_exact_text() {
    isolated(
        "ROCKET_CONFLICT_CASE",
        "add_add_without_base_and_unfinished_save_preserve_exact_text",
        || {
            let f = Fixture::new();
            f.write("keep", "base");
            f.commit("base");
            f.run(&["checkout", "-b", "other"]);
            f.write(NAME, "theirs\n");
            f.commit("theirs");
            f.run(&["checkout", "main"]);
            f.write(NAME, "ours\n");
            f.commit("ours");
            assert!(
                !f.output(&f.repository, &["merge", "other"])
                    .status
                    .success()
            );
            let c = CancellationToken::new();
            let repo = f.repo();
            let p = repo.edit_conflict(&path(), &c).unwrap();
            assert!(p.side(1).is_none());
            let text = "<<<<<<< incomplete\nprivate draft 界";
            let r = repo.save_conflict(&p, text, false, &c).unwrap();
            r.result.unwrap();
            assert!(r.saved && !r.staged);
            assert_eq!(fs::read(f.repository.join(NAME)).unwrap(), text.as_bytes());
            let p = r.preview.unwrap();
            assert_eq!(p.working(), text);
            assert!(matches!(
                repo.save_conflict(&p, text, true, &c),
                Err(Error::Conflict(ConflictFailure::MarkersRemain))
            ));
            let text = "\u{feff}literal leading character\n";
            let r = repo.save_conflict(&p, text, false, &c).unwrap();
            r.result.unwrap();
            assert_eq!(fs::read(f.repository.join(NAME)).unwrap(), text.as_bytes());
            assert_eq!(r.preview.unwrap().working(), text);
        },
    );
}

#[test]
fn stash_conflict_resolution_needs_no_operation_marker_and_keeps_stash() {
    isolated(
        "ROCKET_CONFLICT_CASE",
        "stash_conflict_resolution_needs_no_operation_marker_and_keeps_stash",
        || {
            let f = Fixture::new();
            f.write(NAME, "base\n");
            f.commit("base");
            f.write(NAME, "saved changes 界\n");
            f.run(&["stash", "push", "-m", "synthetic stash"]);
            f.write(NAME, "committed changes 🚀\n");
            f.commit("diverged");
            let head = f.run(&["rev-parse", "HEAD"]);
            let stash = f.run(&["rev-parse", "refs/stash"]);
            assert!(
                !f.output(&f.repository, &["stash", "apply"])
                    .status
                    .success()
            );
            let c = CancellationToken::new();
            let repo = f.repo();
            assert!(repo.operation_state(&c).unwrap().is_idle());
            let p = repo.edit_conflict(&path(), &c).unwrap();
            let r = repo
                .save_conflict(&p, "combined changes 界 🚀\n", true, &c)
                .unwrap();
            r.result.unwrap();
            assert!(r.saved && r.staged && r.state.unwrap().is_idle());
            assert!(
                !r.status
                    .unwrap()
                    .entries
                    .iter()
                    .any(|e| e.kind == EntryKind::Unmerged)
            );
            assert_eq!(f.run(&["rev-parse", "HEAD"]), head);
            assert_eq!(f.run(&["rev-parse", "refs/stash"]), stash);
        },
    );
}

/// Linux: a save keeps the working file's executable bit, so staging the resolution
/// records the mode Git's merge checked out and leaves no mode change behind.
#[cfg(target_os = "linux")]
#[test]
fn saving_keeps_an_executable_file_executable() {
    isolated(
        "ROCKET_CONFLICT_CASE",
        "saving_keeps_an_executable_file_executable",
        || {
            use std::os::unix::fs::PermissionsExt;
            let f = Fixture::new();
            let script = f.repository.join("run.sh");
            f.write("run.sh", "#!/bin/sh\nbase\n");
            fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
            f.commit("base");
            f.run(&["checkout", "-b", "other"]);
            f.write("run.sh", "#!/bin/sh\ntheirs\n");
            f.commit("theirs");
            f.run(&["checkout", "main"]);
            f.write("run.sh", "#!/bin/sh\nours\n");
            f.commit("ours");
            assert!(!f.output(&f.repository, &["merge", "other"]).status.success());
            let mode = fs::metadata(&script).unwrap().permissions().mode() & 0o777;
            assert_ne!(mode & 0o111, 0, "Git checks the conflict out executable");
            let repo = f.repo();
            let c = CancellationToken::new();
            let p = repo
                .edit_conflict(&RepoPath::new("run.sh").unwrap(), &c)
                .unwrap();
            let saved = repo
                .save_conflict(&p, "#!/bin/sh\nresolved\n", true, &c)
                .unwrap();
            saved.result.unwrap();
            assert!(saved.saved && saved.staged);
            assert_eq!(fs::metadata(&script).unwrap().permissions().mode() & 0o777, mode);
            assert!(
                f.run(&["ls-files", "--stage", "--", "run.sh"])
                    .starts_with(b"100755 ")
            );
            assert_eq!(f.run(&["status", "--porcelain", "--", "run.sh"]), b"M  run.sh\n");
        },
    );
}
