//! Forgejo keeps Gitea's repository API, with differences Rocket must honour: its
//! merge style field is still `Do` (Gitea renamed it `do`), it has no endpoint to
//! resolve or reopen a review conversation, and its Actions runs have their own
//! shape (see ci_checks.rs). Responses follow Forgejo's API reference
//! (codeberg.org/swagger.v1.json, Forgejo 17).
use super::*;

macro_rules! case {
    ($name:ident, $body:block) => {
        #[test]
        fn $name() {
            isolated("ROCKET_FORGEJO_CASE", concat!("forgejo_tests::", stringify!($name)), || $body);
        }
    };
}
fn fail_is<T>(result: Result<T>, expected: ForgeFailure) {
    match result {
        Err(Error::Forge(f)) => assert_eq!(f, expected),
        Err(e) => panic!("Unexpected error {e}"),
        Ok(_) => panic!("Unexpected success"),
    }
}
const HEAD: &str = "d3022fc0ca3d65c7f6654eea129d6bf3ca0b6a5c";
const BASE: &str = "ce5965ddd2890b0e0c9d1f2a3b4c5d6e7f8091a2";
const MERGE: &str = "764413d85e294f3e2d1c0b0a99887766554433aa";
const DIFF: &str = "diff --git a/src/lib.rs b/src/lib.rs\nindex 1111111..2222222 100644\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,2 +1,2 @@\n fn main() {}\n-const A: u8 = 1;\n+const A: u8 = 2;\n";
type Reply = (u16, String, Vec<(String, String)>);

