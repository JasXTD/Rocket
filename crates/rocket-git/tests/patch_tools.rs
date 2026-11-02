mod common;
use common::{Fixture, isolated};
use rocket_git::*;
use std::{fs, path::PathBuf};
fn token() -> CancellationToken {
    CancellationToken::new()
}
fn path(name: &str) -> RepoPath {
    RepoPath::new(name.as_bytes().to_vec()).unwrap()
}
fn read(f: &Fixture, name: &str) -> Vec<u8> {
    fs::read(f.repository.join(name)).unwrap()
}
fn setup() -> Fixture {
    let f = Fixture::new();
    f.write("src/検索 🚀.rs", "one\ntwo\nthree\n");
    f.write("other.txt", "base\n");
    f.commit("base");
    f
}
fn destination(f: &Fixture) -> PathBuf {
    f.root.canonicalize().unwrap().join("result.patch")
}
fn exported(f: &Fixture, source: PatchExportSource) -> PathBuf {
    let target = destination(f);
    let p = f
        .repo()
        .preview_patch_export(source, &target, &token())
        .unwrap();
    f.repo().export_patch(&p, &token()).unwrap();
    target
}
fn check_report(r: PatchReport, count: usize) {
    assert!(r.result.is_ok(), "{:?}", r.result);
    assert!(r.verification.is_ok(), "{:?}", r.verification);
    assert!(r.status.is_ok());
    assert_eq!(r.changed.len(), count);
    assert_eq!(r.recovery.len(), count);
}

#[test]
fn roundtrip_multifile_unicode_and_recovery() {
    isolated(
        "ROCKET_PATCH_CASE",
        "roundtrip_multifile_unicode_and_recovery",
        || {
            let f = setup();
            let repo = f.repo();
            let index = read(&f, ".git/index");
            let head = f.run(&["rev-parse", "HEAD"]);
            f.write("src/検索 🚀.rs", "one\n你好 🚀\nthree\n");
            f.write("other.txt", "changed without newline");
            let output = exported(&f, PatchExportSource::Unstaged);
            assert!(
                fs::read_to_string(&output)
                    .unwrap()
                    .contains("No newline at end of file")
            );
            f.write("src/検索 🚀.rs", "one\ntwo\nthree\n");
            f.write("other.txt", "base\n");
            let p = repo.preview_patch_import(&output, &token()).unwrap();
            assert_eq!(p.files().len(), 2);
            assert_eq!(read(&f, "other.txt"), b"base\n");
            check_report(repo.apply_patch_preview(&p, &token()).unwrap(), 2);
            assert_eq!(
                read(&f, "src/検索 🚀.rs"),
                "one\n你好 🚀\nthree\n".as_bytes()
            );
            assert_eq!(read(&f, "other.txt"), b"changed without newline");
            assert_eq!(read(&f, ".git/index"), index);
            assert_eq!(f.run(&["rev-parse", "HEAD"]), head);
            let backups = repo.file_backups(&token()).unwrap();
            assert_eq!(backups.len(), 2);
            for b in backups {
                let p = repo
                    .preview_file_edit(b.path(), FileEditSource::Backup(b.clone()), &token())
                    .unwrap();
                let report = repo.apply_file_edit(&p, &token()).unwrap();
                assert!(report.result.is_ok());
                assert!(report.verification.is_ok());
            }
            assert_eq!(read(&f, "other.txt"), b"base\n");
            assert_eq!(read(&f, "src/検索 🚀.rs"), b"one\ntwo\nthree\n");
        },
    );
}

