mod common;
use common::{Fixture, isolated};
use rocket_git::*;
use std::{collections::BTreeMap, fs, path::Path};

fn request(operation: CommitOperation, targets: Vec<ObjectId>) -> CommitActionRequest {
    CommitActionRequest {
        operation,
        targets,
        mainline: None,
        recovery: None,
    }
}
fn oid(f: &Fixture, revision: &str) -> ObjectId {
    f.repo()
        .resolve_commit(revision, &CancellationToken::new())
        .unwrap()
}
fn preview(f: &Fixture, r: CommitActionRequest) -> CommitPreview {
    f.repo()
        .preview_commit_action(r, &CancellationToken::new())
        .unwrap()
}
fn snapshot(root: &Path) -> BTreeMap<String, Vec<u8>> {
    fn walk(root: &Path, at: &Path, map: &mut BTreeMap<String, Vec<u8>>) {
        for entry in fs::read_dir(at).unwrap() {
            let e = entry.unwrap();
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
    let mut map = BTreeMap::new();
    walk(root, root, &mut map);
    map
}
fn applied(f: &Fixture, p: &CommitPreview) {
    let r = f
        .repo()
        .apply_commit_action(p, None, &CancellationToken::new())
        .unwrap();
    assert!(r.write.result.is_ok(), "{r:?}");
    assert!(r.state.unwrap().is_idle());
}
fn write_attributes(f: &Fixture, bytes: &[u8]) -> std::io::Result<()> {
    fs::create_dir_all(f.repository.join(".git/info"))?;
    fs::write(f.repository.join(".git/info/attributes"), bytes)
}

#[test]
fn clean_unicode_and_rename_predictions_match_actual_replay_without_preview_writes() {
    isolated(
        "ROCKET_PREDICTION_CASE",
        "clean_unicode_and_rename_predictions_match_actual_replay_without_preview_writes",
        || {
            for format in ["sha1", "sha256"] {
                let f = Fixture::with_format(format);
                f.write("文/old.txt", "one\ntwo\nthree\n");
                f.commit("base");
                f.run(&["switch", "-c", "source"]);
                f.run(&["mv", "文/old.txt", "文/new.txt"]);
                f.write("文/new.txt", "one\nchanged\nthree\n");
                f.commit("PRIVATE rename");
                let source = oid(&f, "HEAD");
                f.run(&["switch", "main"]);
                f.write("untracked 保留.txt", "PRIVATE retained");
                let before = snapshot(&f.repository);
                let p = preview(&f, request(CommitOperation::CherryPick, vec![source]));
                let result = p.prediction().unwrap();
                assert_eq!(result.kind, PredictionKind::Clean, "{result:?}");
                assert_eq!(result.checked, 1);
                assert_eq!(snapshot(&f.repository), before);
                applied(&f, &p);
                let picked = oid(&f, "HEAD");
                let before = snapshot(&f.repository);
                let p = preview(&f, request(CommitOperation::Revert, vec![picked]));
                assert_eq!(p.prediction().unwrap().kind, PredictionKind::Clean);
                assert_eq!(snapshot(&f.repository), before);
                applied(&f, &p);
                assert_eq!(
                    fs::read(f.repository.join("文/old.txt")).unwrap(),
                    b"one\ntwo\nthree\n"
                );
                assert_eq!(
                    fs::read(f.repository.join("untracked 保留.txt")).unwrap(),
                    b"PRIVATE retained"
                );
            }
        },
    );
}

#[test]
fn conflicting_sequences_stop_at_first_conflict_and_match_git() {
    isolated(
        "ROCKET_PREDICTION_CASE",
        "conflicting_sequences_stop_at_first_conflict_and_match_git",
        || {
            for operation in [CommitOperation::CherryPick, CommitOperation::Revert] {
                let f = Fixture::new();
                f.write("冲突.txt", "base\n");
                f.commit("base");
                if operation == CommitOperation::CherryPick {
                    f.run(&["switch", "-c", "source"]);
                }
                f.write("first.txt", "first");
                f.commit("first");
                let first = oid(&f, "HEAD");
                f.write("冲突.txt", "source\n");
                f.commit("second");
                let second = oid(&f, "HEAD");
                f.write("third.txt", "third");
                f.commit("third");
                let third = oid(&f, "HEAD");
                if operation == CommitOperation::CherryPick {
                    f.run(&["switch", "main"]);
                }
                f.write("冲突.txt", "destination\n");
                f.commit("dest");
                let original = oid(&f, "HEAD");
                let targets = if operation == CommitOperation::CherryPick {
                    vec![first, second, third]
                } else {
                    vec![third, second, first]
                };
                let before = snapshot(&f.repository);
                let p = preview(&f, request(operation, targets));
                let prediction = p.prediction().unwrap();
                assert_eq!(prediction.kind, PredictionKind::Conflicts, "{prediction:?}");
                assert_eq!(prediction.stopped_at, Some(1));
                assert_eq!(prediction.checked, 2);
                assert_eq!(prediction.total, 3);
                assert_eq!(
                    prediction.paths,
                    vec![RepoPath::new("冲突.txt".as_bytes().to_vec()).unwrap()]
                );
                assert_eq!(snapshot(&f.repository), before);
                let report = f
                    .repo()
                    .apply_commit_action(&p, None, &CancellationToken::new())
                    .unwrap();
                assert!(report.write.result.is_err());
                assert!(!report.state.unwrap().is_idle());
                let abort = preview(
                    &f,
                    CommitActionRequest {
                        operation,
                        targets: Vec::new(),
                        mainline: None,
                        recovery: Some(CommitRecovery::Abort),
                    },
                );
                assert!(abort.prediction().is_none());
                applied(&f, &abort);
                assert_eq!(oid(&f, "HEAD"), original);
            }
        },
    );
}

#[test]
fn empty_root_and_mainline_predictions() {
    isolated(
        "ROCKET_PREDICTION_CASE",
        "empty_root_and_mainline_predictions",
        || {
            let f = Fixture::new();
            f.write("root.txt", "base\n");
            f.commit("root");
            let root = oid(&f, "HEAD");
            let p = preview(&f, request(CommitOperation::CherryPick, vec![root.clone()]));
            assert_eq!(p.prediction().unwrap().kind, PredictionKind::Empty);
            f.run(&["switch", "--orphan", "unrelated"]);
            f.write("other.txt", "other");
            f.commit("other");
            let p = preview(&f, request(CommitOperation::CherryPick, vec![root]));
            assert_eq!(p.prediction().unwrap().kind, PredictionKind::Clean);
            applied(&f, &p);
            f.run(&["switch", "-c", "side"]);
            f.write("side.txt", "side");
            f.commit("side");
            f.run(&["switch", "unrelated"]);
            f.write("main.txt", "main");
            f.commit("main");
            f.run(&["merge", "--no-edit", "--no-ff", "side"]);
            let merged = oid(&f, "HEAD");
            let mut r = request(CommitOperation::Revert, vec![merged]);
            r.mainline = Some(1);
            let p = preview(&f, r);
            assert_eq!(p.prediction().unwrap().kind, PredictionKind::Clean);
            applied(&f, &p);
            assert!(!f.repository.join("side.txt").exists());
            assert!(f.repository.join("main.txt").exists());
        },
    );
}

#[test]
fn merge_settings_attributes_and_external_edits_invalidate_preview() {
    isolated(
        "ROCKET_PREDICTION_CASE",
        "merge_settings_attributes_and_external_edits_invalidate_preview",
        || {
            let f = Fixture::new();
            f.write("base.txt", "base\n");
            f.commit("base");
            f.run(&["switch", "-c", "source"]);
            f.write("base.txt", "source\n");
            f.commit("source");
            let source = oid(&f, "HEAD");
            f.run(&["switch", "main"]);
            let req = request(CommitOperation::CherryPick, vec![source]);
            let p = preview(&f, req.clone());
            f.run(&["config", "merge.renames", "false"]);
            let before = snapshot(&f.repository);
            assert!(matches!(
                f.repo()
                    .apply_commit_action(&p, None, &CancellationToken::new()),
                Err(Error::CommitAction(CommitFailure::StalePreview))
            ));
            assert_eq!(snapshot(&f.repository), before);
            let p = preview(&f, req.clone());
            write_attributes(&f, b"*.txt merge=union\n").unwrap();
            let before = snapshot(&f.repository);
            assert!(matches!(
                f.repo()
                    .apply_commit_action(&p, None, &CancellationToken::new()),
                Err(Error::CommitAction(CommitFailure::StalePreview))
            ));
            assert_eq!(snapshot(&f.repository), before);
            let p = preview(&f, req.clone());
            assert_eq!(
                p.prediction().unwrap().reason,
                Some(PredictionReason::Attributes)
            );
            write_attributes(&f, b"").unwrap();
            let p = preview(&f, req);
            f.write("base.txt", "external edit\n");
            let before = snapshot(&f.repository);
            assert!(matches!(
                f.repo()
                    .apply_commit_action(&p, None, &CancellationToken::new()),
                Err(Error::CommitAction(CommitFailure::DirtyWorktree))
            ));
            assert_eq!(snapshot(&f.repository), before);
        },
    );
}

#[test]
fn custom_drivers_filters_and_attribute_changes_are_not_reported_clean() {
    isolated(
        "ROCKET_PREDICTION_CASE",
        "custom_drivers_filters_and_attribute_changes_are_not_reported_clean",
        || {
            let f = Fixture::new();
            f.write("base.txt", "base\n");
            f.commit("base");
            f.run(&["switch", "-c", "source"]);
            f.write("base.txt", "source\n");
            f.commit("source");
            let target = oid(&f, "HEAD");
            f.run(&["switch", "main"]);
            let r = request(CommitOperation::CherryPick, vec![target]);
            f.run(&[
                "config",
                "merge.danger.driver",
                "echo PRIVATE_DRIVER > should-not-exist",
            ]);
            let before = snapshot(&f.repository);
            let p = preview(&f, r.clone());
            assert_eq!(
                p.prediction().unwrap().reason,
                Some(PredictionReason::MergeSettings)
            );
            assert_eq!(snapshot(&f.repository), before);
            assert!(!f.repository.join("should-not-exist").exists());
            f.run(&["config", "--unset", "merge.danger.driver"]);
            write_attributes(&f, b"*.txt filter=lfs\n").unwrap();
            let p = preview(&f, r);
            assert_eq!(
                p.prediction().unwrap().reason,
                Some(PredictionReason::Attributes)
            );
            write_attributes(&f, b"").unwrap();
            f.run(&["switch", "source"]);
            f.write(".gitattributes", "*.txt text=auto\n");
            f.commit("attributes");
            let target = oid(&f, "HEAD");
            f.run(&["switch", "main"]);
            let p = preview(&f, request(CommitOperation::CherryPick, vec![target]));
            assert_eq!(
                p.prediction().unwrap().reason,
                Some(PredictionReason::AttributeChange)
            );
        },
    );
}

#[test]
fn cancellation_does_not_change_repository() {
    isolated(
        "ROCKET_PREDICTION_CASE",
        "cancellation_does_not_change_repository",
        || {
            let f = Fixture::new();
            f.write("base", "base");
            f.commit("base");
            let target = oid(&f, "HEAD");
            let before = snapshot(&f.repository);
            let cancel = CancellationToken::new();
            cancel.cancel();
            assert!(matches!(
                f.repo().preview_commit_action(
                    request(CommitOperation::CherryPick, vec![target]),
                    &cancel
                ),
                Err(Error::Cancelled)
            ));
            assert_eq!(snapshot(&f.repository), before);
        },
    );
}

#[test]
fn running_prediction_cancels_or_refuses_external_changes_and_cleans_storage() {
    isolated(
        "ROCKET_PREDICTION_CASE",
        "running_prediction_cancels_or_refuses_external_changes_and_cleans_storage",
        || {
            use std::{
                thread,
                time::{Duration, Instant},
            };
            for cancel_check in [true, false] {
                let f = Fixture::new();
                f.write("base.txt", "base\n");
                f.commit("base");
                f.run(&["switch", "-c", "source"]);
                f.write("base.txt", "source\n");
                f.commit("source");
                let target = oid(&f, "HEAD");
                f.run(&["switch", "main"]);
                let temp_entries = || {
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
                let before_temp = temp_entries();
                let before = snapshot(&f.repository);
                let log = DiagnosticLog::open(f.root.join("logs")).unwrap();
                let repo = f.repo().with_log(log.clone());
                let token = CancellationToken::new();
                let worker_token = token.clone();
                let worker = thread::spawn(move || {
                    repo.preview_commit_action(
                        request(CommitOperation::CherryPick, vec![target]),
                        &worker_token,
                    )
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
                    assert!(
                        !worker.is_finished(),
                        "Prediction ended before simulation barrier: {text}"
                    );
                    assert!(start.elapsed() < Duration::from_secs(45));
                    thread::sleep(Duration::from_millis(5));
                }
                if cancel_check {
                    token.cancel();
                } else {
                    f.write("base.txt", "PRIVATE external edit during prediction\n");
                }
                let result = worker.join().unwrap();
                if cancel_check {
                    assert!(matches!(result, Err(Error::Cancelled)), "{result:?}");
                    assert_eq!(snapshot(&f.repository), before);
                } else {
                    assert!(
                        matches!(
                            result,
                            Err(Error::CommitAction(
                                CommitFailure::DirtyWorktree | CommitFailure::StalePreview
                            ))
                        ),
                        "{result:?}"
                    );
                    assert_eq!(
                        fs::read(f.repository.join("base.txt")).unwrap(),
                        b"PRIVATE external edit during prediction\n"
                    );
                }
                assert_eq!(
                    temp_entries(),
                    before_temp,
                    "Prediction left owned scratch storage"
                );
                let text = fs::read_to_string(f.root.join("logs/rocket.jsonl")).unwrap();
                assert!(text.contains("cleanup_preview"));
                assert!(!text.contains("PRIVATE"));
                assert_eq!(log.health().dropped_events, 0);
            }
        },
    );
}

#[test]
fn binary_conflicts_and_replacement_objects_are_explicit() {
    isolated(
        "ROCKET_PREDICTION_CASE",
        "binary_conflicts_and_replacement_objects_are_explicit",
        || {
            let f = Fixture::new();
            f.write("asset.bin", b"\0base");
            f.commit("base");
            let base = oid(&f, "HEAD");
            f.run(&["switch", "-c", "source"]);
            f.write("asset.bin", b"\0source");
            f.commit("source");
            let source = oid(&f, "HEAD");
            f.run(&["switch", "main"]);
            f.write("asset.bin", b"\0dest");
            f.commit("dest");
            let before = snapshot(&f.repository);
            let req = request(CommitOperation::CherryPick, vec![source.clone()]);
            let p = preview(&f, req.clone());
            assert_eq!(p.prediction().unwrap().kind, PredictionKind::Conflicts);
            assert_eq!(p.prediction().unwrap().paths[0].as_bytes(), b"asset.bin");
            assert_eq!(snapshot(&f.repository), before);
            f.run(&["replace", base.as_str(), source.as_str()]);
            let p = preview(&f, req);
            assert_eq!(
                p.prediction().unwrap().reason,
                Some(PredictionReason::ReplacedObjects)
            );
        },
    );
}
