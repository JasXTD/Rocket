//! The Repository manager's per-repository read and its quiet fast-forward pull:
//! local reads only, and a pull that moves only a clean branch behind its upstream
//! and never asks for a sign-in.
#[allow(dead_code)]
mod common;
use common::{Fixture, isolated};
use rocket_git::*;
use std::{
    fs,
    io::{Read, Write},
    net::TcpListener,
    path::Path,
    process::Command,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};

const CASE: &str = "ROCKET_REPOSITORY_SUMMARY_CASE";
/// The fixture's fixed commit time, 2026-01-01T00:00:00+08:00.
const COMMIT_TIME: i64 = 1_767_196_800;

fn token() -> CancellationToken {
    CancellationToken::new()
}
fn text(bytes: Vec<u8>) -> String {
    String::from_utf8(bytes).unwrap().trim().to_owned()
}
/// `f` with `origin`, a bare repository it pushed main to, and a clone of it where
/// someone else works.
fn with_origin(f: &Fixture) -> std::path::PathBuf {
    f.write("a.txt", "1\n");
    f.commit("base");
    let origin = f.root.join("origin.git");
    f.run(&["init", "--quiet", "--bare", "--initial-branch=main", origin.to_str().unwrap()]);
    f.run(&["remote", "add", "origin", origin.to_str().unwrap()]);
    f.run(&["push", "--quiet", "-u", "origin", "main"]);
    let other = f.root.join("other");
    f.run(&["clone", "--quiet", "--branch", "main", origin.to_str().unwrap(), other.to_str().unwrap()]);
    f.run_at(&other, &["config", "user.name", "Other"]);
    f.run_at(&other, &["config", "user.email", "other@example.invalid"]);
    f.run_at(&other, &["config", "core.hooksPath", f.root.join("no-hooks").to_str().unwrap()]);
    other
}
/// Someone else pushes `n` commits to origin.
fn push_elsewhere(f: &Fixture, other: &Path, n: usize) -> String {
    for _ in 0..n {
        // Named after the commit count, so every call adds a file no earlier one made.
        let name = format!("incoming-{}.txt", text(f.run_at(other, &["rev-list", "--count", "HEAD"])));
        fs::write(other.join(&name), "incoming\n").unwrap();
        f.run_at(other, &["add", "--", &name]);
        f.run_at(other, &["commit", "--quiet", "--no-verify", "-m", "incoming"]);
    }
    f.run_at(other, &["push", "--quiet", "origin", "main"]);
    text(f.run_at(other, &["rev-parse", "HEAD"]))
}

#[test]
fn summary_reads_the_branch_upstream_changes_and_times() {
    isolated(CASE, "summary_reads_the_branch_upstream_changes_and_times", || {
        let f = Fixture::new();
        let other = with_origin(&f);
        let repo = f.repo();
        let all = SummaryReads { status: true, last_commit: true };
        let clean = repo.summary(all, &token()).unwrap();
        assert!(matches!(&clean.head, Head::Branch { name, .. } if name == b"main"));
        assert_eq!(clean.upstream.as_deref(), Some(b"origin/main".as_slice()));
        assert_eq!(clean.ahead_behind, Some((0, 0)));
        assert_eq!(clean.changes, Some(SummaryChanges::default()));
        assert!(clean.operation.is_idle());
        assert_eq!(clean.last_commit, Some(COMMIT_TIME));
        // One commit here, two on origin once fetched, a changed file and a new one.
        push_elsewhere(&f, &other, 2);
        f.write("local.txt", "here\n");
        f.commit("local");
        assert_eq!(repo.summary(all, &token()).unwrap().last_fetch, None);
        f.run(&["fetch", "--quiet", "origin"]);
        f.write("a.txt", "changed\n");
        f.write("new.txt", "new\n");
        let busy = repo.summary(all, &token()).unwrap();
        assert_eq!(busy.ahead_behind, Some((1, 2)));
        assert_eq!(busy.changes, Some(SummaryChanges { tracked: 1, new: 1 }));
        assert_eq!(busy.changes.unwrap().total(), 2);
        assert!(busy.last_fetch.is_some());
        // Without the status read: the branch and the fetch time only.
        let light = repo.summary(SummaryReads::default(), &token()).unwrap();
        assert_eq!(light.head, busy.head);
        assert_eq!((light.upstream, light.ahead_behind, light.changes, light.last_commit), (None, None, None, None));
        assert_eq!(light.last_fetch, busy.last_fetch);
        // A merge stopped before its commit is an operation in progress.
        f.run(&["stash", "--include-untracked", "--quiet"]);
        f.run(&["merge", "--quiet", "--no-commit", "--no-ff", "origin/main"]);
        assert!(!repo.summary(all, &token()).unwrap().operation.is_idle());
        f.run(&["merge", "--abort"]);
        // Detached: no branch, the commit's time still reads.
        f.run(&["switch", "--quiet", "--detach", "HEAD"]);
        let detached = repo.summary(all, &token()).unwrap();
        assert!(matches!(detached.head, Head::Detached { .. }));
        assert_eq!(detached.upstream, None);
        assert_eq!(detached.last_commit, Some(COMMIT_TIME));
        // A bare repository has no working tree to count, even when asked.
        let bare = f.git.open(f.root.join("origin.git"), &token()).unwrap();
        let summary = bare.summary(all, &token()).unwrap();
        assert!(matches!(&summary.head, Head::Branch { name, .. } if name == b"main"));
        assert_eq!(summary.changes, None);
        // Cancelled: nothing is returned.
        let cancel = token();
        cancel.cancel();
        assert!(repo.summary(all, &cancel).is_err());
    });
}

