//! One issue's text and comments, from responses shaped like each host's documented
//! examples: comment pages, GitLab's system notes, a pull request answered as an issue,
//! Gitea's single list, Azure DevOps HTML and Markdown, Bitbucket's refusal and
//! malformed responses.
use super::*;

macro_rules! case {
    ($name:ident, $body:block) => {
        #[test]
        fn $name() {
            isolated(
                "ROCKET_ISSUE_TEXT_CASE",
                concat!("issue_text_tests::", stringify!($name)),
                || $body,
            );
        }
    };
}
type Reply = (u16, String, Vec<(String, String)>);
const REPO: &str = "team/資料";
const NOTE: &str = "\n[Truncated; open the issue for the full text.]";

fn split(r: &Request) -> (&str, &str) {
    r.path.split_once('?').unwrap_or((r.path.as_str(), ""))
}
fn page(query: &str) -> usize {
    query.split('&').find_map(|p| p.strip_prefix("page=")).and_then(|p| p.parse().ok()).unwrap_or(0)
}
fn next_link(r: &Request, endpoint: &str, query: &str) -> Vec<(String, String)> {
    vec![("Link".into(), format!("<{}{endpoint}?{query}>; rel=\"next\"", fixture_origin(r)))]
}
fn connect(f: &Fixture, config: &ForgeConfig) -> ForgeSession {
    f.git.forge_session(config.clone(), "", TOKEN, &CancellationToken::new()).unwrap()
}
fn paths(server: &Server) -> Vec<String> {
    server.seen.lock().unwrap().iter().map(|r| r.path.clone()).collect()
}
fn not_found() -> Reply {
    (404, "{}".into(), Vec::new())
}

