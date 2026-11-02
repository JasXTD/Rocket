use super::*;

const SERVER: ForgeProvider = ForgeProvider::BitbucketServer;

#[test]
fn bitbucket_browser_exchange_verification_errors_and_cancel() {
    isolated(
        "ROCKET_FORGE_CASE",
        "bitbucket_tests::bitbucket_browser_exchange_verification_errors_and_cancel",
        || {
            let f = Fixture::new();
            let mode = Arc::new(AtomicUsize::new(0));
            let m = mode.clone();
            let server = serve(Box::new(move |req| {
                if req.path.ends_with("/rocket/browser-sign-in") {
                    assert_eq!(req.method, "POST");
                    assert!(
                        req.head
                            .contains("Authorization: Bearer rocket-disposable-fixture")
                    );
                    match m.load(Ordering::Acquire) {
                        1 => return (401, format!(r#"{{"error":"{TOKEN}"}}"#), vec![]),
                        2 => return response("{}"),
                        3 => thread::sleep(Duration::from_millis(600)),
                        _ => (),
                    }
                    return response(&format!(r#"{{"access_token":"{TOKEN}"}}"#));
                }
                assert!(req.head.contains(&format!("Authorization: Bearer {TOKEN}")));
                route(ForgeProvider::Bitbucket, req, 0)
            }));
            let cfg = config(ForgeProvider::Bitbucket, server.port);
            let c = CancellationToken::new();
            let (session, token) = f.git.forge_browser_session(cfg.clone(), &c).unwrap();
            assert_eq!(token, TOKEN);
            assert_eq!(session.login(), "fixture");
            assert!(session.temporary_browser_session());
            for (n, wanted) in [
                (1, ForgeFailure::Unauthorized),
                (2, ForgeFailure::InvalidResponse),
            ] {
                mode.store(n, Ordering::Release);
                assert_eq!(
                    failure(f.git.forge_browser_session(cfg.clone(), &c)),
                    wanted
                );
            }
            mode.store(3, Ordering::Release);
            let c = CancellationToken::new();
            let cc = c.clone();
            let t = thread::spawn(move || {
                thread::sleep(Duration::from_millis(150));
                cc.cancel();
            });
            assert!(matches!(
                f.git.forge_browser_session(cfg, &c),
                Err(Error::Cancelled)
            ));
            t.join().unwrap();
            assert!(
                f.git
                    .forge_browser_session(config(SERVER, server.port), &CancellationToken::new())
                    .is_err()
            );
        },
    );
}

#[test]
fn bitbucket_server_identity_requires_authenticated_header() {
    isolated(
        "ROCKET_FORGE_CASE",
        "bitbucket_tests::bitbucket_server_identity_requires_authenticated_header",
        || {
            let f = Fixture::new();
            let c = CancellationToken::new();
            for header in [
                None,
                Some("anonymous"),
                Some("%61nonymous"),
                Some("%20Anonymous%20"),
                Some("%0aevil"),
                Some("%FF"),
                Some("%E7%95%8C%40work"),
            ] {
                let server = serve(Box::new(move |r| {
                    assert_eq!(r.path, "/bitbucket-server/application-properties");
                    assert!(r.head.contains(&format!("Authorization: Bearer {TOKEN}")));
                    (
                        200,
                        "{\"version\":\"fixture\"}".into(),
                        header
                            .map(|v| vec![("X-AUSERNAME".into(), v.into())])
                            .unwrap_or_default(),
                    )
                }));
                let cfg = config(SERVER, server.port);
                let result = f.git.forge_session(cfg.clone(), "", TOKEN, &c);
                if header == Some("%E7%95%8C%40work") {
                    assert_eq!(result.unwrap().login(), "界@work");
                    assert_eq!(
                        failure(f.git.forge_session(cfg, "wrong-user", TOKEN, &c)),
                        ForgeFailure::IdentityUnverified
                    );
                } else {
                    assert_eq!(failure(result), ForgeFailure::IdentityUnverified);
                }
            }
        },
    );
}

#[test]
fn bitbucket_server_offsets_filters_exact_branches_and_stale_context() {
    isolated(
        "ROCKET_FORGE_CASE",
        "bitbucket_tests::bitbucket_server_offsets_filters_exact_branches_and_stale_context",
        || {
            let f = Fixture::new();
            let c = CancellationToken::new();
            let mode = Arc::new(AtomicUsize::new(0));
            let m = mode.clone();
            let server = serve(Box::new(move |r| {
                if r.path.contains("/branches?") && !r.path.contains("main") {
                    if r.path.ends_with("&start=0") {
                        return response(&format!(
                            r#"{{"values":[{{"id":"refs/heads/feature/界-extra","latestCommit":"{}"}}],"start":0,"isLastPage":false,"nextPageStart":7}}"#,
                            "d".repeat(40)
                        ));
                    }
                    assert!(r.path.ends_with("&start=7"));
                    return route(
                        SERVER,
                        r,
                        if m.load(Ordering::Acquire) == 9 { 9 } else { 0 },
                    );
                }
                if r.path.contains("/pull-requests?") {
                    let start = if r.path.ends_with("&start=17") { 17 } else { 0 };
                    let a = item(SERVER, 42);
                    let b = item(SERVER, 43).replace("\"OPEN\"", "\"MERGED\"");
                    let (last, next) = if start == 0 {
                        (
                            "false",
                            if m.load(Ordering::Acquire) == 4 {
                                0
                            } else {
                                17
                            },
                        )
                    } else {
                        ("true", 0)
                    };
                    return response(&format!(
                        r#"{{"values":[{a},{b}],"start":{start},"isLastPage":{last},"nextPageStart":{next}}}"#
                    ));
                }
                route(SERVER, r, 0)
            }));
            let cfg = config(SERVER, server.port);
            let s = f.git.forge_session(cfg.clone(), "", TOKEN, &c).unwrap();
            let d = draft(cfg.clone());
            let filter = ForgeFilter {
                state: ForgeState::Closed,
                ..Default::default()
            };
            let page = s
                .list(&d.repository, &filter, ForgeCursor::default(), &c)
                .unwrap();
            assert_eq!(page.items.len(), 1);
            assert_eq!(page.items[0].number, 43);
            let next = page.next.unwrap();
            assert!(
                s.list(&d.repository, &filter, next.clone(), &c)
                    .unwrap()
                    .next
                    .is_none()
            );
            assert_eq!(
                failure(s.list(&d.repository, &ForgeFilter::default(), next.clone(), &c)),
                ForgeFailure::InvalidPagination
            );
            let newer = f.git.forge_session(cfg, "", TOKEN, &c).unwrap();
            assert_eq!(
                failure(newer.list(&d.repository, &filter, next.clone(), &c)),
                ForgeFailure::InvalidPagination
            );
            let cancelled = CancellationToken::new();
            cancelled.cancel();
            let before = server.seen.lock().unwrap().len();
            assert!(matches!(
                s.list(&d.repository, &filter, next, &cancelled),
                Err(Error::Cancelled)
            ));
            assert_eq!(server.seen.lock().unwrap().len(), before);
            mode.store(4, Ordering::Release);
            assert_eq!(
                failure(s.list(&d.repository, &filter, ForgeCursor::default(), &c)),
                ForgeFailure::InvalidPagination
            );
            mode.store(0, Ordering::Release);
            let preview = s.preview(d, &c).unwrap();
            assert_eq!(preview.source_id(), "a".repeat(40));
            mode.store(9, Ordering::Release);
            assert_eq!(failure(s.create(&preview, &c)), ForgeFailure::StaleReview);
            assert!(
                !server
                    .seen
                    .lock()
                    .unwrap()
                    .iter()
                    .any(|r| r.method == "POST")
            );
            let issues = ForgeFilter {
                kind: ForgeKind::Issues,
                ..Default::default()
            };
            assert_eq!(
                failure(s.list(
                    &preview.draft().repository,
                    &issues,
                    ForgeCursor::default(),
                    &c
                )),
                ForgeFailure::Unsupported
            );
        },
    );
}

#[test]
fn bitbucket_server_rejections_redirects_and_cancelled_creation() {
    isolated(
        "ROCKET_FORGE_CASE",
        "bitbucket_tests::bitbucket_server_rejections_redirects_and_cancelled_creation",
        || {
            let f = Fixture::new();
            let c = CancellationToken::new();
            let mode = Arc::new(AtomicUsize::new(0));
            let m = mode.clone();
            let server = serve(Box::new(move |r| {
                route(SERVER, r, m.load(Ordering::Acquire))
            }));
            let cfg = config(SERVER, server.port);
            let s = f.git.forge_session(cfg.clone(), "", TOKEN, &c).unwrap();
            let d = draft(cfg);
            for (status, wanted) in [
                (401, ForgeFailure::Unauthorized),
                (403, ForgeFailure::Forbidden),
                (404, ForgeFailure::NotFound),
                (429, ForgeFailure::RateLimited),
                (7, ForgeFailure::InvalidResponse),
            ] {
                mode.store(status, Ordering::Release);
                assert_eq!(
                    failure(s.list(
                        &d.repository,
                        &ForgeFilter::default(),
                        ForgeCursor::default(),
                        &c
                    )),
                    wanted
                );
            }
            mode.store(0, Ordering::Release);
            let review = s.preview(d.clone(), &c).unwrap();
            mode.store(409, Ordering::Release);
            assert_eq!(failure(s.create(&review, &c)), ForgeFailure::Rejected);
            mode.store(0, Ordering::Release);
            let review = s.preview(d, &c).unwrap();
            mode.store(8, Ordering::Release);
            let sent = server.seen.clone();
            let cc = c.clone();
            let worker = thread::spawn(move || {
                let until = Instant::now() + Duration::from_secs(5);
                while Instant::now() < until {
                    if sent
                        .lock()
                        .unwrap()
                        .iter()
                        .filter(|r| r.method == "POST")
                        .count()
                        == 2
                    {
                        cc.cancel();
                        return;
                    }
                    thread::sleep(Duration::from_millis(10));
                }
                panic!("create was not sent");
            });
            assert_eq!(failure(s.create(&review, &c)), ForgeFailure::OutcomeUnknown);
            worker.join().unwrap();
            assert_eq!(
                failure(s.create(&review, &CancellationToken::new())),
                ForgeFailure::StaleReview
            );
        },
    );
}
