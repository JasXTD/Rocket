#[path = "../../rocket-git/tests/common/mod.rs"]
mod common;

use common::{Fixture, isolated};
use rocket_git::*;
use rocket_ui::backend::Backend;

fn path(name: &str) -> RepoPath {
    RepoPath::new(name.as_bytes().to_vec()).unwrap()
}

#[test]
fn diff_rename_pair_keeps_review_and_unstages_only_text() {
    isolated(
        "ROCKET_FINE_UI_CASE",
        "diff_rename_pair_keeps_review_and_unstages_only_text",
        || {
            let f = Fixture::new();
            let old = "旧名.rs";
            let new = "新名.rs";
            let base = (0..30).map(|i| format!("line {i}\n")).collect::<String>();
            f.write(old, &base);
            f.commit("base");
            f.run(&["mv", old, new]);
            let edited = base.replace("line 5\n", "changed five\n");
            f.write(new, &edited);
            f.run(&["add", new]);
            let backend = Backend::discover(&f.root.join("logs")).unwrap();
            let repo = f.repo();
            let cancel = CancellationToken::new();
            let options = DiffOptions {
                target: DiffTarget::Staged,
                paths: vec![path(new), path(old)],
                ..Default::default()
            };
            let patch = backend.diff(&repo, &options, &cancel).unwrap();
            assert_eq!(patch.files.len(), 1);
            let review = patch
                .review
                .expect("paired rename must retain the sealed review");
            assert_eq!(review.selection_unavailable(), None);
            assert_eq!(review.changed_lines().len(), 2);
            let applied = repo
                .apply_review(&review, &review.changed_lines(), &cancel)
                .unwrap();
            assert!(applied.result.is_ok());
            assert!(applied.status.is_ok());
            assert_eq!(f.run(&["show", &format!(":{new}")]), base.as_bytes());
            assert!(f.run(&["ls-files", "--", old]).is_empty());
            assert_eq!(
                std::fs::read(f.repository.join(new)).unwrap(),
                edited.as_bytes()
            );
            assert!(
                repo.apply_review(&review, &review.changed_lines(), &cancel)
                    .is_err()
            );
            cancel.cancel();
            assert!(matches!(
                backend.diff(&repo, &options, &cancel),
                Err(Error::Cancelled)
            ));
        },
    );
}

#[test]
fn diff_unrelated_pair_remains_display_only() {
    isolated(
        "ROCKET_FINE_UI_CASE",
        "diff_unrelated_pair_remains_display_only",
        || {
            let f = Fixture::new();
            for name in ["one.rs", "two.rs"] {
                f.write(name, "base\n");
            }
            f.commit("base");
            for name in ["one.rs", "two.rs"] {
                f.write(name, "edit\n");
            }
            let before = std::fs::read(f.repository.join(".git/index")).unwrap();
            let backend = Backend::discover(&f.root.join("logs")).unwrap();
            let patch = backend
                .diff(
                    &f.repo(),
                    &DiffOptions {
                        target: DiffTarget::Unstaged,
                        paths: vec![path("one.rs"), path("two.rs")],
                        ..Default::default()
                    },
                    &CancellationToken::new(),
                )
                .unwrap();
            assert_eq!(patch.files.len(), 2);
            assert!(patch.review.is_none());
            assert_eq!(
                std::fs::read(f.repository.join(".git/index")).unwrap(),
                before
            );
        },
    );
}
