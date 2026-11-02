//! Batch branch deletion, tag deletion and tag publication. Each item uses the
//! single-item operation with its own checks; the batch reports every item.
#[allow(dead_code)]
mod common;
use common::{Fixture, isolated};
use rocket_git::*;

const CASE: &str = "ROCKET_BATCH_ACTIONS_CASE";

fn token() -> CancellationToken {
    CancellationToken::new()
}
fn names(values: &[&str]) -> Vec<Vec<u8>> {
    values.iter().map(|v| v.as_bytes().to_vec()).collect()
}
fn refs(f: &Fixture, git_dir: Option<&std::path::Path>, prefix: &str) -> Vec<String> {
    let mut arguments = Vec::new();
    let dir;
    if let Some(path) = git_dir {
        dir = format!("--git-dir={}", path.display());
        arguments.push(dir.as_str());
    }
    arguments.extend(["for-each-ref", "--format=%(refname:short)", prefix]);
    String::from_utf8(f.run(&arguments))
        .unwrap()
        .lines()
        .map(Into::into)
        .collect()
}

#[test]
fn branch_batch_deletes_merged_branches_and_reports_each_refusal() {
    isolated(
        CASE,
        "branch_batch_deletes_merged_branches_and_reports_each_refusal",
        || {
            let f = Fixture::new();
            f.write("a.txt", "1\n");
            f.commit("base");
            for branch in ["merged-one", "merged-two", "unmerged"] {
                f.run(&["branch", branch]);
            }
            f.run(&["switch", "unmerged"]);
            f.write("u.txt", "PRIVATE work\n");
            f.commit("unmerged work");
            f.run(&["switch", "main"]);
            let repo = f.repo();
            let report = repo
                .delete_branches(
                    &names(&["merged-one", "unmerged", "main", "merged-two"]),
                    &token(),
                )
                .unwrap();
            assert!(report.skipped.is_empty());
            let outcome: Vec<(String, bool)> = report
                .results
                .iter()
                .map(|(name, result)| {
                    (
                        String::from_utf8_lossy(name).into_owned(),
                        result.as_ref().is_ok_and(|r| r.write.result.is_ok()),
                    )
                })
                .collect();
            // Git refuses the unmerged and the checked-out branch; the rest go on.
            assert_eq!(
                outcome,
                [
                    ("merged-one".to_owned(), true),
                    ("unmerged".to_owned(), false),
                    ("main".to_owned(), false),
                    ("merged-two".to_owned(), true),
                ]
            );
            assert_eq!(refs(&f, None, "refs/heads"), ["main", "unmerged"]);

            for bad in [names(&[]), names(&["unmerged", "unmerged"])] {
                assert!(matches!(
                    repo.delete_branches(&bad, &token()),
                    Err(Error::InvalidInput(_))
                ));
            }
            let too_many: Vec<Vec<u8>> = (0..=MAX_BRANCH_BATCH)
                .map(|i| format!("b{i}").into_bytes())
                .collect();
            assert!(matches!(
                repo.delete_branches(&too_many, &token()),
                Err(Error::InvalidInput(_))
            ));
            // Cancelled first: nothing is attempted.
            let cancel = token();
            cancel.cancel();
            let report = repo
                .delete_branches(&names(&["unmerged"]), &cancel)
                .unwrap();
            assert!(report.results.is_empty());
            assert_eq!(report.skipped, names(&["unmerged"]));
            assert_eq!(refs(&f, None, "refs/heads"), ["main", "unmerged"]);
        },
    );
}

#[test]
fn tag_batch_deletes_in_order_and_stops_at_the_first_unverified_change() {
    isolated(
        CASE,
        "tag_batch_deletes_in_order_and_stops_at_the_first_unverified_change",
        || {
            let f = Fixture::new();
            f.write("a.txt", "1\n");
            f.commit("base");
            f.run(&["tag", "light"]);
            f.run(&[
                "tag",
                "--no-sign",
                "-a",
                "annotated/文",
                "-m",
                "PRIVATE note",
            ]);
            f.run(&["tag", "third"]);
            f.run(&["tag", "fourth"]);
            f.run(&["symbolic-ref", "refs/tags/indirect", "refs/heads/main"]);
            let repo = f.repo();
            let page = repo.tags(100, &token()).unwrap();
            let entry = |name: &str| {
                page.entries
                    .iter()
                    .find(|e| e.name == name.as_bytes())
                    .unwrap()
                    .clone()
            };
            // A symbolic tag is refused by name while reviewing; the others are ready.
            let review = repo
                .preview_delete_tags(
                    &[entry("light"), entry("indirect"), entry("annotated/文")],
                    &token(),
                )
                .unwrap();
            assert_eq!(review.ready.len(), 2);
            assert!(matches!(
                review.refused.as_slice(),
                [(name, Error::Tag(TagFailure::SymbolicTag))] if name == b"indirect"
            ));
            let report = repo.apply_tags(&review.ready, &token()).unwrap();
            assert!(report.skipped.is_empty());
            assert_eq!(report.results.len(), 2);
            for (name, result) in &report.results {
                let r = result.as_ref().unwrap();
                assert!(
                    r.result.is_ok() && r.verification.is_ok() && r.failure.is_none(),
                    "{name:?}"
                );
                assert!(r.recovery_reference.is_some(), "{name:?}");
            }
            assert_eq!(refs(&f, None, "refs/tags"), ["fourth", "indirect", "third"]);

            // A tag moved after review stops the batch; later tags are untouched.
            let previews = repo
                .preview_delete_tags(&[entry("third"), entry("fourth")], &token())
                .unwrap()
                .ready;
            f.write("a.txt", "2\n");
            f.commit("second");
            f.run(&["tag", "-f", "third", "HEAD"]);
            let report = repo.apply_tags(&previews, &token()).unwrap();
            assert_eq!(report.results.len(), 1);
            assert!(!matches!(
                &report.results[0].1,
                Ok(r) if r.result.is_ok() && r.verification.is_ok() && r.failure.is_none()
            ));
            assert_eq!(report.skipped, names(&["fourth"]));
            assert_eq!(refs(&f, None, "refs/tags"), ["fourth", "indirect", "third"]);

            assert!(matches!(
                repo.preview_delete_tags(&[], &token()),
                Err(Error::InvalidInput(_))
            ));
            assert!(matches!(
                repo.preview_delete_tags(&[entry("fourth"), entry("fourth")], &token()),
                Err(Error::InvalidInput(_))
            ));
        },
    );
}

