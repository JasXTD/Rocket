#![cfg(any(windows, target_os = "linux", target_os = "macos"))]
mod common;
use common::{Fixture, isolated};
use rocket_git::*;
use std::{
    fs,
    path::{Path, PathBuf},
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

fn run_case(name: &str, run: fn()) {
    isolated("ROCKET_CLONE_CASE", name, run);
}
fn clone_to(f: &Fixture, destination: &Path) -> CloneReport {
    f.git
        .clone_repository(
            &CloneSource::Local(f.repository.clone()),
            destination,
            None,
            &CancellationToken::new(),
        )
        .unwrap()
}
fn success(report: &CloneReport) {
    assert!(report.command.is_ok(), "{:?}", report.command);
    assert!(
        matches!(report.verification, Some(Ok(()))),
        "{:?}",
        report.verification
    );
    assert!(
        matches!(report.publication, Some(Ok(()))),
        "{:?}",
        report.publication
    );
    assert!(
        matches!(report.repository, Some(Ok(_))),
        "{:?}",
        report.repository
    );
    assert!(
        matches!(report.cleanup, Some(Ok(()))),
        "{:?}",
        report.cleanup
    );
    assert!(!report.staging_directory.exists());
}
fn workspace_count(f: &Fixture) -> usize {
    fs::read_dir(&f.root)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.file_name()
                .to_string_lossy()
                .starts_with(".rocket-clone-")
        })
        .count()
}
fn wait_for(path: &Path) {
    let deadline = Instant::now() + Duration::from_secs(15);
    while !path.exists() {
        assert!(Instant::now() < deadline, "marker did not appear: {path:?}");
        thread::sleep(Duration::from_millis(20));
    }
}
fn quote(path: &Path) -> String {
    format!(
        "'{}'",
        path.to_string_lossy()
            .replace('\\', "/")
            .replace('\'', "'\\''")
    )
}
// The outer isolated subprocess owns this empty configuration. Never change a
// user's global file or process-wide environment from concurrently running tests.
fn global(f: &Fixture, key: &str, value: &str) {
    let config = std::env::var_os("GIT_CONFIG_GLOBAL").unwrap();
    let output = f
        .command(
            &f.root,
            &["config", "--file", config.to_str().unwrap(), key, value],
        )
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
}
fn filtered_fixture() -> (Fixture, PathBuf, PathBuf) {
    let f = Fixture::new();
    f.write(".gitattributes", "*.dat filter=rocket-clone-test\n");
    f.write("file.dat", "payload\n");
    f.commit("ROCKET_PRIVATE_COMMIT");
    let marker = f.root.join("filter-ready");
    let release = f.root.join("filter-release");
    let script = f.root.join("filter.sh");
    fs::write(
        &script,
        format!(
            "#!/bin/sh\nprintf ready > {}\nwhile ! test -f {}; do sleep 0.05; done\ncat\n",
            quote(&marker),
            quote(&release)
        ),
    )
    .unwrap();
    global(
        &f,
        "filter.rocket-clone-test.smudge",
        &format!("sh {}", quote(&script)),
    );
    global(&f, "filter.rocket-clone-test.clean", "cat");
    global(&f, "filter.rocket-clone-test.required", "true");
    (f, marker, release)
}
fn task(f: &Fixture, destination: &Path) -> WriteTask<CloneReport> {
    let git = f.git.clone();
    let source = CloneSource::Local(f.repository.clone());
    let destination = destination.to_owned();
    WriteTask::spawn(move |cancel| git.clone_repository(&source, destination, None, &cancel))
        .unwrap()
}