#[test]
fn additions_deletions_staged_and_commit_exports() {
    isolated(
        "ROCKET_PATCH_CASE",
        "additions_deletions_staged_and_commit_exports",
        || {
            let f = setup();
            f.write("src/new 中文.txt", "added\n");
            fs::remove_file(f.repository.join("other.txt")).unwrap();
            f.run(&["add", "--all"]);
            let output = exported(&f, PatchExportSource::Staged);
            let bytes = fs::read(&output).unwrap();
            f.run(&["reset", "--hard", "HEAD"]);
            let repo = f.repo();
            let p = repo.preview_patch_import(&output, &token()).unwrap();
            check_report(repo.apply_patch_preview(&p, &token()).unwrap(), 2);
            assert!(!f.repository.join("other.txt").exists());
            assert_eq!(read(&f, "src/new 中文.txt"), b"added\n");
            f.commit("patch applied");
            let target = f.root.canonicalize().unwrap().join("commit.patch");
            let p = repo
                .preview_patch_export(PatchExportSource::Commit("HEAD".into()), &target, &token())
                .unwrap();
            assert!(p.commit().is_some());
            repo.export_patch(&p, &token()).unwrap();
            assert_eq!(fs::read(target).unwrap(), bytes);
            let target = f.root.canonicalize().unwrap().join("root.patch");
            let p = repo
                .preview_patch_export(
                    PatchExportSource::Commit("HEAD~1".into()),
                    &target,
                    &token(),
                )
                .unwrap();
            assert_eq!(p.paths().len(), 2);
        },
    );
}

#[test]
fn stale_targets_index_source_and_export_collisions() {
    isolated(
        "ROCKET_PATCH_CASE",
        "stale_targets_index_source_and_export_collisions",
        || {
            let f = setup();
            f.write("other.txt", "next\n");
            let output = exported(&f, PatchExportSource::Unstaged);
            f.write("other.txt", "base\n");
            let repo = f.repo();
            let p = repo.preview_patch_import(&output, &token()).unwrap();
            f.write("other.txt", "external\n");
            assert!(matches!(
                repo.apply_patch_preview(&p, &token()),
                Err(Error::FileEdit(FileEditFailure::Stale))
            ));
            assert_eq!(read(&f, "other.txt"), b"external\n");
            f.write("other.txt", "base\n");
            let p = repo.preview_patch_import(&output, &token()).unwrap();
            f.write("other.txt", "staged\n");
            f.run(&["add", "other.txt"]);
            f.write("other.txt", "base\n");
            assert!(matches!(
                repo.apply_patch_preview(&p, &token()),
                Err(Error::FileEdit(FileEditFailure::Stale))
            ));
            f.run(&["reset", "HEAD", "--", "other.txt"]);
            let p = repo.preview_patch_import(&output, &token()).unwrap();
            fs::write(&output, b"different patch").unwrap();
            assert!(matches!(
                repo.apply_patch_preview(&p, &token()),
                Err(Error::Patch(PatchFailure::SourceChanged))
            ));
            f.write("other.txt", "next\n");
            let target = f.root.canonicalize().unwrap().join("export.patch");
            let p = repo
                .preview_patch_export(PatchExportSource::Unstaged, &target, &token())
                .unwrap();
            f.write("other.txt", "later\n");
            assert!(matches!(
                repo.export_patch(&p, &token()),
                Err(Error::Patch(PatchFailure::SourceChanged))
            ));
            assert!(!target.exists());
            let p = repo
                .preview_patch_export(PatchExportSource::Unstaged, &target, &token())
                .unwrap();
            fs::write(&target, b"KEEP").unwrap();
            assert!(matches!(
                repo.export_patch(&p, &token()),
                Err(Error::Patch(PatchFailure::OutputExists))
            ));
            assert_eq!(fs::read(target).unwrap(), b"KEEP");
            assert!(repo.file_backups(&token()).unwrap().is_empty());
        },
    );
}

