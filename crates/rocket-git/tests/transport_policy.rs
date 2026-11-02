#[allow(dead_code)]
mod common;
use common::{Fixture, isolated};
use rocket_git::*;
use std::{
    fs,
    io::{Read, Write},
    net::TcpListener,
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};

fn case(name: &str, test: fn()) {
    isolated("ROCKET_TRANSPORT_POLICY_CASE", name, test);
}
fn setup() -> (Fixture, Repository, std::path::PathBuf) {
    let f = Fixture::new();
    f.write("file", "fixture");
    f.commit("fixture");
    let bare = f.root.join("allowed.git");
    f.run(&["init", "--bare", bare.to_str().unwrap()]);
    f.run(&["remote", "add", "origin", bare.to_str().unwrap()]);
    let client = f
        .git
        .clone()
        .with_transport_scope(Some(TransportScope::new(&f.root, vec![]).unwrap()));
    let repo = client
        .open(&f.repository, &CancellationToken::new())
        .unwrap();
    (f, repo, bare)
}
fn hook(directory: &Path, name: &str, marker: &Path) {
    fs::create_dir_all(directory).unwrap();
    let marker = marker
        .to_string_lossy()
        .replace('\\', "/")
        .replace('\'', "'\\''");
    let path = directory.join(name);
    fs::write(
        &path,
        format!("#!/bin/sh\nprintf hook >> '{marker}'\nexit 0\n"),
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
    }
}

#[test]
fn scoped_operations_disable_default_and_configured_hooks() {
    case(
        "scoped_operations_disable_default_and_configured_hooks",
        || {
            let (f, repo, bare) = setup();
            let c = CancellationToken::new();
            let client_marker = f.root.join("client-hook-ran");
            let server_marker = f.root.join("server-hook-ran");
            f.run(&["config", "--unset-all", "core.hooksPath"]);
            hook(&f.repository.join(".git/hooks"), "pre-push", &client_marker);
            hook(
                &f.repository.join(".git/hooks"),
                "reference-transaction",
                &client_marker,
            );
            let result = repo
                .push("origin", "main", "main", PushOptions::default(), &c)
                .unwrap();
            assert!(result.result.is_ok(), "{:?}", result.result);
            assert!(!client_marker.exists(), "default client hook executed");
            assert!(!server_marker.exists(), "local receive-pack hook executed");
            let custom = f.root.join("configured-hooks");
            hook(&custom, "pre-push", &client_marker);
            hook(&custom, "reference-transaction", &client_marker);
            f.run(&["config", "core.hooksPath", custom.to_str().unwrap()]);
            assert!(
                repo.push("origin", "main", "main", PushOptions::default(), &c)
                    .unwrap()
                    .result
                    .is_ok()
            );
            assert!(repo.fetch("origin", None, &c).unwrap().result.is_ok());
            assert!(!client_marker.exists(), "configured hook executed");
            assert!(!server_marker.exists(), "local receiver hook executed");
            assert_eq!(
                String::from_utf8(f.run(&["config", "--get", "core.hooksPath"]))
                    .unwrap()
                    .trim(),
                custom.to_str().unwrap()
            );
            // Local receive-pack drops client config overrides, so an unsafe
            // receiver must be refused rather than relying on inherited -c.
            hook(&bare.join("hooks"), "pre-receive", &server_marker);
            let rejected = repo
                .push("origin", "main", "main", PushOptions::default(), &c)
                .unwrap();
            assert!(matches!(rejected.result, Err(Error::InvalidInput(_))));
            assert!(!server_marker.exists());
            fs::remove_file(bare.join("hooks/pre-receive")).unwrap();
            let receiver_hooks = f.root.join("receiver-hooks");
            hook(&receiver_hooks, "pre-receive", &server_marker);
            f.run_at(
                &bare,
                &["config", "core.hooksPath", receiver_hooks.to_str().unwrap()],
            );
            let rejected = repo
                .push("origin", "main", "main", PushOptions::default(), &c)
                .unwrap();
            assert!(matches!(rejected.result, Err(Error::InvalidInput(_))));
            assert!(!server_marker.exists());
            // Ordinary clients must keep the user's hook behavior.
            assert!(
                f.repo()
                    .push("origin", "main", "main", PushOptions::default(), &c)
                    .unwrap()
                    .result
                    .is_ok()
            );
            assert!(
                client_marker.exists(),
                "normal desktop/CLI hook was disabled"
            );
        },
    );
}

