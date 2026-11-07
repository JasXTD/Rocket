use super::*;
use std::fs;

fn isolated(name: &str, body: fn()) {
    if std::env::var("ROCKET_SCOPE_POLICY_CASE").as_deref() == Ok(name) {
        body();
        return;
    }
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../work/runner-policy")
        .join(format!(
            "{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
    fs::create_dir_all(&root).unwrap();
    let empty = root.join("empty.config");
    fs::write(&empty, b"").unwrap();
    let mut child = Command::new(std::env::current_exe().unwrap());
    child.args(["--exact", name, "--nocapture", "--test-threads=1"]);
    for (key, _) in std::env::vars_os() {
        let text = key.to_string_lossy().to_ascii_uppercase();
        if text.starts_with("GIT_")
            || text.starts_with("SSH_")
            || crate::transport_scope::network_environment(&key)
        {
            child.env_remove(key);
        }
    }
    child
        .env("ROCKET_SCOPE_POLICY_CASE", name)
        .env("ROCKET_SCOPE_POLICY_ROOT", &root)
        .env("GIT_CONFIG_GLOBAL", &empty)
        .env("GIT_CONFIG_SYSTEM", &empty)
        .env("GIT_CONFIG_NOSYSTEM", "1");
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        child.creation_flags(0x0800_0000);
    }
    let output = child.output().unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
fn scoped() -> (Runner, PathBuf) {
    let root = PathBuf::from(std::env::var_os("ROCKET_SCOPE_POLICY_ROOT").unwrap());
    (
        Runner {
            executable: find_git().unwrap(),
            limits: CommandLimits::default(),
            log: None,
            transport_scope: Some(crate::TransportScope::new(&root, vec![]).unwrap()),
            secret_protection: Arc::new(AtomicBool::new(true)),
            guard_ai_api_keys: Arc::new(AtomicBool::new(true)),
            push_check: Default::default(),
            git_http_allowed: true,
            managed_git: None,
        },
        root,
    )
}

#[test]
fn vulnerable_runtime_blocks_http_and_rewrites_before_connecting() {
    isolated(
        "runner::scope_tests::vulnerable_runtime_blocks_http_and_rewrites_before_connecting",
        || {
            let (mut runner, root) = scoped();
            runner.transport_scope = None;
            runner.git_http_allowed = false;
            runner.limits.timeout = Duration::from_secs(5);
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            listener.set_nonblocking(true).unwrap();
            let port = listener.local_addr().unwrap().port();
            for scheme in ["http", "https"] {
                let url = format!("{scheme}://127.0.0.1:{port}/repo");
                for rewritten in [false, true] {
                    let rewrite = format!("url.{url}.insteadOf=ssh://fixture.invalid/repo");
                    let input = args(&[
                        "-c",
                        "protocol.http.allow=always",
                        "-c",
                        "protocol.https.allow=always",
                        "-c",
                        &rewrite,
                        "ls-remote",
                        if rewritten {
                            "ssh://fixture.invalid/repo"
                        } else {
                            &url
                        },
                    ]);
                    let output = runner
                        .run(&root, &input, vec![], &CancellationToken::new())
                        .unwrap();
                    assert!(!output.status.success());
                    assert!(
                        String::from_utf8_lossy(&output.stderr)
                            .contains(&format!("transport '{scheme}' not allowed"))
                    );
                    assert!(
                        matches!(listener.accept(), Err(e) if e.kind() == std::io::ErrorKind::WouldBlock)
                    );
                }
            }
        },
    );
}

#[test]
fn vulnerable_runtime_keeps_local_git_working() {
    isolated(
        "runner::scope_tests::vulnerable_runtime_keeps_local_git_working",
        || {
            let (mut runner, root) = scoped();
            runner.transport_scope = None;
            runner.git_http_allowed = false;
            let cancel = CancellationToken::new();
            // The fixture can itself sit inside the Rocket checkout. Git's
            // remote lookup may discover that parent repository, so use the
            // owned bare repository's absolute path instead of a relative URL.
            let local = root.join("local.git").into_os_string();
            runner
                .run(
                    &root,
                    &[OsString::from("init"), OsString::from("--bare"), local.clone()],
                    vec![],
                    &cancel,
                )
                .unwrap()
                .success("init")
                .unwrap();
            runner
                .run(&root, &[OsString::from("ls-remote"), local], vec![], &cancel)
                .unwrap()
                .success("local refs")
                .unwrap();
        },
    );
}

#[test]
fn patched_runtime_keeps_http_transport_available() {
    isolated(
        "runner::scope_tests::patched_runtime_keeps_http_transport_available",
        || {
            let (mut runner, root) = scoped();
            runner.transport_scope = None;
            runner.git_http_allowed = true;
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            listener.set_nonblocking(true).unwrap();
            let url = format!("http://{}/repo", listener.local_addr().unwrap());
            // Keep the fixture alive for the actual bounded Git attempt. A
            // separate five-second listener deadline can expire during process
            // startup on a busy Windows host, before Git has tried HTTP at all.
            // Dropping the sender also stops the fixture if the caller unwinds.
            let (finished, stop) = std::sync::mpsc::channel::<()>();
            let server = std::thread::spawn(move || {
                loop {
                    match listener.accept() {
                        Ok((mut socket, _)) => {
                            socket.set_nonblocking(false).unwrap();
                            socket
                                .set_read_timeout(Some(Duration::from_secs(2)))
                                .unwrap();
                            socket
                                .set_write_timeout(Some(Duration::from_secs(2)))
                                .unwrap();
                            let mut request = [0; 4096];
                            let count = socket.read(&mut request).unwrap();
                            socket.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").unwrap();
                            return request[..count].starts_with(b"GET /repo/info/refs");
                        }
                        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                            if stop.try_recv() != Err(std::sync::mpsc::TryRecvError::Empty) {
                                return false;
                            }
                            std::thread::sleep(Duration::from_millis(5));
                        }
                        Err(e) => panic!("loopback fixture: {e}"),
                    }
                }
            });
            let output = runner.run(
                &root,
                &args(&["ls-remote", &url]),
                vec![],
                &CancellationToken::new(),
            );
            drop(finished);
            assert!(
                server.join().unwrap(),
                "patched runtime never reached its local HTTP fixture"
            );
            assert!(
                !output.unwrap().status.success(),
                "fixture deliberately returns 404"
            );
        },
    );
}

#[test]
fn vulnerable_runtime_policy_reaches_helper_children() {
    isolated(
        "runner::scope_tests::vulnerable_runtime_policy_reaches_helper_children",
        || {
            let (mut runner, root) = scoped();
            runner.transport_scope = None;
            runner.git_http_allowed = false;
            // Only this isolated subprocess changes its environment.
            unsafe {
                std::env::set_var("GIT_ALLOW_PROTOCOL", "file:https:ssh:ext");
            }
            let output = runner
                .lfs(
                    &std::env::current_exe().unwrap(),
                    &root,
                    &args(&[
                        "runner::scope_tests::http_policy_fixture",
                        "--exact",
                        "--ignored",
                        "--nocapture",
                        "--test-threads=1",
                    ]),
                    &[("protocol.https.allow".into(), "always".into())],
                    &CancellationToken::new(),
                )
                .unwrap()
                .success("policy child")
                .unwrap();
            assert!(String::from_utf8_lossy(&output).contains("1 passed"));
        },
    );
}

#[test]
#[ignore = "subprocess fixture invoked by vulnerable_runtime_policy_reaches_helper_children"]
fn http_policy_fixture() {
    assert!(std::env::var_os("ROCKET_SCOPE_POLICY_ROOT").is_some());
    assert_eq!(std::env::var("GIT_ALLOW_PROTOCOL").unwrap(), "file:ssh");
}

#[test]
#[cfg(any(windows, target_os = "linux"))]
fn system_curl_ignores_environment_and_git_adjacent_helpers() {
    isolated(
        "runner::scope_tests::system_curl_ignores_environment_and_git_adjacent_helpers",
        || {
            let (_, root) = scoped();
            fs::create_dir(root.join("System32")).unwrap();
            fs::write(root.join("System32/curl.exe"), b"inert, never execute").unwrap();
            fs::create_dir(root.join("bin")).unwrap();
            fs::write(root.join("bin/curl.exe"), b"inert, never execute").unwrap();
            // Linux's name for the same helper, beside Git and first on PATH.
            fs::write(root.join("bin/curl"), b"inert, never execute").unwrap();
            // No test thread outside this subprocess sees these overrides.
            unsafe {
                std::env::set_var("SystemRoot", &root);
                std::env::set_var("PATH", root.join("bin"));
            }
            let git = root.join(if cfg!(windows) { "bin/git.exe" } else { "bin/git" });
            let curl = crate::hosting::find_curl(&git).unwrap();
            assert!(curl.is_absolute() && curl.is_file());
            assert!(
                !curl.starts_with(&root),
                "untrusted helper selected: {}",
                curl.display()
            );
            #[cfg(target_os = "linux")]
            assert_eq!(curl, Path::new("/usr/bin/curl"));
        },
    );
}

#[test]
fn invocation_overrides_cannot_restore_hooks_or_disable_tls() {
    isolated(
        "runner::scope_tests::invocation_overrides_cannot_restore_hooks_or_disable_tls",
        || {
            let (runner, root) = scoped();
            let cancel = CancellationToken::new();
            runner
                .run(
                    &root,
                    &args(&["init", "--bare", "bound.git"]),
                    vec![],
                    &cancel,
                )
                .unwrap()
                .success("init")
                .unwrap();
            // Policy follows the caller's -c options whatever comes before them.
            let prefixes: [&[&str]; 3] =
                [&[], &["--no-replace-objects"], &["--git-dir", "bound.git"]];
            for prefix in prefixes {
                for (key, unsafe_value, expected) in [
                    ("core.hooksPath", "untrusted-hooks", "/dev/null"),
                    ("http.sslVerify", "false", "true"),
                    ("http.proxy", "http://127.0.0.1:1", ""),
                ] {
                    let mut command = args(prefix);
                    command.extend(args(&[
                        "-c",
                        &format!("{key}={unsafe_value}"),
                        "config",
                        "--get",
                        key,
                    ]));
                    let output = runner
                        .run(&root, &command, vec![], &cancel)
                        .unwrap()
                        .success("config")
                        .unwrap();
                    assert_eq!(String::from_utf8(output).unwrap().trim(), expected);
                }
            }
        },
    );
}

#[test]
fn global_options_cannot_hide_a_transport_verb() {
    isolated(
        "runner::scope_tests::global_options_cannot_hide_a_transport_verb",
        || {
            let (runner, root) = scoped();
            let cancel = CancellationToken::new();
            runner
                .run(
                    &root,
                    &args(&["init", "--bare", "bound.git"]),
                    vec![],
                    &cancel,
                )
                .unwrap()
                .success("init")
                .unwrap();
            let refused = |command: Vec<OsString>| {
                let result = runner.run(&root, &command, vec![], &cancel);
                let status = result.as_ref().map(|output| output.status);
                assert!(
                    matches!(result, Err(Error::InvalidInput(_))),
                    "{command:?}: {status:?}"
                );
            };
            let prefixes: [&[&str]; 4] = [
                &["--no-replace-objects"],
                &["--no-literal-pathspecs"],
                &["--git-dir", "bound.git", "--work-tree", "."],
                &["--no-replace-objects", "-c", "push.negotiate=false"],
            ];
            // A relative destination is out of scope, and one without -- never gets that far.
            let transports: [&[&str]; 5] = [
                &["push", "--", "outside.git", "HEAD:refs/heads/main"],
                &["push", "outside.git"],
                &["fetch", "--", "outside.git"],
                &["ls-remote", "--", "outside.git"],
                &["clone", "--", "outside.git", "copy"],
            ];
            for prefix in prefixes {
                for transport in transports {
                    let mut command = args(prefix);
                    command.extend(args(transport));
                    refused(command);
                }
            }
            // An option nobody reviewed can't pass as the verb, or hide the one behind it.
            refused(args(&[
                "--namespace",
                "fixture",
                "push",
                "--",
                "outside.git",
            ]));
            refused(args(&["-c"]));
            // Rocket identifies Git with this one, which has no verb at all.
            runner
                .run(&root, &args(&["--version"]), vec![], &cancel)
                .unwrap()
                .success("version")
                .unwrap();
        },
    );
}

#[test]
fn helper_policy_reaches_child_git_and_cleans_version_probe_environment() {
    isolated(
        "runner::scope_tests::helper_policy_reaches_child_git_and_cleans_version_probe_environment",
        || {
            let (runner, root) = scoped();
            // This isolated child alone receives inert overrides. The version probe
            // skips repository config validation but must still sanitize its process.
            for key in [
                "HTTPS_PROXY",
                "GIT_SSL_NO_VERIFY",
                "CURL_CA_BUNDLE",
                "SSL_CERT_FILE",
                "SSL_CERT_DIR",
                "SSLKEYLOGFILE",
                "OPENSSL_CONF",
            ] {
                unsafe {
                    std::env::set_var(key, "inert-fixture-setting");
                }
            }
            let config = vec![
                ("core.hooksPath".into(), "untrusted-hooks".into()),
                ("http.proxy".into(), "http://127.0.0.1:1".into()),
                ("http.sslVerify".into(), "false".into()),
            ];
            // The compiled test executable acts as the helper; it then starts real Git.
            // "version" selects the one ignored fixture and exercises the probe path.
            let output = runner
                .lfs(
                    &std::env::current_exe().unwrap(),
                    &root,
                    &args(&["version", "--ignored", "--nocapture", "--test-threads=1"]),
                    &config,
                    &CancellationToken::new(),
                )
                .unwrap()
                .success("fixture helper")
                .unwrap();
            assert!(
                String::from_utf8_lossy(&output).contains("1 passed"),
                "helper fixture did not run"
            );
        },
    );
}

#[test]
#[ignore = "subprocess fixture invoked by the helper policy test"]
fn version_probe_fixture() {
    assert!(std::env::var_os("ROCKET_SCOPE_POLICY_ROOT").is_some());
    for (key, value) in std::env::vars_os() {
        assert!(
            !crate::transport_scope::network_environment(&key) || value.is_empty(),
            "network override survived into helper"
        );
    }
    let git = find_git().unwrap();
    for (key, expected) in crate::transport_scope::PROCESS_CONFIG {
        let mut command = Command::new(&git);
        command.args(["config", "--get", key]);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x0800_0000);
        }
        let output = command.output().unwrap();
        assert!(output.status.success());
        assert_eq!(
            String::from_utf8(output.stdout).unwrap().trim(),
            *expected,
            "{key}"
        );
    }
}