#[test]
fn quiet_pull_moves_only_a_clean_branch_behind_its_upstream() {
    isolated(CASE, "quiet_pull_moves_only_a_clean_branch_behind_its_upstream", || {
        let f = Fixture::new();
        let other = with_origin(&f);
        let repo = f.repo();
        let head = || text(f.run(&["rev-parse", "HEAD"]));
        // Behind and clean: a fast-forward, and the count of new commits.
        let before = head();
        let tip = push_elsewhere(&f, &other, 2);
        let report = repo.pull_ff_only_quietly(&token()).unwrap();
        assert!(report.fetch.is_ok(), "{:?}", report.fetch);
        let update = report.update.unwrap().unwrap();
        assert_eq!(update.kind, PullUpdateKind::FastForward);
        assert!(update.verification.is_ok());
        assert_eq!(head(), tip);
        assert_eq!(update.before.as_str(), before);
        assert_eq!(repo.commits_between(&update.before, &update.fetched, &token()).unwrap(), 2);
        // Nothing new: up to date, nothing moves.
        let update = repo.pull_ff_only_quietly(&token()).unwrap().update.unwrap().unwrap();
        assert_eq!(update.kind, PullUpdateKind::UpToDate);
        assert_eq!(head(), tip);
        // A new file Git doesn't track doesn't stop it, as with Pull, and stays.
        f.write("draft.txt", "mine\n");
        let tip = push_elsewhere(&f, &other, 1);
        let update = repo.pull_ff_only_quietly(&token()).unwrap().update.unwrap().unwrap();
        assert_eq!(update.kind, PullUpdateKind::FastForward);
        assert_eq!(head(), tip);
        assert_eq!(fs::read_to_string(f.repository.join("draft.txt")).unwrap(), "mine\n");
        // A changed tracked file: refused before any fetch, nothing touched.
        f.write("a.txt", "edited\n");
        push_elsewhere(&f, &other, 1);
        let before = head();
        assert!(matches!(
            repo.pull_ff_only_quietly(&token()),
            Err(Error::Pull(PullFailure::DirtyWorktree))
        ));
        assert_eq!(head(), before);
        assert_eq!(fs::read_to_string(f.repository.join("a.txt")).unwrap(), "edited\n");
        f.run(&["checkout", "--quiet", "--", "a.txt"]);
        // Diverged: fetched, never merged.
        f.write("local.txt", "here\n");
        f.run(&["add", "--", "local.txt"]);
        f.run(&["commit", "--quiet", "--no-verify", "-m", "local"]);
        let before = head();
        let report = repo.pull_ff_only_quietly(&token()).unwrap();
        assert!(report.fetch.is_ok());
        assert!(matches!(report.update, Some(Err(Error::Pull(PullFailure::Diverged)))));
        assert_eq!(head(), before);
        // No upstream: refused before any fetch.
        f.run(&["branch", "--unset-upstream"]);
        assert!(matches!(
            repo.pull_ff_only_quietly(&token()),
            Err(Error::Pull(PullFailure::MissingUpstream))
        ));
    });
}

