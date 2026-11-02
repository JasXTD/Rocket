use super::*;
const PROVIDER: ForgeProvider = ForgeProvider::GitHubEnterprise;
macro_rules! case {
    ($name:ident, $body:block) => {
        #[test]
        fn $name() {
            isolated(
                "ROCKET_GHES_CASE",
                concat!("enterprise_tests::", stringify!($name)),
                || $body,
            );
        }
    };
}
case!(enterprise_auth_and_private_failures, {
    let f = Fixture::new();
    let cancel = CancellationToken::new();
    let server = serve(Box::new(|r| {
        assert_eq!(r.path, "/github-enterprise/api/v3/user");
        assert!(
            r.head
                .to_ascii_lowercase()
                .contains("authorization: bearer fixture-token-do-not-log")
        );
        assert!(
            r.head
                .to_ascii_lowercase()
                .contains("x-github-api-version: 2022-11-28")
        );
        (
            200,
            r#"{"login":"enterprise-user"}"#.into(),
            vec![("X-GitHub-Enterprise-Version".into(), "3.21.0".into())],
        )
    }));
    let cfg = config(PROVIDER, server.port);
    let session = f
        .git
        .forge_session(cfg.clone(), "", TOKEN, &cancel)
        .unwrap();
    assert_eq!(session.login(), "enterprise-user");
    assert_eq!(session.enterprise_version(), Some("3.21.0"));
    assert!(f.git.forge_browser_session(cfg.clone(), &cancel).is_err());
    for token in ["", "bad\nheader", "bad token"] {
        assert!(
            f.git
                .forge_session(cfg.clone(), "", token, &cancel)
                .is_err()
        );
    }
    assert_eq!(server.seen.lock().unwrap().len(), 1);
    for (status, expected) in [
        (401, ForgeFailure::Unauthorized),
        (403, ForgeFailure::Forbidden),
        (404, ForgeFailure::NotFound),
        (429, ForgeFailure::RateLimited),
        (302, ForgeFailure::RedirectRefused),
        (410, ForgeFailure::ApiVersionUnsupported),
        (503, ForgeFailure::ServerUnavailable),
        (200, ForgeFailure::InvalidResponse),
    ] {
        let bad = serve(Box::new(move |_| {
            (
                status,
                format!("<html>PRIVATE {TOKEN}</html>"),
                vec![("Location".into(), "https://outside.invalid/login".into())],
            )
        }));
        let error = f
            .git
            .forge_session(config(PROVIDER, bad.port), "", TOKEN, &cancel)
            .unwrap_err();
        assert!(!error.to_string().contains(TOKEN));
        assert!(!error.to_string().contains("PRIVATE"));
        assert_eq!(failure::<()>(Err(error)), expected);
        assert_eq!(bad.seen.lock().unwrap().len(), 1);
    }
});
case!(enterprise_lists_search_and_pagination_stay_on_server, {
    let f = Fixture::new();
    let c = CancellationToken::new();
    let mode = Arc::new(AtomicUsize::new(0));
    let m = mode.clone();
    let server = serve(Box::new(move |r| {
        assert!(r.path.starts_with("/github-enterprise/api/v3/"));
        if r.path.ends_with("/user") {
            return response(r#"{"login":"fixture"}"#);
        }
        if r.path.contains("/search/issues?") {
            return response(&format!(
                r#"{{"total_count":1,"incomplete_results":false,"items":[{}]}}"#,
                item(PROVIDER, 42).replacen('{', "{\"pull_request\":{},", 1)
            ));
        }
        if r.path.contains("page=2") {
            return response(&format!("[{}]", item(PROVIDER, 43)));
        }
        if r.path.contains("/issues?") {
            return response(&format!(
                "[{},{}]",
                item(PROVIDER, 44),
                item(PROVIDER, 42).replacen('{', "{\"pull_request\":{},", 1)
            ));
        }
        let next = if m.load(Ordering::Acquire) == 0 {
            format!(
                "{}/github-enterprise/api/v3/repos/team/sample/pulls?page=2",
                fixture_origin(r)
            )
        } else {
            "https://outside.invalid/api/v3/repos/team/sample/pulls?page=2".into()
        };
        (
            200,
            format!("[{}]", item(PROVIDER, 42)),
            vec![("Link".into(), format!("<{next}>; rel=\"next\""))],
        )
    }));
    let cfg = config(PROVIDER, server.port);
    let s = f.git.forge_session(cfg.clone(), "", TOKEN, &c).unwrap();
    let repo = ForgeRepository::new(cfg.clone(), "team/sample").unwrap();
    let filter = ForgeFilter::default();
    let first = s.list(&repo, &filter, ForgeCursor::default(), &c).unwrap();
    assert_eq!(
        first.items[0].url,
        "https://github.example.com/team/sample/pull/42"
    );
    let cursor = first.next.unwrap();
    let other = f.git.forge_session(cfg.clone(), "", TOKEN, &c).unwrap();
    assert_eq!(
        failure(other.list(&repo, &filter, cursor.clone(), &c)),
        ForgeFailure::InvalidPagination
    );
    assert_eq!(
        s.list(&repo, &filter, cursor, &c).unwrap().items[0].number,
        43
    );
    let issues = ForgeFilter {
        kind: ForgeKind::Issues,
        ..Default::default()
    };
    assert_eq!(
        s.list(&repo, &issues, ForgeCursor::default(), &c)
            .unwrap()
            .items
            .len(),
        1
    );
    let search = ForgeFilter {
        search: ForgeSearch {
            text: "Unicode 界".into(),
            author: "fixture".into(),
            assignee: String::new(),
        },
        ..Default::default()
    };
    assert_eq!(
        s.list(&repo, &search, ForgeCursor::default(), &c)
            .unwrap()
            .total,
        Some(1)
    );
    mode.store(1, Ordering::Release);
    assert_eq!(
        failure(s.list(&repo, &filter, ForgeCursor::default(), &c)),
        ForgeFailure::InvalidPagination
    );
    let mismatched = ForgeRepository::new(
        ForgeConfig::new(PROVIDER, "https://another.example.com")
            .unwrap()
            .with_fixture(server.port)
            .unwrap(),
        "team/sample",
    )
    .unwrap();
    let before = server.seen.lock().unwrap().len();
    assert!(
        s.list(&mismatched, &filter, ForgeCursor::default(), &c)
            .is_err()
    );
    assert_eq!(server.seen.lock().unwrap().len(), before);
});
case!(enterprise_connect_cancellation_never_returns_a_session, {
    let f = Fixture::new();
    let c = CancellationToken::new();
    let server = serve(Box::new(|_| {
        thread::sleep(Duration::from_millis(900));
        response(r#"{"login":"fixture"}"#)
    }));
    let cfg = config(PROVIDER, server.port);
    c.cancel();
    assert!(matches!(
        f.git.forge_session(cfg.clone(), "", TOKEN, &c),
        Err(Error::Cancelled)
    ));
    assert!(server.seen.lock().unwrap().is_empty());
    let c = CancellationToken::new();
    thread::scope(|scope| {
        let request = scope.spawn(|| f.git.forge_session(cfg, "", TOKEN, &c));
        let start = Instant::now();
        while server.seen.lock().unwrap().is_empty() {
            assert!(start.elapsed() < Duration::from_secs(10));
            thread::sleep(Duration::from_millis(5));
        }
        c.cancel();
        assert!(matches!(request.join().unwrap(), Err(Error::Cancelled)));
    });
    assert_eq!(server.seen.lock().unwrap().len(), 1);
});
case!(enterprise_review_merge_and_cross_fork_use_shared_guards, {
    let f = Fixture::new();
    let c = CancellationToken::new();
    let mode = Arc::new(AtomicUsize::new(0));
    let m = mode.clone();
    let server = serve(Box::new(move |r| {
        assert!(r.path.starts_with("/github-enterprise/api/v3/"));
        pr_workflow_tests::api(r, m.load(Ordering::Acquire))
    }));
    let cfg = config(PROVIDER, server.port);
    let s = f.git.forge_session(cfg.clone(), "", TOKEN, &c).unwrap();
    let repo = ForgeRepository::new(cfg.clone(), "team/sample").unwrap();
    let details = s.pull_details(&repo, 42, &c).unwrap();
    let comment = details
        .inline_comment(0, 2, ForgeDiffSide::Right, "PRIVATE 界")
        .unwrap();
    let preview = s
        .preview_pull_review_with_comments(
            &details,
            ForgeReviewAction::Comment,
            "PRIVATE review",
            &[comment],
            &c,
        )
        .unwrap();
    s.submit_pull_review(&preview, &c).unwrap();
    assert_eq!(
        failure(s.submit_pull_review(&preview, &c)),
        ForgeFailure::PullReviewStale
    );
    let preview = s
        .preview_pull_merge(&details, ForgeMergeMethod::Squash, &c)
        .unwrap();
    assert_eq!(s.merge_pull(&preview, &c).unwrap().commit, "e".repeat(40));
    let preview = s
        .preview_pull_merge(&details, ForgeMergeMethod::Merge, &c)
        .unwrap();
    mode.store(1, Ordering::Release);
    assert_eq!(
        failure(s.merge_pull(&preview, &c)),
        ForgeFailure::PullChanged
    );
    mode.store(0, Ordering::Release);
    let draft = ForgeDraft {
        repository: repo,
        source: "main".into(),
        target: "main".into(),
        title: "PRIVATE PR".into(),
        body: "PRIVATE body".into(),
    };
    let source = ForgeRepository::new(cfg, "author/fork").unwrap();
    let preview = s.preview_from_fork(draft, source, &c).unwrap();
    assert_eq!(s.create(&preview, &c).unwrap().number, 43);
    let requests = server.seen.lock().unwrap();
    assert_eq!(requests.iter().filter(|r| r.method == "PUT").count(), 1);
    assert_eq!(requests.iter().filter(|r| r.method == "POST").count(), 2);
});
case!(enterprise_rejects_untrusted_tls_certificate, { untrusted_tls(PROVIDER); });
pub(super) fn untrusted_tls(provider: ForgeProvider) {
    use std::os::windows::process::CommandExt;
    use std::process::{Command, Stdio};
    let f = Fixture::new();
    // Use the OpenSSL already shipped with this Git for Windows installation;
    // no download, trust-store change or certificate-verification bypass.
    let openssl = f
        .git
        .executable()
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("usr/bin/openssl.exe");
    assert!(
        openssl.is_file(),
        "Git for Windows OpenSSL is required for this loopback TLS fixture"
    );
    let key = f.root.join("disposable-key.pem");
    let cert = f.root.join("disposable-cert.pem");
    let status = Command::new(&openssl)
        .args([
            "req",
            "-x509",
            "-newkey",
            "rsa:2048",
            "-nodes",
            "-days",
            "1",
            "-subj",
            "/CN=localhost",
            "-keyout",
        ])
        .arg(&key)
        .arg("-out")
        .arg(&cert)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .creation_flags(0x0800_0000)
        .status()
        .unwrap();
    assert!(status.success());
    let port = TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    struct OwnedServer(std::process::Child);
    impl Drop for OwnedServer {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    let mut server = OwnedServer(
        Command::new(&openssl)
            .args([
                "s_server",
                "-accept",
                &format!("127.0.0.1:{port}"),
                "-quiet",
                "-www",
                "-key",
            ])
            .arg(&key)
            .arg("-cert")
            .arg(&cert)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .creation_flags(0x0800_0000)
            .spawn()
            .unwrap(),
    );
    let start = Instant::now();
    while TcpStream::connect(("127.0.0.1", port)).is_err() {
        assert!(start.elapsed() < Duration::from_secs(10));
        assert!(server.0.try_wait().unwrap().is_none());
        thread::sleep(Duration::from_millis(10));
    }
    let suffix = if provider == ForgeProvider::AzureServer { "/tfs/DefaultCollection" } else { "" };
    let cfg = ForgeConfig::new(provider, &format!("https://127.0.0.1:{port}{suffix}")).unwrap();
    let error = f
        .git
        .forge_session(cfg, "", TOKEN, &CancellationToken::new())
        .unwrap_err();
    assert!(!error.to_string().contains(TOKEN));
    assert_eq!(failure::<()>(Err(error)), ForgeFailure::TlsVerification);
}