#[test]
fn local_clone_is_independent_and_emits_progress() {
    run_case("local_clone_is_independent_and_emits_progress", || {
        let f = Fixture::new();
        f.write("hello 文.txt", "original\n");
        f.commit("ROCKET_PRIVATE_COMMIT");
        f.run(&["tag", "fixture-tag"]);
        f.run(&["branch", "feature"]);
        f.write("hello 文.txt", "local unstaged edit\n");
        let before = f.run(&["status", "--porcelain=v2", "-z"]);
        let original_head = f.repo().head(&CancellationToken::new()).unwrap();
        let destination = f.root.join("ROCKET_PRIVATE_DEST with spaces 文");
        let (sender, receiver) = mpsc::sync_channel(512);
        let report = f
            .git
            .clone_repository(
                &CloneSource::Local(f.repository.clone()),
                &destination,
                Some(&sender),
                &CancellationToken::new(),
            )
            .unwrap();
        success(&report);
        assert_eq!(
            fs::read(destination.join("hello 文.txt")).unwrap(),
            b"original\n"
        );
        assert_eq!(f.run(&["status", "--porcelain=v2", "-z"]), before);
        assert_eq!(
            f.repo().head(&CancellationToken::new()).unwrap(),
            original_head
        );
        assert_eq!(
            f.run_at(&destination, &["rev-parse", "HEAD"]),
            f.run(&["rev-parse", "HEAD"])
        );
        assert_eq!(
            f.run_at(&destination, &["rev-parse", "@{u}"]),
            f.run(&["rev-parse", "HEAD"])
        );
        assert!(
            f.run_at(&destination, &["show-ref", "refs/remotes/origin/feature"])
                .len()
                > 40
        );
        assert!(
            f.run_at(&destination, &["show-ref", "refs/tags/fixture-tag"])
                .len()
                > 40
        );
        assert!(!destination.join(".git/objects/info/alternates").exists());
        // --no-local transfers into an independent pack rather than linking loose objects.
        assert!(
            fs::read_dir(destination.join(".git/objects/pack"))
                .unwrap()
                .any(|e| e.unwrap().path().extension().is_some_and(|e| e == "pack"))
        );
        let events: Vec<_> = receiver.try_iter().collect();
        assert!(events.contains(&CloneProgress::Phase(ClonePhase::Opening)));
        assert!(
            events
                .iter()
                .any(|e| matches!(e, CloneProgress::Transfer(_))),
            "{events:?}"
        );
        assert!(
            report
                .repository
                .as_ref()
                .unwrap()
                .as_ref()
                .unwrap()
                .status(&CancellationToken::new())
                .unwrap()
                .entries
                .is_empty()
        );
    });
}

#[test]
fn empty_bare_and_sha256_clones_are_valid() {
    run_case("empty_bare_and_sha256_clones_are_valid", || {
        let empty = Fixture::new();
        let report = clone_to(&empty, &empty.root.join("empty-clone"));
        success(&report);
        assert!(matches!(
            report
                .repository
                .unwrap()
                .unwrap()
                .head(&CancellationToken::new())
                .unwrap(),
            Head::Unborn { .. }
        ));
        let f = Fixture::with_format("sha256");
        f.write("file", "content\n");
        f.commit("sha256");
        let bare = f.root.join("bare.git");
        f.run_at(
            &f.root,
            &[
                "clone",
                "--bare",
                "--no-local",
                f.repository.to_str().unwrap(),
                bare.to_str().unwrap(),
            ],
        );
        let report = f
            .git
            .clone_repository(
                &CloneSource::Local(bare),
                f.root.join("sha256-clone"),
                None,
                &CancellationToken::new(),
            )
            .unwrap();
        success(&report);
        assert_eq!(
            report.repository.unwrap().unwrap().info().object_format,
            ObjectFormat::Sha256
        );
    });
}

#[cfg(windows)]
#[test]
fn preflight_preserves_existing_files_directories_and_junctions() {
    run_case(
        "preflight_preserves_existing_files_directories_and_junctions",
        || {
            let f = Fixture::new();
            let file = f.root.join("file");
            fs::write(&file, "KEEP").unwrap();
            let empty = f.root.join("empty");
            fs::create_dir(&empty).unwrap();
            let nonempty = f.root.join("nonempty");
            fs::create_dir(&nonempty).unwrap();
            fs::write(nonempty.join("keep"), "KEEP").unwrap();
            let junction = f.root.join("junction");
            let root = f
                .root
                .canonicalize()
                .unwrap()
                .to_string_lossy()
                .trim_start_matches("\\\\?\\")
                .to_owned();
            let output = std::process::Command::new("cmd")
                .args(["/d", "/c", "mklink", "/J"])
                .arg(Path::new(&root).join("junction"))
                .arg(Path::new(&root).join("nonempty"))
                .output()
                .unwrap();
            assert!(output.status.success(), "{output:?}");
            for path in [&file, &empty, &nonempty, &junction] {
                let error = f
                    .git
                    .clone_repository(
                        &CloneSource::Local(f.repository.clone()),
                        path,
                        None,
                        &CancellationToken::new(),
                    )
                    .unwrap_err();
                assert_eq!(error.clone_failure(), Some(CloneFailure::DestinationExists));
            }
            assert_eq!(fs::read(&file).unwrap(), b"KEEP");
            assert_eq!(fs::read(nonempty.join("keep")).unwrap(), b"KEEP");
            assert_eq!(fs::read_dir(empty).unwrap().count(), 0);
            // A dangling junction is still an existing filesystem entry.
            let dangling = f.root.join("dangling");
            let output = std::process::Command::new("cmd")
                .args(["/d", "/c", "mklink", "/J"])
                .arg(Path::new(&root).join("dangling"))
                .arg(Path::new(&root).join("missing"))
                .output()
                .unwrap();
            assert!(output.status.success(), "{output:?}");
            let error = f
                .git
                .clone_repository(
                    &CloneSource::Local(f.repository.clone()),
                    dangling,
                    None,
                    &CancellationToken::new(),
                )
                .unwrap_err();
            assert_eq!(error.clone_failure(), Some(CloneFailure::DestinationExists));
            assert_eq!(workspace_count(&f), 0);
        },
    );
}

