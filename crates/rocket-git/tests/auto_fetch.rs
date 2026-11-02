//! Automatic fetch: every remote with the manual fetch's checks, no pruning, no
//! merge, and never an askpass program or other prompt.
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

const CASE: &str = "ROCKET_AUTO_FETCH_CASE";

#[test]
fn an_automatic_fetch_brings_the_chosen_tags_and_never_replaces_one() {
    isolated(CASE, "an_automatic_fetch_brings_the_chosen_tags_and_never_replaces_one", || {
        let source = Fixture::new();
        source.write("a.txt", "1\n");
        source.commit("base");
        source.run(&["tag", "release"]);
        let f = Fixture::new();
        f.write("a.txt", "1\n");
        f.commit("base");
        f.run(&["remote", "add", "origin", source.repository.to_str().unwrap()]);
        let repo = f.repo();
        let c = token();
        assert!(repo.fetch_all_quietly(FetchTags::None, &c).unwrap().results[0].1.is_ok());
        assert!(repo.tags(100, &c).unwrap().entries.is_empty());
        assert!(repo.fetch_all_quietly(FetchTags::All, &c).unwrap().results[0].1.is_ok());
        assert_eq!(repo.tags(100, &c).unwrap().entries.len(), 1);
        // A tag that moved on the remote fails the fetch and stays as it is here.
        let original = f.run(&["rev-parse", "refs/tags/release"]);
        source.write("a.txt", "2\n");
        source.commit("next");
        source.run(&["tag", "-f", "release"]);
        assert!(repo.fetch_all_quietly(FetchTags::All, &c).unwrap().results[0].1.is_err());
        assert_eq!(f.run(&["rev-parse", "refs/tags/release"]), original);
    });
}

fn token() -> CancellationToken {
    CancellationToken::new()
}
fn quote(path: &Path) -> String {
    format!(
        "'{}'",
        path.to_str()
            .unwrap()
            .replace('\\', "/")
            .replace('\'', "'\\''")
    )
}
fn script(path: &Path, body: &str) {
    fs::write(path, format!("#!/bin/sh\n{body}\n")).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
    }
}
fn text(bytes: Vec<u8>) -> String {
    String::from_utf8(bytes).unwrap().trim().to_owned()
}

#[test]
fn quiet_fetch_updates_every_remote_without_pruning_or_merging() {
    isolated(
        CASE,
        "quiet_fetch_updates_every_remote_without_pruning_or_merging",
        || {
            let f = Fixture::new();
            f.write("a.txt", "1\n");
            f.commit("base");
            let origin = f.root.join("PRIVATE origin.git");
            let backup = f.root.join("PRIVATE backup.git");
            for bare in [&origin, &backup] {
                f.run(&["init", "--quiet", "--bare", bare.to_str().unwrap()]);
            }
            f.run(&["remote", "add", "origin", origin.to_str().unwrap()]);
            f.run(&["remote", "add", "backup", backup.to_str().unwrap()]);
            f.run(&["push", "--quiet", "origin", "main", "main:gone"]);
            f.run(&["push", "--quiet", "backup", "main"]);
            f.run(&["fetch", "--quiet", "origin"]);
            // Elsewhere: origin gains a commit and loses a branch.
            let other = f.root.join("other");
            f.run(&[
                "clone",
                "--quiet",
                "--branch",
                "main",
                origin.to_str().unwrap(),
                other.to_str().unwrap(),
            ]);
            let other_git = |args: &[&str]| {
                let mut all = vec!["-C", other.to_str().unwrap()];
                all.extend_from_slice(args);
                f.run(&all)
            };
            other_git(&["config", "user.name", "Other"]);
            other_git(&["config", "user.email", "other@example.invalid"]);
            fs::write(other.join("b.txt"), "2\n").unwrap();
            other_git(&["add", "b.txt"]);
            other_git(&["commit", "--quiet", "-m", "remote work"]);
            other_git(&["push", "--quiet", "origin", "main", ":gone"]);
            let advanced = text(other_git(&["rev-parse", "HEAD"]));
            // Even a configured prune must not delete the stale tracking branch.
            f.run(&["config", "fetch.prune", "true"]);
            let head = text(f.run(&["rev-parse", "HEAD"]));
            let repo = f.repo();
            let report = repo.fetch_all_quietly(FetchTags::None, &token()).unwrap();
            assert!(report.skipped.is_empty());
            let names: Vec<&[u8]> = report.results.iter().map(|(n, _)| n.as_slice()).collect();
            assert_eq!(names, [b"backup".as_slice(), b"origin"]);
            for (name, result) in &report.results {
                assert!(result.is_ok(), "{name:?}: {result:?}");
            }
            assert!(report.changed());
            assert_eq!(
                text(f.run(&["rev-parse", "refs/remotes/origin/main"])),
                advanced
            );
            assert!(
                !f.run(&[
                    "rev-parse",
                    "--verify",
                    "--quiet",
                    "refs/remotes/origin/gone"
                ])
                .is_empty()
            );
            assert_eq!(text(f.run(&["rev-parse", "HEAD"])), head);
            // Nothing new: no changes reported.
            assert!(!repo.fetch_all_quietly(FetchTags::None, &token()).unwrap().changed());
            // Cancelled before it starts: nothing is attempted.
            let cancel = token();
            cancel.cancel();
            assert!(matches!(
                repo.fetch_all_quietly(FetchTags::None, &cancel),
                Err(Error::Cancelled)
            ));
        },
    );
}

#[test]
fn quiet_fetch_never_runs_an_askpass_program() {
    if std::env::var(CASE).is_ok() {
        return;
    }
    // A child process with an inherited GIT_ASKPASS, as when launched from an
    // editor's terminal, and a repository-configured core.askPass.
    let f = Fixture::new();
    let inherited = f.root.join("inherited-askpass");
    let marker = f.root.join("asked");
    script(
        &inherited,
        &format!("printf inherited >> {}\nexit 1", quote(&marker)),
    );
    let mut command = Command::new(std::env::current_exe().unwrap());
    command.args(["--exact", "askpass_fixture", "--ignored", "--nocapture"]);
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
#[ignore = "run by quiet_fetch_never_runs_an_askpass_program"]
fn askpass_fixture() {
    let root = std::path::PathBuf::from(std::env::var("ROCKET_ASKPASS_ROOT").unwrap());
    let marker = root.join("asked");
    let f = Fixture::new();
    f.write("a.txt", "1\n");
    f.commit("base");
    let configured = f.root.join("configured-askpass");
    script(
        &configured,
        &format!("printf configured >> {}\nexit 1", quote(&marker)),
    );
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
        f.run(&[
            "remote",
            "add",
            "origin",
            &format!("http://{address}/PRIVATE/repo.git"),
        ]);
        let repo = f.repo();
        // A manual fetch may ask: the inherited askpass runs.
        let manual = repo.fetch("origin", None, &token()).unwrap();
        assert!(manual.result.is_err());
        assert!(fs::read_to_string(&marker).unwrap().contains("inherited"));
        fs::remove_file(&marker).unwrap();
        // An automatic fetch never asks: neither askpass runs, and it fails.
        let report = repo.fetch_all_quietly(FetchTags::None, &token()).unwrap();
        assert!(matches!(report.results.as_slice(), [(_, Err(_))]));
        assert!(!marker.exists(), "an askpass program ran");
    });
}
