mod common;
use common::{Fixture, isolated};
use rocket_git::*;
use std::fs;

fn cancel() -> CancellationToken { CancellationToken::new() }
fn fixture() -> Fixture {
    let f = Fixture::new();
    f.write("readme.md", "synthetic budget test\n");
    f.commit("base");
    let remote = f.root.join("remote.git");
    f.run(&["init", "--bare", remote.to_str().unwrap()]);
    f.run(&["remote", "add", "origin", remote.to_str().unwrap()]);
    f
}
fn write_objects(f: &Fixture) {
    for (name, first) in [("資料/one.bin", 1), ("資料/two.bin", 2)] {
        let mut bytes = vec![0; 600 * 1024]; bytes[0] = first;
        f.write(name, bytes);
    }
    f.repo().stage_all(&cancel()).unwrap().result.unwrap();
}
fn push(repo: &Repository, token: &CancellationToken) -> Result<()> {
    repo.push("origin", "main", "main", PushOptions { set_upstream: false }, token)?.result.map(|_| ())
}

#[test]
fn configurable_budget_retries_without_losing_protection_or_local_changes() {
    isolated("ROCKET_BUDGET", "configurable_budget_retries_without_losing_protection_or_local_changes", || {
        let f = fixture();
        assert_eq!(f.git.secret_scan_mib(), 256);
        let clone = f.git.clone();
        let log = DiagnosticLog::open(f.root.join("logs")).unwrap();
        let request = f.git.for_request(CommandLimits::default(), log.clone()).unwrap();
        let repo = request.open(&f.repository, &cancel()).unwrap();
        for mib in [1, 257, 4097, MAX_SECRET_SCAN_MIB] {
            f.git.set_secret_scan_mib(mib).unwrap();
            assert_eq!(clone.secret_scan_mib(), mib);
            assert_eq!(request.secret_scan_mib(), mib);
        }
        for invalid in [0, MAX_SECRET_SCAN_MIB + 1, u64::MAX] {
            assert!(f.git.set_secret_scan_mib(invalid).is_err());
            assert_eq!(f.git.secret_scan_mib(), MAX_SECRET_SCAN_MIB);
        }
        f.git.set_secret_scan_mib(1).unwrap();
        write_objects(&f);
        let head = f.run(&["rev-parse", "HEAD"]);
        let index = fs::read(f.repository.join(".git/index")).unwrap();
        let error = repo.commit("budget refusal", &cancel()).unwrap().result.unwrap_err();
        assert!(matches!(error, Error::SecretProtection(_)));
        assert!(error.to_string().contains("configured 1 MiB total"));
        assert_eq!(f.run(&["rev-parse", "HEAD"]), head);
        assert_eq!(fs::read(f.repository.join(".git/index")).unwrap(), index);
        clone.set_secret_scan_mib(2).unwrap();
        repo.commit("budget increased", &cancel()).unwrap().result.unwrap();
        f.git.set_secret_scan_mib(1).unwrap();
        let error = push(&repo, &cancel()).unwrap_err();
        assert!(error.to_string().contains("configured 1 MiB total"));
        assert!(f.run_at(&f.root.join("remote.git"), &["for-each-ref"]).is_empty());
        // Ordinary unstaged work is preserved through the retry and refusal.
        f.write("readme.md", "keep my unstaged change\n");
        f.git.set_secret_scan_mib(3).unwrap();
        push(&repo, &cancel()).unwrap();
        assert_eq!(f.run(&["rev-parse", "HEAD"]), f.run_at(&f.root.join("remote.git"), &["rev-parse", "main"]));
        assert_eq!(fs::read_to_string(f.repository.join("readme.md")).unwrap(), "keep my unstaged change\n");
        let token = format!("ghp_{}", "A7".repeat(18));
        f.write("credential.txt", &token);
        f.run(&["add", "--", "credential.txt"]);
        let error = repo.commit("must still reject", &cancel()).unwrap().result.unwrap_err();
        let Error::SecretProtection(failure) = error else { panic!("credential was not refused") };
        assert_eq!(failure.code(), "github_token_detected");
        // Simulate pre-existing history created outside Rocket, then verify push.
        f.commit("synthetic historical credential");
        let remote_head = f.run_at(&f.root.join("remote.git"), &["rev-parse", "main"]);
        assert!(matches!(push(&repo, &cancel()), Err(Error::SecretProtection(_))));
        assert_eq!(f.run_at(&f.root.join("remote.git"), &["rev-parse", "main"]), remote_head);
        assert_eq!(log.health().dropped_events, 0);
        assert!(!fs::read_to_string(f.root.join("logs/rocket.jsonl")).unwrap().contains(&token));
    });
}

