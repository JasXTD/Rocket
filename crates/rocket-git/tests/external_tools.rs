#![cfg(windows)]
mod common;
use common::{Fixture, isolated};
use rocket_git::*;
use std::{
    fs,
    path::PathBuf,
    process::Command,
    sync::Arc,
    time::{Duration, Instant},
};
const NAME: &str = "src/検索/表示 🚀 é.rs";
fn path() -> RepoPath {
    RepoPath::new(NAME.as_bytes()).unwrap()
}
fn helper(f: &Fixture) -> PathBuf {
    let p = f.root.join("tool with spaces 文.exe");
    let output = Command::new("rustc")
        .args(["--edition=2024", "--crate-name=external_tool_fixture"])
        .arg(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples/external_tool_fixture.rs"))
        .arg("-o")
        .arg(&p)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    p
}
fn config(exe: &std::path::Path, kind: ExternalToolKind, mode: &str) -> ExternalToolConfig {
    ExternalToolConfig {
        executable: exe.display().to_string(),
        arguments: format!("{mode}\n{}", kind.placeholders().join("\n")),
    }
}
fn conflict() -> Fixture {
    let f = Fixture::new();
    f.write(NAME, "base\n");
    f.commit("base");
    f.run(&["checkout", "-b", "feature"]);
    f.write(NAME, "theirs\n");
    f.commit("theirs");
    f.run(&["checkout", "main"]);
    f.write(NAME, "ours\n");
    f.commit("ours");
    assert!(
        !f.output(&f.repository, &["merge", "feature"])
            .status
            .success()
    );
    f
}
#[test]
fn merge_import_is_reviewed_recoverable_and_refuses_external_edits() {
    isolated(
        "ROCKET_EXTERNAL_CASE",
        "merge_import_is_reviewed_recoverable_and_refuses_external_edits",
        || {
            let f = conflict();
            let exe = helper(&f);
            let r = f.repo();
            let c = CancellationToken::new();
            let log = DiagnosticLog::open(f.root.join("logs")).unwrap();
            let p = Arc::new(r.edit_conflict(&path(), &c).unwrap());
            let original = fs::read(f.repository.join(NAME)).unwrap();
            let index = fs::read(f.repository.join(".git/index")).unwrap();
            let head = f.run(&["rev-parse", "HEAD"]);
            let report = r.run_external_tool(
                &config(&exe, ExternalToolKind::Merge, "merge"),
                ExternalToolRequest::Merge {
                    preview: p.clone(),
                    draft: p.working().into(),
                },
                &f.root.join("copies"),
                &log,
                &c,
            );
            report.result.unwrap();
            assert_eq!(report.text.as_deref(), Some("resolved 世界 🚀 é\n"));
            assert_eq!(fs::read(f.repository.join(NAME)).unwrap(), original);
            assert_eq!(fs::read(f.repository.join(".git/index")).unwrap(), index);
            r.validate_conflict_preview(&p, &c).unwrap();
            f.write(NAME, "external edit\n");
            assert!(matches!(
                r.validate_conflict_preview(&p, &c),
                Err(Error::Conflict(ConflictFailure::FileChanged))
            ));
            let saved = r
                .save_conflict(&p, report.text.as_ref().unwrap(), true, &c)
                .unwrap();
            assert!(!saved.saved);
            assert!(saved.result.is_err());
            assert_eq!(
                fs::read(f.repository.join(NAME)).unwrap(),
                b"external edit\n"
            );
            f.write(NAME, &original);
            let saved = r
                .save_conflict(&p, report.text.as_ref().unwrap(), true, &c)
                .unwrap();
            saved.result.unwrap();
            assert!(saved.saved && saved.staged);
            assert_eq!(
                fs::read(saved.recovery_directory.unwrap().join("replaced-original")).unwrap(),
                original
            );
            assert_eq!(f.run(&["rev-parse", "HEAD"]), head);
        },
    );
}
#[test]
fn failed_invalid_missing_and_empty_results_do_not_write_repository() {
    isolated(
        "ROCKET_EXTERNAL_CASE",
        "failed_invalid_missing_and_empty_results_do_not_write_repository",
        || {
            let f = conflict();
            let exe = helper(&f);
            let r = f.repo();
            let c = CancellationToken::new();
            let log = DiagnosticLog::open(f.root.join("logs")).unwrap();
            let p = Arc::new(r.edit_conflict(&path(), &c).unwrap());
            let original = fs::read(f.repository.join(NAME)).unwrap();
            let index = fs::read(f.repository.join(".git/index")).unwrap();
            for mode in ["fail", "binary", "large", "missing", "empty"] {
                let result = r.run_external_tool(
                    &config(&exe, ExternalToolKind::Merge, mode),
                    ExternalToolRequest::Merge {
                        preview: p.clone(),
                        draft: p.working().into(),
                    },
                    &f.root.join("copies"),
                    &log,
                    &c,
                );
                assert_eq!(
                    result.result.is_ok(),
                    mode == "empty",
                    "{mode}: {:?}",
                    result.result
                );
                assert!(result.workspace.unwrap().is_dir());
                if mode == "empty" {
                    assert_eq!(result.text.as_deref(), Some(""));
                }
                assert_eq!(fs::read(f.repository.join(NAME)).unwrap(), original);
                assert_eq!(fs::read(f.repository.join(".git/index")).unwrap(), index);
            }
        },
    );
}
#[test]
fn diff_snapshots_handle_rename_index_worktree_and_missing_sides() {
    isolated(
        "ROCKET_EXTERNAL_CASE",
        "diff_snapshots_handle_rename_index_worktree_and_missing_sides",
        || {
            let f = Fixture::new();
            let exe = helper(&f);
            let c = CancellationToken::new();
            let log = DiagnosticLog::open(f.root.join("logs")).unwrap();
            f.write("old.rs", "base\n");
            f.commit("base");
            fs::create_dir_all(f.repository.join("src/検索")).unwrap();
            f.run(&["mv", "old.rs", NAME]);
            f.write(NAME, "index\n");
            f.run(&["add", "--", NAME]);
            f.write(NAME, "working\n");
            let r = f.repo();
            let config = config(&exe, ExternalToolKind::Diff, "diff");
            for (target, left, right) in [
                (DiffTarget::Staged, b"".as_slice(), b"index\n".as_slice()),
                (DiffTarget::Unstaged, b"index\n", b"working\n"),
            ] {
                let report = r.run_external_tool(
                    &config,
                    ExternalToolRequest::Diff {
                        options: DiffOptions {
                            target,
                            ..Default::default()
                        },
                        path: Some(path()),
                        label: None,
                        untracked: false, comparison: None,
                    },
                    &f.root.join("copies"),
                    &log,
                    &c,
                );
                report.result.unwrap();
                let dir = report.workspace.unwrap();
                assert_eq!(fs::read(dir.join("observed-left")).unwrap(), left);
                assert_eq!(fs::read(dir.join("observed-right")).unwrap(), right);
            }
            f.write("new.txt", "untracked\n");
            let report = r.run_external_tool(
                &config,
                ExternalToolRequest::Diff {
                    options: DiffOptions::default(),
                    path: Some(RepoPath::new(b"new.txt").unwrap()),
                    label: None,
                    untracked: true, comparison: None,
                },
                &f.root.join("copies"),
                &log,
                &c,
            );
            report.result.unwrap();
            assert!(
                fs::read(report.workspace.unwrap().join("observed-left"))
                    .unwrap()
                    .is_empty()
            );
            assert_eq!(fs::read(f.repository.join(NAME)).unwrap(), b"working\n");
            let comparison = r.compare_local("HEAD", false, &c).unwrap();
            let selected = comparison.files.iter().position(|f| f.path == path()).unwrap();
            let options = comparison.file_options(selected).unwrap();
            f.write(NAME, "external edit after comparison\n");
            let stale = r.run_external_tool(&config, ExternalToolRequest::Diff {
                options, path: Some(path()), label: None, untracked: false, comparison: Some(std::sync::Arc::new(comparison)),
            }, &f.root.join("copies"), &log, &c);
            assert!(matches!(stale.result, Err(Error::InvalidInput(_))));
            assert!(stale.workspace.is_none(), "A stale comparison must not launch the tool");
        },
    );
}
#[test]
fn cancellation_stops_owned_process_and_retains_merge_workspace() {
    isolated(
        "ROCKET_EXTERNAL_CASE",
        "cancellation_stops_owned_process_and_retains_merge_workspace",
        || {
            let f = conflict();
            let exe = helper(&f);
            let r = f.repo();
            let c = CancellationToken::new();
            let log = DiagnosticLog::open(f.root.join("logs")).unwrap();
            let p = Arc::new(r.edit_conflict(&path(), &c).unwrap());
            let original = fs::read(f.repository.join(NAME)).unwrap();
            let storage = f.root.join("copies");
            let child_storage = storage.clone();
            let config = config(&exe, ExternalToolKind::Merge, "wait");
            let worker = WriteTask::spawn(move |cancel| {
                Ok(r.run_external_tool(
                    &config,
                    ExternalToolRequest::Merge {
                        draft: p.working().into(),
                        preview: p,
                    },
                    &child_storage,
                    &log,
                    &cancel,
                ))
            })
            .unwrap();
            let deadline = Instant::now() + Duration::from_secs(10);
            loop {
                if storage.exists()
                    && fs::read_dir(&storage)
                        .unwrap()
                        .any(|e| e.unwrap().path().join("owned-processes").exists())
                {
                    break;
                }
                assert!(Instant::now() < deadline);
                std::thread::sleep(Duration::from_millis(20));
            }
            let start = Instant::now();
            worker.cancel();
            let report = worker.wait().unwrap();
            assert!(matches!(report.result, Err(Error::Cancelled)));
            assert!(start.elapsed() < Duration::from_secs(3));
            assert!(report.text.is_none());
            assert!(report.workspace.unwrap().is_dir());
            assert_eq!(fs::read(f.repository.join(NAME)).unwrap(), original);
        },
    );
}

#[test]
fn renamed_binary_and_deleted_versions_are_exact_and_precancel_never_launches() {
    isolated(
        "ROCKET_EXTERNAL_CASE",
        "renamed_binary_and_deleted_versions_are_exact_and_precancel_never_launches",
        || {
            let f = Fixture::new();
            let exe = helper(&f);
            let cancel = CancellationToken::new();
            let bytes = [0u8, 255, 5, 10, 70];
            f.write("old.dat", bytes);
            f.commit("binary base");
            fs::create_dir_all(f.repository.join("資料")).unwrap();
            f.run(&["mv", "old.dat", "資料/変更.dat"]);
            f.commit("rename");
            let r = f.repo();
            let path = RepoPath::new("資料/変更.dat".as_bytes()).unwrap();
            let options = DiffOptions {
                target: DiffTarget::Between {
                    base: "HEAD^".into(),
                    target: "HEAD".into(),
                },
                ..Default::default()
            };
            let identities = r.file_comparisons(&options, &cancel).unwrap();
            assert_eq!(identities.len(), 1);
            assert_eq!(identities[0].path(), &path);
            assert!(r.image_comparisons(&options, &cancel).unwrap().is_empty());
            let log = DiagnosticLog::open(f.root.join("logs")).unwrap();
            let config = config(&exe, ExternalToolKind::Diff, "diff");
            let result = r.run_external_tool(
                &config,
                ExternalToolRequest::Diff {
                    options,
                    path: Some(path.clone()),
                    label: None,
                    untracked: false, comparison: None,
                },
                &f.root.join("copies"),
                &log,
                &cancel,
            );
            result.result.unwrap();
            let dir = result.workspace.unwrap();
            assert_eq!(fs::read(dir.join("observed-left")).unwrap(), bytes);
            assert_eq!(fs::read(dir.join("observed-right")).unwrap(), bytes);
            fs::remove_file(f.repository.join("資料/変更.dat")).unwrap();
            let result = r.run_external_tool(
                &config,
                ExternalToolRequest::Diff {
                    options: DiffOptions::default(),
                    path: Some(path.clone()),
                    label: None,
                    untracked: false, comparison: None,
                },
                &f.root.join("copies"),
                &log,
                &cancel,
            );
            result.result.unwrap();
            let dir = result.workspace.unwrap();
            assert_eq!(fs::read(dir.join("observed-left")).unwrap(), bytes);
            assert!(fs::read(dir.join("observed-right")).unwrap().is_empty());
            cancel.cancel();
            let editor_config = ExternalToolConfig {
                executable: exe.display().to_string(),
                arguments: "editor\n{file}".into(),
            };
            let cancelled = r.run_external_tool(
                &editor_config,
                ExternalToolRequest::Editor(path),
                &f.root.join("never-created"),
                &log,
                &cancel,
            );
            assert!(matches!(cancelled.result, Err(Error::Cancelled)));
            assert!(cancelled.workspace.is_none());
            assert!(!f.root.join("never-created").exists());
        },
    );
}