// Force, atomic and tag pushes start with --no-replace-objects, so the scope check only
// sees them if it finds the verb behind that flag. Each case then removes the hook:
// the same push going through proves the hook was what refused it.
fn active_receiver_hook(f: &Fixture, bare: &Path) -> std::path::PathBuf {
    let marker = f.root.join("receiver-hook-ran");
    hook(&bare.join("hooks"), "pre-receive", &marker);
    marker
}
fn refused_by_receiver_hook(result: &Result<WriteOutput>, marker: &Path) {
    assert!(
        matches!(result, Err(Error::InvalidInput(m)) if m.contains("without active hooks")),
        "{result:?}"
    );
    assert!(!marker.exists(), "local receiver hook executed");
}

#[test]
fn scoped_force_push_refuses_a_receiver_with_an_active_hook() {
    case(
        "scoped_force_push_refuses_a_receiver_with_an_active_hook",
        || {
            let (f, repo, bare) = setup();
            let c = CancellationToken::new();
            f.write("file", "old remote");
            f.commit("old remote");
            f.run(&["push", "origin", "main"]);
            f.run(&["reset", "--hard", "HEAD^"]);
            f.write("file", "replacement");
            f.commit("replacement");
            let marker = active_receiver_hook(&f, &bare);
            let preview = repo
                .preview_force_push(b"origin", b"main", b"main", &c)
                .unwrap();
            let report = repo.apply_force_push(&preview, &c).unwrap();
            refused_by_receiver_hook(&report.result, &marker);
            assert_eq!(report.remote_state, ForcePushRemoteState::Unchanged);
            fs::remove_file(bare.join("hooks/pre-receive")).unwrap();
            let report = repo.apply_force_push(&preview, &c).unwrap();
            assert!(report.result.is_ok(), "{:?}", report.result);
            assert_eq!(report.remote_state, ForcePushRemoteState::Replaced);
        },
    );
}

#[test]
fn scoped_atomic_push_refuses_a_receiver_with_an_active_hook() {
    case(
        "scoped_atomic_push_refuses_a_receiver_with_an_active_hook",
        || {
            let (f, repo, bare) = setup();
            let c = CancellationToken::new();
            f.run(&["push", "origin", "main"]);
            f.write("file", "update");
            f.commit("update");
            let marker = active_receiver_hook(&f, &bare);
            let selection = [AtomicPushSelection {
                local_branch: b"main".to_vec(),
                remote_branch: b"main".to_vec(),
            }];
            let preview = repo.preview_atomic_push(b"origin", &selection, &c).unwrap();
            let report = repo.apply_atomic_push(&preview, &c).unwrap();
            refused_by_receiver_hook(&report.result, &marker);
            assert_eq!(report.remote_state, AtomicPushRemoteState::Unchanged);
            fs::remove_file(bare.join("hooks/pre-receive")).unwrap();
            let report = repo.apply_atomic_push(&preview, &c).unwrap();
            assert!(report.result.is_ok(), "{:?}", report.result);
            assert_eq!(report.remote_state, AtomicPushRemoteState::Published);
        },
    );
}

#[test]
fn scoped_tag_publish_refuses_a_receiver_with_an_active_hook() {
    case(
        "scoped_tag_publish_refuses_a_receiver_with_an_active_hook",
        || {
            let (f, repo, bare) = setup();
            let c = CancellationToken::new();
            f.run(&["tag", "version"]);
            let marker = active_receiver_hook(&f, &bare);
            let tags = repo.remote_tags(b"origin", &c).unwrap();
            let preview = repo
                .preview_remote_tag(&tags, b"version", RemoteTagAction::Publish, &c)
                .unwrap();
            let report = repo.apply_remote_tag(&preview, &c).unwrap();
            refused_by_receiver_hook(&report.result, &marker);
            assert!(matches!(report.observed, Ok(None)), "{:?}", report.observed);
            fs::remove_file(bare.join("hooks/pre-receive")).unwrap();
            let report = repo.apply_remote_tag(&preview, &c).unwrap();
            assert!(report.result.is_ok(), "{:?}", report.result);
            assert!(report.verification.is_ok(), "{:?}", report.verification);
        },
    );
}

