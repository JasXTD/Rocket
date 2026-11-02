mod common;
use common::Fixture;
use rocket_git::*;
use std::{
    fs,
    io::{Read, Write},
    net::TcpListener,
    path::Path,
    process::Command,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    thread,
    time::Duration,
};

fn isolated(mode: &str) {
    let f = Fixture::new();
    let mut command = Command::new(std::env::current_exe().unwrap());
    command.args([
        "--exact",
        "authentication_fixture",
        "--ignored",
        "--nocapture",
    ]);
    // Never mutate the test process environment or use a real credential helper,
    // SSH identity, proxy, or Git configuration in these failure fixtures.
    for (key, _) in std::env::vars_os() {
        let name = key.to_string_lossy().to_ascii_uppercase();
        if name.starts_with("GIT_") || name.starts_with("SSH_") || name.ends_with("_PROXY") {
            command.env_remove(key);
        }
    }
    command
        .env("ROCKET_AUTH_FIXTURE", mode)
        .env("GIT_CONFIG_GLOBAL", f.root.join("empty.config"))
        .env("GIT_CONFIG_SYSTEM", f.root.join("empty.config"))
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_ALLOW_PROTOCOL", "ssh:http:file")
        .env("GIT_TERMINAL_PROMPT", "0");
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
fn ssh_failures_preserve_state_and_trace_each_step_without_secrets() {
    isolated("ssh");
}

#[test]
fn http_challenge_and_access_errors_use_real_git_without_credentials() {
    isolated("http");
}

#[test]
fn preflight_rejection_is_traced_without_starting_transport() {
    isolated("preflight");
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

fn number(line: &str, name: &str) -> u64 {
    line.split(&format!("\"{name}\":"))
        .nth(1)
        .unwrap()
        .split([',', '}'])
        .next()
        .unwrap()
        .parse()
        .unwrap()
}

fn assert_pairs(text: &str) {
    let lines: Vec<_> = text.lines().collect();
    for (start, line) in lines
        .iter()
        .enumerate()
        .filter(|(_, line)| line.contains("\"event\":\"step_start\""))
    {
        let id = number(line, "id");
        let ends: Vec<_> = lines
            .iter()
            .enumerate()
            .filter(|(_, line)| line.contains("\"event\":\"step_end\"") && number(line, "id") == id)
            .collect();
        assert_eq!(ends.len(), 1);
        assert!(ends[0].0 > start);
        assert_eq!(number(line, "request"), number(ends[0].1, "request"));
        assert_ne!(number(line, "request"), 0);
    }
}

fn check_failure(f: &Fixture, expected: FetchFailure) {
    let log = DiagnosticLog::open(f.root.join(format!("logs-{}", expected.code()))).unwrap();
    let before = f.repo().status(&CancellationToken::new()).unwrap();
    let index = fs::read(f.repository.join(".git/index")).unwrap();
    let refs = f.run(&["show-ref"]);
    let error = log
        .request("fetch", |context| {
            let repo = GitClient::new_with_log(
                f.git.executable(),
                CommandLimits::default(),
                context.clone(),
            )?
            .open(&f.repository, &CancellationToken::new())?;
            let report = repo.fetch("origin", None, &CancellationToken::new())?;
            assert!(report.branches.is_ok());
            let after = report.status.unwrap().unwrap();
            assert_eq!(after.head, before.head);
            assert_eq!(after.entries, before.entries);
            report.result
        })
        .unwrap_err();
    assert_eq!(
        error.fetch_failure(),
        Some(expected),
        "unexpected diagnostic: {error:?}"
    );
    assert!(error.to_string().contains(expected.code()));
    assert!(!error.to_string().contains("PRIVATE"));
    assert_eq!(fs::read(f.repository.join(".git/index")).unwrap(), index);
    assert_eq!(f.run(&["show-ref"]), refs);
    assert_eq!(fs::read(f.repository.join("file")).unwrap(), b"unstaged\n");
    assert_eq!(
        fs::read(f.repository.join("untracked")).unwrap(),
        b"PRIVATE contents\n"
    );
    let text = fs::read_to_string(log.directory().join("rocket.jsonl")).unwrap();
    assert_pairs(&text);
    for private in [
        "PRIVATE",
        "auth.invalid",
        "127.0.0.1",
        "Authorization",
        "Bearer",
    ] {
        assert!(!text.contains(private), "leaked {private}");
    }
    assert!(!text.contains(f.root.to_str().unwrap()));
    assert!(
        !text.contains(
            std::str::from_utf8(&f.run(&["rev-parse", "HEAD"]))
                .unwrap()
                .trim()
        )
    );
    for step in [
        "verify_git",
        "repository_discovery",
        "validate_selection",
        "write_queue",
        "validate_remote_and_refspecs",
        "write_result",
        "refresh",
    ] {
        assert!(
            text.contains(&format!("\"step\":\"{step}\"")),
            "missing {step}"
        );
    }
    for event in ["write_result", "request_end"] {
        let line = text
            .lines()
            .find(|line| line.contains(&format!("\"event\":\"{event}\"")))
            .unwrap();
        assert!(line.contains("\"outcome\":\"git_error\""));
        assert!(line.contains(&format!("\"failure_code\":\"{}\"", expected.code())));
    }
    assert_eq!(
        text.lines()
            .filter(|line| line.contains("\"event\":\"git_start\"")
                && line.contains("\"operation\":\"fetch\""))
            .count(),
        1
    );
    assert_eq!(
        text.lines()
            .filter(|line| line.contains("\"event\":\"refresh\"")
                && line.contains("\"outcome\":\"ok\""))
            .count(),
        2
    );
    assert!(!text.contains("\"step\":\"parse\""));
    assert_eq!(log.health().dropped_events, 0);
}

#[test]
#[ignore = "isolated subprocess entry point exercised by authentication tests"]
fn authentication_fixture() {
    let f = Fixture::new();
    f.write("file", "base\n");
    f.commit("base");
    f.write("file", "staged\n");
    f.run(&["add", "file"]);
    f.write("file", "unstaged\n");
    f.write("untracked", "PRIVATE contents\n");
    match std::env::var("ROCKET_AUTH_FIXTURE").unwrap().as_str() {
        "ssh" => {
            let helper = f.root.join("ssh-fixture");
            let diagnostic = f.root.join("diagnostic");
            let calls = f.root.join("calls");
            script(
                &helper,
                &format!(
                    "printf 'call\\n' >> {}\ncat {} >&2\nexit 1",
                    quote(&calls),
                    quote(&diagnostic)
                ),
            );
            f.run(&["config", "core.sshCommand", &quote(&helper)]);
            f.run(&["config", "ssh.variant", "ssh"]);
            f.run(&[
                "remote",
                "add",
                "origin",
                "git@auth.invalid:PRIVATE/repo.git",
            ]);
            use FetchFailure::*;
            let cases = [
                ("Host key verification failed.", SshHostVerificationFailed),
                (
                    "WARNING: REMOTE HOST IDENTIFICATION HAS CHANGED!\nHost key verification failed.",
                    SshHostKeyChanged,
                ),
                (
                    "git@auth.invalid: Permission denied (publickey).",
                    SshAuthenticationRejected,
                ),
                (
                    "Warning: Identity file PRIVATE not accessible: No such file or directory.",
                    SshKeyUnavailable,
                ),
                (
                    "Load key \"PRIVATE\": incorrect passphrase supplied to decrypt private key",
                    SshKeyUnlockFailed,
                ),
                (
                    "sign_and_send_pubkey: signing failed: agent refused operation",
                    SshAgentFailure,
                ),
                (
                    "ssh: Could not resolve hostname PRIVATE: Name or service not known",
                    NameResolutionFailed,
                ),
                (
                    "ssh: connect to host PRIVATE port 22: Connection refused",
                    ConnectionFailed,
                ),
                (
                    "ssh: connect to host PRIVATE port 22: Connection timed out",
                    ConnectionTimedOut,
                ),
                (
                    "fetch-pack: unexpected disconnect while reading sideband packet",
                    ConnectionInterrupted,
                ),
                ("unknown helper failure PRIVATE", Other),
            ];
            for (message, expected) in cases {
                fs::write(&diagnostic, format!("{message}\nPRIVATE token: github_pat_PRIVATE\nAuthorization: Bearer PRIVATE\n")).unwrap();
                check_failure(&f, expected);
            }
            assert_eq!(
                fs::read_to_string(calls).unwrap().lines().count(),
                cases.len()
            );
        }
        "http" => {
            // A real loopback HTTP server challenges Git without contacting GitHub
            // or handling any real credentials. No TLS/SSO behavior is simulated
            // as proof of a live provider flow.
            f.run(&["config", "credential.helper", ""]);
            let askpass = f.root.join("askpass-fixture");
            script(&askpass, "exit 1");
            f.run(&["config", "core.askPass", askpass.to_str().unwrap()]);
            f.run(&["config", "http.proxy", ""]);
            for (status, expected) in [
                (401, FetchFailure::CredentialsUnavailable),
                (403, FetchFailure::AccessDenied),
                (404, FetchFailure::RepositoryUnavailable),
                (407, FetchFailure::ProxyAuthenticationRequired),
            ] {
                let listener = TcpListener::bind("127.0.0.1:0").unwrap();
                let address = listener.local_addr().unwrap();
                listener.set_nonblocking(true).unwrap();
                let done = Arc::new(AtomicBool::new(false));
                let count = Arc::new(AtomicUsize::new(0));
                thread::scope(|scope| {
                    let done = done.clone();
                    let count = count.clone();
                    // Drop also stops the listener if an assertion unwinds.
                    struct Stop(Arc<AtomicBool>);
                    impl Drop for Stop {
                        fn drop(&mut self) {
                            self.0.store(true, Ordering::Release);
                        }
                    }
                    let _stop = Stop(done.clone());
                    scope.spawn(move || {
                        while !done.load(Ordering::Acquire) {
                            match listener.accept() {
                                Ok((mut stream, _)) => {
                                    stream.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
                                    stream.set_write_timeout(Some(Duration::from_secs(2))).unwrap();
                                    let mut request = Vec::new();
                                    let mut byte = [0];
                                    while !request.ends_with(b"\r\n\r\n") && request.len() < 16 * 1024 {
                                        if stream.read(&mut byte).unwrap_or(0) == 0 { break; }
                                        request.push(byte[0]);
                                    }
                                    assert!(!String::from_utf8_lossy(&request).to_ascii_lowercase().contains("authorization:"));
                                    count.fetch_add(1, Ordering::Relaxed);
                                    let response = format!("HTTP/1.1 {status} Fixture\r\nWWW-Authenticate: Basic realm=\"PRIVATE\"\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
                                    let _ = stream.write_all(response.as_bytes());
                                }
                                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => thread::sleep(Duration::from_millis(5)),
                                Err(error) => panic!("{error}"),
                            }
                        }
                    });
                    f.run(&[
                        "config",
                        "remote.origin.url",
                        &format!("http://{address}/PRIVATE/repo.git"),
                    ]);
                    f.run(&[
                        "config",
                        "remote.origin.fetch",
                        "+refs/heads/*:refs/remotes/origin/*",
                    ]);
                    check_failure(&f, expected);
                });
                assert!(count.load(Ordering::Relaxed) > 0);
            }
        }
        "preflight" => {
            let log = DiagnosticLog::open(f.root.join("logs")).unwrap();
            let error = log
                .request("fetch", |context| {
                    GitClient::new_with_log(
                        f.git.executable(),
                        CommandLimits::default(),
                        context.clone(),
                    )?
                    .open(&f.repository, &CancellationToken::new())?
                    .fetch("PRIVATE-missing", None, &CancellationToken::new())
                })
                .unwrap_err();
            assert!(matches!(error, Error::InvalidInput(_)));
            let text = fs::read_to_string(log.directory().join("rocket.jsonl")).unwrap();
            assert_pairs(&text);
            assert!(text.lines().any(|line| {
                line.contains("\"step\":\"validate_remote_and_refspecs\"")
                    && line.contains("\"outcome\":\"invalid_input\"")
            }));
            assert!(!text.contains("\"failure_code\":"));
            assert!(!text.contains("PRIVATE"));
            assert!(
                !text
                    .lines()
                    .any(|line| line.contains("\"event\":\"git_start\"")
                        && line.contains("\"operation\":\"fetch\""))
            );
        }
        _ => panic!("unknown authentication fixture"),
    }
}
