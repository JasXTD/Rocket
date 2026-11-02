#[path = "../../rocket-git/tests/common/mod.rs"]
#[allow(dead_code)]
mod common;
use common::{Fixture, isolated};
use rocket_git::*;
use std::fs;

fn commands(log: &DiagnosticLog) -> usize {
    fs::read_to_string(log.directory().join("rocket.jsonl"))
        .unwrap()
        .lines()
        .filter(|line| line.contains("\"event\":\"git_start\""))
        .count()
}

#[test]
fn image_pairs_batch_reads_and_deduplicate_identical_blobs() {
    isolated(
        "ROCKET_READ_PERFORMANCE",
        "image_pairs_batch_reads_and_deduplicate_identical_blobs",
        || {
            let f = Fixture::new();
            f.write("image.png", b"before\0image");
            f.commit("before");
            f.write("image.png", b"after\0image");
            f.run(&["add", "image.png"]);
            let log = DiagnosticLog::open(f.root.join("read-logs")).unwrap();
            let git = GitClient::discover_with_log(log.clone()).unwrap();
            let cancel = CancellationToken::new();
            let repo = git.open(&f.repository, &cancel).unwrap();
            let options = DiffOptions {
                target: DiffTarget::Staged,
                ..Default::default()
            };
            let comparison = repo.image_comparisons(&options, &cancel).unwrap().remove(0);
            let before = commands(&log);
            let versions = repo.image_versions(&comparison, &cancel).unwrap();
            assert_eq!(
                versions.before.as_deref(),
                Some(b"before\0image".as_slice())
            );
            assert_eq!(versions.after.as_deref(), Some(b"after\0image".as_slice()));
            assert_eq!(
                commands(&log) - before,
                2,
                "one size batch and one body batch"
            );
            f.commit("after");
            f.run(&["mv", "image.png", "renamed.png"]);
            let comparison = repo.image_comparisons(&options, &cancel).unwrap().remove(0);
            let before = commands(&log);
            let versions = repo.image_versions(&comparison, &cancel).unwrap();
            assert_eq!(versions.before, versions.after);
            assert_eq!(
                commands(&log) - before,
                2,
                "a rename reads its shared blob only once"
            );
        },
    );
}

#[test]
fn image_batch_preserves_per_file_and_custom_command_limits() {
    isolated(
        "ROCKET_READ_PERFORMANCE",
        "image_batch_preserves_per_file_and_custom_command_limits",
        || {
            for (size, limit) in [
                (IMAGE_FILE_LIMIT, CommandLimits::default().stdout_bytes),
                (4096, 4096),
            ] {
                let f = Fixture::new();
                let before = vec![0; size];
                let mut after = vec![1; size];
                after[0] = 0;
                f.write("image.png", &before);
                f.commit("before");
                f.write("image.png", &after);
                f.run(&["add", "image.png"]);
                let log = DiagnosticLog::open(f.root.join("read-logs")).unwrap();
                let client = GitClient::new(
                    f.git.executable(),
                    CommandLimits {
                        stdout_bytes: limit,
                        ..Default::default()
                    },
                )
                .unwrap()
                .with_log(log.clone());
                let cancel = CancellationToken::new();
                let repo = client.open(&f.repository, &cancel).unwrap();
                let options = DiffOptions {
                    target: DiffTarget::Staged,
                    ..Default::default()
                };
                let comparison = repo.image_comparisons(&options, &cancel).unwrap().remove(0);
                let count = commands(&log);
                let versions = repo.image_versions(&comparison, &cancel).unwrap();
                assert_eq!(versions.before.as_deref(), Some(before.as_slice()));
                assert_eq!(versions.after.as_deref(), Some(after.as_slice()));
                assert_eq!(
                    commands(&log) - count,
                    3,
                    "size batch plus separate bounded bodies"
                );
                if size == IMAGE_FILE_LIMIT {
                    after.push(1);
                    f.write("image.png", &after);
                    f.run(&["add", "image.png"]);
                    let comparison = repo.image_comparisons(&options, &cancel).unwrap().remove(0);
                    let count = commands(&log);
                    assert!(repo.image_versions(&comparison, &cancel).is_err());
                    assert_eq!(
                        commands(&log) - count,
                        1,
                        "reject oversize metadata before reading any body"
                    );
                }
            }
        },
    );
}

#[test]
fn submodule_previews_reuse_metadata_and_reject_foreign_records() {
    isolated(
        "ROCKET_READ_PERFORMANCE",
        "submodule_previews_reuse_metadata_and_reject_foreign_records",
        || {
            let f = Fixture::new();
            f.write("a", "first");
            f.commit("first");
            let first = String::from_utf8(f.run(&["rev-parse", "HEAD"])).unwrap();
            f.write("a", "second");
            f.commit("second");
            let second = String::from_utf8(f.run(&["rev-parse", "HEAD"])).unwrap();
            f.run(&[
                "update-index",
                "--add",
                "--cacheinfo",
                &format!("160000,{},module", first.trim()),
            ]);
            f.run(&["commit", "-m", "gitlink"]);
            f.run(&[
                "update-index",
                "--cacheinfo",
                &format!("160000,{},module", second.trim()),
            ]);
            let log = DiagnosticLog::open(f.root.join("read-logs")).unwrap();
            let cancel = CancellationToken::new();
            let repo = f
                .git
                .with_log(log.clone())
                .open(&f.repository, &cancel)
                .unwrap();
            let options = DiffOptions {
                target: DiffTarget::Staged,
                ..Default::default()
            };
            let count = commands(&log);
            let files = repo.file_comparisons(&options, &cancel).unwrap();
            let metadata = commands(&log) - count;
            let count = commands(&log);
            let reused = repo.submodule_diffs_for_files(&files, &cancel).unwrap();
            let shared = commands(&log) - count;
            let count = commands(&log);
            let fresh = repo.submodule_diffs(&options, &cancel).unwrap();
            assert!(metadata > 0);
            assert_eq!(commands(&log) - count, shared + metadata);
            assert_eq!(reused.len(), 1);
            assert_eq!(fresh.len(), 1);
            assert_eq!(reused[0].before, fresh[0].before);
            assert_eq!(reused[0].after, fresh[0].after);
            assert!(
                reused[0].notice.is_some(),
                "missing submodule stays explicitly unavailable"
            );
            let other = Fixture::new();
            assert!(
                other
                    .repo()
                    .submodule_diffs_for_files(&files, &cancel)
                    .is_err()
            );
            cancel.cancel();
            assert!(matches!(
                repo.submodule_diffs_for_files(&files, &cancel),
                Err(Error::Cancelled)
            ));
        },
    );
}
