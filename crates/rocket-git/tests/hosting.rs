#![cfg(windows)]
//! GitHub hosting against a loopback fake API and a fake credential helper. The
//! ignored live test reads (never changes) the real account through Git
//! Credential Manager and prints counts only.
#[allow(dead_code)]
mod common;
use common::{Fixture, isolated};
use rocket_git::*;
use std::{
    fs,
    io::{Read, Write},
    net::TcpListener,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};

const CASE: &str = "ROCKET_HOSTING_CASE";
const TOKEN: &str = "gho_rocketFakeToken_0123456789";

type Route = Box<dyn Fn(&str) -> (u16, Vec<(&'static str, String)>, String, Duration) + Send>;

/// One request per connection, as curl makes them. Records request heads.
struct FakeGitHub {
    endpoint: GitHubEndpoint,
    requests: Arc<Mutex<Vec<String>>>,
}
fn serve(route: Route) -> FakeGitHub {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let requests = Arc::new(Mutex::new(Vec::new()));
    let seen = requests.clone();
    thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut head = Vec::new();
            let mut byte = [0u8; 1];
            while !head.ends_with(b"\r\n\r\n") && head.len() < 65536 {
                match stream.read(&mut byte) {
                    Ok(1) => head.push(byte[0]),
                    _ => break,
                }
            }
            let head = String::from_utf8_lossy(&head).into_owned();
            let path = head.split(' ').nth(1).unwrap_or("").to_owned();
            seen.lock().unwrap().push(head);
            let (status, headers, body, delay) = route(&path);
            thread::sleep(delay);
            let mut response = format!(
                "HTTP/1.1 {status} Fake\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n",
                body.len()
            );
            for (name, value) in headers {
                let value = value.replace("BASE", &format!("http://127.0.0.1:{port}"));
                response.push_str(&format!("{name}: {value}\r\n"));
            }
            response.push_str("\r\n");
            response.push_str(&body);
            let _ = stream.write_all(response.as_bytes());
        }
    });
    FakeGitHub {
        endpoint: GitHubEndpoint::Loopback(port),
        requests,
    }
}
fn ok(body: &str) -> (u16, Vec<(&'static str, String)>, String, Duration) {
    (200, Vec::new(), body.into(), Duration::ZERO)
}
fn status(code: u16) -> (u16, Vec<(&'static str, String)>, String, Duration) {
    (
        code,
        Vec::new(),
        "{\"message\":\"x\"}".into(),
        Duration::ZERO,
    )
}
/// Point this isolated test's global Git config at a fake credential helper that
/// records `GCM_INTERACTIVE` and, unless `token` is None, answers with it.
fn helper(f: &Fixture, token: Option<&str>) -> PathBuf {
    let marker = f.root.join("helper-calls");
    let answer = token.map_or(String::new(), |t| {
        format!("echo username=rocket-fake; echo password={t};")
    });
    let config = format!(
        "[credential]\n\thelper = \"!f() {{ cat >/dev/null; [ \\\"$1\\\" = get ] || exit 0; echo \\\"$GCM_INTERACTIVE\\\" >> '{}'; {answer} }}; f\"\n",
        marker.to_str().unwrap().replace('\\', "/")
    );
    fs::write(std::env::var_os("GIT_CONFIG_GLOBAL").unwrap(), config).unwrap();
    marker
}
fn logs(directory: &Path) -> String {
    fs::read_dir(directory)
        .unwrap()
        .filter_map(|e| fs::read_to_string(e.unwrap().path()).ok())
        .collect()
}
fn failure<T: std::fmt::Debug>(result: Result<T>) -> HostingFailure {
    match result {
        Err(Error::Hosting(failure)) => failure,
        other => panic!("expected a hosting failure, got {other:?}"),
    }
}

const USER: &str =
    r#"{"login":"rocket-fake","name":"Rocket 测试","html_url":"https://github.com/rocket-fake"}"#;
fn repos_page(path: &str) -> (u16, Vec<(&'static str, String)>, String, Duration) {
    if path.contains("page=2") {
        return ok(
            r#"[{"name":"third","owner":{"login":"rocket-org"},"private":false,"fork":false,"archived":false,"default_branch":"trunk","html_url":"https://github.com/rocket-org/third","description":null}]"#,
        );
    }
    let mut response = ok(r#"[
        {"name":"hosting.fixture","owner":{"login":"rocket-fake"},"private":true,"fork":false,"archived":false,"default_branch":"main","html_url":"https://github.com/rocket-fake/hosting.fixture","description":"界面 🚀 description","pushed_at":"2026-10-21T01:02:03Z"},
        {"name":"forked_repo","owner":{"login":"rocket-fake"},"private":false,"fork":true,"archived":true,"default_branch":"dev","html_url":"javascript:alert(1)"},
        {"name":"bad/name","owner":{"login":"rocket-fake"},"html_url":"https://github.com/x/y"}
    ]"#);
    response.1.push((
        "Link",
        "<BASE/user/repos?per_page=100&page=2>; rel=\"next\"".into(),
    ));
    response
}

#[test]
fn sign_in_reads_the_helper_and_lists_repositories_notifications_and_pulls() {
    isolated(
        CASE,
        "sign_in_reads_the_helper_and_lists_repositories_notifications_and_pulls",
        || {
            let f = Fixture::new();
            let marker = helper(&f, Some(TOKEN));
            let fake = serve(Box::new(|path| match path {
                "/user" => ok(USER),
                p if p.starts_with("/user/repos?") => repos_page(p),
                p if p.starts_with("/notifications?all=false") => ok(r#"[
                    {"id":"9001","unread":true,"reason":"review_requested","updated_at":"2026-10-21T00:00:00Z","subject":{"title":"Fix 中文 paths","url":"https://api.github.com/repos/rocket-fake/hosting.fixture/pulls/42","type":"PullRequest"},"repository":{"name":"hosting.fixture","owner":{"login":"rocket-fake"}}},
                    {"id":9002,"unread":false,"reason":"mention","updated_at":"2026-10-20T00:00:00Z","subject":{"title":"Question","url":null,"type":"Discussion"},"repository":{"name":"third","owner":{"login":"rocket-org"}}}
                ]"#),
                "/repos/rocket-fake/forked_repo" => ok(
                    r#"{"name":"forked_repo","owner":{"login":"rocket-fake"},"fork":true,"default_branch":"dev","html_url":"https://github.com/rocket-fake/forked_repo","parent":{"name":"upstream","owner":{"login":"rocket-org"},"default_branch":"main"}}"#,
                ),
                p if p.starts_with("/repos/rocket-org/upstream/pulls?") => ok(
                    r#"[{"number":7,"title":"Feature 界","html_url":"https://github.com/rocket-org/upstream/pull/7","draft":true,"base":{"ref":"main"}}]"#,
                ),
                _ => status(404),
            }));
            let c = CancellationToken::new();
            let log = DiagnosticLog::open(f.root.join("logs")).unwrap();
            let session = log
                .request("github-sign-in", |log| {
                    f.git
                        .with_log(log.clone())
                        .github_session(fake.endpoint, false, &c)
                })
                .unwrap();
            assert_eq!(session.account().login, "rocket-fake");
            assert_eq!(session.account().name.as_deref(), Some("Rocket 测试"));
            assert_eq!(fs::read_to_string(&marker).unwrap().trim(), "never");
            assert!(!format!("{session:?}").contains(TOKEN));

            let page = log
                .request("github-repositories", |log| {
                    session.with_log(log.clone()).repositories(1, &c)
                })
                .unwrap();
            // An entry with an invalid name is skipped; an unsafe page link is replaced.
            assert_eq!(page.items.len(), 2);
            let first = &page.items[0];
            assert_eq!(first.remote.full_name(), "rocket-fake/hosting.fixture");
            assert!(first.private && !first.fork);
            assert_eq!(first.description.as_deref(), Some("界面 🚀 description"));
            assert_eq!(
                first.clone_url,
                "https://github.com/rocket-fake/hosting.fixture.git"
            );
            assert_eq!(
                first.ssh_url,
                "git@github.com:rocket-fake/hosting.fixture.git"
            );
            let fork = &page.items[1];
            assert!(fork.fork && fork.archived);
            assert_eq!(fork.html_url, "https://github.com/rocket-fake/forked_repo");
            assert_eq!(page.next_page, Some(2));
            let last = session.repositories(2, &c).unwrap();
            assert_eq!(last.items[0].default_branch, "trunk");
            assert_eq!(last.next_page, None);

            let notes = log
                .request("github-notifications", |log| {
                    session.with_log(log.clone()).notifications(false, 1, &c)
                })
                .unwrap();
            assert_eq!(notes.items.len(), 2);
            assert_eq!(notes.items[0].id, "9001");
            assert_eq!(
                notes.items[0].web_url,
                "https://github.com/rocket-fake/hosting.fixture/pull/42"
            );
            assert_eq!(notes.items[0].title, "Fix 中文 paths");
            assert_eq!(notes.items[1].id, "9002");
            assert_eq!(
                notes.items[1].web_url,
                "https://github.com/rocket-org/third/discussions"
            );

            let fork = GitHubRemote::new("rocket-fake", "forked_repo").unwrap();
            let info = log
                .request("github-repository", |log| {
                    session.with_log(log.clone()).repository(&fork, &c)
                })
                .unwrap();
            let (parent, base) = info.parent.clone().unwrap();
            assert_eq!(
                (parent.full_name().as_str(), base.as_str()),
                ("rocket-org/upstream", "main")
            );
            let pull = log
                .request("github-pulls", |log| {
                    session.with_log(log.clone()).open_pull_request(
                        &parent,
                        "rocket-fake",
                        "feature/界",
                        &c,
                    )
                })
                .unwrap()
                .unwrap();
            assert_eq!(
                (pull.number, pull.draft, pull.base.as_str()),
                (7, true, "main")
            );
            assert_eq!(
                pull.html_url,
                "https://github.com/rocket-org/upstream/pull/7"
            );

            let requests = fake.requests.lock().unwrap().clone();
            let pulls = requests.iter().find(|r| r.contains("/pulls?")).unwrap();
            assert!(
                pulls.contains("head=rocket-fake%3Afeature%2F%E7%95%8C"),
                "{pulls}"
            );
            for request in &requests {
                assert!(request.starts_with("GET "), "{request}");
                assert!(request.contains(&format!("Authorization: Bearer {TOKEN}\r\n")));
                assert!(request.contains("Accept: application/vnd.github+json\r\n"));
                assert!(request.contains("X-GitHub-Api-Version: 2022-11-28\r\n"));
                assert!(request.contains("User-Agent: Rocket/"));
            }
            // Interactive sign-in lets the helper prompt.
            f.git.github_session(fake.endpoint, true, &c).unwrap();
            assert!(
                fs::read_to_string(&marker)
                    .unwrap()
                    .ends_with("never\nauto\n")
            );

            // Diagnostics name operations and steps, never the token, account or content.
            let text = logs(&f.root.join("logs"));
            for step in [
                "read_credential",
                "read_github_user",
                "read_github_repositories",
                "read_github_notifications",
                "read_github_repository",
                "read_github_pulls",
            ] {
                assert!(text.contains(step), "missing {step}");
            }
            for secret in [
                TOKEN,
                "rocket-fake",
                "rocket-org",
                "hosting.fixture",
                "中文",
                "Feature 界",
            ] {
                assert!(!text.contains(secret), "diagnostics leaked {secret}");
            }
        },
    );
}

#[test]
fn failures_are_typed_and_never_expose_the_token() {
    isolated(
        CASE,
        "failures_are_typed_and_never_expose_the_token",
        || {
            let f = Fixture::new();
            let c = CancellationToken::new();
            let fake = serve(Box::new(|path| match path {
                "/user" => ok(USER),
                "/repos/a/unauthorized" => status(401),
                "/repos/a/limited" => (
                    403,
                    vec![("X-RateLimit-Remaining", "0".into())],
                    "{}".into(),
                    Duration::ZERO,
                ),
                "/repos/a/forbidden" => status(403),
                "/repos/a/missing" => status(404),
                "/repos/a/garbled" => ok("{\"name\":"),
                "/repos/a/moved" => (
                    302,
                    vec![("Location", "https://example.com/".into())],
                    "{}".into(),
                    Duration::ZERO,
                ),
                _ => status(500),
            }));
            // No stored credential: the helper answers nothing and prompts are disabled.
            helper(&f, None);
            let result = f.git.github_session(fake.endpoint, false, &c);
            assert_eq!(failure(result), HostingFailure::SignInRequired);
            // A password that is not a token is never put into a request.
            helper(&f, Some("not a token"));
            assert_eq!(
                failure(f.git.github_session(fake.endpoint, false, &c)),
                HostingFailure::Unauthorized
            );
            assert_eq!(fake.requests.lock().unwrap().len(), 0);

            helper(&f, Some(TOKEN));
            let session = f.git.github_session(fake.endpoint, false, &c).unwrap();
            for (name, expected) in [
                ("unauthorized", HostingFailure::Unauthorized),
                ("limited", HostingFailure::RateLimited),
                ("forbidden", HostingFailure::Forbidden),
                ("missing", HostingFailure::NotFound),
                ("garbled", HostingFailure::InvalidResponse),
                ("moved", HostingFailure::InvalidResponse),
                ("broken", HostingFailure::InvalidResponse),
            ] {
                let error = session
                    .repository(&GitHubRemote::new("a", name).unwrap(), &c)
                    .unwrap_err();
                assert!(!format!("{error} {error:?}").contains(TOKEN));
                assert_eq!(failure::<()>(Err(error)), expected, "{name}");
            }
            // Nothing listens on a released port.
            let closed = TcpListener::bind("127.0.0.1:0").unwrap();
            let endpoint = GitHubEndpoint::Loopback(closed.local_addr().unwrap().port());
            drop(closed);
            assert_eq!(
                failure(f.git.github_session(endpoint, false, &c)),
                HostingFailure::Network
            );
        },
    );
}

#[test]
fn cancellation_stops_a_slow_request() {
    isolated(CASE, "cancellation_stops_a_slow_request", || {
        let f = Fixture::new();
        helper(&f, Some(TOKEN));
        let fake = serve(Box::new(|path| match path {
            "/user" => ok(USER),
            _ => (200, Vec::new(), "[]".into(), Duration::from_secs(20)),
        }));
        let c = CancellationToken::new();
        let session = f.git.github_session(fake.endpoint, false, &c).unwrap();
        let cancel = CancellationToken::new();
        let later = cancel.clone();
        thread::spawn(move || {
            thread::sleep(Duration::from_millis(400));
            later.cancel();
        });
        let started = Instant::now();
        assert!(matches!(
            session.notifications(true, 1, &cancel),
            Err(Error::Cancelled)
        ));
        assert!(
            started.elapsed() < Duration::from_secs(8),
            "{:?}",
            started.elapsed()
        );
    });
}

/// Real github.com, read-only, through the user's Git Credential Manager sign-in.
/// Run explicitly: `cargo test -p rocket-git --test hosting live -- --ignored`.
#[test]
#[ignore = "contacts api.github.com with the user's stored credential"]
fn live_read_only_github_api() {
    let git = GitClient::discover().unwrap();
    let c = CancellationToken::new();
    let session = git
        .github_session(GitHubEndpoint::Public, false, &c)
        .unwrap();
    assert!(!session.account().login.is_empty());
    let repositories = session.repositories(1, &c).unwrap();
    let notifications = session.notifications(false, 1, &c).unwrap();
    for repository in &repositories.items {
        assert!(github_web_url(&repository.html_url).is_some());
    }
    for note in &notifications.items {
        assert!(github_web_url(&note.web_url).is_some());
    }
    if let Some(first) = repositories.items.first() {
        let info = session.repository(&first.remote, &c).unwrap();
        assert_eq!(info.remote, first.remote);
    }
    println!(
        "live GitHub: signed in; {} repositories on page 1 (more: {}); {} unread notifications on page 1",
        repositories.items.len(),
        repositories.next_page.is_some(),
        notifications.items.len()
    );
}
