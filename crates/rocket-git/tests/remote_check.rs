#![cfg(windows)]
mod common;
use common::Fixture;
use rocket_git::*;
use std::{
    io::{Read, Write},
    net::TcpListener,
    path::PathBuf,
    process::Command,
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};
fn token() -> CancellationToken {
    CancellationToken::new()
}

/// Like `common::isolated`, but Git may also use http(s) to reach this test's
/// loopback listeners. No other network endpoint is used.
fn isolated(name: &str, test: fn()) {
    const CASE: &str = "ROCKET_REMOTE_CHECK_CASE";
    if std::env::var(CASE).as_deref() == Ok(name) {
        test();
        return;
    }
    let f = Fixture::new();
    let mut command = Command::new(std::env::current_exe().unwrap());
    command.args(["--exact", name, "--nocapture"]);
    for (key, _) in std::env::vars_os() {
        let upper = key.to_string_lossy().to_ascii_uppercase();
        if upper.starts_with("GIT_")
            || upper.starts_with("SSH_")
            || upper.starts_with("GCM_")
            || upper.ends_with("_PROXY")
        {
            command.env_remove(key);
        }
    }
    command
        .env(CASE, name)
        .env("GIT_CONFIG_GLOBAL", f.root.join("empty.config"))
        .env("GIT_CONFIG_SYSTEM", f.root.join("empty.config"))
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_ALLOW_PROTOCOL", "file:http:https")
        .env("NO_PROXY", "127.0.0.1")
        .env("GIT_TEMPLATE_DIR", f.root.join("no-hooks"))
        .env("GIT_TERMINAL_PROMPT", "0");
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

/// A loopback server that accepts connections and answers each request with
/// `reply`, or never answers when `reply` is None. Returns its port.
fn server(reply: Option<&'static [u8]>) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    thread::spawn(move || {
        let mut held = Vec::new();
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { return };
            match reply {
                Some(reply) => {
                    let mut request = Vec::new();
                    let mut buffer = [0; 1024];
                    while !request.windows(4).any(|w| w == b"\r\n\r\n") {
                        match stream.read(&mut buffer) {
                            Ok(0) | Err(_) => break,
                            Ok(n) => request.extend_from_slice(&buffer[..n]),
                        }
                    }
                    let _ = stream.write_all(reply);
                }
                None => held.push(stream),
            }
        }
    });
    port
}

fn bare(f: &Fixture, name: &str) -> PathBuf {
    let path = f.root.join(name);
    f.run(&[
        "init",
        "--bare",
        "--initial-branch=main",
        path.to_str().unwrap(),
    ]);
    path
}

#[test]
fn local_remotes_report_branches_and_the_default_branch() {
    isolated(
        "local_remotes_report_branches_and_the_default_branch",
        || {
            let f = Fixture::new();
            f.write("a.txt", "a");
            f.commit("first");
            f.run(&["branch", "feature/x"]);
            let remote = bare(&f, "remote 文.git");
            f.run(&["push", remote.to_str().unwrap(), "main", "feature/x"]);
            let repo = f.repo();
            let check = repo
                .check_remote(remote.to_str().unwrap().as_bytes(), &token())
                .unwrap();
            assert_eq!(check.branches, 2);
            assert_eq!(check.default_branch.as_deref(), Some(&b"main"[..]));
            assert!(check.elapsed < REMOTE_CHECK_LIMIT);
            let empty = bare(&f, "empty.git");
            let check = repo
                .check_remote(empty.to_str().unwrap().as_bytes(), &token())
                .unwrap();
            assert_eq!(check.branches, 0);
            let source = CloneSource::Local(remote.clone());
            let check = f
                .git
                .check_clone_source(&source, &f.root, &token())
                .unwrap();
            assert_eq!(check.branches, 2);
            // A folder that is not a repository is a classified failure.
            let missing = f.root.join("not a repository");
            std::fs::create_dir(&missing).unwrap();
            let error = repo
                .check_remote(missing.to_str().unwrap().as_bytes(), &token())
                .unwrap_err();
            assert!(
                matches!(
                    error,
                    Error::Git {
                        operation: "remote-check",
                        ..
                    }
                ),
                "{error:?}"
            );
            assert!(error.fetch_failure().is_some());
            // Nothing changed in the repository: no remote, no fetched refs.
            assert!(f.run(&["remote"]).is_empty());
            assert!(f.run(&["for-each-ref", "refs/remotes"]).is_empty());
        },
    );
}