fn quote(path: &Path) -> String {
    format!("'{}'", path.to_str().unwrap().replace('\\', "/").replace('\'', "'\\''"))
}
fn script(path: &Path, body: &str) {
    fs::write(path, format!("#!/bin/sh\n{body}\n")).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
    }
}

#[test]
fn quiet_pull_never_runs_an_askpass_program() {
    if std::env::var(CASE).is_ok() {
        return;
    }
    // A child process with an inherited GIT_ASKPASS, as when launched from an
    // editor's terminal, and a repository-configured core.askPass.
    let f = Fixture::new();
    let inherited = f.root.join("inherited-askpass");
    let marker = f.root.join("asked");
    script(&inherited, &format!("printf inherited >> {}\nexit 1", quote(&marker)));
    let mut command = Command::new(std::env::current_exe().unwrap());
    command.args(["--exact", "pull_askpass_fixture", "--ignored", "--nocapture"]);
    for (key, _) in std::env::vars_os() {
        let name = key.to_string_lossy().to_ascii_uppercase();
        if name.starts_with("GIT_") || name.starts_with("SSH_") || name.ends_with("_PROXY") {
            command.env_remove(key);
        }
    }
    command
        .env(CASE, "askpass")
        .env("ROCKET_ASKPASS_ROOT", &f.root)
        .env("GIT_ASKPASS", &inherited)
        .env("GIT_CONFIG_GLOBAL", f.root.join("empty.config"))
        .env("GIT_CONFIG_SYSTEM", f.root.join("empty.config"))
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_ALLOW_PROTOCOL", "http:file");
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000);
    }
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
#[ignore = "run by quiet_pull_never_runs_an_askpass_program"]
fn pull_askpass_fixture() {
    let root = std::path::PathBuf::from(std::env::var("ROCKET_ASKPASS_ROOT").unwrap());
    let marker = root.join("asked");
    let f = Fixture::new();
    f.write("a.txt", "1\n");
    f.commit("base");
    let configured = f.root.join("configured-askpass");
    script(&configured, &format!("printf configured >> {}\nexit 1", quote(&marker)));
    f.run(&["config", "core.askPass", configured.to_str().unwrap()]);
    f.run(&["config", "credential.helper", ""]);
    f.run(&["config", "http.proxy", ""]);
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    listener.set_nonblocking(true).unwrap();
    let done = Arc::new(AtomicBool::new(false));
    thread::scope(|scope| {
        struct Stop(Arc<AtomicBool>);
        impl Drop for Stop {
            fn drop(&mut self) {
                self.0.store(true, Ordering::Release);
            }
        }
        let _stop = Stop(done.clone());
        let done = done.clone();
        scope.spawn(move || {
            while !done.load(Ordering::Acquire) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        stream.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
                        let mut request = Vec::new();
                        let mut byte = [0];
                        while !request.ends_with(b"\r\n\r\n") && request.len() < 16 * 1024 {
                            if stream.read(&mut byte).unwrap_or(0) == 0 {
                                break;
                            }
                            request.push(byte[0]);
                        }
                        let _ = stream.write_all(b"HTTP/1.1 401 Fixture\r\nWWW-Authenticate: Basic realm=\"PRIVATE\"\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(5))
                    }
                    Err(e) => panic!("{e}"),
                }
            }
        });
        f.run(&["remote", "add", "origin", &format!("http://{address}/PRIVATE/repo.git")]);
        f.run(&["config", "branch.main.remote", "origin"]);
        f.run(&["config", "branch.main.merge", "refs/heads/main"]);
        let repo = f.repo();
        let head = text(f.run(&["rev-parse", "HEAD"]));
        // The ordinary Pull may ask: the inherited askpass runs.
        let manual = repo.pull_ff_only(None, &token()).unwrap();
        assert!(manual.fetch.is_err());
        assert!(fs::read_to_string(&marker).unwrap().contains("inherited"));
        fs::remove_file(&marker).unwrap();
        // Pull all's pull never asks: neither askpass runs, it fails, nothing moves.
        let quiet = repo.pull_ff_only_quietly(&token()).unwrap();
        assert!(quiet.fetch.is_err());
        assert!(quiet.update.is_none());
        assert!(!marker.exists(), "an askpass program ran");
        assert_eq!(text(f.run(&["rev-parse", "HEAD"])), head);
    });
}
