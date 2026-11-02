mod common;
use common::Fixture;
use rocket_git::*;
use std::{
    fs,
    path::Path,
    thread,
    time::{Duration, Instant},
};

fn token() -> CancellationToken {
    CancellationToken::new()
}

#[test]
fn terminal_and_review_requests_keep_fixed_names_and_redact_unknown_inputs() {
    let fixture = Fixture::new();
    let log = log(&fixture);
    for name in [
        "terminal_start",
        "terminal_ready",
        "terminal_input_queued",
        "terminal_interrupt",
        "terminal_clear_scrollback",
        "terminal_stop",
        "terminal_exit",
        "file-preview",
        "conflict-preview",
        "mark-resolved",
        "stage-selection",
        "unstage-selection",
    ] {
        log.request(name, |_| Ok(())).unwrap();
        assert!(contents(&log).contains(&format!("\"operation\":\"{name}\"")));
    }
    log.request("PRIVATE_COMMAND_CONTENT", |_| Ok(())).unwrap();
    let text = contents(&log);
    assert!(!text.contains("PRIVATE_COMMAND_CONTENT"));
    assert!(text.contains("\"operation\":\"other\""));
}
fn log(fixture: &Fixture) -> DiagnosticLog {
    DiagnosticLog::open(fixture.root.join("diagnostics")).unwrap()
}
fn contents(log: &DiagnosticLog) -> String {
    fs::read_to_string(log.directory().join("rocket.jsonl")).unwrap()
}
fn repo(fixture: &Fixture, log: &DiagnosticLog, limits: CommandLimits) -> Repository {
    GitClient::new_with_log(fixture.git.executable(), limits, log.clone())
        .unwrap()
        .open(&fixture.repository, &token())
        .unwrap()
}
fn hook(fixture: &Fixture, name: &str, script: &str) {
    let path = fixture.root.join("no-hooks").join(name);
    fs::write(&path, script).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
    }
}
fn wait_file(path: &Path) {
    let deadline = Instant::now() + CommandLimits::default().timeout;
    while !path.is_file() {
        assert!(Instant::now() < deadline, "fixture hook did not start");
        thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn real_git_requests_record_outcomes_without_repository_data() {
    let fixture = Fixture::new();
    fixture.write("PRIVATE-filename", "PRIVATE-file-content\n");
    fixture.run(&["symbolic-ref", "HEAD", "refs/heads/PRIVATE-branch"]);
    let log = log(&fixture);
    log.request("commit", |context| {
        let repo = repo(&fixture, context, CommandLimits::default());
        assert!(matches!(repo.head(&token())?, Head::Unborn { .. }));
        let stage = repo.stage_all(&token())?;
        stage.result?;
        stage.status?;
        let commit = repo.commit("PRIVATE-commit-\"message\"\nsecond line", &token())?;
        commit.result?;
        commit.status?;
        repo.history(HistoryStart::Head, 5, &token())?;
        Ok(())
    })
    .unwrap();
    let rejected = log.request("stage", |context| {
        repo(&fixture, context, CommandLimits::default()).stage(&[], &token())
    });
    assert!(matches!(rejected, Err(Error::InvalidInput(_))));
    let failed = log.request("commit", |context| {
        repo(&fixture, context, CommandLimits::default())
            .commit("PRIVATE-empty-commit", &token())?
            .result
    });
    assert!(matches!(failed, Err(Error::Git { code: Some(1), .. })));
    let text = contents(&log);
    assert!(!text.contains("PRIVATE"));
    assert!(!text.contains(&fixture.repository.to_string_lossy().to_string()));
    assert!(!text.contains("Rocket Test"));
    assert!(!text.contains("rocket@example.invalid"));
    for expected in [
        "\"event\":\"write_queue\"",
        "\"event\":\"write_result\"",
        "\"event\":\"refresh\"",
        "\"outcome\":\"git_error\"",
        "\"outcome\":\"invalid_input\"",
        "\"stdin_bytes\":",
        "\"stdout_bytes\":",
    ] {
        assert!(text.contains(expected), "missing {expected}");
    }
    // Absent-ref probes retain their exit code without claiming request failure.
    assert!(
        text.lines()
            .any(|line| line.contains("\"operation\":\"show-ref\"")
                && line.contains("\"outcome\":\"exited\"")
                && line.contains("\"exit_code\":2"))
    );
    let requests: Vec<_> = text
        .lines()
        .filter(|line| line.contains("\"event\":\"request_end\""))
        .collect();
    assert_eq!(requests.len(), 3);
    assert!(requests[0].contains("\"outcome\":\"ok\""));
    assert!(requests[1].contains("\"outcome\":\"invalid_input\""));
    assert!(requests[2].contains("\"outcome\":\"git_error\""));
    assert_eq!(log.health().dropped_events, 0);
}

#[test]
fn process_phase_timings_are_numeric_bounded_and_private() {
    let fixture = Fixture::new();
    fixture.write("PRIVATE-name", "PRIVATE-content\n");
    let log = log(&fixture);
    repo(&fixture, &log, CommandLimits::default())
        .status(&token())
        .unwrap();
    let text = contents(&log);
    let ends: Vec<_> = text
        .lines()
        .filter(|line| line.contains("\"event\":\"git_end\""))
        .collect();
    assert!(!ends.is_empty());
    let number = |line: &str, field: &str| -> u64 {
        let needle = format!("\"{field}\":");
        let value = line.split_once(&needle).unwrap().1;
        value
            .split(|c: char| !c.is_ascii_digit())
            .next()
            .unwrap()
            .parse()
            .unwrap()
    };
    for line in ends {
        let sum: u64 = [
            "prepare_us",
            "spawn_us",
            "contain_us",
            "execute_us",
            "drain_us",
        ]
        .into_iter()
        .map(|field| number(line, field))
        .sum();
        assert!(
            sum <= number(line, "elapsed_ms") * 1000 + 999,
            "overlapping phases: {line}"
        );
        assert!(!line.contains("cancel_to_stop_us"));
        assert!(!line.contains("verify_us"), "system Git has no managed-runtime recheck");
    }
    assert!(!text.contains("PRIVATE"));
    assert!(!text.contains(&fixture.repository.to_string_lossy().to_string()));
    assert_eq!(log.health().dropped_events, 0);
}

#[test]
fn broken_log_destination_does_not_turn_successful_writes_into_errors() {
    let fixture = Fixture::new();
    fixture.write("file", "committed bytes\n");
    let log = log(&fixture);
    let repo = repo(&fixture, &log, CommandLimits::default());
    let blocker = log.directory().join("rocket.1.jsonl");
    fs::create_dir(&blocker).unwrap();
    repo.stage_all(&token()).unwrap().result.unwrap();
    let report = repo.commit("saved despite log failure", &token()).unwrap();
    assert!(report.result.is_ok());
    assert!(report.status.unwrap().entries.is_empty());
    assert_eq!(fixture.run(&["show", "HEAD:file"]), b"committed bytes\n");
    assert_eq!(
        fixture
            .repo()
            .history(HistoryStart::Head, 5, &token())
            .unwrap()
            .commits
            .len(),
        1
    );
    assert!(log.health().dropped_events > 0);
    assert_eq!(
        log.health().last_error_kind,
        Some(std::io::ErrorKind::InvalidInput)
    );
    fs::remove_dir(&blocker).unwrap();
    repo.status(&token()).unwrap();
    assert!(
        contents(&log)
            .lines()
            .last()
            .unwrap()
            .contains("\"outcome\":\"exited\"")
    );
}

#[test]
fn cancelled_commit_logs_reconciliation_of_the_commit_that_already_happened() {
    let fixture = Fixture::new();
    fixture.write("file", "base\n");
    fixture.commit("base");
    let before = fixture.run(&["rev-parse", "HEAD"]);
    fixture.write("file", "changed\n");
    fixture.run(&["add", "--", "file"]);
    hook(
        &fixture,
        "post-commit",
        "#!/bin/sh\nprintf ready > .rocket-log-ready\nsleep 30\n",
    );
    let log = log(&fixture);
    let repo = repo(&fixture, &log, CommandLimits::default());
    let task = WriteTask::spawn(move |cancel| repo.commit("PRIVATE cancellation message", &cancel))
        .unwrap();
    wait_file(&fixture.repository.join(".rocket-log-ready"));
    task.cancel();
    let report = task.wait().unwrap();
    assert!(matches!(report.result, Err(Error::Cancelled)));
    assert!(report.status.is_ok());
    assert_ne!(fixture.run(&["rev-parse", "HEAD"]), before);
    assert_eq!(fixture.run(&["rev-list", "--count", "HEAD"]), b"2\n");
    let text = contents(&log);
    let command = text
        .lines()
        .find(|line| line.contains("\"event\":\"write_result\""))
        .unwrap();
    assert!(command.contains("\"outcome\":\"cancelled\""));
    let refresh = text.lines().last().unwrap();
    assert!(refresh.contains("\"event\":\"refresh\"") && refresh.contains("\"outcome\":\"ok\""));
    assert_eq!(
        text.lines()
            .filter(|line| line.contains("\"event\":\"git_start\"")
                && line.contains("\"operation\":\"commit\""))
            .count(),
        1
    );
    assert!(!text.contains("PRIVATE"));
    assert_eq!(log.health().dropped_events, 0);
}

#[test]
fn successful_commit_and_failed_refresh_are_distinct_log_events() {
    let fixture = Fixture::new();
    fixture.write("file", "base\n");
    fixture.commit("base");
    fixture.write("file", "changed\n");
    fixture.run(&["add", "--", "file"]);
    hook(
        &fixture,
        "post-commit",
        "#!/bin/sh\nn=0\nwhile test \"$n\" -lt 80; do\n  printf data > \"untracked-$n\"\n  n=$((n + 1))\ndone\n",
    );
    let log = log(&fixture);
    let repo = repo(
        &fixture,
        &log,
        CommandLimits {
            stdout_bytes: 512,
            ..Default::default()
        },
    );
    let report = repo.commit("write succeeded", &token()).unwrap();
    assert!(report.result.is_ok());
    assert!(matches!(report.status, Err(Error::OutputLimit { .. })));
    assert_eq!(fixture.run(&["show", "HEAD:file"]), b"changed\n");
    let text = contents(&log);
    assert!(
        text.lines()
            .any(|line| line.contains("\"event\":\"write_result\"")
                && line.contains("\"outcome\":\"ok\""))
    );
    assert!(
        text.lines()
            .any(|line| line.contains("\"event\":\"refresh\"")
                && line.contains("\"outcome\":\"output_limit\"")
                && line.contains("\"stream\":\"stdout\""))
    );
    assert_eq!(log.health().dropped_events, 0);
}

#[test]
fn timed_out_hook_is_reported_and_reconciled_without_retry() {
    let fixture = Fixture::new();
    fixture.write("file", "base\n");
    fixture.commit("base");
    let before = fixture.run(&["rev-parse", "HEAD"]);
    fixture.write("file", "changed\n");
    fixture.run(&["add", "--", "file"]);
    hook(&fixture, "pre-commit", "#!/bin/sh\nsleep 30\n");
    let log = log(&fixture);
    let repo = repo(
        &fixture,
        &log,
        CommandLimits {
            timeout: Duration::from_secs(2),
            ..Default::default()
        },
    );
    let report = repo.commit("must time out", &token()).unwrap();
    assert!(matches!(report.result, Err(Error::TimedOut)));
    assert!(report.status.is_ok());
    assert_eq!(fixture.run(&["rev-parse", "HEAD"]), before);
    let text = contents(&log);
    assert!(
        text.lines()
            .any(|line| line.contains("\"event\":\"git_end\"")
                && line.contains("\"outcome\":\"timed_out\""))
    );
    assert!(
        text.lines()
            .any(|line| line.contains("\"event\":\"write_result\"")
                && line.contains("\"outcome\":\"timed_out\""))
    );
    assert_eq!(log.health().dropped_events, 0);
}