/// Linux: symbolic links, to a folder or dangling, are existing entries as junctions are.
#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn preflight_preserves_existing_files_directories_and_links() {
    run_case(
        "preflight_preserves_existing_files_directories_and_links",
        || {
            let f = Fixture::new();
            let file = f.root.join("file");
            fs::write(&file, "KEEP").unwrap();
            let empty = f.root.join("empty");
            fs::create_dir(&empty).unwrap();
            let nonempty = f.root.join("nonempty");
            fs::create_dir(&nonempty).unwrap();
            fs::write(nonempty.join("keep"), "KEEP").unwrap();
            let link = f.root.join("link");
            std::os::unix::fs::symlink(&nonempty, &link).unwrap();
            let dangling = f.root.join("dangling");
            std::os::unix::fs::symlink(f.root.join("missing"), &dangling).unwrap();
            for path in [&file, &empty, &nonempty, &link, &dangling] {
                let error = f
                    .git
                    .clone_repository(
                        &CloneSource::Local(f.repository.clone()),
                        path,
                        None,
                        &CancellationToken::new(),
                    )
                    .unwrap_err();
                assert_eq!(
                    error.clone_failure(),
                    Some(CloneFailure::DestinationExists),
                    "{}",
                    path.display()
                );
            }
            assert_eq!(fs::read(&file).unwrap(), b"KEEP");
            assert_eq!(fs::read(nonempty.join("keep")).unwrap(), b"KEEP");
            assert_eq!(fs::read_dir(empty).unwrap().count(), 0);
            assert!(fs::symlink_metadata(&dangling).unwrap().file_type().is_symlink());
            assert_eq!(workspace_count(&f), 0);
        },
    );
}

#[test]
fn invalid_sources_destinations_and_precancellation_do_not_allocate() {
    run_case(
        "invalid_sources_destinations_and_precancellation_do_not_allocate",
        || {
            let f = Fixture::new();
            for leaf in [
                "..",
                ".",
                "NUL",
                "COM1.txt",
                "bad:stream",
                "trailing.",
                "trailing ",
                "missing/child",
            ] {
                let result = f.git.clone_repository(
                    &CloneSource::Local(f.repository.clone()),
                    f.root.join(leaf),
                    None,
                    &CancellationToken::new(),
                );
                assert!(result.is_err(), "{leaf}");
            }
            for url in [
                "ext::evil",
                "file:///somewhere",
                "https://user:ROCKET_SECRET@host/repo",
                "ssh://user:ROCKET_SECRET@host/repo",
                "https://host/repo?ROCKET_SECRET",
                "git@host:repo",
                "https://host/",
            ] {
                let error = f
                    .git
                    .clone_repository(
                        &CloneSource::Url(url.into()),
                        f.root.join("new"),
                        None,
                        &CancellationToken::new(),
                    )
                    .unwrap_err();
                assert_eq!(error.clone_failure(), Some(CloneFailure::UnsupportedSource));
                assert!(!error.to_string().contains("ROCKET_SECRET"));
            }
            let error = f
                .git
                .clone_repository(
                    &CloneSource::Local(f.root.join("absent")),
                    f.root.join("new"),
                    None,
                    &CancellationToken::new(),
                )
                .unwrap_err();
            assert_eq!(error.clone_failure(), Some(CloneFailure::SourceUnavailable));
            let cancel = CancellationToken::new();
            cancel.cancel();
            assert!(matches!(
                f.git.clone_repository(
                    &CloneSource::Local(f.repository.clone()),
                    f.root.join("new"),
                    None,
                    &cancel
                ),
                Err(Error::Cancelled)
            ));
            assert_eq!(workspace_count(&f), 0);
        },
    );
}

