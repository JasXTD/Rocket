//! Explore's repository search on GitLab and Gitea through the real transport: a
//! session without a token sends no Authorization header, a signed-in one does and asks
//! GitLab for sizes, the paths and headers each host documents, and the answers that
//! aren't errors (a rate limit with its wait, a server that wants a sign-in).
use super::*;

macro_rules! case {
    ($name:ident, $body:block) => {
        #[test]
        fn $name() {
            isolated(
                "ROCKET_EXPLORE_SEARCH_CASE",
                concat!("explore_search_tests::", stringify!($name)),
                || $body,
            );
        }
    };
}
type Reply = (u16, String, Vec<(String, String)>);

fn authorized(r: &Request) -> Option<String> {
    r.head
        .lines()
        .find_map(|l| l.split_once(':').filter(|(k, _)| k.eq_ignore_ascii_case("authorization")).map(|(_, v)| v.trim().to_owned()))
}
fn gitlab(r: &Request, mode: usize) -> Reply {
    let (endpoint, _) = r.path.split_once('?').unwrap_or((r.path.as_str(), ""));
    match endpoint {
        "/gitlab/user" => response(r#"{"username":"sample-user"}"#),
        "/gitlab/projects" => match mode {
            1 => (429, r#"{"message":"Retry later"}"#.into(), vec![("Retry-After".into(), "7".into())]),
            2 => (401, r#"{"message":"401 Unauthorized"}"#.into(), Vec::new()),
            _ => (
                200,
                r#"[{"path_with_namespace":"acme/tools/fuzzyfind","star_count":1204,"forks_count":98,"visibility":"public",
                    "http_url_to_repo":"C:\\fixtures\\fuzzyfind","last_activity_at":"2026-11-15T03:12:44.123Z"}]"#
                    .into(),
                vec![("X-Total".into(), "1248".into()), ("X-Next-Page".into(), "2".into())],
            ),
        },
        _ => (404, "{}".into(), Vec::new()),
    }
}
fn gitea(r: &Request, mode: usize) -> Reply {
    let (endpoint, _) = r.path.split_once('?').unwrap_or((r.path.as_str(), ""));
    match (endpoint, mode) {
        ("/gitea/repos/search", 1) => (403, r#"{"message":"Only signed in user is allowed to call APIs."}"#.into(), Vec::new()),
        ("/gitea/repos/search", _) => (
            200,
            r#"{"ok":true,"data":[{"full_name":"sample-user/blog-builder","stars_count":42,"language":"Go","size":12288,"mirror":false,
                "clone_url":"C:\\fixtures\\blog-builder","updated_at":"2026-11-13T10:20:30+01:00"}]}"#
                .into(),
            vec![("X-Total-Count".into(), "57".into())],
        ),
        _ => (404, "{}".into(), Vec::new()),
    }
}
fn page(outcome: RepositorySearchOutcome) -> RepositorySearchPage {
    match outcome {
        RepositorySearchOutcome::Page(page) => page,
        other => panic!("expected a page, got {other:?}"),
    }
}

case!(gitlab_without_a_token_sends_no_authorization_and_reads_the_headers, {
    let f = Fixture::new();
    let server = serve(Box::new(|r| gitlab(r, 0)));
    let session = f.git.forge_public_session(config(ForgeProvider::GitLab, server.port)).unwrap();
    assert!(!session.signed_in());
    let result = page(session.search_repositories("fuzzy finder", RepositorySearchSort::Stars, Some("Rust"), 1, &CancellationToken::new()).unwrap());
    assert_eq!((result.total, result.next_page), (Some(1248), Some(2)));
    let item = &result.items[0];
    assert_eq!(item.path, "acme/tools/fuzzyfind");
    assert_eq!(item.clone_url, "C:\\fixtures\\fuzzyfind", "a fixture names the local source");
    assert_eq!(item.web_url, "https://gitlab.com/acme/tools/fuzzyfind");
    let seen = server.seen.lock().unwrap();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].method, "GET");
    assert_eq!(seen[0].path, "/gitlab/projects?search=fuzzy%20finder&order_by=star_count&sort=desc&per_page=30&page=1&with_programming_language=Rust");
    assert_eq!(authorized(&seen[0]), None, "no Authorization header without an account");
});