#[test]
fn tag_batch_publishes_in_order_and_stops_at_a_changed_remote_tag() {
    isolated(
        CASE,
        "tag_batch_publishes_in_order_and_stops_at_a_changed_remote_tag",
        || {
            let f = Fixture::new();
            f.write("a.txt", "1\n");
            f.commit("base");
            let remote = f.root.join("PRIVATE remote 文.git");
            f.run(&["init", "--bare", remote.to_str().unwrap()]);
            f.run(&["remote", "add", "origin", remote.to_str().unwrap()]);
            for tag in ["v1", "v2", "v3", "v4"] {
                f.run(&["tag", tag]);
            }
            let repo = f.repo();
            let snapshot = repo.remote_tags(b"origin", &token()).unwrap();
            let review = repo
                .preview_publish_tags(&snapshot, &names(&["v1", "v2"]), &token())
                .unwrap();
            assert!(review.refused.is_empty());
            let report = repo.apply_remote_tags(&review.ready, &token()).unwrap();
            assert!(report.skipped.is_empty());
            for (name, result) in &report.results {
                let r = result.as_ref().unwrap();
                assert!(
                    r.result.is_ok() && r.observed.is_ok() && r.verification.is_ok(),
                    "{name:?}"
                );
            }
            assert_eq!(refs(&f, Some(&remote), "refs/tags"), ["v1", "v2"]);

            // v3 is published by someone else after review: it is refused before
            // any push, and v4 is skipped without being sent.
            let snapshot = repo.remote_tags(b"origin", &token()).unwrap();
            let previews = repo
                .preview_publish_tags(&snapshot, &names(&["v3", "v4"]), &token())
                .unwrap()
                .ready;
            f.run(&["push", "origin", "refs/tags/v3:refs/tags/v3"]);
            let report = repo.apply_remote_tags(&previews, &token()).unwrap();
            assert_eq!(report.results.len(), 1);
            assert!(matches!(
                &report.results[0].1,
                Err(Error::Tag(TagFailure::RemoteChanged))
            ));
            assert_eq!(report.skipped, names(&["v4"]));
            assert_eq!(refs(&f, Some(&remote), "refs/tags"), ["v1", "v2", "v3"]);

            // A tag the remote already has is refused by name while reviewing, and
            // so is one published by someone else after the snapshot was read.
            let snapshot = repo.remote_tags(b"origin", &token()).unwrap();
            f.run(&["tag", "v5"]);
            let snapshot_with_v5 = repo.remote_tags(b"origin", &token()).unwrap();
            f.run(&["push", "origin", "refs/tags/v5:refs/tags/v5"]);
            let review = repo
                .preview_publish_tags(&snapshot_with_v5, &names(&["v4", "v1", "v5"]), &token())
                .unwrap();
            assert_eq!(review.ready.len(), 1);
            assert_eq!(review.ready[0].entry().name, b"v4");
            let refused: Vec<(&[u8], &Error)> = review
                .refused
                .iter()
                .map(|(n, e)| (n.as_slice(), e))
                .collect();
            assert!(matches!(
                refused.as_slice(),
                [
                    (b"v1", Error::Tag(TagFailure::RemoteExists)),
                    (b"v5", Error::Tag(TagFailure::RemoteChanged)),
                ]
            ));
            assert_eq!(
                refs(&f, Some(&remote), "refs/tags"),
                ["v1", "v2", "v3", "v5"]
            );
            assert!(matches!(
                repo.preview_publish_tags(&snapshot, &names(&["v4", "v4"]), &token()),
                Err(Error::InvalidInput(_))
            ));
        },
    );
}