#[test]
fn command_failure_retains_workspace_and_hides_payloads() {
    run_case(
        "command_failure_retains_workspace_and_hides_payloads",
        || {
            let f = Fixture::new();
            let source = f.root.join("ROCKET_PRIVATE_not_git");
            fs::create_dir(&source).unwrap();
            fs::write(source.join("keep"), "KEEP").unwrap();
            let report = f
                .git
                .clone_repository(
                    &CloneSource::Local(source.clone()),
                    f.root.join("new"),
                    None,
                    &CancellationToken::new(),
                )
                .unwrap();
            let error = report.command.as_ref().unwrap_err();
            assert_eq!(
                error.clone_failure(),
                Some(CloneFailure::Transport(FetchFailure::RepositoryUnavailable))
            );
            assert!(!error.to_string().contains("ROCKET_PRIVATE"));
            assert!(report.staging_directory.is_dir());
            assert!(!report.destination.exists());
            assert!(report.verification.is_none());
            assert!(report.publication.is_none());
            assert!(report.cleanup.is_none());
            assert_eq!(fs::read(source.join("keep")).unwrap(), b"KEEP");
        },
    );
}

#[test]
fn missing_default_branch_is_retained_without_publication() {
    run_case(
        "missing_default_branch_is_retained_without_publication",
        || {
            let f = Fixture::new();
            f.write("file", "content");
            f.commit("commit");
            f.run(&["symbolic-ref", "HEAD", "refs/heads/absent"]);
            let report = clone_to(&f, &f.root.join("new"));
            assert!(report.command.is_ok(), "{:?}", report.command);
            assert_eq!(
                report.verification.unwrap().unwrap_err().clone_failure(),
                Some(CloneFailure::MissingDefaultBranch)
            );
            assert!(report.publication.is_none());
            assert!(!report.destination.exists());
            assert!(report.staging_directory.join("repository/.git").is_dir());
        },
    );
}

#[test]
fn cancellation_stops_real_checkout_and_retains_partial_workspace() {
    run_case(
        "cancellation_stops_real_checkout_and_retains_partial_workspace",
        || {
            let (f, marker, _release) = filtered_fixture();
            let destination = f.root.join("new");
            let task = task(&f, &destination);
            wait_for(&marker);
            let started = Instant::now();
            task.cancel();
            let report = task.wait().unwrap();
            assert!(
                matches!(report.command, Err(Error::Cancelled)),
                "{:?}",
                report.command
            );
            assert!(started.elapsed() < Duration::from_secs(5));
            assert!(!destination.exists());
            assert!(report.staging_directory.is_dir());
            assert!(report.publication.is_none());
            assert!(report.cleanup.is_none());
            assert_eq!(
                fs::read(f.repository.join("file.dat")).unwrap(),
                b"payload\n"
            );
        },
    );
}

#[test]
fn cancellation_after_git_success_prevents_publication() {
    run_case(
        "cancellation_after_git_success_prevents_publication",
        || {
            let f = Fixture::new();
            f.write("file", "content");
            f.commit("commit");
            let (sender, receiver) = mpsc::sync_channel(512);
            let cancel = CancellationToken::new();
            let worker_cancel = cancel.clone();
            let source = CloneSource::Local(f.repository.clone());
            let destination = f.root.join("new");
            let git = f.git.clone();
            let worker = thread::spawn(move || {
                git.clone_repository(&source, destination, Some(&sender), &worker_cancel)
            });
            loop {
                let event = receiver.recv_timeout(Duration::from_secs(15)).unwrap();
                if event == CloneProgress::Phase(ClonePhase::Verifying) {
                    cancel.cancel();
                    break;
                }
            }
            let report = worker.join().unwrap().unwrap();
            assert!(report.command.is_ok());
            assert!(
                matches!(report.verification, Some(Err(Error::Cancelled))),
                "{:?}",
                report.verification
            );
            assert!(report.publication.is_none());
            assert!(report.staging_directory.join("repository/file").exists());
            assert!(!report.destination.exists());
        },
    );
}