/// An open pull request by someone else, with one review holding one line comment.
fn forgejo(r: &Request, merged: &AtomicBool) -> Reply {
    let path = r.path.strip_prefix("/forgejo").expect("every request goes to Forgejo's API");
    let path = path.split('?').next().unwrap();
    let pr = "/repos/team/web/pulls/9";
    let pull = |merged: bool| {
        format!(
            r#"{{"number":9,"title":"Fix","body":"Body","state":"{}","merged":{merged},"draft":false,"mergeable":{},"flow":0,"user":{{"login":"author","id":2}},"head":{{"ref":"feature","sha":"{HEAD}"}},"base":{{"ref":"main","sha":"{BASE}"}},"merge_commit_sha":{}}}"#,
            if merged { "closed" } else { "open" },
            !merged,
            if merged { format!("\"{MERGE}\"") } else { "null".into() },
        )
    };
    match (r.method.as_str(), path) {
        ("GET", "/user") => response(r#"{"login":"fixture","id":3}"#),
        ("GET", p) if p == pr => response(&pull(merged.load(Ordering::Acquire))),
        ("GET", "/repos/team/web/branches/main") => response(&format!(r#"{{"name":"main","commit":{{"id":"{BASE}"}}}}"#)),
        ("GET", "/repos/team/web") => response(
            r#"{"full_name":"team/web","archived":false,"allow_merge_commits":true,"allow_rebase":true,"allow_squash_merge":false,"permissions":{"admin":false,"push":true,"pull":true}}"#,
        ),
        ("GET", p) if p == format!("{pr}/commits") => response(&format!(
            r#"[{{"sha":"{HEAD}","commit":{{"message":"Fix the constant\n","author":{{"name":"Author"}}}}}}]"#
        )),
        ("GET", p) if p == format!("{pr}/files") => {
            response(r#"[{"filename":"src/lib.rs","status":"modified","additions":1,"deletions":1,"changes":2}]"#)
        }
        ("GET", p) if p == format!("{pr}.diff") => (200, DIFF.into(), Vec::new()),
        ("GET", "/repos/team/web/issues/9/comments") => {
            response(r#"[{"id":30,"body":"Thanks","user":{"login":"author"},"created_at":"2026-11-01T09:00:00Z"}]"#)
        }
        ("GET", p) if p == format!("{pr}/reviews") => response(&format!(
            r#"[{{"id":4,"state":"COMMENT","body":"","user":{{"login":"reviewer"}},"commit_id":"{HEAD}","comments_count":1,"stale":false,"official":false,"dismissed":false,"submitted_at":"2026-11-01T10:00:00Z"}}]"#
        )),
        ("GET", p) if p == format!("{pr}/reviews/4/comments") => response(&format!(
            r#"[{{"id":40,"body":"Use a constant","path":"src/lib.rs","position":2,"original_position":0,"commit_id":"{HEAD}","original_commit_id":"{HEAD}","pull_request_review_id":4,"resolver":null,"user":{{"login":"reviewer"}},"created_at":"2026-11-01T10:00:00Z"}}]"#
        )),
        ("GET", p) if p == format!("/repos/team/web/commits/{HEAD}/statuses") => response(
            r#"[{"id":1,"context":"CI / build (push)","status":"success","target_url":"https://codeberg.org/team/web/actions/runs/1/jobs/0"}]"#,
        ),
        ("POST", p) if p == format!("{pr}/reviews") => response(&format!(
            r#"{{"id":5,"state":"COMMENT","body":"Looks good","user":{{"login":"fixture"}},"commit_id":"{HEAD}","comments_count":1}}"#
        )),
        // Forgejo confirms a merge with an empty 200; the PR is then read again.
        ("POST", p) if p == format!("{pr}/merge") => {
            merged.store(true, Ordering::Release);
            (200, String::new(), Vec::new())
        }
        _ => (404, r#"{"message":"not found"}"#.into(), Vec::new()),
    }
}
struct Forgejo {
    _fixture: Fixture,
    server: Server,
    session: ForgeSession,
    repo: ForgeRepository,
}
impl Forgejo {
    fn new() -> Self {
        let fixture = Fixture::new();
        let merged = Arc::new(AtomicBool::new(false));
        let server = serve(Box::new(move |r| forgejo(r, &merged)));
        let cfg = config(ForgeProvider::Forgejo, server.port);
        let session = fixture.git.forge_session(cfg.clone(), "", TOKEN, &CancellationToken::new()).unwrap();
        let repo = ForgeRepository::new(cfg, "team/web").unwrap();
        Self { _fixture: fixture, server, session, repo }
    }
    fn posts(&self) -> Vec<Request> {
        self.server.seen.lock().unwrap().iter().filter(|r| r.method == "POST").cloned().collect()
    }
}

case!(details_and_a_review_use_gitea_paths, {
    let h = Forgejo::new();
    let cancel = CancellationToken::new();
    assert_eq!(h.session.login(), "fixture");
    let details = h.session.pull_details(&h.repo, 9, &cancel).unwrap();
    let summary = details.summary();
    assert_eq!((summary.head.as_str(), summary.base.as_str(), summary.source.as_str(), summary.target.as_str()), (HEAD, BASE, "feature", "main"));
    assert_eq!(details.commits.items[0].subject, "Fix the constant");
    assert!(details.files.error.is_none(), "{:?}", details.files.error);
    assert_eq!(details.discussion.items[0].body, "Thanks");
    assert_eq!(details.reviews.items[0].state, "COMMENTED");
    assert_eq!(details.inline_comments.items[0].path.as_deref(), Some("src/lib.rs"));
    assert_eq!(details.checks.items[0].name, "CI / build (push)");
    // Gitea's inline contract: up to 32 single-line comments in one review.
    assert!(details.supports_inline_review());
    assert_eq!(details.inline_comment_limit(), 32);
    let comment = details.inline_comment(0, 2, ForgeDiffSide::Right, "Name it").unwrap();
    let preview = h
        .session
        .preview_pull_review_with_comments(&details, ForgeReviewAction::Comment, "Looks good", &[comment], &cancel)
        .unwrap();
    let review = h.session.submit_pull_review(&preview, &cancel).unwrap();
    assert_eq!((review.state.as_str(), review.author.as_str()), ("COMMENTED", "fixture"));
    let posts = h.posts();
    assert_eq!(posts.len(), 1);
    assert_eq!(posts[0].path, "/forgejo/repos/team/web/pulls/9/reviews");
    assert!(posts[0].body.contains(&format!(r#""commit_id":"{HEAD}","event":"COMMENT""#)), "{}", posts[0].body);
    assert!(posts[0].body.contains(r#""path":"src/lib.rs","old_position":0,"new_position":2"#), "{}", posts[0].body);
    // The pending-review check ran before the review was sent, as on Gitea.
    let seen = h.server.seen.lock().unwrap();
    let post = seen.iter().position(|r| r.method == "POST").unwrap();
    assert!(seen[..post].iter().any(|r| r.path.starts_with("/forgejo/repos/team/web/pulls/9/reviews?limit=50&page=1")));
});

case!(merge_names_its_style_do_and_honours_the_repository_flags, {
    let h = Forgejo::new();
    let cancel = CancellationToken::new();
    let details = h.session.pull_details(&h.repo, 9, &cancel).unwrap();
    // The repository turns squash merges off.
    fail_is(h.session.preview_pull_merge(&details, ForgeMergeMethod::Squash, &cancel), ForgeFailure::PullMergeBlocked);
    let preview = h.session.preview_pull_merge(&details, ForgeMergeMethod::Merge, &cancel).unwrap();
    assert_eq!(h.session.merge_pull(&preview, &cancel).unwrap().commit, MERGE);
    let posts = h.posts();
    assert_eq!(posts.len(), 1);
    assert_eq!(posts[0].path, "/forgejo/repos/team/web/pulls/9/merge");
    // Forgejo's MergePullRequestOption names the style `Do`.
    assert!(posts[0].body.starts_with(&format!(r#"{{"Do":"merge","head_commit_id":"{HEAD}""#)), "{}", posts[0].body);
});

case!(conversations_are_read_only, {
    let h = Forgejo::new();
    let cancel = CancellationToken::new();
    let threads = h.session.pull_threads(&h.repo, 9, &cancel).unwrap();
    let t = &threads.threads[0];
    assert_eq!((t.path.as_str(), t.line, t.comments[0].body.as_str()), ("src/lib.rs", Some(2), "Use a constant"));
    assert!(!t.resolved);
    // Forgejo has no API to reply to, resolve or reopen a conversation.
    assert!(!t.can_reply && !t.can_resolve && !t.can_unresolve);
    for action in [ForgeThreadAction::Resolve, ForgeThreadAction::Unresolve, ForgeThreadAction::Reply] {
        let text = if action == ForgeThreadAction::Reply { "Done" } else { "" };
        assert!(h.session.preview_pull_thread(&threads, 0, action, text, &cancel).is_err(), "{action:?}");
    }
    assert!(h.posts().is_empty());
});
