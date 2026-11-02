#![cfg(windows)]
#[path = "../../rocket-git/tests/common/mod.rs"]
mod common;
use common::Fixture;
use rocket_git::*;
use rocket_ui::{
    backend::Backend,
    benchmark::{Operation, Report},
};
use std::{
    fs,
    sync::{Arc, Mutex},
};

#[test]
fn benchmark_reads_real_dirty_repository_without_changing_it() {
    common::isolated(
        "ROCKET_BENCHMARK_CASE",
        "benchmark_reads_real_dirty_repository_without_changing_it",
        || {
            let f = Fixture::new();
            f.write("資料.rs", "fn old() {}\n");
            f.commit("Synthetic benchmark base");
            f.write("資料.rs", "fn staged() {}\n");
            f.run(&["add", "資料.rs"]);
            f.write("資料.rs", "fn working() {}\n");
            f.write("untracked.txt", "keep\n");
            let originals: [Vec<u8>; 4] = [".git/HEAD", ".git/index", ".git/config", "資料.rs"]
                .map(|p| fs::read(f.repository.join(p)).unwrap());
            let b = Backend::discover(&f.root.join("logs")).unwrap();
            let original_head = f.repo().head(&CancellationToken::new()).unwrap();
            let progress = Arc::new(Mutex::new(Report::default()));
            b.benchmark(&f.repository, &progress, &CancellationToken::new())
                .unwrap();
            let r = progress.lock().unwrap();
            assert_eq!(r.samples.len(), 9);
            assert!(r.samples.iter().all(|s| s.elapsed_us.is_some()));
            assert_eq!(
                r.samples
                    .iter()
                    .filter(|s| s.operation == Operation::History)
                    .map(|s| s.items)
                    .collect::<Vec<_>>(),
                vec![1, 1, 1]
            );
            assert_eq!(
                r.samples
                    .iter()
                    .filter(|s| s.operation == Operation::Diff)
                    .map(|s| s.items)
                    .collect::<Vec<_>>(),
                vec![1, 1, 1]
            );
            assert!(!r.changed);
            assert!(!r.observation_unavailable);
            assert_eq!(
                f.repo().head(&CancellationToken::new()).unwrap(),
                original_head
            );
            for (p, original) in [".git/HEAD", ".git/index", ".git/config", "資料.rs"]
                .into_iter()
                .zip(originals)
            {
                assert_eq!(fs::read(f.repository.join(p)).unwrap(), original, "{p}");
            }
            assert_eq!(
                fs::read_to_string(f.repository.join("untracked.txt")).unwrap(),
                "keep\n"
            );
            let log = fs::read_to_string(f.root.join("logs/rocket.jsonl")).unwrap();
            for label in [
                "benchmark",
                "benchmark-refresh",
                "benchmark-history",
                "benchmark-diff",
            ] {
                assert!(log.contains(&format!("\"operation\":\"{label}\"")));
            }
            assert!(!log.contains("資料"));
            assert!(!r.summary().to_string().contains("資料"));
        },
    );
}
#[test]
fn benchmark_unborn_and_bare_use_explicit_skips_and_real_reads() {
    common::isolated(
        "ROCKET_BENCHMARK_CASE",
        "benchmark_unborn_and_bare_use_explicit_skips_and_real_reads",
        || {
            let f = Fixture::new();
            f.write("existing.txt", "keep");
            let b = Backend::discover(&f.root.join("logs")).unwrap();
            let progress = Arc::new(Mutex::new(Report::default()));
            b.benchmark(&f.repository, &progress, &CancellationToken::new())
                .unwrap();
            let report = progress.lock().unwrap().clone();
            assert_eq!(report.samples.len(), 9);
            assert_eq!(
                report
                    .samples
                    .iter()
                    .filter(|s| s.elapsed_us.is_none())
                    .count(),
                6
            );
            assert!(!f.repository.join(".git/index").exists());
            f.commit("base");
            let bare = f.root.join("bare.git");
            f.run(&[
                "clone",
                "--bare",
                f.repository.to_str().unwrap(),
                bare.to_str().unwrap(),
            ]);
            let progress = Arc::new(Mutex::new(Report::default()));
            b.benchmark(&bare, &progress, &CancellationToken::new())
                .unwrap();
            assert_eq!(progress.lock().unwrap().samples.len(), 9);
            assert!(
                progress
                    .lock()
                    .unwrap()
                    .samples
                    .iter()
                    .all(|s| s.elapsed_us.is_some())
            );
        },
    );
}
#[test]
fn benchmark_reports_setup_failure_and_pre_cancel_without_success_samples() {
    common::isolated(
        "ROCKET_BENCHMARK_CASE",
        "benchmark_reports_setup_failure_and_pre_cancel_without_success_samples",
        || {
            let f = Fixture::new();
            let b = Backend::discover(&f.root.join("logs")).unwrap();
            let progress = Arc::new(Mutex::new(Report::default()));
            assert!(
                b.benchmark(
                    &f.root.join("missing"),
                    &progress,
                    &CancellationToken::new()
                )
                .is_err()
            );
            assert_eq!(progress.lock().unwrap().failure, Some("read_failed"));
            assert!(progress.lock().unwrap().samples.is_empty());
            let cancel = CancellationToken::new();
            cancel.cancel();
            let progress = Arc::new(Mutex::new(Report::default()));
            assert!(matches!(
                b.benchmark(&f.repository, &progress, &cancel),
                Err(Error::Cancelled)
            ));
            assert!(progress.lock().unwrap().samples.is_empty());
            assert_eq!(progress.lock().unwrap().failure, Some("cancelled"));
        },
    );
}