#[test]
fn patch_conflicts_and_hostile_metadata_are_read_only() {
    isolated(
        "ROCKET_PATCH_CASE",
        "patch_conflicts_and_hostile_metadata_are_read_only",
        || {
            let f = setup();
            let repo = f.repo();
            let input = f.root.canonicalize().unwrap().join("input.patch");
            let before = read(&f, "other.txt");
            let patch = "diff --git a/other.txt b/other.txt\n--- a/other.txt\n+++ b/other.txt\n@@ -1 +1 @@\n-base\n+new\n";
            for text in [
                patch.replace("-base", "-wrong"),
                patch.replace("other.txt", "../outside"),
                patch.replace("other.txt", ".git/config"),
                patch.replace("other.txt", ".gitattributes"),
                patch.replace(
                    "--- a/other.txt",
                    "old mode 100644\nnew mode 120000\n--- a/other.txt",
                ),
                patch.replace("@@ -1 +1 @@", "@@ -99,888 +1 @@"),
                format!("{patch}{patch}"),
            ] {
                fs::write(&input, text).unwrap();
                assert!(repo.preview_patch_import(&input, &token()).is_err());
                assert_eq!(read(&f, "other.txt"), before);
                assert!(repo.file_backups(&token()).unwrap().is_empty());
            }
            fs::write(&input, patch).unwrap();
            f.run(&["update-index", "--skip-worktree", "other.txt"]);
            assert!(repo.preview_patch_import(&input, &token()).is_err());
        },
    );
}

#[test]
fn selected_lines_and_hunks_preserve_other_edits_and_index() {
    isolated(
        "ROCKET_PATCH_CASE",
        "selected_lines_and_hunks_preserve_other_edits_and_index",
        || {
            let f = setup();
            f.write("other.txt", "staged content\n");
            f.run(&["add", "other.txt"]);
            let index = read(&f, ".git/index");
            f.write("src/検索 🚀.rs", "one\nCHANGED\nthree\nKEEP\n");
            let repo = f.repo();
            let review = repo
                .review_change(&path("src/検索 🚀.rs"), false, &token())
                .unwrap();
            let lines: Vec<_> = review
                .patch()
                .split(|b| *b == b'\n')
                .enumerate()
                .filter_map(|(i, l)| (l == b"-two" || l == b"+CHANGED").then_some(i))
                .collect();
            assert_eq!(lines.len(), 2);
            let p = repo
                .preview_discard_selection(&review, &lines, &token())
                .unwrap();
            assert_eq!(read(&f, "src/検索 🚀.rs"), b"one\nCHANGED\nthree\nKEEP\n");
            check_report(repo.apply_patch_preview(&p, &token()).unwrap(), 1);
            assert_eq!(read(&f, "src/検索 🚀.rs"), b"one\ntwo\nthree\nKEEP\n");
            assert_eq!(read(&f, ".git/index"), index);
            let review = repo
                .review_change(&path("src/検索 🚀.rs"), false, &token())
                .unwrap();
            let p = repo
                .preview_discard_selection(&review, &review.changed_lines(), &token())
                .unwrap();
            check_report(repo.apply_patch_preview(&p, &token()).unwrap(), 1);
            assert_eq!(read(&f, "src/検索 🚀.rs"), b"one\ntwo\nthree\n");
            let other = setup();
            assert!(matches!(
                other
                    .repo()
                    .preview_discard_selection(&review, &review.changed_lines(), &token()),
                Err(Error::FileEdit(FileEditFailure::WrongRepository))
            ));
        },
    );
}

