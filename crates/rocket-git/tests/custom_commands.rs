//! Custom commands: the confirmed program and arguments run without a shell in
//! the repository folder; output is bounded, exit codes are reported and
//! cancellation stops what the command started.
#[allow(dead_code)]
mod common;
use common::Fixture;
use rocket_git::*;
use std::{
    ffi::OsString,
    path::Path,
    thread,
    time::{Duration, Instant},
};

fn token() -> CancellationToken {
    CancellationToken::new()
}
fn args(values: &[&str]) -> Vec<OsString> {
    values.iter().map(OsString::from).collect()
}

#[test]
fn commands_run_in_the_repository_without_a_shell_and_report_their_exit() {
    let f = Fixture::new();
    f.write("a.txt", "1\n");
    f.commit("base");
    let head = String::from_utf8(f.run(&["rev-parse", "HEAD"])).unwrap();
    let repo = f.repo();
    let git = f.git.executable();
    let storage = f.root.join("commands");
    let report = repo
        .run_custom_command(git, &args(&["rev-parse", "HEAD"]), &storage, &token())
        .unwrap();
    assert_eq!(report.exit.unwrap(), Some(0));
    assert_eq!(String::from_utf8(report.stdout).unwrap(), head);
    assert!(report.status.is_ok() && !report.truncated);
    // Shell syntax is passed literally as one argument.
    let report = repo
        .run_custom_command(
            git,
            &args(&["rev-parse", "--verify", "HEAD && echo PRIVATE"]),
            &storage,
            &token(),
        )
        .unwrap();
    assert_eq!(report.exit.unwrap(), Some(128));
    assert!(String::from_utf8_lossy(&report.stderr).contains("fatal"));
    assert!(report.stdout.is_empty());
    // Output beyond the limit is not kept.
    f.write("big.txt", "x".repeat(CUSTOM_OUTPUT_LIMIT * 2));
    f.commit("big");
    let report = repo
        .run_custom_command(
            git,
            &args(&["cat-file", "-p", "HEAD:big.txt"]),
            &storage,
            &token(),
        )
        .unwrap();
    assert_eq!(report.stdout.len(), CUSTOM_OUTPUT_LIMIT);
    assert!(report.truncated);
    // The private output files are removed.
    assert_eq!(std::fs::read_dir(&storage).unwrap().count(), 0);
    // Only absolute, existing programs run.
    for program in [Path::new("git.exe"), &f.root.join("missing.exe")] {
        assert!(
            repo.run_custom_command(program, &[], &storage, &token())
                .is_err()
        );
    }
}

#[cfg(windows)]
#[test]
fn cancelling_stops_the_command_and_a_program_it_left_running_does_not_block() {
    let f = Fixture::new();
    f.write("a.txt", "1\n");
    f.commit("base");
    let repo = f.repo();
    let storage = f.root.join("commands");
    let system = std::path::PathBuf::from(std::env::var_os("SystemRoot").unwrap()).join("System32");
    let ping = system.join("PING.EXE");
    let cancel = token();
    let started = Instant::now();
    thread::scope(|scope| {
        let canceller = cancel.clone();
        scope.spawn(move || {
            thread::sleep(Duration::from_millis(500));
            canceller.cancel();
        });
        let report = repo
            .run_custom_command(&ping, &args(&["-n", "30", "127.0.0.1"]), &storage, &cancel)
            .unwrap();
        assert!(matches!(report.exit, Err(Error::Cancelled)));
    });
    assert!(started.elapsed() < Duration::from_secs(10));
    // cmd.exe starts ping in the background and exits at once; Rocket does not wait
    // for the program that keeps running.
    let started = Instant::now();
    let report = repo
        .run_custom_command(
            &system.join("cmd.exe"),
            &args(&[
                "/c",
                "start",
                "",
                "/b",
                ping.to_str().unwrap(),
                "-n",
                "6",
                "127.0.0.1",
            ]),
            &storage,
            &token(),
        )
        .unwrap();
    assert_eq!(report.exit.unwrap(), Some(0));
    assert!(
        started.elapsed() < Duration::from_secs(4),
        "{:?}",
        started.elapsed()
    );
}
