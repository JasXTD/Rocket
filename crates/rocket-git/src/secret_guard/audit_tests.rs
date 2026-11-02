//! Opt-in read-only audit using the exact production preflight. Never starts a
//! commit or push, creates Git objects, changes config, or accesses a remote.
use super::*;
use crate::{DiagnosticLog, GitClient};
use std::{path::PathBuf, sync::atomic::Ordering, time::Instant};

#[test]
#[ignore = "explicit repository and external diagnostic directory required"]
fn audit_existing_repository_without_writing() {
    let repository =
        PathBuf::from(std::env::var_os("ROCKET_SECRET_AUDIT_REPO").expect("repository path"))
            .canonicalize()
            .unwrap();
    let directory = PathBuf::from(
        std::env::var_os("ROCKET_SECRET_AUDIT_LOG_DIR").expect("diagnostic directory"),
    )
    .canonicalize()
    .unwrap();
    assert!(
        !directory.starts_with(&repository),
        "diagnostics must be outside the audited repository"
    );
    let log = DiagnosticLog::open(directory).unwrap();
    let client = GitClient::discover_with_log(log.clone()).unwrap();
    assert!(client.runner.secret_protection.load(Ordering::Acquire));
    let cancel = CancellationToken::new();
    let head = client
        .runner
        .run(
            &repository,
            &args(&["--no-replace-objects", "rev-parse", "--verify", "HEAD"]),
            vec![],
            &cancel,
        )
        .unwrap()
        .success("audit-head")
        .unwrap();
    let head = objects::oid(head.trim_ascii()).expect("valid HEAD object");
    let started = Instant::now();
    let staged = log.step("secret-protection", "scan_before_write", || {
        check_commit(
            &client.runner,
            &repository,
            &args(&["commit"]),
            b"Read-only safety audit",
            &cancel,
        )
    });
    eprintln!(
        "staged_scan_ms={}; result={staged:?}",
        started.elapsed().as_millis()
    );
    let started = Instant::now();
    let history = log.step("secret-protection", "scan_before_write", || {
        // These arguments are parsed by the preflight only. No Git push is run.
        check_push(
            &client.runner,
            &repository,
            &args(&[
                "push",
                "--",
                "read-only-audit",
                &format!("{head}:refs/heads/audit"),
            ]),
            &cancel,
        )
    });
    eprintln!(
        "head_history_scan_ms={}; result={history:?}",
        started.elapsed().as_millis()
    );
    assert_eq!(log.health().dropped_events, 0);
    staged.expect("staged snapshot safety scan");
    history.expect("current HEAD history safety scan");
}
