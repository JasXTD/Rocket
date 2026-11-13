#![cfg(windows)]
#[allow(dead_code)]
mod common;
#[path = "forge/bitbucket.rs"]
mod bitbucket_tests;
#[path = "forge/pull_review.rs"]
mod pull_review_tests;
#[path = "forge/pr_workflows.rs"]
mod pr_workflow_tests;
#[path = "forge/enterprise.rs"]
mod enterprise_tests;
#[path = "forge/azure_server.rs"]
mod azure_server_tests;
#[path = "forge/pr_collaboration.rs"]
mod pr_collaboration_tests;
#[path = "forge/ci_checks.rs"]
mod ci_checks_tests;
#[path = "forge/search_gitlab.rs"]
mod search_gitlab_tests;
#[path = "forge/insights_reviews.rs"]
mod insights_reviews_tests;
#[path = "forge/provider_contracts.rs"]
mod provider_contract_tests;
use common::{Fixture, isolated};
use rocket_git::*;
use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    thread,
    time::{Duration, Instant},
};
const TOKEN: &str = "fixture-token-DO-NOT-LOG";
#[derive(Clone)]
struct Request {
    method: String,
    path: String,
    head: String,
    body: String,
}
type Route = Box<dyn Fn(&Request) -> (u16, String, Vec<(String, String)>) + Send>;
struct Server {
    port: u16,
    seen: Arc<Mutex<Vec<Request>>>,
    stop: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}
impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        let _ = TcpStream::connect(("127.0.0.1", self.port));
        if let Some(t) = self.thread.take() {
            t.join().unwrap();
        }
    }
}
fn serve(route: Route) -> Server {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = l.local_addr().unwrap().port();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let requests = seen.clone();
    let stop = Arc::new(AtomicBool::new(false));
    let stopping = stop.clone();
    let worker = thread::spawn(move || {
        for stream in l.incoming() {
            if stopping.load(Ordering::Acquire) {
                break;
            }
            let Ok(mut s) = stream else { continue };
            s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
            let mut bytes = Vec::new();
            let mut byte = [0];
            while !bytes.ends_with(b"\r\n\r\n") && bytes.len() < 65536 {
                if !matches!(s.read(&mut byte), Ok(1)) {
                    break;
                }
                bytes.push(byte[0]);
            }
            let head = String::from_utf8(bytes).unwrap();
            let first = head
                .lines()
                .next()
                .unwrap_or("")
                .split_whitespace()
                .collect::<Vec<_>>();
            let size = head
                .lines()
                .find_map(|line| {
                    line.to_ascii_lowercase()
                        .strip_prefix("content-length:")
                        .and_then(|s| s.trim().parse::<usize>().ok())
                })
                .unwrap_or(0);
            assert!(size < 128 * 1024);
            let mut body = vec![0; size];
            if s.read_exact(&mut body).is_err() {
                continue;
            }
            let req = Request {
                method: first.first().unwrap_or(&"").to_string(),
                path: first.get(1).unwrap_or(&"").to_string(),
                head: head.clone(),
                body: String::from_utf8(body).unwrap(),
            };
            requests.lock().unwrap().push(req.clone());
            let (status, body, headers) = route(&req);
            let mut reply = format!(
                "HTTP/1.1 {status} Fixture\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n",
                body.len()
            );
            for (k, v) in headers {
                reply.push_str(&format!("{k}: {v}\r\n"));
            }
            reply.push_str("\r\n");
            reply.push_str(&body);
            let _ = s.write_all(reply.as_bytes());
        }
    });
    Server {
        port,
        seen,
        stop,
        thread: Some(worker),
    }
}
fn response(s: &str) -> (u16, String, Vec<(String, String)>) {
    (200, s.into(), Vec::new())
}
fn item(p: ForgeProvider, n: u64) -> String {
    match p {
        ForgeProvider::GitHub | ForgeProvider::GitHubEnterprise | ForgeProvider::Gitea => format!(
            r#"{{"number":{n},"title":"PRIVATE 界 🚀","state":"open","user":{{"login":"fixture"}},"head":{{"ref":"feature/界"}},"base":{{"ref":"main"}}}}"#
        ),
        ForgeProvider::GitLab => format!(
            r#"{{"iid":{n},"title":"PRIVATE 界 🚀","state":"opened","author":{{"username":"fixture"}},"source_branch":"feature/界","target_branch":"main"}}"#
        ),
        ForgeProvider::Bitbucket => format!(
            r#"{{"id":{n},"title":"PRIVATE 界 🚀","state":"OPEN","author":{{"display_name":"fixture"}},"source":{{"branch":{{"name":"feature/界"}}}},"destination":{{"branch":{{"name":"main"}}}}}}"#
        ),
        ForgeProvider::Azure | ForgeProvider::AzureServer => format!(
            r#"{{"pullRequestId":{n},"title":"PRIVATE 界 🚀","status":"active","createdBy":{{"displayName":"fixture"}},"sourceRefName":"refs/heads/feature/界","targetRefName":"refs/heads/main"}}"#
        ),
        ForgeProvider::BitbucketServer => format!(r#"{{"id":{n},"title":"PRIVATE 界 🚀","state":"OPEN","author":{{"user":{{"displayName":"fixture"}}}},"fromRef":{{"id":"refs/heads/feature/界"}},"toRef":{{"id":"refs/heads/main"}}}}"#),
    }
}
fn route(p: ForgeProvider, req: &Request, mode: usize) -> (u16, String, Vec<(String, String)>) {
    if p == ForgeProvider::BitbucketServer && req.path.ends_with("/application-properties") {
        return (200, "{}".into(), vec![("X-AUSERNAME".into(),"fixture".into())]);
    }
    if req.path.ends_with("/user") || req.path.contains("connectionData") {
        return response(match p {
            ForgeProvider::GitLab => r#"{"username":"fixture"}"#,
            ForgeProvider::Bitbucket => r#"{"display_name":"fixture"}"#,
            ForgeProvider::AzureServer => r#"{"authenticatedUser":{"id":"11111111-2222-3333-4444-555555555555","isActive":true,"isContainer":false,"providerDisplayName":"fixture"}}"#,
            ForgeProvider::Azure => r#"{"authenticatedUser":{"providerDisplayName":"fixture"}}"#,
            _ => r#"{"login":"fixture"}"#,
        });
    }
    if req.path.contains("/branches/") || req.path.contains("/refs?") || req.path.contains("/branches?") {
        let main = req.path.contains("main");
        let id = if mode == 9 && !main {
            "c".repeat(40)
        } else {
            if main { "b" } else { "a" }.repeat(40)
        };
        return response(&match p {
            ForgeProvider::GitHub | ForgeProvider::GitHubEnterprise => format!(r#"{{"commit":{{"sha":"{id}"}}}}"#),
            ForgeProvider::GitLab | ForgeProvider::Gitea => {
                format!(r#"{{"commit":{{"id":"{id}"}}}}"#)
            }
            ForgeProvider::Bitbucket => format!(r#"{{"target":{{"hash":"{id}"}}}}"#),
            ForgeProvider::BitbucketServer => format!(r#"{{"values":[{{"id":"refs/heads/{}","latestCommit":"{id}"}}],"isLastPage":true,"start":0}}"#,if main {"main"}else{"feature/界"}),
            ForgeProvider::Azure | ForgeProvider::AzureServer => format!(
                r#"{{"value":[{{"name":"refs/heads/{}","objectId":"{id}"}}]}}"#,
                if main { "main" } else { "feature/界" }
            ),
        });
    }
    if mode == 8 {
        thread::sleep(Duration::from_secs(2));
    }
    if mode == 7 {
        return (
            302,
            "{}".into(),
            vec![("Location".into(), "http://127.0.0.1:1/stolen".into())],
        );
    }
    if (400..=599).contains(&mode) {
        return (
            mode as u16,
            format!(r#"{{"message":"{TOKEN}"}}"#),
            Vec::new(),
        );
    }
    if req.method == "POST" {
        return (201, item(p, 43), Vec::new());
    }
    let one = item(p, 42);
    response(&match p {
        ForgeProvider::Bitbucket => {
            let endpoint = req.path.split('?').next().unwrap();
            format!(
                r#"{{"values":[{one}],"next":"{}{endpoint}?cursor=next"}}"#,
                fixture_origin(req)
            )
        }
        ForgeProvider::Azure | ForgeProvider::AzureServer => format!(r#"{{"value":[{one}]}}"#),
        ForgeProvider::BitbucketServer => format!(r#"{{"values":[{one}],"isLastPage":true,"start":0}}"#),
        _ => format!("[{one}]"),
    })
}
fn config(p: ForgeProvider, port: u16) -> ForgeConfig {
    ForgeConfig::new(
        p,
        if p == ForgeProvider::Azure {
            "https://dev.azure.com/fixture"
        } else {
            p.default_server()
        },
    )
    .unwrap()
    .with_fixture(port)
    .unwrap()
}
fn draft(c: ForgeConfig) -> ForgeDraft {
    ForgeDraft {
        repository: ForgeRepository::new(c, "team/資料").unwrap(),
        source: "feature/界".into(),
        target: "main".into(),
        title: "PRIVATE 界 🚀".into(),
        body: "Quoted \"text\"\nSecond line \\ path".into(),
    }
}
fn failure<T: std::fmt::Debug>(result: Result<T>) -> ForgeFailure {
    match result {
        Err(Error::Forge(f)) => f,
        other => panic!("expected hosting failure, got {other:?}"),
    }
}

#[test]
fn security_display_bounds_provider_result_fields() {
    isolated("ROCKET_FORGE_CASE", "security_display_bounds_provider_result_fields", || {
        let f = Fixture::new();
        let cancel = CancellationToken::new();
        for provider in ForgeProvider::ALL {
            let server = serve(Box::new(move |request| {
                let (status, mut body, headers) = route(provider, request, 0);
                if body.contains("PRIVATE 界 🚀") {
                    body = body.replace("PRIVATE 界 🚀", &"t".repeat(4096))
                        .replace("\"fixture\"", &format!("\"{}\"", "a".repeat(8192)))
                        .replace("feature/界", &"s".repeat(8192))
                        .replace("main", &"b".repeat(8192));
                    for state in ["open", "opened", "OPEN", "active"] {
                        body = body.replace(&format!("\"{state}\""), &format!("\"{}\"", "v".repeat(8192)));
                    }
                }
                (status, body, headers)
            }));
            let config = config(provider, server.port);
            let session = f.git.forge_session(config.clone(), if provider == ForgeProvider::BitbucketServer {"fixture"}else{"fixture@example.invalid"}, TOKEN, &cancel).unwrap();
            let repository = draft(config).repository;
            if provider == ForgeProvider::BitbucketServer {
                assert_eq!(failure(session.list(&repository, &ForgeFilter::default(), ForgeCursor::default(), &cancel)),ForgeFailure::InvalidResponse);
                continue; // Server states are an enum; unknown states are rejected, not displayed.
            }
            let page = session.list(&repository, &ForgeFilter::default(), ForgeCursor::default(), &cancel).unwrap();
            let item = &page.items[0];
            for (field, maximum) in [(&item.title, 1024), (&item.author, 256), (&item.state, 64), (&item.source, 1024), (&item.target, 1024)] {
                assert!(field.len() <= maximum, "{provider:?}: field exceeds {maximum} bytes");
            }
            assert_eq!(item.url, repository.item_url(ForgeKind::Pulls, item.number));
        }
    });
}

#[test]
fn security_display_controls_are_visible_for_every_provider() {
    isolated("ROCKET_FORGE_CASE", "security_display_controls_are_visible_for_every_provider", || {
        let f = Fixture::new();
        let cancel = CancellationToken::new();
        for provider in ForgeProvider::ALL {
            let server = serve(Box::new(move |request| {
                let (status, mut body, headers) = route(provider, request, 0);
                if body.contains("PRIVATE 界 🚀") {
                    let hostile = r"visible\u202e\u2066\u061c\u200f\n\t";
                    body = body.replace("PRIVATE 界 🚀", hostile)
                        .replace("\"fixture\"", &format!("\"{hostile}\""))
                        .replace("feature/界", hostile).replace("main", hostile);
                    for state in ["open", "opened", "OPEN", "active"] {
                        body = body.replace(&format!("\"{state}\""), &format!("\"{hostile}\""));
                    }
                }
                (status, body, headers)
            }));
            let config = config(provider, server.port);
            let session = f.git.forge_session(config.clone(), if provider == ForgeProvider::BitbucketServer {"fixture"}else{"fixture@example.invalid"}, TOKEN, &cancel).unwrap();
            let repository = draft(config).repository;
            if provider == ForgeProvider::BitbucketServer {
                assert_eq!(failure(session.list(&repository, &ForgeFilter::default(), ForgeCursor::default(), &cancel)),ForgeFailure::InvalidResponse);
                continue;
            }
            let page = session.list(&repository, &ForgeFilter::default(), ForgeCursor::default(), &cancel).unwrap();
            let item = &page.items[0];
            for field in [&item.title, &item.author, &item.state, &item.source, &item.target] {
                assert!(field.contains("\\u{202e}"), "{provider:?}: hidden characters must be visible");
                assert!(!field.chars().any(|c| c.is_control() || matches!(c, '\u{202e}' | '\u{2066}' | '\u{61c}' | '\u{200f}')));
            }
            assert_eq!(item.url, repository.item_url(ForgeKind::Pulls, item.number));
        }
    });
}

#[test]
fn security_display_azure_work_item_fields_use_the_same_limits() {
    isolated("ROCKET_FORGE_CASE", "security_display_azure_work_item_fields_use_the_same_limits", || {
        let f = Fixture::new();
        let cancel = CancellationToken::new();
        let server = serve(Box::new(|request| {
            if request.path.contains("/_apis/wit/wiql?") {
                return response(r#"{"workItems":[{"id":1}]}"#);
            }
            if request.path.contains("/_apis/wit/workitems?") {
                let hostile = format!("\\u202e{}", "x".repeat(4096));
                return response(&format!(r#"{{"value":[{{"id":1,"fields":{{"System.Title":"{hostile}","System.State":"{hostile}","System.CreatedBy":{{"displayName":"{hostile}"}}}}}}]}}"#));
            }
            route(ForgeProvider::Azure, request, 0)
        }));
        let config = config(ForgeProvider::Azure, server.port);
        let session = f.git.forge_session(config.clone(), "fixture@example.invalid", TOKEN, &cancel).unwrap();
        let repository = draft(config).repository;
        let filter = ForgeFilter { kind: ForgeKind::Issues, ..Default::default() };
        let page = session.list(&repository, &filter, ForgeCursor::default(), &cancel).unwrap();
        let item = &page.items[0];
        for (field, limit) in [(&item.title, 1024), (&item.author, 256), (&item.state, 64)] {
            assert!(field.len() <= limit && field.starts_with("\\u{202e}"));
            assert!(!field.contains('\u{202e}'));
        }
        assert_eq!(item.url, repository.item_url(ForgeKind::Issues, 1));
    });
}

fn fixture_origin(req: &Request) -> String {
    let host = req
        .head
        .lines()
        .find_map(|line| {
            let (key, value) = line.split_once(':')?;
            key.eq_ignore_ascii_case("host").then(|| value.trim())
        })
        .unwrap();
    format!("http://{host}")
}

#[test]
fn contract_bitbucket_uses_opaque_continuation() {
    isolated(
        "ROCKET_FORGE_CASE",
        "contract_bitbucket_uses_opaque_continuation",
        || {
            let f = Fixture::new();
            let server = serve(Box::new(|r| {
                if r.path.ends_with("/user") {
                    return response(r#"{"display_name":"fixture"}"#);
                }
                if r.path.contains("cursor=opaque%2Bnext%2Fpage") {
                    return response(&format!(
                        r#"{{"values":[{}]}}"#,
                        item(ForgeProvider::Bitbucket, 2)
                    ));
                }
                let endpoint = r.path.split('?').next().unwrap();
                let next = format!(
                    "{}{endpoint}?cursor=opaque%2Bnext%2Fpage&pagelen=1",
                    fixture_origin(r)
                );
                response(&format!(
                    r#"{{"values":[{}],"next":"{next}"}}"#,
                    item(ForgeProvider::Bitbucket, 1)
                ))
            }));
            let config = config(ForgeProvider::Bitbucket, server.port);
            let cancel = CancellationToken::new();
            let session = f
                .git
                .forge_session(config.clone(), "fixture@example.invalid", TOKEN, &cancel)
                .unwrap();
            let repo = ForgeRepository::new(config, "team/資料").unwrap();
            let page = session
                .list(
                    &repo,
                    &ForgeFilter::default(),
                    ForgeCursor::default(),
                    &cancel,
                )
                .unwrap();
            let second = session
                .list(&repo, &ForgeFilter::default(), page.next.unwrap(), &cancel)
                .unwrap();
            assert_eq!(
                second.items[0].number, 2,
                "must use the server's opaque continuation, not page=2"
            );
            assert!(second.next.is_none());
        },
    );
}

#[test]
fn contract_short_pages_follow_link_headers() {
    isolated(
        "ROCKET_FORGE_CASE",
        "contract_short_pages_follow_link_headers",
        || {
            let f = Fixture::new();
            for provider in [
                ForgeProvider::GitHub,
                ForgeProvider::GitLab,
                ForgeProvider::Gitea,
            ] {
                let server = serve(Box::new(move |r| {
                    if r.path.ends_with("/user") {
                        return route(provider, r, 0);
                    }
                    let second = r.path.contains("page=2");
                    let endpoint = r.path.split('?').next().unwrap();
                    let link = format!(
                        "<{}{endpoint}?page=2&limit=1>; rel=\"next\"",
                        fixture_origin(r)
                    );
                    (
                        200,
                        format!("[{}]", item(provider, if second { 2 } else { 1 })),
                        if second {
                            vec![]
                        } else {
                            vec![("Link".into(), link)]
                        },
                    )
                }));
                let config = config(provider, server.port);
                let cancel = CancellationToken::new();
                let session = f
                    .git
                    .forge_session(config.clone(), "", TOKEN, &cancel)
                    .unwrap();
                let repo = ForgeRepository::new(config, "team/資料").unwrap();
                let first = session
                    .list(
                        &repo,
                        &ForgeFilter::default(),
                        ForgeCursor::default(),
                        &cancel,
                    )
                    .unwrap();
                let next = first
                    .next
                    .expect("a short page with a next Link must not end the list");
                let second = session
                    .list(&repo, &ForgeFilter::default(), next, &cancel)
                    .unwrap();
                assert_eq!(second.items[0].number, 2);
                assert!(second.next.is_none());
            }
        },
    );
}

#[test]
fn contract_bitbucket_token_without_email() {
    isolated(
        "ROCKET_FORGE_CASE",
        "contract_bitbucket_token_without_email",
        || {
            let f = Fixture::new();
            let server = serve(Box::new(|r| {
                if !r.head.contains(&format!("Authorization: Bearer {TOKEN}")) {
                    return (401, "{}".into(), vec![]);
                }
                response(r#"{"display_name":"fixture"}"#)
            }));
            let session = f
                .git
                .forge_session(
                    config(ForgeProvider::Bitbucket, server.port),
                    "",
                    TOKEN,
                    &CancellationToken::new(),
                )
                .unwrap();
            assert_eq!(session.login(), "fixture");
            assert_eq!(server.seen.lock().unwrap().len(), 1);
        },
    );
}

#[test]
fn github_search_scopes_filters_pages_and_rejects_stale_context() {
    isolated("ROCKET_FORGE_CASE", "github_search_scopes_filters_pages_and_rejects_stale_context", || {
        let f = Fixture::new();
        let mode = Arc::new(AtomicUsize::new(0)); let m = mode.clone();
        let server = serve(Box::new(move |r| {
            if r.path.ends_with("/user") { return response(r#"{"login":"fixture"}"#); }
            assert!(r.path.starts_with("/github/search/issues?q="), "{}", r.path);
            assert!(r.path.contains("repo%3Ateam%2F"));
            assert!(r.path.contains("author%3Afixture"));
            assert!(r.path.contains("assignee%3Areviewer"));
            assert!(r.path.contains("is%3Apr"));
            if m.load(Ordering::Acquire) == 1 { return (403, r#"{"message":"API rate limit exceeded"}"#.into(), vec![("x-ratelimit-remaining".into(), "0".into())]); }
            let n = if r.path.ends_with("page=2") { 51 } else { 1 };
            let one = item(ForgeProvider::GitHub,n).replacen('{', r#"{"pull_request":{},"#, 1);
            response(&format!(r#"{{"total_count":83,"incomplete_results":false,"items":[{one}]}}"#))
        }));
        let cfg=config(ForgeProvider::GitHub,server.port); let cancel=CancellationToken::new();
        let session=f.git.forge_session(cfg.clone(), "", TOKEN, &cancel).unwrap();
        let repo=ForgeRepository::new(cfg.clone(),"team/資料").unwrap();
        let mut filter=ForgeFilter { search: ForgeSearch {text:"Unicode".into(),author:"fixture".into(),assignee:"reviewer".into()}, ..Default::default() };
        let page=session.list(&repo,&filter,ForgeCursor::default(),&cancel).unwrap();
        assert_eq!(page.total,Some(83));assert_eq!(page.items[0].number,1);assert!(!page.incomplete);
        let next=page.next.unwrap();
        assert_eq!(session.list(&repo,&filter,next.clone(),&cancel).unwrap().items[0].number,51);
        filter.search.author="different".into();
        assert_eq!(failure(session.list(&repo,&filter,next,&cancel)),ForgeFailure::InvalidPagination);
        let count=server.seen.lock().unwrap().len();
        filter.search.author="fixture".into(); filter.search.text="repo:other/repository".into();
        assert!(matches!(session.list(&repo,&filter,ForgeCursor::default(),&cancel),Err(Error::InvalidInput(_))));
        assert_eq!(server.seen.lock().unwrap().len(),count);
        filter.search.text="Unicode".into();
        let spaced=ForgeRepository::new(cfg,"team/space OR other").unwrap();
        assert!(matches!(session.list(&spaced,&filter,ForgeCursor::default(),&cancel),Err(Error::InvalidInput(_))));
        assert_eq!(server.seen.lock().unwrap().len(),count);
        cancel.cancel(); assert!(matches!(session.list(&repo,&filter,ForgeCursor::default(),&cancel),Err(Error::Cancelled)));
        assert_eq!(server.seen.lock().unwrap().len(),count);
        mode.store(1,Ordering::Release);
        assert_eq!(failure(session.list(&repo,&filter,ForgeCursor::default(),&CancellationToken::new())),ForgeFailure::RateLimited);
    });
}

#[test]
fn contract_pagination_rejects_changed_destinations_and_stale_context() {
    isolated(
        "ROCKET_FORGE_CASE",
        "contract_pagination_rejects_changed_destinations_and_stale_context",
        || {
            let f = Fixture::new();
            let mode = Arc::new(AtomicUsize::new(0));
            let m = mode.clone();
            let server = serve(Box::new(move |r| {
                if r.path.ends_with("/user") {
                    return response(r#"{"display_name":"fixture"}"#);
                }
                let origin = fixture_origin(r);
                let endpoint = r.path.split('?').next().unwrap();
                let good = format!("{origin}{endpoint}?cursor=PRIVATE_CURSOR");
                let next = match m.load(Ordering::Acquire) {
                    1 => format!("https://untrusted.invalid{endpoint}?cursor=PRIVATE_CURSOR"),
                    2 => format!(
                        "{origin}/bitbucket/repositories/team/other/pullrequests?cursor=PRIVATE_CURSOR"
                    ),
                    3 => format!("{origin}{endpoint}/../issues?cursor=PRIVATE_CURSOR"),
                    4 => format!("{origin}{endpoint}?cursor=PRIVATE_CURSOR#fragment"),
                    5 => format!("{origin}{}", r.path),
                    6 => format!("{good}{}", "x".repeat(17 * 1024)),
                    _ => good,
                };
                response(&format!(
                    r#"{{"values":[{}],"next":"{next}"}}"#,
                    item(ForgeProvider::Bitbucket, 1)
                ))
            }));
            let config = config(ForgeProvider::Bitbucket, server.port);
            let cancel = CancellationToken::new();
            let session = f
                .git
                .forge_session(config.clone(), "", TOKEN, &cancel)
                .unwrap();
            let repo = ForgeRepository::new(config.clone(), "team/資料").unwrap();
            let filter = ForgeFilter::default();
            let next = session
                .list(&repo, &filter, ForgeCursor::default(), &cancel)
                .unwrap()
                .next
                .unwrap();
            assert!(!format!("{next:?}").contains("PRIVATE_CURSOR"));
            let count = server.seen.lock().unwrap().len();
            let other = ForgeRepository::new(config.clone(), "team/other").unwrap();
            assert_eq!(
                failure(session.list(&other, &filter, next.clone(), &cancel)),
                ForgeFailure::InvalidPagination
            );
            let closed = ForgeFilter {
                state: ForgeState::Closed,
                ..Default::default()
            };
            assert_eq!(
                failure(session.list(&repo, &closed, next.clone(), &cancel)),
                ForgeFailure::InvalidPagination
            );
            let stopped = CancellationToken::new();
            stopped.cancel();
            assert!(matches!(
                session.list(&repo, &filter, next.clone(), &stopped),
                Err(Error::Cancelled)
            ));
            assert_eq!(
                server.seen.lock().unwrap().len(),
                count,
                "stale/cancelled cursors must not contact a server"
            );
            let other_session = f.git.forge_session(config, "", TOKEN, &cancel).unwrap();
            let count = server.seen.lock().unwrap().len();
            assert_eq!(
                failure(other_session.list(&repo, &filter, next, &cancel)),
                ForgeFailure::InvalidPagination
            );
            assert_eq!(server.seen.lock().unwrap().len(), count);
            for value in 1..=6 {
                mode.store(value, Ordering::Release);
                let before = server.seen.lock().unwrap().len();
                let error = session
                    .list(&repo, &filter, ForgeCursor::default(), &cancel)
                    .unwrap_err();
                assert!(!error.to_string().contains("PRIVATE_CURSOR"));
                assert_eq!(
                    failure::<ForgePage>(Err(error)),
                    ForgeFailure::InvalidPagination
                );
                assert_eq!(server.seen.lock().unwrap().len(), before + 1);
            }
        },
    );
}
#[test]
fn all_provider_routes_create_and_privacy() {
    isolated(
        "ROCKET_FORGE_CASE",
        "all_provider_routes_create_and_privacy",
        || {
            let f = Fixture::new();
            let c = CancellationToken::new();
            let log = DiagnosticLog::open(f.root.join("forge-logs")).unwrap();
            for p in ForgeProvider::ALL {
                let server = serve(Box::new(move |r| route(p, r, 0)));
                let config = config(p, server.port);
                let s = log
                    .request("hosting-connect", |l| {
                        f.git.with_log(l.clone()).forge_session(
                            config.clone(),
                            if p == ForgeProvider::BitbucketServer { "fixture" } else { "fixture@example.invalid" },
                            TOKEN,
                            &c,
                        )
                    })
                    .unwrap();
                assert_eq!(s.login(), "fixture");
                assert!(!format!("{s:?}").contains(TOKEN));
                let d = draft(config);
                let page = s
                    .list(
                        &d.repository,
                        &ForgeFilter::default(),
                        ForgeCursor::default(),
                        &c,
                    )
                    .unwrap();
                assert_eq!(page.items.len(), 1);
                assert_eq!(page.items[0].title, "PRIVATE 界 🚀");
                assert!(!page.items[0].url.contains("untrusted"));
                let preview = log
                    .request("hosting-preview", |l| {
                        s.with_log(l.clone()).preview(d.clone(), &c)
                    })
                    .unwrap();
                let created = log
                    .request("hosting-create", |l| {
                        s.with_log(l.clone()).create(&preview, &c)
                    })
                    .unwrap();
                assert_eq!(created.number, 43);
                assert_eq!(failure(s.create(&preview, &c)), ForgeFailure::StaleReview);
                let seen = server.seen.lock().unwrap();
                assert_eq!(seen.iter().filter(|r| r.method == "POST").count(), 1);
                let post = seen.iter().find(|r| r.method == "POST").unwrap();
                // Contract expectations come from the provider API docs, not the
                // fixture router (which deliberately accepts unknown paths).
                let (creation_path, source_field, target_field) = match p {
                    ForgeProvider::GitHub => (
                        "/github/repos/team/%E8%B3%87%E6%96%99/pulls",
                        r#""head":"feature/界""#,
                        r#""base":"main""#,
                    ),
                    ForgeProvider::GitHubEnterprise => (
                        "/github-enterprise/api/v3/repos/team/%E8%B3%87%E6%96%99/pulls",
                        r#""head":"feature/界""#,
                        r#""base":"main""#,
                    ),
                    ForgeProvider::GitLab => (
                        "/gitlab/projects/team%2F%E8%B3%87%E6%96%99/merge_requests",
                        r#""source_branch":"feature/界""#,
                        r#""target_branch":"main""#,
                    ),
                    ForgeProvider::Bitbucket => (
                        "/bitbucket/repositories/team/%E8%B3%87%E6%96%99/pullrequests",
                        r#""source":{"branch":{"name":"feature/界"}}"#,
                        r#""destination":{"branch":{"name":"main"}}"#,
                    ),
                    ForgeProvider::BitbucketServer => (
                        "/bitbucket-server/projects/team/repos/%E8%B3%87%E6%96%99/pull-requests",
                        r#""fromRef":{"id":"refs/heads/feature/界""#,
                        r#""toRef":{"id":"refs/heads/main""#,
                    ),
                    ForgeProvider::Azure => (
                        "/azure/team/_apis/git/repositories/%E8%B3%87%E6%96%99/pullrequests?api-version=7.1",
                        r#""sourceRefName":"refs/heads/feature/界""#,
                        r#""targetRefName":"refs/heads/main""#,
                    ),
                    ForgeProvider::AzureServer => (
                        "/azure-server/tfs/DefaultCollection/team/_apis/git/repositories/%E8%B3%87%E6%96%99/pullrequests?api-version=7.0",
                        r#""sourceRefName":"refs/heads/feature/界""#,
                        r#""targetRefName":"refs/heads/main""#,
                    ),
                    ForgeProvider::Gitea => (
                        "/gitea/repos/team/%E8%B3%87%E6%96%99/pulls",
                        r#""head":"feature/界""#,
                        r#""base":"main""#,
                    ),
                };
                assert_eq!(post.path, creation_path);
                assert!(post.body.contains(source_field));
                assert!(post.body.contains(target_field));
                assert!(
                    seen.iter()
                        .any(|r| r.method == "GET" && r.path.contains("feature%2F%E7%95%8C")),
                    "Unicode branch must be URL-encoded"
                );
                assert!(post.body.contains(r#""title":"PRIVATE 界 🚀""#));
                assert!(
                    post.body
                        .contains(r#"Quoted \"text\"\nSecond line \\ path"#)
                );
                assert!(
                    seen.iter()
                        .all(|r| r.head.contains("Authorization: ") && !r.path.contains(TOKEN))
                );
                if p == ForgeProvider::Bitbucket {
                    assert!(page.next.is_some());
                    assert!(!seen.iter().any(|r| r.path.contains("stolen")));
                }
            }
            drop(log);
            thread::sleep(Duration::from_millis(100));
            let logs = std::fs::read_dir(f.root.join("forge-logs"))
                .unwrap()
                .filter_map(|e| std::fs::read_to_string(e.unwrap().path()).ok())
                .collect::<String>();
            assert!(logs.contains("hosting-create"));
            for secret in [
                TOKEN,
                "PRIVATE",
                "资料",
                "fixture@example.invalid",
                "Authorization",
            ] {
                assert!(!logs.contains(secret), "log leak {secret}");
            }
        },
    );
}
#[test]
fn stale_review_rejections_redirect_and_cancelled_submission() {
    isolated(
        "ROCKET_FORGE_CASE",
        "stale_review_rejections_redirect_and_cancelled_submission",
        || {
            let f = Fixture::new();
            let mode = Arc::new(AtomicUsize::new(0));
            let m = mode.clone();
            let server = serve(Box::new(move |r| {
                route(ForgeProvider::GitLab, r, m.load(Ordering::Acquire))
            }));
            let c = CancellationToken::new();
            let config = config(ForgeProvider::GitLab, server.port);
            let s = f.git.forge_session(config.clone(), "", TOKEN, &c).unwrap();
            let d = draft(config);
            let preview = s.preview(d.clone(), &c).unwrap();
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
            for (code, expected) in [
                (401, ForgeFailure::Unauthorized),
                (403, ForgeFailure::Forbidden),
                (404, ForgeFailure::NotFound),
                (429, ForgeFailure::RateLimited),
                (422, ForgeFailure::Rejected),
                (7, ForgeFailure::InvalidResponse),
            ] {
                mode.store(code, Ordering::Release);
                let err = s
                    .list(
                        &d.repository,
                        &ForgeFilter::default(),
                        ForgeCursor::default(),
                        &c,
                    )
                    .unwrap_err();
                assert!(!err.to_string().contains(TOKEN));
                assert_eq!(failure::<ForgePage>(Err(err)), expected);
            }
            mode.store(0, Ordering::Release);
            let pre = s.preview(d.clone(), &c).unwrap();
            let s2 = s.clone();
            let t = CancellationToken::new();
            let t2 = t.clone();
            mode.store(8, Ordering::Release);
            let task = thread::spawn(move || s2.create(&pre, &t2));
            let deadline = Instant::now() + Duration::from_secs(10);
            while !server
                .seen
                .lock()
                .unwrap()
                .iter()
                .any(|r| r.method == "POST")
            {
                assert!(Instant::now() < deadline);
                thread::sleep(Duration::from_millis(10));
            }
            t.cancel();
            assert_eq!(failure(task.join().unwrap()), ForgeFailure::OutcomeUnknown);
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
        },
    );
}
