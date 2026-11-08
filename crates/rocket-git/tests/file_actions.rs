#![cfg(any(windows, target_os = "linux", target_os = "macos"))]
mod common;
use common::{Fixture, isolated};
use rocket_git::*;
use std::fs;
const PATH: &str = "src/検索/結果 🚀 é.rs";
fn token() -> CancellationToken {
    CancellationToken::new()
}
fn path() -> RepoPath {
    RepoPath::new(PATH).unwrap()
}
fn setup() -> Fixture {
    let f = Fixture::new();
    f.write(PATH, "original\n");
    f.commit("initial");
    f
}
fn apply(repo: &Repository, p: &FileEditPreview) {
    let r = repo.apply_file_edit(p, &token()).unwrap();
    assert!(r.applied);
    r.result.unwrap();
    r.verification.unwrap();
    r.status.unwrap();
    assert!(r.recovery_directory.unwrap().is_dir());
}
#[test]
fn discard_preserves_staged_content_and_recovers_exact_unicode_file() {
    isolated(
        "ROCKET_FILE_CASE",
        "discard_preserves_staged_content_and_recovers_exact_unicode_file",
        || {
            for format in ["sha1", "sha256"] {
                let f = Fixture::with_format(format);
                f.write(PATH, "original\n");
                f.commit("initial");
                f.write(PATH, "staged\n");
                f.run(&["add", "--", PATH]);
                f.write(PATH, "PRIVATE_你好 🚀 é\r\n");
                f.write("untracked", "keep");
                f.write(".git/index.lock", "owned elsewhere");
                let index = fs::read(f.repository.join(".git/index")).unwrap();
                let head = f.run(&["rev-parse", "HEAD"]);
                let repo = f.repo();
                let p = repo
                    .preview_file_edit(&path(), FileEditSource::Index, &token())
                    .unwrap();
                assert_eq!(p.after().unwrap(), b"staged\n");
                apply(&repo, &p);
                assert_eq!(fs::read(f.repository.join(PATH)).unwrap(), b"staged\n");
                assert_eq!(index, fs::read(f.repository.join(".git/index")).unwrap());
                assert_eq!(head, f.run(&["rev-parse", "HEAD"]));
                let b = repo.file_backups(&token()).unwrap().remove(0);
                let p = repo
                    .preview_file_edit(&path(), FileEditSource::Backup(b), &token())
                    .unwrap();
                apply(&repo, &p);
                assert_eq!(
                    fs::read_to_string(f.repository.join(PATH)).unwrap(),
                    "PRIVATE_你好 🚀 é\r\n"
                );
                assert_eq!(
                    fs::read_to_string(f.repository.join("untracked")).unwrap(),
                    "keep"
                );
                assert_eq!(
                    fs::read_to_string(f.repository.join(".git/index.lock")).unwrap(),
                    "owned elsewhere"
                );
            }
        },
    );
}
#[test]
fn restore_pins_commit_and_preserves_index() {
    isolated(
        "ROCKET_FILE_CASE",
        "restore_pins_commit_and_preserves_index",
        || {
            let f = setup();
            f.run(&["tag", "snapshot"]);
            f.write(PATH, "new\n");
            f.commit("new");
            let index = fs::read(f.repository.join(".git/index")).unwrap();
            let repo = f.repo();
            let p = repo
                .preview_file_edit(&path(), FileEditSource::Commit("snapshot".into()), &token())
                .unwrap();
            f.run(&["tag", "-f", "snapshot"]);
            apply(&repo, &p);
            assert_eq!(fs::read(f.repository.join(PATH)).unwrap(), b"original\n");
            assert_eq!(index, fs::read(f.repository.join(".git/index")).unwrap());
            assert!(
                repo.preview_file_edit(&path(), FileEditSource::Commit("--all".into()), &token())
                    .is_err()
            );
            assert!(
                repo.preview_file_edit(&path(), FileEditSource::Commit("missing".into()), &token())
                    .is_err()
            );
        },
    );
}
#[test]
fn deletion_and_creation_have_recoverable_existence() {
    isolated(
        "ROCKET_FILE_CASE",
        "deletion_and_creation_have_recoverable_existence",
        || {
            let f = setup();
            let repo = f.repo();
            fs::remove_file(f.repository.join(PATH)).unwrap();
            let p = repo
                .preview_file_edit(&path(), FileEditSource::Index, &token())
                .unwrap();
            assert!(p.before().is_none());
            apply(&repo, &p);
            let b = repo.file_backups(&token()).unwrap().remove(0);
            let p = repo
                .preview_file_edit(&path(), FileEditSource::Backup(b), &token())
                .unwrap();
            assert!(p.after().is_none());
            apply(&repo, &p);
            assert!(!f.repository.join(PATH).exists());
            let untracked = RepoPath::new("notes 文").unwrap();
            f.write("notes 文", [0, 255, 42]);
            let p = repo
                .preview_file_edit(&untracked, FileEditSource::Index, &token())
                .unwrap();
            apply(&repo, &p);
            assert!(!f.repository.join("notes 文").exists());
            let b = repo.file_backups(&token()).unwrap().remove(0);
            apply(
                &repo,
                &repo
                    .preview_file_edit(&untracked, FileEditSource::Backup(b), &token())
                    .unwrap(),
            );
            assert_eq!(
                fs::read(f.repository.join("notes 文")).unwrap(),
                [0, 255, 42]
            );
        },
    );
}
#[test]
fn external_edits_and_context_changes_refuse_without_overwrite() {
    isolated(
        "ROCKET_FILE_CASE",
        "external_edits_and_context_changes_refuse_without_overwrite",
        || {
            let f = setup();
            f.write(PATH, "local\n");
            let repo = f.repo();
            let p = repo
                .preview_file_edit(&path(), FileEditSource::Index, &token())
                .unwrap();
            f.write(PATH, "external\n");
            assert!(matches!(
                repo.apply_file_edit(&p, &token()),
                Err(Error::FileEdit(FileEditFailure::Stale))
            ));
            assert_eq!(fs::read(f.repository.join(PATH)).unwrap(), b"external\n");
            let p = repo
                .preview_file_edit(&path(), FileEditSource::Index, &token())
                .unwrap();
            f.run(&["add", "--", PATH]);
            assert!(matches!(
                repo.apply_file_edit(&p, &token()),
                Err(Error::FileEdit(FileEditFailure::Stale))
            ));
            let other = setup();
            assert!(matches!(
                other.repo().apply_file_edit(&p, &token()),
                Err(Error::FileEdit(FileEditFailure::WrongRepository))
            ));
            assert!(repo.file_backups(&token()).unwrap().is_empty());
        },
    );
}
#[test]
fn unsupported_inputs_and_cancel_preserve_data() {
    isolated(
        "ROCKET_FILE_CASE",
        "unsupported_inputs_and_cancel_preserve_data",
        || {
            let f = setup();
            f.write(PATH, "local\n");
            let repo = f.repo();
            let p = repo
                .preview_file_edit(&path(), FileEditSource::Index, &token())
                .unwrap();
            let cancelled = token();
            cancelled.cancel();
            assert!(matches!(
                repo.apply_file_edit(&p, &cancelled),
                Err(Error::Cancelled)
            ));
            assert!(matches!(
                repo.preview_file_edit(&path(), FileEditSource::Index, &cancelled),
                Err(Error::Cancelled)
            ));
            let mut perms = fs::metadata(f.repository.join(PATH)).unwrap().permissions();
            let original_permissions = perms.clone();
            perms.set_readonly(true);
            fs::set_permissions(f.repository.join(PATH), perms.clone()).unwrap();
            let report = repo.apply_file_edit(&p, &token()).unwrap();
            assert!(!report.applied && report.result.is_err());
            fs::set_permissions(f.repository.join(PATH), original_permissions).unwrap();
            assert_eq!(fs::read(f.repository.join(PATH)).unwrap(), b"local\n");
            assert!(
                repo.preview_file_edit(
                    &RepoPath::new(".git/config").unwrap(),
                    FileEditSource::Index,
                    &token()
                )
                .is_err()
            );
            f.run(&["update-index", "--skip-worktree", PATH]);
            assert!(
                repo.preview_file_edit(&path(), FileEditSource::Index, &token())
                    .is_err()
            );
            f.run(&["update-index", "--no-skip-worktree", PATH]);
            f.write(".gitattributes", "*.rs filter=evil\n");
            f.run(&["config", "filter.evil.smudge", "false"]);
            assert!(matches!(
                repo.preview_file_edit(&path(), FileEditSource::Index, &token()),
                Err(Error::FileEdit(FileEditFailure::Unsupported))
            ));
            f.write(".gitattributes", "");
            f.write(PATH, vec![b'a'; FILE_EDIT_LIMIT + 1]);
            assert!(
                repo.preview_file_edit(&path(), FileEditSource::Index, &token())
                    .is_err()
            );
        },
    );
}
#[test]
fn checkout_line_endings_match_git_without_running_filters() {
    isolated(
        "ROCKET_FILE_CASE",
        "checkout_line_endings_match_git_without_running_filters",
        || {
            let f = setup();
            f.run(&["config", "core.autocrlf", "true"]);
            f.write(PATH, "local\n");
            let repo = f.repo();
            let p = repo
                .preview_file_edit(&path(), FileEditSource::Index, &token())
                .unwrap();
            assert_eq!(p.after().unwrap(), b"original\r\n");
            apply(&repo, &p);
            assert!(f.run(&["diff", "--", PATH]).is_empty());
            f.write(".gitattributes", "*.rs text eol=lf\n");
            let p = repo
                .preview_file_edit(&path(), FileEditSource::Index, &token())
                .unwrap();
            assert_eq!(p.after().unwrap(), b"original\n");
            apply(&repo, &p);
            assert!(f.run(&["diff", "--", PATH]).is_empty());
        },
    );
}