#[test]
fn converted_crlf_and_cancelled_operations() {
    isolated(
        "ROCKET_PATCH_CASE",
        "converted_crlf_and_cancelled_operations",
        || {
            let f = setup();
            f.run(&["config", "core.autocrlf", "true"]);
            f.write("src/検索 🚀.rs", "one\r\nCHANGED\r\nthree\r\nKEEP\r\n");
            let repo = f.repo();
            let review = repo
                .review_change(&path("src/検索 🚀.rs"), false, &token())
                .unwrap();
            let lines: Vec<_> = review
                .patch()
                .split(|b| *b == b'\n')
                .enumerate()
                .filter_map(|(i, l)| (l == b"-two" || l == b"+CHANGED").then_some(i))
                .collect();
            let p = repo
                .preview_discard_selection(&review, &lines, &token())
                .unwrap();
            let stopped = token();
            stopped.cancel();
            assert!(matches!(
                repo.apply_patch_preview(&p, &stopped),
                Err(Error::Cancelled)
            ));
            assert!(repo.file_backups(&token()).unwrap().is_empty());
            check_report(repo.apply_patch_preview(&p, &token()).unwrap(), 1);
            assert_eq!(
                read(&f, "src/検索 🚀.rs"),
                b"one\r\ntwo\r\nthree\r\nKEEP\r\n"
            );
            let output = f.root.canonicalize().unwrap().join("cancelled.patch");
            let p = repo
                .preview_patch_export(PatchExportSource::Unstaged, &output, &token())
                .unwrap();
            assert!(matches!(
                repo.export_patch(&p, &stopped),
                Err(Error::Cancelled)
            ));
            assert!(!output.exists());
            assert!(matches!(
                repo.preview_patch_export(PatchExportSource::Unstaged, &output, &stopped),
                Err(Error::Cancelled)
            ));
        },
    );
}

#[test]
fn cancellation_after_first_publication_reports_and_recovers_partial_batch() {
    isolated(
        "ROCKET_PATCH_CASE",
        "cancellation_after_first_publication_reports_and_recovers_partial_batch",
        || {
            let f = setup();
            f.write("other.txt", "next\n");
            f.write("src/検索 🚀.rs", "changed\n");
            let input = exported(&f, PatchExportSource::Unstaged);
            f.write("other.txt", "base\n");
            f.write("src/検索 🚀.rs", "one\ntwo\nthree\n");
            let repo = f.repo();
            let p = repo.preview_patch_import(&input, &token()).unwrap();
            assert_eq!(p.files()[0].path(), &path("other.txt"));
            let cancelled = token();
            let stop = cancelled.clone();
            let watch = f.repository.join("other.txt");
            let observer = std::thread::spawn(move || {
                let start = std::time::Instant::now();
                while start.elapsed() < std::time::Duration::from_secs(40) {
                    if fs::read(&watch).is_ok_and(|b| b == b"next\n") {
                        stop.cancel();
                        return true;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(1));
                }
                false
            });
            let report = repo.apply_patch_preview(&p, &cancelled).unwrap();
            assert!(observer.join().unwrap());
            assert!(matches!(report.result, Err(Error::Cancelled)));
            assert!(report.verification.is_ok());
            assert_eq!(report.changed, vec![path("other.txt")]);
            assert_eq!(read(&f, "src/検索 🚀.rs"), b"one\ntwo\nthree\n");
            let backup = repo
                .file_backups(&token())
                .unwrap()
                .into_iter()
                .find(|b| b.path() == &path("other.txt"))
                .unwrap();
            let restore = repo
                .preview_file_edit(
                    backup.path(),
                    FileEditSource::Backup(backup.clone()),
                    &token(),
                )
                .unwrap();
            repo.apply_file_edit(&restore, &token())
                .unwrap()
                .result
                .unwrap();
            assert_eq!(read(&f, "other.txt"), b"base\n");
        },
    );
}

