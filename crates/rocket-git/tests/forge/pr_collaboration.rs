use super::*;

macro_rules! case {
    ($name:ident,$body:block) => {
        #[test]
        fn $name() {
            isolated(
                "ROCKET_PR_COLLAB_CASE",
                concat!("pr_collaboration_tests::", stringify!($name)),
                || $body,
            );
        }
    };
}
struct Threads {
    _fixture: Fixture,
    server: Server,
    session: ForgeSession,
    repo: ForgeRepository,
    mode: Arc<AtomicUsize>,
    resolved: Arc<AtomicBool>,
    mutation: Arc<AtomicBool>,
}
impl Threads {
    fn new(provider: ForgeProvider) -> Self {
        let f = Fixture::new();
        let mode = Arc::new(AtomicUsize::new(0));
        let resolved = Arc::new(AtomicBool::new(false));
        let mutation = Arc::new(AtomicBool::new(false));
        let (m, r, w) = (mode.clone(), resolved.clone(), mutation.clone());
        let server = serve(Box::new(move |req| {
            let mode = m.load(Ordering::Acquire);
            if req.path.ends_with("/graphql") {
                let expected = if provider == ForgeProvider::GitHubEnterprise {
                    "/github-enterprise/api/graphql"
                } else {
                    "/github/graphql"
                };
                assert_eq!(req.path, expected);
                if req.body.contains("query RocketReviewThreads") {
                    if mode >= 10 {
                        let kind = match mode {
                            10 => "RATE_LIMITED",
                            11 => "FORBIDDEN",
                            _ => "UNRECOGNIZED",
                        };
                        return response(&format!(
                            r#"{{"errors":[{{"type":"{kind}","message":"PRIVATE detail"}}]}}"#
                        ));
                    }
                    if mode == 8 {
                        thread::sleep(Duration::from_millis(900));
                    }
                    let can = mode != 3;
                    let nodes = if mode == 4 {
                        "[]".into()
                    } else {
                        format!(
                            r#"[{{"id":"PRRT_fixture","path":"資料/新.rs","startLine":1,"line":2,"isResolved":{},"isOutdated":false,"viewerCanResolve":{can},"viewerCanUnresolve":true,"comments":{{"pageInfo":{{"hasNextPage":false}},"nodes":[{{"fullDatabaseId":"8","author":{{"login":"contributor"}},"body":"PRIVATE original","createdAt":"today","path":"資料/新.rs","line":2,"replyTo":null}}]}}}}]"#,
                            r.load(Ordering::Acquire)
                        )
                    };
                    return response(&format!(
                        r#"{{"data":{{"repository":{{"nameWithOwner":"team/sample","pullRequest":{{"number":42,"headRefOid":"{}","baseRefOid":"{}","reviewThreads":{{"pageInfo":{{"hasNextPage":false}},"nodes":{nodes}}}}}}}}}}}"#,
                        "a".repeat(40),
                        "b".repeat(40)
                    ));
                }
                w.store(true, Ordering::Release);
                if mode == 6 {
                    return (403, "{}".into(), vec![]);
                }
                if mode == 9 {
                    thread::sleep(Duration::from_millis(900));
                }
                let yes = req.body.contains("{resolveReviewThread");
                r.store(yes, Ordering::Release);
                return response(&format!(
                    r#"{{"data":{{"{}":{{"thread":{{"id":"PRRT_fixture","isResolved":{yes}}}}}}}}}"#,
                    if yes {
                        "resolveReviewThread"
                    } else {
                        "unresolveReviewThread"
                    }
                ));
            }
            if req.path.ends_with("/comments/8/replies") {
                w.store(true, Ordering::Release);
                assert!(req.body.contains("PRIVATE reply"));
                if mode == 9 {
                    thread::sleep(Duration::from_millis(900));
                }
                return response(
                    r#"{"id":99,"user":{"login":"fixture"},"body":"PRIVATE reply","created_at":"today","path":"資料/新.rs","line":2,"in_reply_to_id":8}"#,
                );
            }
            pr_workflow_tests::api(req, if mode == 1 { 1 } else { 0 })
        }));
        let cfg = config(provider, server.port);
        let c = CancellationToken::new();
        let session = f.git.forge_session(cfg.clone(), "", TOKEN, &c).unwrap();
        let repo = ForgeRepository::new(cfg, "team/sample").unwrap();
        Self {
            _fixture: f,
            server,
            session,
            repo,
            mode,
            resolved,
            mutation,
        }
    }
    fn read(&self) -> ForgePullThreads {
        self.session
            .pull_threads(&self.repo, 42, &CancellationToken::new())
            .unwrap()
    }
}
case!(threads_preview_reply_resolve_and_enterprise_endpoint, {
    for provider in [ForgeProvider::GitHub, ForgeProvider::GitHubEnterprise] {
        let h = Threads::new(provider);
        let c = CancellationToken::new();
        for action in [
            ForgeThreadAction::Reply,
            ForgeThreadAction::Resolve,
            ForgeThreadAction::Unresolve,
        ] {
            let d = h.read();
            assert_eq!(d.threads[0].path, "資料/新.rs");
            let p = h
                .session
                .preview_pull_thread(&d, 0, action, "PRIVATE reply", &c)
                .unwrap();
            h.session.submit_pull_thread(&p, &c).unwrap();
            assert!(matches!(
                h.session.submit_pull_thread(&p, &c),
                Err(Error::Forge(ForgeFailure::PullReviewStale))
            ));
            assert_eq!(
                h.resolved.load(Ordering::Acquire),
                action == ForgeThreadAction::Resolve
            );
        }
        assert!(
            h.server
                .seen
                .lock()
                .unwrap()
                .iter()
                .all(|r| !r.path.contains("PRIVATE"))
        );
    }
});
case!(changed_or_unpermitted_threads_never_mutate, {
    let h = Threads::new(ForgeProvider::GitHub);
    let c = CancellationToken::new();
    let d = h.read();
    for mode in [1, 3, 4] {
        h.mode.store(0, Ordering::Release);
        let p = h
            .session
            .preview_pull_thread(&d, 0, ForgeThreadAction::Resolve, "", &c)
            .unwrap();
        h.mode.store(mode, Ordering::Release);
        assert!(h.session.submit_pull_thread(&p, &c).is_err());
        assert!(!h.mutation.load(Ordering::Acquire));
    }
    h.mode.store(0, Ordering::Release);
    let p = h
        .session
        .preview_pull_thread(&d, 0, ForgeThreadAction::Resolve, "", &c)
        .unwrap();
    h.mode.store(6, Ordering::Release);
    assert!(matches!(
        h.session.submit_pull_thread(&p, &c),
        Err(Error::Forge(ForgeFailure::Forbidden))
    ));
});
case!(received_thread_mutation_cancellation_is_unknown, {
    let h = Threads::new(ForgeProvider::GitHub);
    let c = CancellationToken::new();
    let d = h.read();
    let p = h
        .session
        .preview_pull_thread(&d, 0, ForgeThreadAction::Reply, "PRIVATE reply", &c)
        .unwrap();
    h.mode.store(9, Ordering::Release);
    let signal = h.mutation.clone();
    let stop = c.clone();
    let worker = thread::spawn(move || {
        let start = Instant::now();
        while !signal.load(Ordering::Acquire) {
            assert!(start.elapsed() < Duration::from_secs(5));
            thread::sleep(Duration::from_millis(5));
        }
        stop.cancel();
    });
    assert!(matches!(
        h.session.submit_pull_thread(&p, &c),
        Err(Error::Forge(ForgeFailure::PullReviewUnknown))
    ));
    worker.join().unwrap();
    let n = h
        .server
        .seen
        .lock()
        .unwrap()
        .iter()
        .filter(|r| r.path.ends_with("/comments/8/replies"))
        .count();
    assert_eq!(n, 1);
});

case!(thread_query_errors_have_distinct_redacted_failures, {
    let h = Threads::new(ForgeProvider::GitHub);
    for (mode, expected) in [
        (10, ForgeFailure::RateLimited),
        (11, ForgeFailure::Forbidden),
        (12, ForgeFailure::InvalidResponse),
    ] {
        h.mode.store(mode, Ordering::Release);
        let error = h
            .session
            .pull_threads(&h.repo, 42, &CancellationToken::new())
            .unwrap_err();
        assert!(matches!(error, Error::Forge(actual) if actual == expected));
        assert!(!error.to_string().contains("PRIVATE"));
    }
    assert!(!h.mutation.load(Ordering::Acquire));
});