#[test]
fn cancellation_after_publication_preserves_success_and_reports_reopen_failure() {
    run_case(
        "cancellation_after_publication_preserves_success_and_reports_reopen_failure",
        || {
            let f = Fixture::new();
            f.write("file", "content");
            f.commit("commit");
            let (sender, receiver) = mpsc::sync_channel(512);
            let cancel = CancellationToken::new();
            let worker_cancel = cancel.clone();
            let source = CloneSource::Local(f.repository.clone());
            let destination = f.root.join("new");
            let git = f.git.clone();
            let worker = thread::spawn(move || {
                git.clone_repository(&source, destination, Some(&sender), &worker_cancel)
            });
            loop {
                if receiver.recv_timeout(Duration::from_secs(15)).unwrap()
                    == CloneProgress::Phase(ClonePhase::Opening)
                {
                    cancel.cancel();
                    break;
                }
            }
            let report = worker.join().unwrap().unwrap();
            assert!(report.command.is_ok());
            assert!(matches!(report.verification, Some(Ok(()))));
            assert!(matches!(report.publication, Some(Ok(()))));
            assert!(matches!(report.repository, Some(Err(Error::Cancelled))));
            assert!(matches!(report.cleanup, Some(Ok(()))));
            assert_eq!(
                fs::read(report.destination.join("file")).unwrap(),
                b"content"
            );
        },
    );
}

#[test]
fn successful_clone_keeps_error_diagnostic_warning_visible() {
    run_case(
        "successful_clone_keeps_error_diagnostic_warning_visible",
        || {
            let (f, _marker, _release) = filtered_fixture();
            fs::write(
                f.root.join("filter.sh"),
                "#!/bin/sh\necho 'error: ROCKET_PRIVATE_WARNING' >&2\ncat\n",
            )
            .unwrap();
            let log = DiagnosticLog::open(f.root.join("logs")).unwrap();
            let git =
                GitClient::new_with_log(f.git.executable(), CommandLimits::default(), log.clone())
                    .unwrap();
            let report = git
                .clone_repository(
                    &CloneSource::Local(f.repository.clone()),
                    f.root.join("new"),
                    None,
                    &CancellationToken::new(),
                )
                .unwrap();
            success(&report);
            assert!(report.error_diagnostics);
            assert!(
                String::from_utf8_lossy(&report.command.unwrap().stderr)
                    .contains("ROCKET_PRIVATE_WARNING")
            );
            let text = fs::read_to_string(log.directory().join("rocket.jsonl")).unwrap();
            assert!(
                text.lines()
                    .any(|l| l.contains("\"event\":\"clone_outcome\"")
                        && l.contains("\"level\":\"warn\"")
                        && l.contains("\"error_diagnostics\":true"))
            );
            assert!(!text.contains("ROCKET_PRIVATE_WARNING"));
        },
    );
}

#[test]
fn timeout_and_output_limit_do_not_publish_or_retry() {
    run_case("timeout_and_output_limit_do_not_publish_or_retry", || {
        let (f, marker, _release) = filtered_fixture();
        let git = GitClient::new(
            f.git.executable(),
            CommandLimits {
                timeout: Duration::from_secs(3),
                ..CommandLimits::default()
            },
        )
        .unwrap();
        let report = git
            .clone_repository(
                &CloneSource::Local(f.repository.clone()),
                f.root.join("timeout"),
                None,
                &CancellationToken::new(),
            )
            .unwrap();
        assert!(marker.exists());
        assert!(
            matches!(report.command, Err(Error::TimedOut)),
            "{:?}",
            report.command
        );
        assert!(!report.destination.exists());
        assert!(report.staging_directory.is_dir());
        fs::write(f.root.join("filter.sh"), "#!/bin/sh\nwhile :; do printf 'PRIVATE_OUTPUT_LIMIT_PAYLOAD_1234567890\\n' >&2; done\n").unwrap();
        let git = GitClient::new(
            f.git.executable(),
            CommandLimits {
                stderr_bytes: 4096,
                ..CommandLimits::default()
            },
        )
        .unwrap();
        let report = git
            .clone_repository(
                &CloneSource::Local(f.repository.clone()),
                f.root.join("limited"),
                None,
                &CancellationToken::new(),
            )
            .unwrap();
        assert!(
            matches!(
                report.command,
                Err(Error::OutputLimit {
                    stream: "stderr",
                    ..
                })
            ),
            "{:?}",
            report.command
        );
        assert!(!report.destination.exists());
        assert!(report.staging_directory.is_dir());
        assert_eq!(workspace_count(&f), 2);
    });
}

