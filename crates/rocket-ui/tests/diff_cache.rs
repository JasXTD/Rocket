#[path = "../../rocket-git/tests/common/mod.rs"]
mod common;
use common::{Fixture, isolated};
use rocket_git::*;
use rocket_ui::backend::Backend;
use std::sync::Arc;

fn options() -> DiffOptions {
    DiffOptions {
        target: DiffTarget::Unstaged,
        paths: vec![RepoPath::new(b"sample.rs".to_vec()).unwrap()],
        ..Default::default()
    }
}

#[test]
fn cached_display_keeps_external_edits_and_review_authority_fresh() {
    isolated(
        "ROCKET_DIFF_CACHE_CASE",
        "cached_display_keeps_external_edits_and_review_authority_fresh",
        || {
            let a = Fixture::new();
            let b = Fixture::new();
            for fixture in [&a, &b] {
                fixture.write("sample.rs", "fn old() {}\n");
                fixture.commit("base");
                fixture.write("sample.rs", "fn new() {}\n");
            }
            let backend = Backend::discover(&a.root.join("logs")).unwrap();
            let cancel = CancellationToken::new();
            let repo_a = a.repo();
            let repo_b = b.repo();
            let first = backend.diff(&repo_a, &options(), &cancel).unwrap();
            let second = backend.clone().diff(&repo_b, &options(), &cancel).unwrap();
            assert!(Arc::ptr_eq(&first.files[0].lines, &second.files[0].lines));
            assert_ne!(first.files[0].id, second.files[0].id);
            let first_review = first.review.unwrap();
            let second_review = second.review.unwrap();
            assert!(!Arc::ptr_eq(&first_review, &second_review));
            assert!(
                repo_b
                    .apply_review(&first_review, &first_review.changed_lines(), &cancel)
                    .is_err()
            );
            a.write("sample.rs", "fn now() {}\n"); // Same byte length, different contents.
            let fresh = backend.diff(&repo_a, &options(), &cancel).unwrap();
            assert!(
                fresh.files[0]
                    .lines
                    .iter()
                    .any(|line| line.text == "+fn now() {}")
            );
            assert!(!Arc::ptr_eq(&first.files[0].lines, &fresh.files[0].lines));
            assert!(
                repo_a
                    .apply_review(&first_review, &first_review.changed_lines(), &cancel)
                    .is_err()
            );
            let applied = repo_b
                .apply_review(&second_review, &second_review.changed_lines(), &cancel)
                .unwrap();
            assert!(applied.result.is_ok());
            assert_eq!(b.run(&["show", ":sample.rs"]), b"fn new() {}\n");
            assert_eq!(a.run(&["show", ":sample.rs"]), b"fn old() {}\n");
            cancel.cancel();
            assert!(matches!(
                backend.diff(&repo_a, &options(), &cancel),
                Err(Error::Cancelled)
            ));
        },
    );
}

#[test]
fn measure_repeated_diff_reads() {
    isolated(
        "ROCKET_DIFF_CACHE_CASE",
        "measure_repeated_diff_reads",
        || {
            let f = Fixture::new();
            let base = (0..4000)
                .map(|i| format!("fn item_{i}() {{ println!(\"old 你好\"); }}\n"))
                .collect::<String>();
            f.write("sample.rs", &base);
            f.commit("base");
            f.write("sample.rs", base.replace("old", "new"));
            let backend = Backend::discover(&f.root.join("logs")).unwrap();
            let repo = f.repo();
            let cancel = CancellationToken::new();
            let id = repo.resolve_commit("HEAD", &cancel).unwrap();
            let mut results = Vec::new();
            for (label, options) in [
                ("unstaged", options()),
                (
                    "commit",
                    DiffOptions {
                        target: DiffTarget::Commit { id },
                        ..Default::default()
                    },
                ),
            ] {
                let mut times = Vec::new();
                for _ in 0..9 {
                    let start = std::time::Instant::now();
                    let patch = backend.diff(&repo, &options, &cancel).unwrap();
                    assert_eq!(patch.files.len(), 1);
                    assert!(!patch.limited);
                    times.push(start.elapsed().as_micros() as u64);
                }
                let cold = times.remove(0);
                times.sort_unstable();
                results.push(serde_json::json!({"kind": label, "cold_us": cold, "warm_median_us": times[times.len()/2], "samples_us": times}));
            }
            if let Some(path) = std::env::var_os("ROCKET_PERFORMANCE_REPORT") {
                std::fs::write(path, serde_json::to_vec_pretty(&results).unwrap()).unwrap();
            }
        },
    );
}