#[test]
fn invalid_urls_and_clone_sources_are_refused_before_git_runs() {
    isolated(
        "invalid_urls_and_clone_sources_are_refused_before_git_runs",
        || {
            let f = Fixture::new();
            let repo = f.repo();
            for url in [
                &b""[..],
                b"--upload-pack=calc",
                b"https://x\n.example/r.git",
            ] {
                assert!(matches!(
                    repo.check_remote(url, &token()),
                    Err(Error::InvalidInput(_))
                ));
            }
            for source in [
                CloneSource::Url("http://127.0.0.1/r.git".into()),
                CloneSource::Url("https://user@example.invalid/r.git".into()),
                CloneSource::Local(f.root.join("missing")),
            ] {
                assert!(matches!(
                    f.git.check_clone_source(&source, &f.root, &token()),
                    Err(Error::Clone(_))
                ));
            }
        },
    );
}

#[test]
fn authentication_is_never_prompted_for() {
    isolated("authentication_is_never_prompted_for", || {
        let f = Fixture::new();
        let port = server(Some(
            b"HTTP/1.1 401 Unauthorized\r\nWWW-Authenticate: Basic realm=\"rocket\"\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        ));
        let started = Instant::now();
        let error = f
            .repo()
            .check_remote(
                format!("http://127.0.0.1:{port}/r.git").as_bytes(),
                &token(),
            )
            .unwrap_err();
        assert!(started.elapsed() < Duration::from_secs(15));
        assert!(
            matches!(
                error.fetch_failure(),
                Some(FetchFailure::CredentialsUnavailable | FetchFailure::AuthenticationRejected)
            ),
            "{error}"
        );
    });
}

#[test]
fn a_silent_server_is_cancelled_promptly() {
    isolated("a_silent_server_is_cancelled_promptly", || {
        let f = Fixture::new();
        let port = server(None);
        let cancel = token();
        let stop = cancel.clone();
        let (sender, receiver) = mpsc::channel();
        let repo = f.repo();
        thread::spawn(move || {
            let result =
                repo.check_remote(format!("http://127.0.0.1:{port}/r.git").as_bytes(), &cancel);
            sender.send(result).unwrap();
        });
        thread::sleep(Duration::from_millis(800));
        let cancelled = Instant::now();
        stop.cancel();
        let result = receiver.recv_timeout(Duration::from_secs(10)).unwrap();
        assert!(matches!(result, Err(Error::Cancelled)), "{result:?}");
        assert!(cancelled.elapsed() < Duration::from_secs(5));
        // The clone-source path (https through the clone allowlist) cancels too.
        let cancel = token();
        let stop = cancel.clone();
        let (sender, receiver) = mpsc::channel();
        let git = f.git.clone();
        let folder = f.root.clone();
        thread::spawn(move || {
            let source = CloneSource::Url(format!("https://127.0.0.1:{port}/r.git"));
            sender
                .send(git.check_clone_source(&source, &folder, &cancel))
                .unwrap();
        });
        thread::sleep(Duration::from_millis(800));
        stop.cancel();
        let result = receiver.recv_timeout(Duration::from_secs(10)).unwrap();
        assert!(matches!(result, Err(Error::Cancelled)), "{result:?}");
    });
}

#[test]
fn a_silent_server_hits_the_time_limit() {
    isolated("a_silent_server_hits_the_time_limit", || {
        let f = Fixture::new();
        let port = server(None);
        let started = Instant::now();
        let result = f.repo().check_remote(
            format!("http://127.0.0.1:{port}/r.git").as_bytes(),
            &token(),
        );
        assert!(matches!(result, Err(Error::TimedOut)), "{result:?}");
        let elapsed = started.elapsed();
        assert!(
            elapsed >= REMOTE_CHECK_LIMIT && elapsed < REMOTE_CHECK_LIMIT + Duration::from_secs(10),
            "{elapsed:?}"
        );
    });
}