case!(gitlab_with_an_account_sends_its_token_and_asks_for_sizes, {
    let f = Fixture::new();
    let server = serve(Box::new(|r| gitlab(r, 0)));
    let cancel = CancellationToken::new();
    let session = f.git.forge_session(config(ForgeProvider::GitLab, server.port), "", TOKEN, &cancel).unwrap();
    assert!(session.signed_in());
    page(session.search_repositories("finder", RepositorySearchSort::Updated, None, 3, &cancel).unwrap());
    let seen = server.seen.lock().unwrap();
    let search = seen.iter().find(|r| r.path.starts_with("/gitlab/projects")).unwrap();
    assert_eq!(search.path, "/gitlab/projects?search=finder&order_by=last_activity_at&sort=desc&per_page=30&page=3&statistics=true");
    assert_eq!(authorized(search).as_deref(), Some(format!("Bearer {TOKEN}").as_str()));
});

case!(gitlab_limits_and_refusals_are_told_apart, {
    let f = Fixture::new();
    let limited = serve(Box::new(|r| gitlab(r, 1)));
    let session = f.git.forge_public_session(config(ForgeProvider::GitLab, limited.port)).unwrap();
    let cancel = CancellationToken::new();
    assert_eq!(
        session.search_repositories("x", RepositorySearchSort::Stars, None, 1, &cancel).unwrap(),
        RepositorySearchOutcome::RateLimited { retry_after: Some(7) }
    );
    let refusing = serve(Box::new(|r| gitlab(r, 2)));
    let public = f.git.forge_public_session(config(ForgeProvider::GitLab, refusing.port)).unwrap();
    assert_eq!(public.search_repositories("x", RepositorySearchSort::Stars, None, 1, &cancel).unwrap(), RepositorySearchOutcome::SignInRequired);
    // An account whose token the server refuses is an error, not a sign-in request.
    let accepted = serve(Box::new(|r| if r.path.ends_with("/user") { gitlab(r, 0) } else { gitlab(r, 2) }));
    let signed = f.git.forge_session(config(ForgeProvider::GitLab, accepted.port), "", TOKEN, &cancel).unwrap();
    assert_eq!(failure(signed.search_repositories("x", RepositorySearchSort::Stars, None, 1, &cancel)), ForgeFailure::Unauthorized);
});

case!(gitea_reads_ok_and_data_and_a_server_that_wants_a_sign_in, {
    let f = Fixture::new();
    let server = serve(Box::new(|r| gitea(r, 0)));
    let cancel = CancellationToken::new();
    let session = f.git.forge_public_session(config(ForgeProvider::Gitea, server.port)).unwrap();
    let result = page(session.search_repositories("static site", RepositorySearchSort::Stars, None, 1, &cancel).unwrap());
    assert_eq!((result.total, result.next_page, result.items.len()), (Some(57), Some(2), 1), "57 in all, so a second page");
    assert_eq!(result.items[0].language.as_deref(), Some("Go"));
    assert_eq!(result.items[0].size_kb, Some(12288));
    let seen = server.seen.lock().unwrap().clone();
    assert_eq!(seen[0].path, "/gitea/repos/search?q=static%20site&includeDesc=true&sort=stars&order=desc&limit=30&page=1");
    assert_eq!(authorized(&seen[0]), None);
    assert!(session.search_repositories("site", RepositorySearchSort::Stars, Some("Go"), 1, &cancel).is_err(), "Gitea can't filter by language");
    let closed = serve(Box::new(|r| gitea(r, 1)));
    let public = f.git.forge_public_session(config(ForgeProvider::Gitea, closed.port)).unwrap();
    assert_eq!(public.search_repositories("site", RepositorySearchSort::Stars, None, 1, &cancel).unwrap(), RepositorySearchOutcome::SignInRequired);
});

case!(sessions_without_a_token_are_only_for_gitlab_and_gitea, {
    let f = Fixture::new();
    for provider in [ForgeProvider::GitHub, ForgeProvider::Bitbucket, ForgeProvider::Azure] {
        assert!(f.git.forge_public_session(config(provider, 4321)).is_err(), "{provider:?}");
    }
});
