mod common;
use common::{Fixture, isolated};
use rocket_git::*;
use std::{
    collections::BTreeMap,
    fs,
    path::Path,
    thread,
    time::{Duration, Instant},
};
fn token() -> CancellationToken {
    CancellationToken::new()
}
fn oid(f: &Fixture, name: &str) -> ObjectId {
    f.repo().resolve_commit(name, &token()).unwrap()
}
fn snapshot(root: &Path) -> BTreeMap<String, Vec<u8>> {
    fn walk(root: &Path, at: &Path, map: &mut BTreeMap<String, Vec<u8>>) {
        for e in fs::read_dir(at).unwrap() {
            let e = e.unwrap();
            let p = e.path();
            let t = e.file_type().unwrap();
            if t.is_dir() {
                walk(root, &p, map)
            } else if t.is_file() {
                map.insert(
                    p.strip_prefix(root).unwrap().to_string_lossy().into(),
                    fs::read(p).unwrap(),
                );
            }
        }
    }
    let mut m = BTreeMap::new();
    walk(root, root, &mut m);
    m
}
fn divergent(conflict: bool) -> Fixture {
    let f = Fixture::new();
    f.write("文/検索 🚀.txt", "base\n");
    f.commit("PRIVATE base");
    f.run(&["switch", "-c", "destination"]);
    f.write("文/検索 🚀.txt", "destination\n");
    f.commit("PRIVATE destination");
    f.run(&["switch", "main"]);
    f.write(
        if conflict {
            "文/検索 🚀.txt"
        } else {
            "local.txt"
        },
        "local\n",
    );
    f.commit("PRIVATE local");
    f
}
fn preview(f: &Fixture) -> RebasePreview {
    f.repo()
        .preview_rebase(RebaseAction::Start, "destination", &token())
        .unwrap()
}
fn apply(f: &Fixture, p: &RebasePreview) -> RebaseWriteReport {
    f.repo().apply_rebase(p, &token()).unwrap()
}
fn abort(f: &Fixture) {
    let p = f
        .repo()
        .preview_rebase(RebaseAction::Abort, "", &token())
        .unwrap();
    assert!(p.prediction().is_none());
    assert!(apply(f, &p).write.result.is_ok());
}
#[test]
fn clean_renames_unicode_and_object_formats_match_rebase_without_preview_writes() {
    isolated(
        "ROCKET_REBASE_PREDICT",
        "clean_renames_unicode_and_object_formats_match_rebase_without_preview_writes",
        || {
            for format in ["sha1", "sha256"] {
                let f = Fixture::with_format(format);
                f.write("文/old.txt", "one\ntwo\nthree\n");
                f.write(".gitattributes", "*.txt text eol=lf\n");
                f.commit("base");
                f.run(&["switch", "-c", "destination"]);
                f.write("remote.txt", "remote\n");
                f.commit("destination");
                f.run(&["switch", "main"]);
                f.run(&["mv", "文/old.txt", "文/new 🚀.txt"]);
                f.write("文/new 🚀.txt", "one\nlocal\nthree\n");
                f.commit("rename");
                f.write("keep untracked.txt", "keep");
                let before = snapshot(&f.repository);
                let p = preview(&f);
                assert_eq!(
                    p.prediction().unwrap().kind,
                    PredictionKind::Clean,
                    "{:?}",
                    p.prediction()
                );
                assert_eq!(p.prediction().unwrap().checked, 1);
                assert_eq!(snapshot(&f.repository), before);
                let r = apply(&f, &p);
                assert!(r.write.result.is_ok(), "{r:?}");
                assert!(r.state.unwrap().is_idle());
                assert_eq!(
                    fs::read(f.repository.join("文/new 🚀.txt")).unwrap(),
                    b"one\nlocal\nthree\n"
                );
                assert_eq!(
                    fs::read(f.repository.join("keep untracked.txt")).unwrap(),
                    b"keep"
                );
            }
        },
    );
}
#[test]
fn first_conflict_matches_stopped_commit_and_abort_restores_original() {
    isolated(
        "ROCKET_REBASE_PREDICT",
        "first_conflict_matches_stopped_commit_and_abort_restores_original",
        || {
            let f = divergent(false);
            f.write("文/検索 🚀.txt", "local change\n");
            f.commit("PRIVATE conflict");
            let stopped = oid(&f, "HEAD");
            f.write("last.txt", "last");
            f.commit("last");
            let original = oid(&f, "HEAD");
            let before = snapshot(&f.repository);
            let p = preview(&f);
            let c = p.prediction().unwrap();
            assert_eq!(c.kind, PredictionKind::Conflicts);
            assert_eq!((c.checked, c.total, c.stopped_at), (2, 3, Some(1)));
            assert_eq!(c.paths[0].as_bytes(), "文/検索 🚀.txt".as_bytes());
            assert_eq!(snapshot(&f.repository), before);
            let r = apply(&f, &p);
            assert!(r.write.result.is_err());
            assert!(!r.state.unwrap().is_idle());
            assert_eq!(oid(&f, "REBASE_HEAD"), stopped);
            abort(&f);
            assert_eq!(oid(&f, "HEAD"), original);
            assert_eq!(
                fs::read(f.repository.join("文/検索 🚀.txt")).unwrap(),
                b"local change\n"
            );
        },
    );
}
#[test]
fn originally_empty_is_kept_but_newly_empty_stops_prediction_and_execution() {
    isolated(
        "ROCKET_REBASE_PREDICT",
        "originally_empty_is_kept_but_newly_empty_stops_prediction_and_execution",
        || {
            let f = divergent(false);
            f.run(&["commit", "--allow-empty", "-m", "originally empty"]);
            let p = preview(&f);
            assert_eq!(p.prediction().unwrap().kind, PredictionKind::Clean);
            assert_eq!(p.prediction().unwrap().checked, 2);
            assert!(apply(&f, &p).write.result.is_ok());
            assert_eq!(
                String::from_utf8(f.run(&["log", "-1", "--format=%s"]))
                    .unwrap()
                    .trim(),
                "originally empty"
            );
            let f = divergent(false);
            f.write("文/検索 🚀.txt", "destination\n");
            f.commit("same as destination");
            let empty = oid(&f, "HEAD");
            f.write("last", "last");
            f.commit("last");
            let p = preview(&f);
            let c = p.prediction().unwrap();
            assert_eq!(c.kind, PredictionKind::Empty);
            assert_eq!((c.checked, c.total, c.stopped_at), (2, 3, Some(1)));
            assert!(apply(&f, &p).write.result.is_err());
            assert_eq!(oid(&f, "REBASE_HEAD"), empty);
            abort(&f);
        },
    );
}
#[test]
fn attributes_and_custom_drivers_are_unavailable_and_never_executed() {
    isolated(
        "ROCKET_REBASE_PREDICT",
        "attributes_and_custom_drivers_are_unavailable_and_never_executed",
        || {
            let f = divergent(false);
            f.run(&[
                "config",
                "merge.custom.driver",
                "echo PRIVATE_DRIVER > never-run",
            ]);
            let before = snapshot(&f.repository);
            let p = preview(&f);
            assert_eq!(
                p.prediction().unwrap().reason,
                Some(PredictionReason::MergeSettings)
            );
            assert_eq!(snapshot(&f.repository), before);
            f.run(&["config", "--unset", "merge.custom.driver"]);
            f.write(".gitattributes", "*.txt text\n");
            f.commit("attributes");
            assert_eq!(
                preview(&f).prediction().unwrap().reason,
                Some(PredictionReason::AttributeChange)
            );
            let f = divergent(false);
            f.run(&["switch", "destination"]);
            f.write(".gitattributes", "*.txt merge=union\n");
            f.commit("destination attributes");
            f.run(&["switch", "main"]);
            assert_eq!(
                preview(&f).prediction().unwrap().reason,
                Some(PredictionReason::AttributeChange)
            );
        },
    );
}
#[test]
fn checkout_destination_index_and_merge_setting_changes_refuse_stale_apply() {
    isolated(
        "ROCKET_REBASE_PREDICT",
        "checkout_destination_index_and_merge_setting_changes_refuse_stale_apply",
        || {
            for change in [
                "worktree",
                "index",
                "head",
                "destination",
                "settings",
                "attributes",
                "collision",
            ] {
                let f = divergent(false);
                if change == "collision" {
                    f.run(&["switch", "destination"]);
                    f.write("destination-only.txt", "remote");
                    f.commit("destination file");
                    f.run(&["switch", "main"]);
                }
                let p = preview(&f);
                match change {
                    "worktree" => f.write("local.txt", "PRIVATE external\n"),
                    "index" => {
                        f.write("local.txt", "staged\n");
                        f.run(&["add", "local.txt"]);
                    }
                    "head" => {
                        f.write("next", "next");
                        f.commit("next");
                    }
                    "destination" => {
                        f.run(&[
                            "update-ref",
                            "refs/heads/destination",
                            oid(&f, "destination^").as_str(),
                        ]);
                    }
                    "settings" => {
                        f.run(&["config", "merge.renames", "false"]);
                    }
                    "attributes" => {
                        fs::create_dir_all(f.repository.join(".git/info")).unwrap();
                        fs::write(
                            f.repository.join(".git/info/attributes"),
                            "*.txt merge=union\n",
                        )
                        .unwrap();
                    }
                    "collision" => {
                        f.write("destination-only.txt", "PRIVATE untracked collision");
                    }
                    _ => unreachable!(),
                }
                let before = snapshot(&f.repository);
                assert!(
                    f.repo().apply_rebase(&p, &token()).is_err(),
                    "accepted {change}"
                );
                assert_eq!(snapshot(&f.repository), before, "mutated {change}");
            }
        },
    );
}
#[test]
fn running_check_cancellation_and_concurrent_changes_discard_results_and_clean_scratch() {
    isolated(
        "ROCKET_REBASE_PREDICT",
        "running_check_cancellation_and_concurrent_changes_discard_results_and_clean_scratch",
        || {
            for change in ["cancel", "worktree", "destination", "settings"] {
                let f = divergent(false);
                let log = DiagnosticLog::open(f.root.join("logs")).unwrap();
                let repo = f.repo().with_log(log.clone());
                let entries = || {
                    fs::read_dir(std::env::temp_dir())
                        .unwrap()
                        .filter_map(|e| e.ok())
                        .map(|e| e.file_name())
                        .filter(|n| {
                            n.to_string_lossy()
                                .starts_with(&format!("rocket-patch-{}-", std::process::id()))
                        })
                        .collect::<std::collections::BTreeSet<_>>()
                };
                let scratch = entries();
                let before = snapshot(&f.repository);
                let cancel = token();
                let child = cancel.clone();
                let worker = thread::spawn(move || {
                    repo.preview_rebase(RebaseAction::Start, "destination", &child)
                });
                let start = Instant::now();
                loop {
                    let text =
                        fs::read_to_string(f.root.join("logs/rocket.jsonl")).unwrap_or_default();
                    if text.lines().any(|l| {
                        l.contains("\"event\":\"step_start\"")
                            && l.contains("\"step\":\"simulate_commits\"")
                    }) {
                        break;
                    }
                    assert!(!worker.is_finished(), "No simulation barrier: {text}");
                    assert!(start.elapsed() < Duration::from_secs(45));
                    thread::sleep(Duration::from_millis(5));
                }
                match change {
                    "cancel" => cancel.cancel(),
                    "worktree" => f.write("local.txt", "PRIVATE concurrent\n"),
                    "destination" => {
                        f.run(&[
                            "update-ref",
                            "refs/heads/destination",
                            oid(&f, "destination^").as_str(),
                        ]);
                    }
                    "settings" => {
                        f.run(&["config", "merge.renames", "false"]);
                    }
                    _ => unreachable!(),
                }
                let result = worker.join().unwrap();
                if change == "cancel" {
                    assert!(matches!(result, Err(Error::Cancelled)), "{result:?}");
                    assert_eq!(snapshot(&f.repository), before)
                } else {
                    assert!(matches!(result, Err(Error::Rebase(_))), "{result:?}")
                }
                assert_eq!(entries(), scratch);
                let text = fs::read_to_string(f.root.join("logs/rocket.jsonl")).unwrap();
                assert!(text.contains("cleanup_preview"));
                assert!(!text.contains("PRIVATE"));
                assert_eq!(log.health().dropped_events, 0);
            }
        },
    );
}
#[test]
fn pull_prediction_uses_fetched_tip_and_rechecks_upstream_and_settings() {
    isolated(
        "ROCKET_REBASE_PREDICT",
        "pull_prediction_uses_fetched_tip_and_rechecks_upstream_and_settings",
        || {
            for conflict in [false, true] {
                let f = divergent(conflict);
                let bare = f.root.join("remote.git");
                f.run(&[
                    "clone",
                    "--bare",
                    f.repository.to_str().unwrap(),
                    bare.to_str().unwrap(),
                ]);
                f.run(&["remote", "add", "origin", bare.to_str().unwrap()]);
                f.run(&["fetch", "origin"]);
                f.run(&["branch", "--set-upstream-to=origin/destination"]);
                let repo = f.repo();
                let ticket = repo
                    .fetch_for_pull(None, &token())
                    .unwrap()
                    .prepared
                    .unwrap()
                    .unwrap();
                let before = snapshot(&f.repository);
                let p = repo
                    .preview_pull(&ticket, PullStrategy::Rebase, &token())
                    .unwrap();
                assert_eq!(
                    p.prediction().unwrap().kind,
                    if conflict {
                        PredictionKind::Conflicts
                    } else {
                        PredictionKind::Clean
                    }
                );
                assert_eq!(snapshot(&f.repository), before);
                f.run(&["config", "merge.renames", "false"]);
                let before = snapshot(&f.repository);
                assert!(matches!(
                    repo.apply_pull(&p, &token()),
                    Err(Error::Rebase(RebaseFailure::StalePreview))
                ));
                assert_eq!(snapshot(&f.repository), before);
                let p = repo
                    .preview_pull(&ticket, PullStrategy::Rebase, &token())
                    .unwrap();
                match repo.apply_pull(&p, &token()).unwrap() {
                    PullApplyReport::Rebase(r) => assert_eq!(r.write.result.is_ok(), !conflict),
                    other => panic!("{other:?}"),
                };
                if conflict {
                    abort(&f)
                }
            }
            let f = divergent(false);
            let bare = f.root.join("remote.git");
            f.run(&[
                "clone",
                "--bare",
                f.repository.to_str().unwrap(),
                bare.to_str().unwrap(),
            ]);
            f.run(&["remote", "add", "origin", bare.to_str().unwrap()]);
            f.run(&["fetch", "origin"]);
            f.run(&["branch", "--set-upstream-to=origin/destination"]);
            let repo = f.repo();
            let t = repo
                .fetch_for_pull(None, &token())
                .unwrap()
                .prepared
                .unwrap()
                .unwrap();
            let p = repo
                .preview_pull(&t, PullStrategy::Rebase, &token())
                .unwrap();
            f.run(&[
                "update-ref",
                "refs/remotes/origin/destination",
                oid(&f, "HEAD").as_str(),
            ]);
            let before = snapshot(&f.repository);
            assert!(matches!(
                repo.apply_pull(&p, &token()),
                Err(Error::Pull(PullFailure::StateChanged))
            ));
            assert_eq!(snapshot(&f.repository), before);
        },
    );
}
#[test]
fn no_replay_interactive_and_pre_cancel_do_not_claim_simulation() {
    isolated(
        "ROCKET_REBASE_PREDICT",
        "no_replay_interactive_and_pre_cancel_do_not_claim_simulation",
        || {
            let f = divergent(false);
            let p = f
                .repo()
                .preview_rebase(RebaseAction::Start, "HEAD", &token())
                .unwrap();
            assert!(p.up_to_date());
            assert!(p.prediction().is_none());
            let p = f
                .repo()
                .preview_rebase(RebaseAction::Interactive, "HEAD^", &token())
                .unwrap();
            assert!(p.prediction().is_none());
            let c = token();
            c.cancel();
            let before = snapshot(&f.repository);
            assert!(matches!(
                f.repo()
                    .preview_rebase(RebaseAction::Start, "destination", &c),
                Err(Error::Cancelled)
            ));
            assert_eq!(snapshot(&f.repository), before);
            f.run(&["switch", "-c", "behind", "destination^"]);
            let p = preview(&f);
            assert!(p.fast_forward());
            assert!(p.prediction().is_none());
        },
    );
}
