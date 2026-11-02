use super::*;
const PROVIDER: ForgeProvider = ForgeProvider::AzureServer;
const PREFIX: &str = "/azure-server/tfs/DefaultCollection";
const IDENTITY: &str = r#"{"authenticatedUser":{"id":"11111111-2222-3333-4444-555555555555","providerDisplayName":"Fixture User","isActive":true,"isContainer":false}}"#;
macro_rules! case {
    ($name:ident, $body:block) => {
        #[test]
        fn $name() {
            isolated(
                "ROCKET_AZURE_SERVER_CASE",
                concat!("azure_server_tests::", stringify!($name)),
                || $body,
            );
        }
    };
}
fn cfg(port: u16, version: &str) -> ForgeConfig {
    config(PROVIDER, port)
        .with_azure_api_version(version)
        .unwrap()
}
case!(azure_server_auth_identity_and_errors, {
    let f = Fixture::new();
    let c = CancellationToken::new();
    let server = serve(Box::new(|r| {
        assert_eq!(r.path, format!("{PREFIX}/_apis/connectionData"));
        assert!(
            r.head
                .contains("Authorization: Basic OmZpeHR1cmUtdG9rZW4tRE8tTk9ULUxPRw==")
        );
        response(IDENTITY)
    }));
    let session = f
        .git
        .forge_session(cfg(server.port, "7.0"), "", TOKEN, &c)
        .unwrap();
    assert_eq!(session.login(), "Fixture User");
    assert!(
        f.git
            .forge_browser_session(cfg(server.port, "7.0"), &c)
            .is_err()
    );
    for token in ["", "bad\nheader", "bad token"] {
        assert!(
            f.git
                .forge_session(cfg(server.port, "7.0"), "", token, &c)
                .is_err()
        );
    }
    assert_eq!(server.seen.lock().unwrap().len(), 1);
    let unicode = serve(Box::new(|_| {
        response(&IDENTITY.replace("Fixture User", "開發者\\u202e"))
    }));
    let session = f
        .git
        .forge_session(cfg(unicode.port, "7.0"), "", TOKEN, &c)
        .unwrap();
    assert!(session.login().contains("開發者"));
    assert!(session.login().contains("\\u{202e}"));
    assert!(!session.login().contains('\u{202e}'));
    for bad in [
        "{}".to_owned(),
        IDENTITY.replace(
            "11111111-2222-3333-4444-555555555555",
            "00000000-0000-0000-0000-000000000000",
        ),
        IDENTITY.replace("Fixture User", "anonymous"),
        IDENTITY.replace("Fixture User", "   "),
        IDENTITY.replace("\"isActive\":true", "\"isActive\":false"),
        IDENTITY.replace("\"isContainer\":false", "\"isContainer\":true"),
        IDENTITY.replace(
            "\"id\":\"11111111-2222-3333-4444-555555555555\"",
            "\"id\":\"unverified\"",
        ),
        IDENTITY.replacen('{', "{\"authorizedUser\":{\"id\":\"other\"},", 1),
    ] {
        let server = serve(Box::new(move |_| response(&bad)));
        assert_eq!(
            failure(f.git.forge_session(cfg(server.port, "7.0"), "", TOKEN, &c)),
            ForgeFailure::AzureIdentityUnverified
        );
    }
    for (status, expected) in [
        (401, ForgeFailure::Unauthorized),
        (403, ForgeFailure::Forbidden),
        (404, ForgeFailure::NotFound),
        (429, ForgeFailure::RateLimited),
        (302, ForgeFailure::RedirectRefused),
        (503, ForgeFailure::ServerUnavailable),
        (400, ForgeFailure::AzureApiRejected),
        (410, ForgeFailure::AzureApiRejected),
        (200, ForgeFailure::InvalidResponse),
    ] {
        let server = serve(Box::new(move |_| {
            (
                status,
                format!("<html>PRIVATE {TOKEN}</html>"),
                vec![("Location".into(), "https://outside.invalid/login".into())],
            )
        }));
        let e = f
            .git
            .forge_session(cfg(server.port, "7.0"), "", TOKEN, &c)
            .unwrap_err();
        assert!(!e.to_string().contains(TOKEN));
        assert!(!e.to_string().contains("PRIVATE"));
        assert_eq!(failure::<()>(Err(e)), expected);
        assert_eq!(server.seen.lock().unwrap().len(), 1);
    }
});
case!(azure_server_routes_versions_and_reviewed_create, {
    let f = Fixture::new();
    let c = CancellationToken::new();
    for version in ["6.0", "7.0", "7.1"] {
        let mode = Arc::new(AtomicUsize::new(0));
        let m = mode.clone();
        let server = serve(Box::new(move |r| {
            assert!(r.path.starts_with(PREFIX));
            if r.path.ends_with("/_apis/connectionData") {
                return response(IDENTITY);
            }
            assert!(
                r.path.ends_with(&format!("api-version={version}")),
                "{}",
                r.path
            );
            if r.method == "POST" && r.path.contains("/pullrequests?") {
                assert!(
                    r.body
                        .contains(r#""sourceRefName":"refs/heads/feature/界""#)
                );
                assert!(r.body.contains(r#""targetRefName":"refs/heads/main""#));
            }
            route(PROVIDER, r, m.load(Ordering::Acquire))
        }));
        let config = cfg(server.port, version);
        let session = f.git.forge_session(config.clone(), "", TOKEN, &c).unwrap();
        let d = draft(config.clone());
        let listed = session
            .list(
                &d.repository,
                &ForgeFilter::default(),
                ForgeCursor::default(),
                &c,
            )
            .unwrap();
        assert_eq!(
            listed.items[0].url,
            "https://devops.example.com/tfs/DefaultCollection/team/_git/%E8%B3%87%E6%96%99/pullrequest/42"
        );
        let p = session.preview(draft(config.clone()), &c).unwrap();
        assert_eq!(session.create(&p, &c).unwrap().number, 43);
        assert_eq!(failure(session.create(&p, &c)), ForgeFailure::StaleReview);
        let p = session.preview(draft(config.clone()), &c).unwrap();
        mode.store(9, Ordering::Release);
        assert_eq!(failure(session.create(&p, &c)), ForgeFailure::StaleReview);
        assert_eq!(
            server
                .seen
                .lock()
                .unwrap()
                .iter()
                .filter(|r| r.method == "POST")
                .count(),
            1
        );
        mode.store(0, Ordering::Release);
        let p = session.preview(draft(config), &c).unwrap();
        mode.store(422, Ordering::Release);
        assert_eq!(failure(session.create(&p, &c)), ForgeFailure::Rejected);
    }
});
case!(azure_server_pagination_and_work_items_are_scoped, {
    let f = Fixture::new();
    let c = CancellationToken::new();
    let mode = Arc::new(AtomicUsize::new(0));
    let m = mode.clone();
    let server = serve(Box::new(move |r| {
        assert!(r.path.starts_with(PREFIX));
        if r.path.ends_with("/_apis/connectionData") {
            return response(IDENTITY);
        }
        assert!(r.path.contains("api-version=7.0"));
        let state = m.load(Ordering::Acquire);
        if r.path.contains("/wit/wiql?") {
            assert_eq!(r.method, "POST");
            assert!(r.body.contains("[System.TeamProject] = 'team'"));
            let ids = if r.body.contains("[System.Id] > 50") {
                vec![51]
            } else {
                (1..=50).collect::<Vec<_>>()
            };
            return response(&format!(
                r#"{{"workItems":[{}]}}"#,
                ids.iter()
                    .map(|id| format!(r#"{{"id":{id}}}"#))
                    .collect::<Vec<_>>()
                    .join(",")
            ));
        }
        if r.path.contains("/wit/workitems?") {
            assert!(r.path.contains("System.TeamProject"));
            let ids = if r.path.contains("ids=51&") {
                vec![51]
            } else {
                (1..=50).collect::<Vec<_>>()
            };
            let project = if state == 2 { "other" } else { "team" };
            return response(&format!(r#"{{"value":[{}]}}"#,ids.iter().map(|id|format!(r#"{{"id":{id},"fields":{{"System.Title":"PRIVATE work 界","System.State":"Active","System.TeamProject":"{project}","System.CreatedBy":{{"displayName":"fixture"}}}}}}"#)).collect::<Vec<_>>().join(",")));
        }
        if state == 1 {
            return (
                200,
                format!(r#"{{"value":[{}]}}"#, item(PROVIDER, 1)),
                vec![(
                    "Link".into(),
                    "<https://outside.invalid/collection>; rel=\"next\"".into(),
                )],
            );
        }
        let rows = if r.path.contains("$skip=50") {
            vec![item(PROVIDER, 51)]
        } else {
            (1..=50).map(|id| item(PROVIDER, id)).collect::<Vec<_>>()
        };
        response(&format!(r#"{{"value":[{}]}}"#, rows.join(",")))
    }));
    let config = cfg(server.port, "7.0");
    let session = f.git.forge_session(config.clone(), "", TOKEN, &c).unwrap();
    let repo = ForgeRepository::new(config.clone(), "team/資料").unwrap();
    let filter = ForgeFilter::default();
    let page = session
        .list(&repo, &filter, ForgeCursor::default(), &c)
        .unwrap();
    assert_eq!(page.items.len(), 50);
    let next = page.next.unwrap();
    let other = f.git.forge_session(config.clone(), "", TOKEN, &c).unwrap();
    let before = server.seen.lock().unwrap().len();
    assert_eq!(
        failure(other.list(&repo, &filter, next.clone(), &c)),
        ForgeFailure::InvalidPagination
    );
    assert_eq!(server.seen.lock().unwrap().len(), before);
    assert_eq!(
        session.list(&repo, &filter, next, &c).unwrap().items[0].number,
        51
    );
    mode.store(1, Ordering::Release);
    let before = server.seen.lock().unwrap().len();
    assert_eq!(
        failure(session.list(&repo, &filter, ForgeCursor::default(), &c)),
        ForgeFailure::InvalidPagination
    );
    assert_eq!(server.seen.lock().unwrap().len(), before + 1);
    mode.store(0, Ordering::Release);
    let work = ForgeFilter {
        kind: ForgeKind::Issues,
        work_item_state: "Active".into(),
        ..Default::default()
    };
    let page = session
        .list(&repo, &work, ForgeCursor::default(), &c)
        .unwrap();
    assert_eq!(page.items.len(), 50);
    assert_eq!(
        page.items[0].url,
        "https://devops.example.com/tfs/DefaultCollection/team/_workitems/edit/1"
    );
    let next = page.next.unwrap();
    let changed = ForgeFilter {
        work_item_state: "Done".into(),
        ..work.clone()
    };
    let before = server.seen.lock().unwrap().len();
    assert_eq!(
        failure(session.list(&repo, &changed, next.clone(), &c)),
        ForgeFailure::InvalidPagination
    );
    assert_eq!(server.seen.lock().unwrap().len(), before);
    assert_eq!(
        session.list(&repo, &work, next, &c).unwrap().items[0].number,
        51
    );
    mode.store(2, Ordering::Release);
    assert_eq!(
        failure(session.list(&repo, &work, ForgeCursor::default(), &c)),
        ForgeFailure::InvalidResponse
    );
    let version_repo =
        ForgeRepository::new(config.with_azure_api_version("6.0").unwrap(), "team/資料").unwrap();
    let before = server.seen.lock().unwrap().len();
    assert!(
        session
            .list(&version_repo, &filter, ForgeCursor::default(), &c)
            .is_err()
    );
    assert_eq!(server.seen.lock().unwrap().len(), before);
});
case!(azure_server_cancelled_auth_never_connects, {
    let f = Fixture::new();
    let server = serve(Box::new(|_| {
        thread::sleep(Duration::from_millis(900));
        response(IDENTITY)
    }));
    let c = CancellationToken::new();
    c.cancel();
    assert!(matches!(
        f.git.forge_session(cfg(server.port, "7.0"), "", TOKEN, &c),
        Err(Error::Cancelled)
    ));
    assert!(server.seen.lock().unwrap().is_empty());
    let c = CancellationToken::new();
    thread::scope(|scope| {
        let call = scope.spawn(|| f.git.forge_session(cfg(server.port, "7.0"), "", TOKEN, &c));
        let start = Instant::now();
        while server.seen.lock().unwrap().is_empty() {
            assert!(start.elapsed() < Duration::from_secs(10));
            thread::sleep(Duration::from_millis(5));
        }
        c.cancel();
        assert!(matches!(call.join().unwrap(), Err(Error::Cancelled)));
    });
});
case!(azure_server_rejects_untrusted_tls_certificate, {
    enterprise_tests::untrusted_tls(PROVIDER);
});