#[test]
fn publication_race_preserves_new_destination_and_completed_checkout() {
    run_case(
        "publication_race_preserves_new_destination_and_completed_checkout",
        || {
            let (f, marker, release) = filtered_fixture();
            for is_empty in [false, true] {
                let destination = f.root.join(if is_empty {
                    "raced-empty"
                } else {
                    "raced-data"
                });
                let task = task(&f, &destination);
                wait_for(&marker);
                fs::create_dir(&destination).unwrap();
                if !is_empty {
                    fs::write(destination.join("keep"), "OUTSIDER").unwrap();
                }
                fs::write(&release, "go").unwrap();
                let report = task.wait().unwrap();
                assert!(report.command.is_ok(), "{:?}", report.command);
                assert!(
                    matches!(report.verification, Some(Ok(()))),
                    "{:?}",
                    report.verification
                );
                assert_eq!(
                    report.publication.unwrap().unwrap_err().clone_failure(),
                    Some(CloneFailure::DestinationExists)
                );
                assert!(
                    report
                        .staging_directory
                        .join("repository/file.dat")
                        .exists()
                );
                assert!(!destination.join(".git").exists());
                if !is_empty {
                    assert_eq!(fs::read(destination.join("keep")).unwrap(), b"OUTSIDER");
                } else {
                    assert_eq!(fs::read_dir(&destination).unwrap().count(), 0);
                }
                fs::remove_file(&release).unwrap();
                fs::remove_file(&marker).unwrap();
            }
        },
    );
}

#[test]
fn concurrent_clones_publish_once_and_keep_losing_workspace() {
    run_case(
        "concurrent_clones_publish_once_and_keep_losing_workspace",
        || {
            let (f, marker, release) = filtered_fixture();
            let destination = f.root.join("shared-target");
            let first = task(&f, &destination);
            wait_for(&marker);
            let second = task(&f, &destination);
            let deadline = Instant::now() + Duration::from_secs(15);
            while workspace_count(&f) < 2 {
                assert!(Instant::now() < deadline);
                thread::sleep(Duration::from_millis(20));
            }
            fs::write(&release, "go").unwrap();
            let reports = [first.wait().unwrap(), second.wait().unwrap()];
            assert_eq!(
                reports
                    .iter()
                    .filter(|r| matches!(r.publication, Some(Ok(()))))
                    .count(),
                1
            );
            let loser = reports
                .iter()
                .find(|r| matches!(r.publication, Some(Err(_))))
                .unwrap();
            assert_eq!(
                loser
                    .publication
                    .as_ref()
                    .unwrap()
                    .as_ref()
                    .unwrap_err()
                    .clone_failure(),
                Some(CloneFailure::DestinationExists)
            );
            assert!(loser.staging_directory.join("repository/file.dat").exists());
            assert_eq!(
                fs::read(destination.join("file.dat")).unwrap(),
                b"payload\n"
            );
        },
    );
}

#[test]
fn cleanup_never_deletes_added_files_and_publication_stays_successful() {
    run_case(
        "cleanup_never_deletes_added_files_and_publication_stays_successful",
        || {
            let (f, marker, release) = filtered_fixture();
            let task = task(&f, &f.root.join("new"));
            wait_for(&marker);
            let stage = fs::read_dir(&f.root)
                .unwrap()
                .map(|e| e.unwrap().path())
                .find(|p| {
                    p.file_name()
                        .unwrap()
                        .to_string_lossy()
                        .starts_with(".rocket-clone-")
                })
                .unwrap();
            fs::write(stage.join("do-not-delete"), "KEEP").unwrap();
            fs::write(&release, "go").unwrap();
            let report = task.wait().unwrap();
            assert!(matches!(report.publication, Some(Ok(()))));
            assert!(matches!(report.repository, Some(Ok(_))));
            assert!(matches!(report.cleanup, Some(Err(_))));
            assert_eq!(fs::read(stage.join("do-not-delete")).unwrap(), b"KEEP");
            assert_eq!(
                fs::read(report.destination.join("file.dat")).unwrap(),
                b"payload\n"
            );
        },
    );
}