/// Linux: a restored file gets the executable bit its index entry records, as a Git
/// checkout would, and keeps its other permission bits; no mode change is left.
#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn restored_files_get_the_executable_bit_git_records() {
    isolated(
        "ROCKET_FILE_CASE",
        "restored_files_get_the_executable_bit_git_records",
        || {
            use std::os::unix::fs::PermissionsExt;
            let f = Fixture::new();
            let script = f.repository.join("run.sh");
            let plain = f.repository.join("plain.txt");
            f.write("run.sh", "#!/bin/sh\noriginal\n");
            fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
            f.write("plain.txt", "original\n");
            f.commit("initial");
            let repo = f.repo();
            let mode = |path: &std::path::Path| fs::metadata(path).unwrap().permissions().mode() & 0o777;
            let discard = |name: &str| {
                let p = repo
                    .preview_file_edit(&RepoPath::new(name).unwrap(), FileEditSource::Index, &token())
                    .unwrap();
                apply(&repo, &p);
                assert!(f.run(&["status", "--porcelain", "--", name]).is_empty(), "{name}");
            };
            f.write("run.sh", "#!/bin/sh\nedited\n");
            discard("run.sh");
            assert_eq!(mode(&script), 0o755);
            // A deleted script comes back executable.
            fs::remove_file(&script).unwrap();
            discard("run.sh");
            assert_ne!(mode(&script) & 0o111, 0);
            // A file edited and made executable by hand loses the bit its entry lacks.
            f.write("plain.txt", "edited\n");
            fs::set_permissions(&plain, fs::Permissions::from_mode(0o750)).unwrap();
            discard("plain.txt");
            assert_eq!(mode(&plain), 0o640);
        },
    );
}