#[test]
fn scoped_https_refuses_loopback_proxy_before_connect() {
    case("scoped_https_refuses_loopback_proxy_before_connect", || {
        let f = Fixture::new();
        let origin = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("https://{}/fixture.git", origin.local_addr().unwrap());
        let proxy = TcpListener::bind("127.0.0.1:0").unwrap();
        proxy.set_nonblocking(true).unwrap();
        let address = format!("http://{}", proxy.local_addr().unwrap());
        f.run(&["remote", "add", "origin", &url]);
        f.run(&["config", "http.proxy", &address]);
        f.run(&["config", "http.sslVerify", "false"]);
        unsafe {
            std::env::set_var("GIT_ALLOW_PROTOCOL", "https");
        }
        let stop = Arc::new(AtomicBool::new(false));
        let done = stop.clone();
        let server = thread::spawn(move || {
            while !done.load(Ordering::Relaxed) {
                match proxy.accept() {
                    Ok((mut stream, _)) => {
                        stream
                            .set_read_timeout(Some(Duration::from_secs(2)))
                            .unwrap();
                        let mut bytes = [0; 4096];
                        let n = stream.read(&mut bytes).unwrap_or(0);
                        let _ = stream.write_all(
                            b"HTTP/1.1 502 Fixture stopped here\r\nContent-Length: 0\r\n\r\n",
                        );
                        return String::from_utf8_lossy(&bytes[..n]).into_owned();
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(2))
                    }
                    Err(e) => panic!("{e}"),
                }
            }
            String::new()
        });
        let client = f.git.clone().with_transport_scope(Some(
            TransportScope::new(&f.root, vec![url.clone()]).unwrap(),
        ));
        let c = CancellationToken::new();
        let repo = client.open(&f.repository, &c).unwrap();
        let result = repo.check_remote(url.as_bytes(), &c);
        stop.store(true, Ordering::Relaxed);
        let request = server.join().unwrap();
        assert!(request.is_empty(), "proxy was contacted: {request}");
        assert!(matches!(result, Err(Error::InvalidInput(_))), "{result:?}");
    });
}

#[test]
fn scoped_transports_reject_proxy_tls_and_route_configuration() {
    case(
        "scoped_transports_reject_proxy_tls_and_route_configuration",
        || {
            let (f, repo, bare) = setup();
            let c = CancellationToken::new();
            for (key, value) in [
                ("http.proxy", "http://127.0.0.1:1"),
                ("remote.origin.proxy", "http://127.0.0.1:1"),
                ("http.https://example.invalid.proxy", "http://127.0.0.1:1"),
                ("http.sslVerify", "false"),
                ("http.https://example.invalid.sslVerify", "false"),
                ("http.sslCAInfo", "never-read.pem"),
                ("http.sslCAPath", "never-read"),
                ("http.sslBackend", "openssl"),
                ("http.schannelCheckRevoke", "false"),
                ("http.curloptResolve", "example.invalid:443:127.0.0.1"),
                ("http.https://example.invalid.followRedirects", "true"),
            ] {
                f.run(&["config", key, value]);
                let result = repo.check_remote(bare.to_str().unwrap().as_bytes(), &c);
                f.run(&["config", "--unset-all", key]);
                assert!(
                    matches!(result, Err(Error::InvalidInput(_))),
                    "accepted {key}: {result:?}"
                );
            }
            let include = f.root.join("network.config");
            fs::write(
                &include,
                "[http \"https://example.invalid\"]\n sslVerify = false\n",
            )
            .unwrap();
            f.run(&["config", "include.path", include.to_str().unwrap()]);
            assert!(matches!(
                repo.fetch("origin", None, &c).unwrap().result,
                Err(Error::InvalidInput(_))
            ));
            f.run(&["config", "--unset-all", "include.path"]);
            for (key, value) in [
                ("http.proxy", ""),
                ("remote.origin.proxy", ""),
                ("http.sslVerify", "true"),
                ("http.https://example.invalid.sslVerify", "true"),
                ("http.followRedirects", "false"),
            ] {
                f.run(&["config", key, value]);
            }
            assert!(
                repo.check_remote(bare.to_str().unwrap().as_bytes(), &c)
                    .is_ok()
            );
        },
    );
}

#[test]
fn scoped_transports_refuse_inherited_proxy_and_tls_overrides() {
    case(
        "scoped_transports_refuse_inherited_proxy_and_tls_overrides",
        || {
            let (_fixture, repo, bare) = setup();
            let c = CancellationToken::new();
            for key in [
                "HTTP_PROXY",
                "https_proxy",
                "ALL_PROXY",
                "NO_PROXY",
                "GIT_SSL_NO_VERIFY",
                "GIT_SSL_CAINFO",
                "GIT_SSL_CAPATH",
                "GIT_SSL_VERSION",
                "GIT_PROXY_SSL_CAINFO",
                "CURL_CA_BUNDLE",
                "SSL_CERT_FILE",
                "SSL_CERT_DIR",
                "SSLKEYLOGFILE",
                "OPENSSL_CONF",
                "OPENSSL_MODULES",
            ] {
                let prior = std::env::var_os(key);
                unsafe {
                    std::env::set_var(key, "inert-fixture-setting");
                }
                let result = repo.check_remote(bare.to_str().unwrap().as_bytes(), &c);
                unsafe {
                    match prior {
                        Some(v) => std::env::set_var(key, v),
                        None => std::env::remove_var(key),
                    }
                }
                assert!(
                    matches!(result, Err(Error::InvalidInput(_))),
                    "accepted {key}: {result:?}"
                );
            }
        },
    );
}
