#![cfg(windows)]
mod common;
use common::{Fixture, isolated};
use rocket_git::*;
use std::fs;
fn token() -> CancellationToken {
    CancellationToken::new()
}
fn p(name: &str) -> RepoPath {
    RepoPath::new(name).unwrap()
}
fn fixture() -> Fixture {
    let f = Fixture::new();
    f.write("hello", "base\n");
    f.commit("base");
    f
}
#[test]
fn restoring_attributes_verifies_the_intended_change() {
    isolated(
        "ROCKET_FILE_EDGE",
        "restoring_attributes_verifies_the_intended_change",
        || {
            let f = fixture();
            f.write(".gitattributes", "* text eol=lf\n");
            f.commit("attributes");
            f.write(".gitattributes", "* text eol=crlf\n");
            let r = f.repo();
            let preview = r
                .preview_file_edit(&p(".gitattributes"), FileEditSource::Index, &token())
                .unwrap();
            let report = r.apply_file_edit(&preview, &token()).unwrap();
            report.result.unwrap();
            assert!(report.applied);
            report.verification.unwrap();
            assert!(f.run(&["diff", "--", ".gitattributes"]).is_empty());
        },
    );
}
#[test]
fn nested_repositories_junctions_and_active_operations_are_protected() {
    isolated(
        "ROCKET_FILE_EDGE",
        "nested_repositories_junctions_and_active_operations_are_protected",
        || {
            let f = fixture();
            let nested = f.repository.join("nested");
            fs::create_dir(&nested).unwrap();
            f.run_at(&nested, &["init", "."]);
            fs::write(nested.join("keep"), "PRIVATE_NESTED").unwrap();
            let repo = f.repo();
            assert!(matches!(
                repo.preview_file_edit(&p("nested/keep"), FileEditSource::Index, &token()),
                Err(Error::FileEdit(FileEditFailure::Unsupported))
            ));
            assert_eq!(
                fs::read_to_string(nested.join("keep")).unwrap(),
                "PRIVATE_NESTED"
            );
            let outside = f.root.join("outside");
            fs::create_dir(&outside).unwrap();
            fs::write(outside.join("keep"), "PRIVATE_OUTSIDE").unwrap();
            let junction = f.repository.canonicalize().unwrap().join("junction");
            use std::os::windows::process::CommandExt;
            let out = std::process::Command::new("cmd.exe")
                .args(["/d", "/c", "mklink", "/J"])
                .arg(&junction)
                .arg(outside.canonicalize().unwrap())
                .creation_flags(0x08000000)
                .output()
                .unwrap();
            assert!(
                out.status.success(),
                "{}",
                String::from_utf8_lossy(&out.stderr)
            );
            assert!(
                repo.preview_file_edit(&p("junction/keep"), FileEditSource::Index, &token())
                    .is_err()
            );
            assert_eq!(
                fs::read_to_string(outside.join("keep")).unwrap(),
                "PRIVATE_OUTSIDE"
            );
            fs::remove_dir(&junction).unwrap();
            f.write("hello", "working\n");
            f.write(".git/rebase-apply/fixture", "active");
            assert!(matches!(
                repo.preview_file_edit(&p("hello"), FileEditSource::Index, &token()),
                Err(Error::FileEdit(FileEditFailure::ActiveOperation))
            ));
            assert_eq!(fs::read(f.repository.join("hello")).unwrap(), b"working\n");
        },
    );
}
#[test]
fn linked_worktree_backups_are_isolated_and_binary_bytes_round_trip() {
    isolated(
        "ROCKET_FILE_EDGE",
        "linked_worktree_backups_are_isolated_and_binary_bytes_round_trip",
        || {
            let f = fixture();
            f.write("binary", [0, 255, 2]);
            f.commit("binary");
            let linked = f.root.join("linked 文");
            f.run(&["worktree", "add", "-b", "linked", linked.to_str().unwrap()]);
            let repo = f.git.open(&linked, &token()).unwrap();
            fs::write(linked.join("binary"), [0, 128, 4, 5]).unwrap();
            let path = p("binary");
            let preview = repo
                .preview_file_edit(&path, FileEditSource::Index, &token())
                .unwrap();
            let result = repo.apply_file_edit(&preview, &token()).unwrap();
            result.result.unwrap();
            result.verification.unwrap();
            assert_eq!(fs::read(linked.join("binary")).unwrap(), [0, 255, 2]);
            let backup = repo.file_backups(&token()).unwrap().remove(0);
            assert!(f.repo().file_backups(&token()).unwrap().is_empty());
            assert!(
                f.repo()
                    .preview_file_edit(&path, FileEditSource::Backup(backup.clone()), &token())
                    .is_err()
            );
            let preview = repo
                .preview_file_edit(&path, FileEditSource::Backup(backup), &token())
                .unwrap();
            let result = repo.apply_file_edit(&preview, &token()).unwrap();
            result.result.unwrap();
            result.verification.unwrap();
            assert_eq!(fs::read(linked.join("binary")).unwrap(), [0, 128, 4, 5]);
            assert_eq!(fs::read(f.repository.join("binary")).unwrap(), [0, 255, 2]);
        },
    );
}