/// "Get an issue" and "List issue comments": comments by ascending ID, 50 to a page,
/// `Link` naming the next. Modes: 0 two pages (53 comments), 1 pages that never end,
/// 2 the number is a pull request; from 10 on, malformed answers.
fn github(r: &Request, mode: usize) -> Reply {
    let (endpoint, query) = split(r);
    if endpoint.ends_with("/user") {
        return response(r#"{"login":"fixture"}"#);
    }
    if endpoint.ends_with("/issues/7") {
        let issue = r#"{"number":7,"title":"PRIVATE issue 界\u202e","body":"Steps:\r\n1. Run `cargo test`\r\n\tindented \\ path","state":"open","user":{"login":"author"},"created_at":"2026-11-01T10:00:00Z","comments":53}"#;
        return response(&match mode {
            2 => issue.replacen('{', r#"{"pull_request":{"url":"https://api.github.com/repos/team/x/pulls/7"},"#, 1),
            10 => "{}".into(),
            11 => issue.replace(r#""number":7"#, r#""number":8"#),
            12 => issue.replace(r#""title":"PRIVATE issue 界\u202e","#, ""),
            13 => issue.replace(r#""body":"Steps:\r\n1. Run `cargo test`\r\n\tindented \\ path""#, r#""body":5"#),
            _ => issue.into(),
        });
    }
    if !endpoint.ends_with("/issues/7/comments") {
        return not_found();
    }
    let comment = |id: usize| match id {
        1 => r#"{"id":1,"user":{"login":"first"},"body":"First line\r\nsecond \u202e line","created_at":"2026-11-01T11:00:00Z"}"#.to_owned(),
        2 => r#"{"id":2,"user":null,"body":null,"created_at":"2026-11-01T12:00:00Z"}"#.to_owned(),
        id => format!(r#"{{"id":{id},"user":{{"login":"user{id}"}},"body":"comment {id}","created_at":"2026-11-02T00:00:00Z"}}"#),
    };
    let page = page(query);
    let rows: Vec<String> = match mode {
        14 => return response(r#"{"message":"PRIVATE"}"#),
        15 => (1..=101).map(comment).collect(),
        16 => vec![r#"{"user":{"login":"x"},"body":"no ID"}"#.into()],
        17 => vec![r#"{"id":5,"user":{"login":"x"},"body":5}"#.into()],
        _ => {
            let first = (page - 1) * 50 + 1;
            let count = if mode == 1 || page == 1 { 50 } else { 3 };
            (first..first + count).map(comment).collect()
        }
    };
    let following = format!("per_page=50&page={}", page + 1);
    let headers = match mode {
        18 => vec![("Link".into(), "<http://127.0.0.1:1/stolen>; rel=\"next\"".into())],
        19 => next_link(r, endpoint, query),
        1 => next_link(r, endpoint, &following),
        0 | 2 if page == 1 => next_link(r, endpoint, &following),
        _ => Vec::new(),
    };
    (200, format!("[{}]", rows.join(",")), headers)
}

case!(github_and_enterprise_read_the_issue_and_its_comment_pages, {
    let f = Fixture::new();
    for provider in [ForgeProvider::GitHub, ForgeProvider::GitHubEnterprise] {
        let mode = Arc::new(AtomicUsize::new(0));
        let m = mode.clone();
        let server = serve(Box::new(move |r| github(r, m.load(Ordering::Acquire))));
        let config = config(provider, server.port);
        let session = connect(&f, &config);
        let repo = ForgeRepository::new(config, REPO).unwrap();
        let c = CancellationToken::new();
        let issue = session.issue_text(&repo, 7, &c).unwrap();
        assert_eq!(
            (issue.number, issue.state.as_str(), issue.author.as_str(), issue.date.as_str()),
            (7, "open", "author", "2026-11-01T10:00:00Z")
        );
        assert_eq!(issue.title, "PRIVATE issue 界\\u{202e}");
        // Markdown stays as written: only its line endings change.
        assert_eq!(issue.body, "Steps:\n1. Run `cargo test`\n\tindented \\ path");
        assert_eq!(issue.url, repo.item_url(ForgeKind::Issues, 7));
        assert!(!issue.is_pull && !issue.limited);
        assert_eq!(issue.comments.len(), 53);
        let first = &issue.comments[0];
        assert_eq!((first.author.as_str(), first.date.as_str()), ("first", "2026-11-01T11:00:00Z"));
        // Text stays as written, hidden characters too: whoever shows or sends it deals
        // with them (an agent's first message leaves them out and lists them).
        assert_eq!(first.body, "First line\nsecond \u{202e} line");
        assert_eq!((issue.comments[1].author.as_str(), issue.comments[1].body.as_str()), ("Unknown author", ""));
        assert_eq!(issue.comments[52].body, "comment 53");
        let prefix = if provider == ForgeProvider::GitHub { "/github" } else { "/github-enterprise/api/v3" };
        let issue_path = format!("{prefix}/repos/team/%E8%B3%87%E6%96%99/issues/7");
        assert_eq!(
            paths(&server)[1..],
            [
                issue_path.clone(),
                format!("{issue_path}/comments?per_page=50&page=1"),
                format!("{issue_path}/comments?per_page=50&page=2"),
            ]
        );
        // GitHub answers a pull request's number too: the text is read, marked as one.
        mode.store(2, Ordering::Release);
        let pull = session.issue_text(&repo, 7, &c).unwrap();
        assert!(pull.is_pull);
        assert_eq!((pull.title.as_str(), pull.comments.len()), (issue.title.as_str(), 53));
        // Four full pages and still a next one: 200 comments, marked as more.
        mode.store(1, Ordering::Release);
        let before = server.seen.lock().unwrap().len();
        let long = session.issue_text(&repo, 7, &c).unwrap();
        assert_eq!(long.comments.len(), 200);
        assert_eq!(long.comments[199].body, "comment 200");
        assert!(long.limited);
        let read = server.seen.lock().unwrap()[before..].iter().filter(|r| r.path.contains("/comments?")).count();
        assert_eq!(read, 4);
    }
});

case!(malformed_answers_fail_and_bad_numbers_send_nothing, {
    let f = Fixture::new();
    let mode = Arc::new(AtomicUsize::new(0));
    let m = mode.clone();
    let server = serve(Box::new(move |r| github(r, m.load(Ordering::Acquire))));
    let cfg = config(ForgeProvider::GitHub, server.port);
    let session = connect(&f, &cfg);
    let repo = ForgeRepository::new(cfg, REPO).unwrap();
    let c = CancellationToken::new();
    for (value, expected) in [
        (10, ForgeFailure::InvalidResponse),   // not an issue
        (11, ForgeFailure::InvalidResponse),   // another issue's number
        (12, ForgeFailure::InvalidResponse),   // no title
        (13, ForgeFailure::InvalidResponse),   // a body that is not text
        (14, ForgeFailure::InvalidResponse),   // comments that are not a list
        (15, ForgeFailure::InvalidResponse),   // more rows than a page holds
        (16, ForgeFailure::InvalidResponse),   // a comment without its ID
        (17, ForgeFailure::InvalidResponse),   // a comment body that is not text
        (18, ForgeFailure::InvalidPagination), // a next page somewhere else
        (19, ForgeFailure::InvalidPagination), // the same page again
    ] {
        mode.store(value, Ordering::Release);
        let error = session.issue_text(&repo, 7, &c).unwrap_err();
        assert!(!error.to_string().contains(TOKEN) && !error.to_string().contains("PRIVATE"));
        assert_eq!(failure::<ForgeIssueText>(Err(error)), expected, "mode {value}");
    }
    assert!(paths(&server).iter().all(|p| !p.contains("stolen")));
    // Nothing is asked for a number no host gives, another server's repository or
    // after Stop.
    mode.store(0, Ordering::Release);
    let before = server.seen.lock().unwrap().len();
    for number in [0, u64::from(u32::MAX)] {
        assert!(matches!(session.issue_text(&repo, number, &c), Err(Error::InvalidInput(_))));
    }
    let elsewhere = ForgeRepository::new(config(ForgeProvider::GitLab, server.port), REPO).unwrap();
    assert!(matches!(session.issue_text(&elsewhere, 7, &c), Err(Error::InvalidInput(_))));
    let stopped = CancellationToken::new();
    stopped.cancel();
    assert!(matches!(session.issue_text(&repo, 7, &stopped), Err(Error::Cancelled)));
    assert_eq!(server.seen.lock().unwrap().len(), before);
});

/// "Get single issue" and "List project issue notes": notes oldest first when asked,
/// GitLab's own marked `"system": true`, the next page's link keeping the query.
fn gitlab(r: &Request) -> Reply {
    let (endpoint, query) = split(r);
    let issue = "/gitlab/projects/team%2F%E8%B3%87%E6%96%99/issues/7";
    if endpoint.ends_with("/user") {
        return response(r#"{"username":"fixture"}"#);
    }
    if endpoint == issue {
        return response(
            r#"{"iid":7,"title":"PRIVATE GitLab issue","description":"Line one\r\nLine two","state":"opened","author":{"username":"alice","name":"Alice"},"created_at":"2026-11-02T08:00:00.000Z"}"#,
        );
    }
    if endpoint != format!("{issue}/notes") {
        return not_found();
    }
    if page(query) == 2 {
        return response(r#"[{"id":14,"body":"Fixed in !3","author":{"username":"carol"},"created_at":"2026-11-02T11:00:00.000Z","system":false}]"#);
    }
    (
        200,
        r#"[{"id":11,"body":"changed the description","author":{"username":"alice"},"created_at":"2026-11-02T08:01:00.000Z","system":true},{"id":12,"body":"Repro attached\r\nsee the log","author":{"username":"bob"},"created_at":"2026-11-02T09:00:00.000Z","system":false},{"id":13,"body":"assigned to @bob","author":{"username":"alice"},"created_at":"2026-11-02T10:00:00.000Z","system":true}]"#.into(),
        next_link(r, endpoint, &query.replace("&page=1", "&page=2")),
    )
}

case!(gitlab_reads_comments_without_system_notes, {
    let f = Fixture::new();
    let server = serve(Box::new(gitlab));
    let config = config(ForgeProvider::GitLab, server.port);
    let session = connect(&f, &config);
    let repo = ForgeRepository::new(config, REPO).unwrap();
    let issue = session.issue_text(&repo, 7, &CancellationToken::new()).unwrap();
    assert_eq!(
        (issue.title.as_str(), issue.state.as_str(), issue.author.as_str()),
        ("PRIVATE GitLab issue", "opened", "alice")
    );
    assert_eq!(issue.body, "Line one\nLine two");
    assert_eq!(issue.url, "https://gitlab.com/team/%E8%B3%87%E6%96%99/-/issues/7");
    let comments: Vec<_> = issue.comments.iter().map(|c| (c.author.as_str(), c.body.as_str(), c.date.as_str())).collect();
    assert_eq!(
        comments,
        [("bob", "Repro attached\nsee the log", "2026-11-02T09:00:00.000Z"), ("carol", "Fixed in !3", "2026-11-02T11:00:00.000Z")]
    );
    assert!(!issue.limited && !issue.is_pull);
    let notes = "/gitlab/projects/team%2F%E8%B3%87%E6%96%99/issues/7/notes?activity_filter=only_comments&sort=asc&order_by=created_at&per_page=50";
    assert_eq!(paths(&server)[2..], [format!("{notes}&page=1"), format!("{notes}&page=2")]);
});

/// Gitea lists an issue's comments in one answer, with no page or limit (`since` and
/// `before` only), counted in X-Total-Count. Mode 1 is a pull request's number.
fn gitea(r: &Request, mode: usize) -> Reply {
    let (endpoint, _) = split(r);
    let issue = "/gitea/repos/team/%E8%B3%87%E6%96%99/issues/7";
    if endpoint.ends_with("/user") {
        return response(r#"{"login":"fixture"}"#);
    }
    if endpoint == issue {
        return response(&if mode == 1 {
            r#"{"number":7,"title":"Fix login","body":"","state":"closed","user":{"login":"dev"},"created_at":"2026-11-04T08:00:00+08:00","pull_request":{"merged":false,"html_url":"https://gitea.com/team/x/pulls/7"}}"#.to_owned()
        } else {
            format!(
                r#"{{"number":7,"title":"{}","body":"{}","state":"open","user":{{"login":"{}"}},"created_at":"2026-11-04T08:00:00+08:00","pull_request":null}}"#,
                "T".repeat(1100),
                "x".repeat(70_000),
                "a".repeat(300)
            )
        });
    }
    if endpoint != format!("{issue}/comments") {
        return not_found();
    }
    let count = if mode == 1 { 2 } else { 201 };
    let rows: Vec<String> = (1..=count)
        .map(|id| format!(r#"{{"id":{id},"user":{{"login":"u{id}"}},"body":"c{id}","created_at":"2026-11-04T09:00:00+08:00"}}"#))
        .collect();
    let headers = if mode == 1 { vec![("X-Total-Count".into(), "3".into())] } else { Vec::new() };
    (200, format!("[{}]", rows.join(",")), headers)
}

case!(gitea_reads_its_single_comment_list_within_bounds, {
    let f = Fixture::new();
    let mode = Arc::new(AtomicUsize::new(0));
    let m = mode.clone();
    let server = serve(Box::new(move |r| gitea(r, m.load(Ordering::Acquire))));
    let config = config(ForgeProvider::Gitea, server.port);
    let session = connect(&f, &config);
    let repo = ForgeRepository::new(config, REPO).unwrap();
    let c = CancellationToken::new();
    let issue = session.issue_text(&repo, 7, &c).unwrap();
    // Titles stay within 1 KiB, authors 256 bytes, bodies 64 KiB with a note.
    assert!(issue.title.len() <= 1024 && issue.title.starts_with("TTT") && issue.title.ends_with('…'));
    assert!(issue.author.len() <= 256 && issue.author.starts_with("aaa") && issue.author.ends_with('…'));
    assert!(issue.body.len() <= 64 * 1024 && issue.body.starts_with("xxx") && issue.body.ends_with(NOTE));
    assert!(!issue.is_pull);
    assert_eq!(issue.comments.len(), 200);
    assert_eq!((issue.comments[199].author.as_str(), issue.comments[199].body.as_str()), ("u200", "c200"));
    assert!(issue.limited);
    assert_eq!(paths(&server).last().unwrap(), "/gitea/repos/team/%E8%B3%87%E6%96%99/issues/7/comments");
    mode.store(1, Ordering::Release);
    let pull = session.issue_text(&repo, 7, &c).unwrap();
    assert!(pull.is_pull);
    assert_eq!((pull.state.as_str(), pull.body.as_str(), pull.comments.len()), ("closed", "", 2));
    // The host counts more comments than it listed.
    assert!(pull.limited);
});

const AZURE_ITEM: &str = "/azure/team/_apis/wit/workitems/7?fields=System.Title,System.Description,System.State,System.CreatedBy,System.CreatedDate,System.TeamProject&api-version=7.1";
const AZURE_COMMENTS: &str = "/azure/team/_apis/wit/workItems/7/comments?$top=200&order=asc&api-version=7.1-preview.4";
/// A description as Azure's editor saves it, written for JSON.
const DESCRIPTION: &str = r#"<div>Steps to <b>reproduce</b>:</div><ol><li>Open &quot;Settings&quot; &amp; sign in</li><li>Type <code>a &lt; b</code>&nbsp;&nbsp;then save</li></ol><p>Expected:<br>no error<br/>at all</p><!-- PRIVATE note --><pre>fn main() {\n    run();\n}</pre><div><a href=\"https://example.invalid/x?a=1&amp;b=2\" title=\"a > b\">link</a> &#x1F680; &#39;quoted&#39; &bogus; x < y&#x202E;</div>"#;
/// "Comments - Get Comments": HTML unless `format` says Markdown, a deleted comment,
/// and a next page that must not be followed.
const COMMENTS: &str = r#"{"totalCount":4,"count":3,"comments":[{"workItemId":7,"id":45,"version":1,"text":"<div>Looks <i>good</i><br>Ship it &amp; close</div>","createdBy":{"displayName":"Ana"},"createdDate":"2026-11-03T10:00:00Z","isDeleted":false},{"workItemId":7,"commentId":46,"version":2,"text":"PRIVATE removed","createdBy":{"displayName":"Ana"},"createdDate":"2026-11-03T10:30:00Z","isDeleted":true},{"workItemId":7,"commentId":47,"version":1,"format":"markdown","text":"Use `Vec<String>` &amp; keep **bold**\r\nsecond line","createdBy":{"displayName":"Ben"},"createdDate":"2026-11-03T11:00:00Z","isDeleted":false}],"nextPage":"http://127.0.0.1:1/stolen?continuationToken=PRIVATE_TOKEN","continuationToken":"PRIVATE_TOKEN"}"#;

/// "Work Items - Get Work Item" and its comments. Mode 1 is another project's work
/// item; mode 2 a description saved as Markdown.
fn azure(r: &Request, mode: usize) -> Reply {
    if r.path.contains("connectionData") {
        return route(ForgeProvider::Azure, r, 0);
    }
    if r.path.contains("/_apis/wit/workitems/7?") {
        let project = if mode == 1 { "other" } else { "team" };
        let (description, format) = if mode == 2 {
            (r"# Steps\r\nUse `Vec<T>` &amp; more", r#","multilineFieldsFormat":{"System.Description":"markdown"}"#)
        } else {
            (DESCRIPTION, "")
        };
        return response(&format!(
            r#"{{"id":7,"rev":3,"fields":{{"System.TeamProject":"{project}","System.State":"Active","System.Title":"PRIVATE work item","System.CreatedBy":{{"displayName":"Jamal"}},"System.CreatedDate":"2026-11-03T09:00:00.05Z","System.Description":"{description}"}}{format},"url":"https://dev.azure.com/fixture/_apis/wit/workItems/7"}}"#
        ));
    }
    if r.path.contains("/_apis/wit/workItems/7/comments?") {
        return response(COMMENTS);
    }
    not_found()
}

case!(azure_html_becomes_text_and_markdown_stays_as_written, {
    let f = Fixture::new();
    let mode = Arc::new(AtomicUsize::new(0));
    let m = mode.clone();
    let server = serve(Box::new(move |r| azure(r, m.load(Ordering::Acquire))));
    let config = config(ForgeProvider::Azure, server.port);
    let session = connect(&f, &config);
    let repo = ForgeRepository::new(config, REPO).unwrap();
    let c = CancellationToken::new();
    let item = session.issue_text(&repo, 7, &c).unwrap();
    assert_eq!(
        (item.title.as_str(), item.state.as_str(), item.author.as_str(), item.date.as_str()),
        ("PRIVATE work item", "Active", "Jamal", "2026-11-03T09:00:00.05Z")
    );
    // Blocks and <br> end lines, tags and comments go, references are decoded,
    // whitespace collapses except in <pre>, and the hidden character stays as written.
    assert_eq!(
        item.body,
        "Steps to reproduce:\nOpen \"Settings\" & sign in\nType a < b  then save\nExpected:\nno error\nat all\nfn main() {\n    run();\n}\nlink 🚀 'quoted' &bogus; x < y\u{202e}"
    );
    assert_eq!(item.url, "https://dev.azure.com/fixture/team/_workitems/edit/7");
    let comments: Vec<_> = item.comments.iter().map(|c| (c.author.as_str(), c.body.as_str(), c.date.as_str())).collect();
    assert_eq!(
        comments,
        [
            ("Ana", "Looks good\nShip it & close", "2026-11-03T10:00:00Z"),
            ("Ben", "Use `Vec<String>` &amp; keep **bold**\nsecond line", "2026-11-03T11:00:00Z"),
        ]
    );
    assert!(item.limited && !item.is_pull);
    let seen = paths(&server);
    assert_eq!(seen[1..], [AZURE_ITEM.to_owned(), AZURE_COMMENTS.to_owned()]);
    assert!(seen.iter().all(|p| !p.contains("stolen") && !p.contains("PRIVATE_TOKEN")));
    // Another project's work item is not this repository's issue; nothing more is read.
    mode.store(1, Ordering::Release);
    let before = server.seen.lock().unwrap().len();
    assert_eq!(failure(session.issue_text(&repo, 7, &c)), ForgeFailure::NotFound);
    assert_eq!(server.seen.lock().unwrap().len(), before + 1);
    // A description saved as Markdown stays as written.
    mode.store(2, Ordering::Release);
    assert_eq!(session.issue_text(&repo, 7, &c).unwrap().body, "# Steps\nUse `Vec<T>` &amp; more");
});

case!(azure_server_asks_for_comments_in_each_versions_preview, {
    let f = Fixture::new();
    for (version, preview) in [("6.0", "6.0-preview.3"), ("7.0", "7.0-preview.3"), ("7.1", "7.1-preview.4")] {
        let server = serve(Box::new(|r| {
            if r.path.ends_with("/_apis/connectionData") {
                return route(ForgeProvider::AzureServer, r, 0);
            }
            azure(r, 0)
        }));
        let config = config(ForgeProvider::AzureServer, server.port).with_azure_api_version(version).unwrap();
        let session = connect(&f, &config);
        let repo = ForgeRepository::new(config, REPO).unwrap();
        let item = session.issue_text(&repo, 7, &CancellationToken::new()).unwrap();
        assert_eq!((item.title.as_str(), item.comments.len()), ("PRIVATE work item", 2));
        assert_eq!(item.url, "https://devops.example.com/tfs/DefaultCollection/team/_workitems/edit/7");
        let wit = "/azure-server/tfs/DefaultCollection/team/_apis/wit";
        assert_eq!(
            paths(&server)[1..],
            [
                format!("{wit}/workitems/7?fields=System.Title,System.Description,System.State,System.CreatedBy,System.CreatedDate,System.TeamProject&api-version={version}"),
                format!("{wit}/workItems/7/comments?$top=200&order=asc&api-version={preview}"),
            ]
        );
    }
});

case!(bitbucket_has_no_issues_to_read, {
    let f = Fixture::new();
    for provider in [ForgeProvider::Bitbucket, ForgeProvider::BitbucketServer] {
        let server = serve(Box::new(move |r| route(provider, r, 0)));
        let config = config(provider, server.port);
        let user = if provider == ForgeProvider::BitbucketServer { "fixture" } else { "" };
        let session = f.git.forge_session(config.clone(), user, TOKEN, &CancellationToken::new()).unwrap();
        let repo = ForgeRepository::new(config, REPO).unwrap();
        let before = server.seen.lock().unwrap().len();
        assert_eq!(failure(session.issue_text(&repo, 7, &CancellationToken::new())), ForgeFailure::Unsupported);
        assert_eq!(server.seen.lock().unwrap().len(), before, "{provider:?} sent a request");
    }
});

case!(diagnostics_name_the_read_but_keep_no_text, {
    let f = Fixture::new();
    let server = serve(Box::new(|r| github(r, 0)));
    let config = config(ForgeProvider::GitHub, server.port);
    let session = connect(&f, &config);
    let repo = ForgeRepository::new(config, REPO).unwrap();
    let dir = f.root.join("issue-text-logs");
    let log = DiagnosticLog::open(&dir).unwrap();
    log.request("hosting-issue-text", |l| session.with_log(l.clone()).issue_text(&repo, 7, &CancellationToken::new()))
        .unwrap();
    drop(log);
    let logs = std::fs::read_dir(&dir)
        .unwrap()
        .filter_map(|e| std::fs::read_to_string(e.unwrap().path()).ok())
        .collect::<String>();
    // The request is logged under its own name, not as "other".
    assert!(
        logs.lines().any(|l| l.contains(r#""event":"request_end""#) && l.contains(r#""operation":"hosting-issue-text""#)),
        "{logs}"
    );
    for step in ["read_issue", "read_issue_comments"] {
        assert!(logs.contains(&format!(r#""step":"{step}""#)), "missing {step}");
    }
    for secret in [TOKEN, "PRIVATE", "資料", "%E8%B3%87", "Authorization", "cargo test", "comment 53"] {
        assert!(!logs.contains(secret), "leak {secret}");
    }
});