#[test]
fn configured_templates_hooks_and_worktree_do_not_redirect_clone() {
    run_case(
        "configured_templates_hooks_and_worktree_do_not_redirect_clone",
        || {
            let f = Fixture::new();
            f.write("file", "checkout\n");
            f.commit("commit");
            let outside = f.root.join("outside");
            fs::create_dir(&outside).unwrap();
            fs::write(outside.join("file"), "KEEP").unwrap();
            let template = f.root.join("hostile-template");
            fs::create_dir_all(template.join("hooks")).unwrap();
            fs::write(
                template.join("hooks/post-checkout"),
                format!(
                    "#!/bin/sh\nprintf BAD > {}\n",
                    quote(&outside.join("hook-ran"))
                ),
            )
            .unwrap();
            fs::write(
                template.join("config"),
                format!(
                    "[core]\nworktree = {}\n",
                    outside.to_string_lossy().replace('\\', "/")
                ),
            )
            .unwrap();
            global(&f, "init.templateDir", template.to_str().unwrap());
            global(
                &f,
                "core.hooksPath",
                template.join("hooks").to_str().unwrap(),
            );
            global(&f, "core.worktree", outside.to_str().unwrap());
            global(&f, "clone.defaultRemoteName", "wrong-remote");
            let report = clone_to(&f, &f.root.join("new"));
            success(&report);
            assert_eq!(fs::read(outside.join("file")).unwrap(), b"KEEP");
            assert!(!outside.join("hook-ran").exists());
            assert_eq!(
                fs::read(report.destination.join("file")).unwrap(),
                b"checkout\n"
            );
            assert!(!report.destination.join(".git/hooks/post-checkout").exists());
            assert_eq!(f.run_at(&report.destination, &["remote"]), b"origin\n");
        },
    );
}

#[test]
fn clone_diagnostics_are_complete_and_payload_free() {
    run_case("clone_diagnostics_are_complete_and_payload_free", || {
        let f = Fixture::new();
        f.write("ROCKET_PRIVATE_FILE", "ROCKET_PRIVATE_CONTENT");
        f.commit("ROCKET_PRIVATE_COMMIT");
        let log = DiagnosticLog::open(f.root.join("logs")).unwrap();
        let destination = f.root.join("ROCKET_PRIVATE_DEST");
        let report = log
            .request("clone", |context| {
                let git = GitClient::new_with_log(
                    f.git.executable(),
                    CommandLimits::default(),
                    context.clone(),
                )?;
                git.clone_repository(
                    &CloneSource::Local(f.repository.clone()),
                    &destination,
                    None,
                    &CancellationToken::new(),
                )
            })
            .unwrap();
        success(&report);
        let error = log
            .request("clone", |context| {
                GitClient::new_with_log(
                    f.git.executable(),
                    CommandLimits::default(),
                    context.clone(),
                )?
                .clone_repository(
                    &CloneSource::Local(f.repository.clone()),
                    &destination,
                    None,
                    &CancellationToken::new(),
                )
            })
            .unwrap_err();
        assert_eq!(error.clone_failure(), Some(CloneFailure::DestinationExists));
        assert_eq!(log.health().dropped_events, 0);
        let text = fs::read_to_string(log.directory().join("rocket.jsonl")).unwrap();
        assert!(!text.contains("ROCKET_PRIVATE"));
        assert!(!text.contains(f.root.to_str().unwrap()));
        for step in [
            "validate_clone",
            "allocate_workspace",
            "clone_command",
            "verify_clone",
            "publish_clone",
            "open_clone",
            "cleanup_workspace",
        ] {
            assert!(
                text.contains(&format!("\"step\":\"{step}\"")),
                "missing {step}"
            );
        }
        assert!(text.contains("\"publication\":\"succeeded\""));
        assert!(text.contains("\"failure_code\":\"clone_destination_exists\""));
        for kind in ["request", "step", "git"] {
            assert_eq!(
                text.matches(&format!("\"event\":\"{kind}_start\"")).count(),
                text.matches(&format!("\"event\":\"{kind}_end\"")).count()
            );
        }
        assert_eq!(
            text.lines()
                .filter(|l| l.contains("\"event\":\"git_start\"")
                    && l.contains("\"operation\":\"clone\""))
                .count(),
            1
        );
    });
}