#[cfg(unix)]
#[test]
fn scan_budget_is_snapshotted_and_cancel_stops_content_reads() {
    isolated("ROCKET_BUDGET", "scan_budget_is_snapshotted_and_cancel_stops_content_reads", || {
        use std::{os::unix::fs::PermissionsExt, thread, time::{Duration, Instant}};
        let f = fixture();
        write_objects(&f);
        let marker = f.root.join("content-started");
        let release = f.root.join("release");
        let relay = f.root.join("git-relay");
        let quote = |p: &std::path::Path| format!("'{}'", p.to_str().unwrap().replace('\'', "'\\''"));
        fs::write(&relay, format!("#!/bin/sh\nfor arg in \"$@\"; do\n if [ \"$arg\" = --batch ]; then\n  : > {}\n  while [ ! -f {} ]; do sleep 0.02; done\n fi\ndone\nexec {} \"$@\"\n", quote(&marker), quote(&release), quote(f.git.executable()))).unwrap();
        fs::set_permissions(&relay, fs::Permissions::from_mode(0o700)).unwrap();
        let log = DiagnosticLog::open(f.root.join("logs")).unwrap();
        let client = GitClient::new_with_log(&relay, CommandLimits { timeout: Duration::from_secs(20), ..Default::default() }, log.clone()).unwrap();
        let repo = client.open(&f.repository, &cancel()).unwrap();
        for action in ["snapshot", "cancel_commit", "cancel_push"] {
            client.set_secret_scan_mib(2).unwrap();
            if marker.exists() { fs::remove_file(&marker).unwrap(); }
            if release.exists() { fs::remove_file(&release).unwrap(); }
            let before = f.run(&["rev-parse", "HEAD"]);
            let index = fs::read(f.repository.join(".git/index")).unwrap();
            let c = cancel();
            let observer_cancel = c.clone();
            let (marker, release, watcher_client) = (marker.clone(), release.clone(), client.clone());
            let watcher = thread::spawn(move || {
                let started = Instant::now();
                while !marker.exists() && started.elapsed() < Duration::from_secs(10) { thread::sleep(Duration::from_millis(5)); }
                let observed = marker.exists();
                let stopped = Instant::now();
                if action == "snapshot" && observed {
                    watcher_client.set_secret_scan_mib(1).unwrap();
                    fs::write(release, b"go").unwrap();
                } else { observer_cancel.cancel(); }
                (observed, stopped)
            });
            let result = if action == "cancel_push" { push(&repo, &c) }
                else { repo.commit(action, &c).and_then(|r| r.result.map(|_| ())) };
            let (observed, stopped) = watcher.join().unwrap();
            assert!(observed, "did not reach an actual content read");
            if action == "snapshot" {
                result.unwrap();
                assert_eq!(client.secret_scan_mib(), 1);
                f.write("readme.md", "next staged change\n");
                f.run(&["add", "--all"]);
            } else {
                assert!(matches!(result, Err(Error::Cancelled)), "{result:?}");
                assert!(stopped.elapsed() < Duration::from_secs(5));
                assert_eq!(f.run(&["rev-parse", "HEAD"]), before);
                assert_eq!(fs::read(f.repository.join(".git/index")).unwrap(), index);
            }
        }
        assert!(f.run_at(&f.root.join("remote.git"), &["for-each-ref"]).is_empty());
        assert_eq!(log.health().dropped_events, 0);
    });
}
