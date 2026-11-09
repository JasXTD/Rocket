use super::*;

#[test]
fn gitlab_search_preserves_literal_filters_and_binds_every_page() {
    isolated(
        "ROCKET_FORGE_CASE",
        "gitlab_search_preserves_literal_filters_and_binds_every_page",
        || {
            let f = Fixture::new();
            let server = serve(Box::new(|r| {
                if r.path.ends_with("/user") {
                    return response(r#"{"username":"fixture"}"#);
                }
                assert!(
                    r.path
                        .starts_with("/gitlab/projects/team%2F%E8%B3%87%E6%96%99/"),
                    "{}",
                    r.path
                );
                assert!(r.path.contains("scope=all&state=all"));
                assert!(
                    r.path
                        .contains("search=%E7%95%8C%3A%20%22literal%22%20%26state%3Dall")
                );
                assert!(r.path.contains("author_username=author.name"));
                assert!(r.path.contains("assignee_username[]=reviewer_one"));
                assert!(r.path.contains("in=title,description"));
                let second = r.path.contains("&page=2&");
                (
                    200,
                    format!(
                        "[{}]",
                        item(ForgeProvider::GitLab, if second { 51 } else { 1 })
                    ),
                    vec![("x-next-page".into(), if second { "" } else { "2" }.into())],
                )
            }));
            let cfg = config(ForgeProvider::GitLab, server.port);
            let c = CancellationToken::new();
            let s = f.git.forge_session(cfg.clone(), "", TOKEN, &c).unwrap();
            let repo = ForgeRepository::new(cfg.clone(), "team/資料").unwrap();
            let mut filter = ForgeFilter {
                state: ForgeState::All,
                search: ForgeSearch {
                    text: "界: \"literal\" &state=all".into(),
                    author: "author.name".into(),
                    assignee: "reviewer_one".into(),
                },
                ..Default::default()
            };
            for kind in [ForgeKind::Pulls, ForgeKind::Issues] {
                filter.kind = kind;
                let first = s.list(&repo, &filter, ForgeCursor::default(), &c).unwrap();
                assert_eq!(first.items[0].number, 1);
                let next = first.next.unwrap();
                let second = s.list(&repo, &filter, next.clone(), &c).unwrap();
                assert_eq!(second.items[0].number, 51);
                assert!(second.next.is_none());
                let count = server.seen.lock().unwrap().len();
                let mut other = filter.clone();
                other.search.author = "different".into();
                assert_eq!(
                    failure(s.list(&repo, &other, next.clone(), &c)),
                    ForgeFailure::InvalidPagination
                );
                let other_repo = ForgeRepository::new(cfg.clone(), "team/elsewhere").unwrap();
                assert_eq!(
                    failure(s.list(&other_repo, &filter, next.clone(), &c)),
                    ForgeFailure::InvalidPagination
                );
                assert_eq!(server.seen.lock().unwrap().len(), count);
                let s2 = f.git.forge_session(cfg.clone(), "", TOKEN, &c).unwrap();
                assert_eq!(
                    failure(s2.list(&repo, &filter, next, &c)),
                    ForgeFailure::InvalidPagination
                );
            }
            let before = server.seen.lock().unwrap().len();
            c.cancel();
            assert!(matches!(
                s.list(&repo, &filter, ForgeCursor::default(), &c),
                Err(Error::Cancelled)
            ));
            filter.search.author = "user&scope=all".into();
            assert!(matches!(
                s.list(
                    &repo,
                    &filter,
                    ForgeCursor::default(),
                    &CancellationToken::new()
                ),
                Err(Error::InvalidInput(_))
            ));
            assert_eq!(server.seen.lock().unwrap().len(), before);
        },
    );
}

#[test]
fn gitlab_search_rejects_redirected_pagination_auth_errors_and_bad_responses() {
    isolated(
        "ROCKET_FORGE_CASE",
        "gitlab_search_rejects_redirected_pagination_auth_errors_and_bad_responses",
        || {
            let f = Fixture::new();
            let mode = Arc::new(AtomicUsize::new(0));
            let m = mode.clone();
            let server = serve(Box::new(move |r| {
                if r.path.ends_with("/user") {
                    return response(r#"{"username":"fixture"}"#);
                }
                match m.load(Ordering::Acquire) {
                1 => (401,format!(r#"{{"message":"{TOKEN}"}}"#),vec![]),
                2 => (429,"{}".into(),vec![]),
                3 => response("{}"),
                4 => (200,"[]".into(),vec![("Link".into(),"<https://elsewhere.invalid/projects/other/issues?page=2>; rel=\"next\"".into())]),
                5 => (200,"[]".into(),vec![("x-next-page".into(),"9".into())]),
                _ => response(&format!("[{},{}]",item(ForgeProvider::GitLab,1),item(ForgeProvider::GitLab,2).replace("opened","merged"))),
            }
            }));
            let cfg = config(ForgeProvider::GitLab, server.port);
            let c = CancellationToken::new();
            let s = f.git.forge_session(cfg.clone(), "", TOKEN, &c).unwrap();
            let repo = ForgeRepository::new(cfg, "team/project").unwrap();
            let filter = ForgeFilter {
                state: ForgeState::Closed,
                search: ForgeSearch {
                    text: "query".into(),
                    ..Default::default()
                },
                ..Default::default()
            };
            let page = s.list(&repo, &filter, ForgeCursor::default(), &c).unwrap();
            assert_eq!(page.items.len(), 1);
            assert_eq!(page.items[0].state, "merged");
            for (n, expected) in [
                (1, ForgeFailure::Unauthorized),
                (2, ForgeFailure::RateLimited),
                (3, ForgeFailure::InvalidResponse),
                (4, ForgeFailure::InvalidPagination),
                (5, ForgeFailure::InvalidPagination),
            ] {
                mode.store(n, Ordering::Release);
                assert_eq!(
                    failure(s.list(&repo, &filter, ForgeCursor::default(), &c)),
                    expected
                );
            }
        },
    );
}
