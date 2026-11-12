use super::*;

const CREATED: u64 = 1_709_251_200; // 2024-03-01T00:00:00Z
const NOW: u64 = CREATED + 86_400;

fn pull(number: u64) -> ForgeItem {
    ForgeItem {
        timing: Some(ForgePullTiming {
            created: Some(CREATED),
            open: true,
            ..Default::default()
        }),
        reviewers: None,
        number,
        title: "PRIVATE 界".into(),
        state: "open".into(),
        author: "fixture".into(),
        source: "feature/界".into(),
        target: "main".into(),
        url: String::new(),
    }
}
fn reads(server: &Server) -> Vec<String> {
    server
        .seen
        .lock()
        .unwrap()
        .iter()
        .filter(|r| r.path.contains("/reviews"))
        .map(|r| r.path.clone())
        .collect()
}

#[test]
fn insights_first_review_reads_are_bounded_per_host() {
    isolated(
        "ROCKET_FORGE_CASE",
        "insights_first_review_reads_are_bounded_per_host",
        || {
            let f = Fixture::new();
            let cancel = CancellationToken::new();
            // GitHub: one page of 100 even when the host offers more, because reviews
            // come oldest first.
            let github = serve(Box::new(|r| {
                if r.path.ends_with("/user") {
                    return response(r#"{"login":"fixture"}"#);
                }
                assert!(r.path.contains("/pulls/7/reviews?per_page=100&page=1"), "{}", r.path);
                let next = format!("{}{}", fixture_origin(r), r.path.replace("page=1", "page=2"));
                (
                    200,
                    r#"[{"id":1,"state":"COMMENTED","user":{"login":"fixture"},"submitted_at":"2024-03-01T00:01:00Z"},
                        {"id":2,"state":"APPROVED","user":{"login":"copilot[bot]","type":"Bot"},"submitted_at":"2024-03-01T00:02:00Z"},
                        {"id":3,"state":"CHANGES_REQUESTED","user":{"login":"Bob"},"submitted_at":"2024-03-01T02:00:00Z"}]"#
                        .into(),
                    vec![("Link".into(), format!("<{next}>; rel=\"next\""))],
                )
            }));
            let cfg = config(ForgeProvider::GitHub, github.port);
            let session = f.git.forge_session(cfg.clone(), "fixture@example.invalid", TOKEN, &cancel).unwrap();
            let repo = ForgeRepository::new(cfg, "team/資料").unwrap();
            let read = session.first_review(&repo, &pull(7), NOW, &cancel).unwrap();
            assert_eq!(read.first, Some(("Bob".to_owned(), CREATED + 7200)));
            assert_eq!(read.reviewers, ["copilot[bot]", "Bob"]);
            assert!(!read.limited && !read.undated);
            assert_eq!(reads(&github).len(), 1, "GitHub's first page holds the first review");

            // GitHub pages read on only while no review counts: #8's first page has the
            // author and a bot, its second Bob; #10 has only bots on three pages and
            // more after them; #11's one review is newer than the clock allows; #12's
            // newer review doesn't spoil Bob's earlier one.
            let paged = serve(Box::new(|r| {
                if r.path.ends_with("/user") {
                    return response(r#"{"login":"fixture"}"#);
                }
                let number: u64 = r.path.split("/pulls/").nth(1).unwrap().split('/').next().unwrap().parse().unwrap();
                let page: u32 = r.path.rsplit("page=").next().unwrap().parse().unwrap();
                assert!(page <= 3, "never past three pages: {}", r.path);
                let next = format!("{}{}", fixture_origin(r), r.path.replace(&format!("page={page}"), &format!("page={}", page + 1)));
                let link = vec![("Link".into(), format!("<{next}>; rel=\"next\""))];
                let bot = r#"{"id":1,"state":"COMMENTED","user":{"login":"helper[bot]","type":"Bot"},"submitted_at":"2024-03-01T00:01:00Z"}"#;
                match (number, page) {
                    (8, 1) => (200, format!(r#"[{bot},{{"id":2,"state":"COMMENTED","user":{{"login":"fixture"}},"submitted_at":"2024-03-01T00:02:00Z"}}]"#), link),
                    (8, _) => (200, r#"[{"id":3,"state":"APPROVED","user":{"login":"Bob"},"submitted_at":"2024-03-01T03:00:00Z"}]"#.into(), Vec::new()),
                    (10, _) => (200, format!("[{bot}]"), link),
                    (11, _) => (200, r#"[{"id":4,"state":"APPROVED","user":{"login":"Cy"},"submitted_at":"2024-03-05T00:00:00Z"}]"#.into(), Vec::new()),
                    _ => (200, r#"[{"id":5,"state":"APPROVED","user":{"login":"Cy"},"submitted_at":"2024-03-05T00:00:00Z"},
                        {"id":6,"state":"APPROVED","user":{"login":"Bob"},"submitted_at":"2024-03-01T01:00:00Z"}]"#.into(), Vec::new()),
                }
            }));
            let cfg = config(ForgeProvider::GitHub, paged.port);
            let session = f.git.forge_session(cfg.clone(), "fixture@example.invalid", TOKEN, &cancel).unwrap();
            let repo = ForgeRepository::new(cfg, "team/資料").unwrap();
            let read = session.first_review(&repo, &pull(8), NOW, &cancel).unwrap();
            assert_eq!((read.first, read.limited), (Some(("Bob".to_owned(), CREATED + 3 * 3600)), false));
            let read = session.first_review(&repo, &pull(10), NOW, &cancel).unwrap();
            assert_eq!((read.first, read.limited, read.undated), (None, true, false), "no review counted in three pages");
            let read = session.first_review(&repo, &pull(11), NOW, &cancel).unwrap();
            assert_eq!((read.first, read.undated), (None, true), "only a review newer than the clock");
            let read = session.first_review(&repo, &pull(12), NOW, &cancel).unwrap();
            assert_eq!((read.first, read.undated), (Some(("Bob".to_owned(), CREATED + 3600)), false));
            let pages = |n: u64| reads(&paged).iter().filter(|p| p.contains(&format!("/pulls/{n}/"))).count();
            assert_eq!((pages(8), pages(10), pages(11), pages(12)), (2, 3, 1, 1));

            // Gitea: two pages of 50 at most; a third would be needed, so the first
            // review cannot be told for sure.
            let gitea = serve(Box::new(|r| {
                if r.path.ends_with("/user") {
                    return response(r#"{"login":"fixture"}"#);
                }
                assert!(r.path.contains("/pulls/9/reviews?limit=50&page="), "{}", r.path);
                let page: u32 = r.path.rsplit("page=").next().unwrap().parse().unwrap();
                assert!(page <= 2, "never past the bound: {}", r.path);
                let next = format!("{}{}", fixture_origin(r), r.path.replace(&format!("page={page}"), &format!("page={}", page + 1)));
                let body = if page == 1 {
                    r#"[{"id":1,"state":"REQUEST_REVIEW","user":{"login":"Cy"}},{"id":2,"state":"COMMENT","user":{"login":"Cy"},"submitted_at":"2024-03-01T03:00:00Z"}]"#
                } else {
                    r#"[{"id":3,"state":"APPROVED","user":{"login":"Dee"},"submitted_at":"2024-03-01T01:00:00Z"}]"#
                };
                (200, body.into(), vec![("Link".into(), format!("<{next}>; rel=\"next\""))])
            }));
            let cfg = config(ForgeProvider::Gitea, gitea.port);
            let session = f.git.forge_session(cfg.clone(), "fixture@example.invalid", TOKEN, &cancel).unwrap();
            let repo = ForgeRepository::new(cfg, "team/資料").unwrap();
            let read = session.first_review(&repo, &pull(9), NOW, &cancel).unwrap();
            assert_eq!(read.first, Some(("Dee".to_owned(), CREATED + 3600)));
            assert!(read.limited, "more reviews than were read on a host without an order");
            assert_eq!(reads(&gitea).len(), 2);

            // A host without review times sends nothing.
            let gitlab = serve(Box::new(|r| {
                if r.path.ends_with("/user") {
                    return response(r#"{"username":"fixture"}"#);
                }
                panic!("unexpected request {}", r.path)
            }));
            let cfg = config(ForgeProvider::GitLab, gitlab.port);
            let session = f.git.forge_session(cfg.clone(), "fixture@example.invalid", TOKEN, &cancel).unwrap();
            let repo = ForgeRepository::new(cfg, "team/資料").unwrap();
            assert_eq!(failure(session.first_review(&repo, &pull(7), NOW, &cancel)), ForgeFailure::Unsupported);
            assert!(reads(&gitlab).is_empty());
        },
    );
}

#[test]
fn insights_bitbucket_cloud_asks_for_reviewers_only_when_told() {
    isolated(
        "ROCKET_FORGE_CASE",
        "insights_bitbucket_cloud_asks_for_reviewers_only_when_told",
        || {
            let f = Fixture::new();
            let server = serve(Box::new(|r| {
                if r.path.ends_with("/user") {
                    return response(r#"{"display_name":"fixture"}"#);
                }
                let asked = r.path.contains("&fields=%2Bvalues.reviewers,%2Bvalues.participants");
                let extra = if asked {
                    r#","reviewers":[{"display_name":"Bob","uuid":"{1}"},{"display_name":"Cy","uuid":"{2}"}],"participants":[{"user":{"display_name":"Bob","uuid":"{1}"},"role":"REVIEWER","approved":true,"state":"approved"}]"#
                } else {
                    ""
                };
                response(&format!(
                    r#"{{"values":[{{"id":4,"title":"PRIVATE 界","state":"OPEN","author":{{"display_name":"fixture"}},"source":{{"branch":{{"name":"feature/界"}}}},"destination":{{"branch":{{"name":"main"}}}},"created_on":"2024-03-01T00:00:00Z"{extra}}}]}}"#
                ))
            }));
            let cfg = config(ForgeProvider::Bitbucket, server.port);
            let cancel = CancellationToken::new();
            let session = f.git.forge_session(cfg.clone(), "fixture@example.invalid", TOKEN, &cancel).unwrap();
            let repo = ForgeRepository::new(cfg, "team/資料").unwrap();
            let plain = session.list(&repo, &ForgeFilter::default(), ForgeCursor::default(), &cancel).unwrap();
            assert_eq!(plain.items[0].reviewers, None, "the Hosting list does not pay for reviewers");
            let filter = ForgeFilter { reviewers: true, ..Default::default() };
            let page = session.list(&repo, &filter, ForgeCursor::default(), &cancel).unwrap();
            let reviewers = page.items[0].reviewers.clone().expect("asked for");
            assert!(reviewers.known);
            let names = |list: &[ForgeReviewer]| list.iter().map(|r| r.name.clone()).collect::<Vec<_>>();
            assert_eq!((names(&reviewers.waiting), names(&reviewers.reviewed)), (vec!["Cy".to_owned()], vec!["Bob".to_owned()]));
            let paths: Vec<String> = server.seen.lock().unwrap().iter().map(|r| r.path.clone()).collect();
            assert_eq!(paths.iter().filter(|p| p.contains("fields=")).count(), 1, "{paths:?}");
        },
    );
}