#[test]
fn separated_hunks_keep_staged_target_and_unselected_changes() {
    isolated(
        "ROCKET_PATCH_CASE",
        "separated_hunks_keep_staged_target_and_unselected_changes",
        || {
            let f = setup();
            let base = (1..=40).map(|i| format!("line {i}\n")).collect::<String>();
            f.write("other.txt", &base);
            f.commit("long base");
            let staged = base.replace("line 2\n", "STAGED\n");
            f.write("other.txt", &staged);
            f.run(&["add", "other.txt"]);
            let index = read(&f, ".git/index");
            let working = staged
                .replace("STAGED\n", "UNSTAGED\n")
                .replace("line 20\n", "KEEP\n")
                .replace("line 39\n", "CHANGED39\nNEW40\n");
            f.write("other.txt", &working);
            let repo = f.repo();
            let review = repo
                .review_change(&path("other.txt"), false, &token())
                .unwrap();
            let lines = review
                .patch()
                .split(|b| *b == b'\n')
                .enumerate()
                .filter_map(|(i, l)| {
                    matches!(
                        l,
                        b"-STAGED" | b"+UNSTAGED" | b"-line 39" | b"+CHANGED39" | b"+NEW40"
                    )
                    .then_some(i)
                })
                .collect::<Vec<_>>();
            assert_eq!(lines.len(), 5);
            let p = repo
                .preview_discard_selection(&review, &lines, &token())
                .unwrap();
            check_report(repo.apply_patch_preview(&p, &token()).unwrap(), 1);
            assert_eq!(
                read(&f, "other.txt"),
                staged.replace("line 20\n", "KEEP\n").as_bytes()
            );
            assert_eq!(read(&f, ".git/index"), index);
        },
    );
}

#[test]
fn external_replacement_after_first_publication_stops_remaining_files() {
    isolated(
        "ROCKET_PATCH_CASE",
        "external_replacement_after_first_publication_stops_remaining_files",
        || {
            let f = setup();
            f.write("other.txt", "next\n");
            f.write("src/検索 🚀.rs", "changed\n");
            let input = exported(&f, PatchExportSource::Unstaged);
            f.write("other.txt", "base\n");
            f.write("src/検索 🚀.rs", "one\ntwo\nthree\n");
            let repo = f.repo();
            let p = repo.preview_patch_import(&input, &token()).unwrap();
            let index = read(&f, ".git/index");
            let watch = f.repository.join("other.txt");
            let destination = f.repository.join("src/検索 🚀.rs");
            let replacement = f.root.join("external-replacement");
            fs::write(&replacement, b"EXTERNAL EDIT\n").unwrap();
            let observer = std::thread::spawn(move || {
                let start = std::time::Instant::now();
                while start.elapsed() < std::time::Duration::from_secs(40) {
                    if fs::read(&watch).is_ok_and(|b| b == b"next\n") {
                        // Model an editor's replace-on-save even if Rocket holds
                        // a read handle to the previously reviewed file.
                        fs::rename(replacement, destination).unwrap();
                        return true;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(1));
                }
                false
            });
            let report = repo.apply_patch_preview(&p, &token()).unwrap();
            assert!(observer.join().unwrap());
            assert!(matches!(
                report.result,
                Err(Error::FileEdit(FileEditFailure::Stale))
            ));
            assert!(report.verification.is_err());
            assert_eq!(report.changed, vec![path("other.txt")]);
            assert_eq!(read(&f, "other.txt"), b"next\n");
            assert_eq!(read(&f, "src/検索 🚀.rs"), b"EXTERNAL EDIT\n");
            assert_eq!(read(&f, ".git/index"), index);
            assert!(
                repo.file_backups(&token())
                    .unwrap()
                    .iter()
                    .any(|b| b.path() == &path("other.txt"))
            );
        },
    );
}

#[test]
fn read_diff_preserves_index_stat_cache() {
    isolated(
        "ROCKET_PATCH_CASE",
        "read_diff_preserves_index_stat_cache",
        || {
            let f = setup();
            let index = read(&f, ".git/index");
            let file = fs::OpenOptions::new()
                .write(true)
                .open(f.repository.join("other.txt"))
                .unwrap();
            file.set_times(
                fs::FileTimes::new().set_modified(
                    std::time::SystemTime::now() + std::time::Duration::from_secs(10),
                ),
            )
            .unwrap();
            drop(file);
            assert!(
                f.repo()
                    .diff(&DiffOptions::default(), &token())
                    .unwrap()
                    .patch
                    .is_empty()
            );
            assert_eq!(
                read(&f, ".git/index"),
                index,
                "Read-only diff changed the index stat cache"
            );
        },
    );
}
