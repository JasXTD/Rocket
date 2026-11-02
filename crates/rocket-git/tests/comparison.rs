mod common;
use common::{Fixture, isolated};
use rocket_git::*;
use std::fs;

fn token() -> CancellationToken {
    CancellationToken::new()
}
fn patch(repo: &Repository, comparison: &Comparison, index: usize) -> Vec<u8> {
    repo.diff(&comparison.file_options(index).unwrap(), &token())
        .unwrap()
        .patch
}

#[test]
fn comparison_preserves_unicode_renames_literal_paths_and_local_changes() {
    isolated(
        "ROCKET_COMPARE_CASE",
        "comparison_preserves_unicode_renames_literal_paths_and_local_changes",
        || {
            let f = Fixture::new();
            let old = "src/検索/旧名 🚀 e\u{301}.rs";
            let new = "src/検索/表示 🚀 e\u{301}.rs";
            f.write(old, "pub fn text() { println!(\"你好\"); }\n");
            f.write("literal [x].txt", "before\n");
            f.write("deleted.md", "deleted\n");
            f.write("binary.bin", [0, 1, 2]);
            f.commit("base");
            f.run(&["branch", "base"]);
            f.run(&["mv", old, new]);
            f.write("literal [x].txt", "after\n");
            f.write("literal x.txt", "other literal path\n");
            f.write("docs/新增.md", "new\n");
            f.write("binary.bin", [0, 3, 4]);
            f.run(&["rm", "deleted.md"]);
            f.commit("target");
            f.run(&["branch", "feature/検索"]);
            f.run(&["config", "diff.external", "never-run-rocket-compare-helper"]);
            f.write("literal [x].txt", "staged local change\n");
            f.run(&["add", "--", "literal [x].txt"]);
            f.write("literal [x].txt", "unstaged local change\n");
            f.write("untracked.txt", "keep this\n");
            let head = f.run(&["rev-parse", "HEAD"]);
            let index = fs::read(f.repository.join(".git/index")).unwrap();
            let config = fs::read(f.repository.join(".git/config")).unwrap();
            let repo = f.repo();
            let comparison = repo
                .compare_revisions("base", "feature/検索", &token())
                .unwrap();
            assert_eq!(comparison.files.len(), 6);
            let renamed = comparison
                .files
                .iter()
                .position(|file| file.path.as_bytes() == new.as_bytes())
                .unwrap();
            let file = &comparison.files[renamed];
            assert_eq!(file.change, Change::Renamed);
            assert_eq!(
                file.original_path.as_ref().unwrap().as_bytes(),
                old.as_bytes()
            );
            assert_eq!(file.similarity, Some(100));
            assert!(
                String::from_utf8_lossy(&patch(&repo, &comparison, renamed))
                    .contains("similarity index 100%")
            );
            let literal = comparison
                .files
                .iter()
                .position(|file| file.path.as_bytes() == b"literal [x].txt")
                .unwrap();
            let text = String::from_utf8(patch(&repo, &comparison, literal)).unwrap();
            assert!(text.contains("-before\n+after"));
            assert!(!text.contains("other literal path"));
            assert!(!text.contains("local change"));
            let reverse = repo
                .compare_revisions("feature/検索", "base", &token())
                .unwrap();
            assert!(
                reverse
                    .files
                    .iter()
                    .any(|file| file.path.as_bytes() == old.as_bytes()
                        && file.original_path.as_ref().unwrap().as_bytes() == new.as_bytes())
            );
            assert_eq!(f.run(&["rev-parse", "HEAD"]), head);
            assert_eq!(fs::read(f.repository.join(".git/index")).unwrap(), index);
            assert_eq!(fs::read(f.repository.join(".git/config")).unwrap(), config);
            assert_eq!(
                fs::read(f.repository.join("literal [x].txt")).unwrap(),
                b"unstaged local change\n"
            );
            assert_eq!(
                fs::read(f.repository.join("untracked.txt")).unwrap(),
                b"keep this\n"
            );
        },
    );
}

#[test]
fn comparison_pins_refs_and_handles_identical_missing_and_cancelled_revisions() {
    isolated(
        "ROCKET_COMPARE_CASE",
        "comparison_pins_refs_and_handles_identical_missing_and_cancelled_revisions",
        || {
            let f = Fixture::with_format("sha256");
            let repo = f.repo();
            assert!(repo.compare_revisions("HEAD", "HEAD", &token()).is_err());
            f.write("file.txt", "one\n");
            f.commit("one");
            f.run(&["tag", "v1"]);
            f.write("file.txt", "two\n");
            f.commit("two");
            let c = repo.compare_revisions("v1", "main", &token()).unwrap();
            assert_eq!(c.target.as_str().len(), 64);
            let before = patch(&repo, &c, 0);
            f.write("file.txt", "external new commit\n");
            f.commit("three");
            assert_eq!(patch(&repo, &c, 0), before);
            assert_ne!(
                repo.compare_revisions("v1", "main", &token())
                    .unwrap()
                    .target,
                c.target
            );
            assert!(
                repo.compare_revisions("HEAD", "HEAD", &token())
                    .unwrap()
                    .files
                    .is_empty()
            );
            assert!(
                repo.compare_revisions("not-a-revision", "main", &token())
                    .is_err()
            );
            assert!(
                repo.compare_revisions("main", "--output=unwanted", &token())
                    .is_err()
            );
            assert!(c.file_options(99).is_err());
            let cancel = token();
            cancel.cancel();
            assert!(matches!(
                repo.compare_revisions("v1", "main", &cancel),
                Err(Error::Cancelled)
            ));
        },
    );
}
