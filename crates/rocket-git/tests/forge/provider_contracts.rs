//! Responses shaped like the hosts' documented examples, for contracts the generic
//! fixtures above don't model: GitHub's repository-ID page links, Bitbucket Cloud's
//! abbreviated pull request commits, and GitLab's merge 401, fast-forward merges and
//! Community Edition approval state.
use super::*;

macro_rules! case {
    ($name:ident, $body:block) => {
        #[test]
        fn $name() {
            isolated(
                "ROCKET_PROVIDER_CONTRACT_CASE",
                concat!("provider_contract_tests::", stringify!($name)),
                || $body,
            );
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
type Reply = (u16, String, Vec<(String, String)>);

case!(github_links_that_name_the_repository_by_id_continue_the_list, {
    let f = Fixture::new();
    // "Using pagination in the REST API": a request to /repos/octocat/Spoon-Knife/issues
    // is answered with links to /repositories/1300192/issues?page=N.
    let server = serve(Box::new(|r| {
        if r.path.ends_with("/user") {
            return response(r#"{"login":"fixture"}"#);
        }
        let second = r.path.ends_with("&page=2");
        let query = r.path.split_once('?').unwrap().1.replace("&page=1", "&page=2");
        let origin = fixture_origin(r);
        let link = format!(
            "<{origin}/github/repositories/1300192/pulls?{query}>; rel=\"next\", <{origin}/github/repositories/1300192/pulls?page=9>; rel=\"last\""
        );
        let headers = if second { vec![] } else { vec![("Link".into(), link)] };
        (200, format!("[{}]", item(ForgeProvider::GitHub, if second { 2 } else { 1 })), headers)
    }));
    let config = config(ForgeProvider::GitHub, server.port);
    let cancel = CancellationToken::new();
    let session = f.git.forge_session(config.clone(), "", TOKEN, &cancel).unwrap();
    let repo = ForgeRepository::new(config, "team/web").unwrap();
    let first = session.list(&repo, &ForgeFilter::default(), ForgeCursor::default(), &cancel).unwrap();
    let next = first.next.expect("a link naming the repository by ID continues the list");
    let second = session.list(&repo, &ForgeFilter::default(), next, &cancel).unwrap();
    assert_eq!(second.items[0].number, 2);
    assert!(second.next.is_none());
    // The next request keeps Rocket's own repository path; only the query is taken.
    let seen = server.seen.lock().unwrap();
    assert_eq!(
        seen.last().unwrap().path,
        "/github/repos/team/web/pulls?state=open&sort=updated&direction=desc&per_page=50&page=2"
    );
    assert!(seen.iter().all(|r| !r.path.contains("/repositories/")));
});

/// Bitbucket Cloud's pull request object carries 12-character commit hashes, as in
/// Atlassian's "Event payloads" example (`"hash": "d3022fc0ca3d"`).
fn bitbucket(r: &Request, merged: &AtomicBool) -> Reply {
    let path = r.path.strip_prefix("/bitbucket").unwrap_or(&r.path);
    let path = path.split('?').next().unwrap();
    let pr = "/repositories/team/web/pullrequests/42";
    let pull = |state: &str| {
        let merge = if state == "MERGED" { format!(r#","merge_commit":{{"hash":"{}"}}"#, &MERGE[..12]) } else { String::new() };
        format!(
            r#"{{"id":42,"title":"Fix","description":"","state":"{state}","draft":false,"author":{{"display_name":"Author","uuid":"{{author}}"}},"source":{{"branch":{{"name":"feature"}},"commit":{{"hash":"{}"}}}},"destination":{{"branch":{{"name":"main"}},"commit":{{"hash":"{}"}}}},"participants":[]{merge}}}"#,
            &HEAD[..12],
            &BASE[..12]
        )
    };
    match (r.method.as_str(), path) {
        ("GET", "/user") => response(r#"{"display_name":"fixture","uuid":"{fixture}"}"#),
        ("GET", p) if p == pr => response(&pull(if merged.load(Ordering::Acquire) { "MERGED" } else { "OPEN" })),
        ("POST", p) if p == format!("{pr}/merge") => {
            merged.store(true, Ordering::Release);
            response(&pull("MERGED"))
        }
        ("GET", p) if p.starts_with("/repositories/team/web/commit/") => {
            let asked = &p["/repositories/team/web/commit/".len()..];
            match [HEAD, BASE, MERGE].into_iter().find(|id| asked.len() >= 7 && id.starts_with(asked)) {
                Some(id) => response(&format!(r#"{{"hash":"{id}","type":"commit"}}"#)),
                None => (404, "{}".into(), Vec::new()),
            }
        }
        ("GET", "/repositories/team/web/refs/branches/main") => response(&format!(r#"{{"target":{{"hash":"{BASE}"}}}}"#)),
        _ => response(r#"{"values":[]}"#),
    }
}
case!(bitbucket_cloud_resolves_abbreviated_pull_request_commits, {
    let f = Fixture::new();
    let merged = Arc::new(AtomicBool::new(false));
    let state = merged.clone();
    let server = serve(Box::new(move |r| bitbucket(r, &state)));
    let config = config(ForgeProvider::Bitbucket, server.port);
    let cancel = CancellationToken::new();
    let session = f.git.forge_session(config.clone(), "", TOKEN, &cancel).unwrap();
    let repo = ForgeRepository::new(config, "team/web").unwrap();
    let details = session.pull_details(&repo, 42, &cancel).unwrap();
    assert_eq!((details.summary().head.as_str(), details.summary().base.as_str()), (HEAD, BASE));
    assert!(details.files.error.is_none());
    let preview = session.preview_pull_merge(&details, ForgeMergeMethod::Merge, &cancel).unwrap();
    let result = session.merge_pull(&preview, &cancel).unwrap();
    assert_eq!(result.commit, MERGE);
    let seen = server.seen.lock().unwrap();
    assert!(seen.iter().any(|r| r.path.contains(&format!("/diffstat/{HEAD}..{BASE}?"))));
    assert!(seen.iter().any(|r| r.path.ends_with(&format!("/commit/{}", &HEAD[..12]))));
    assert_eq!(seen.iter().filter(|r| r.method == "POST").count(), 1);
});
case!(bitbucket_cloud_refuses_a_commit_that_does_not_match_its_abbreviation, {
    let f = Fixture::new();
    let server = serve(Box::new(|r| {
        if r.path.contains("/commit/") {
            // A full ID that does not start with the requested abbreviation.
            return response(&format!(r#"{{"hash":"{MERGE}"}}"#));
        }
        bitbucket(r, &AtomicBool::new(false))
    }));
    let config = config(ForgeProvider::Bitbucket, server.port);
    let cancel = CancellationToken::new();
    let session = f.git.forge_session(config.clone(), "", TOKEN, &cancel).unwrap();
    let repo = ForgeRepository::new(config, "team/web").unwrap();
    fail_is(session.pull_details(&repo, 42, &cancel), ForgeFailure::InvalidResponse);
});

/// A GitLab merge request that is open and mergeable, by someone else.
fn gitlab(r: &Request, attempted: &AtomicBool, revoked: bool) -> Reply {
    let path = r.path.strip_prefix("/gitlab").unwrap_or(&r.path);
    let path = path.split('?').next().unwrap();
    let mr = "/projects/team%2Fweb/merge_requests/7";
    match (r.method.as_str(), path) {
        ("GET", "/user") if revoked && attempted.load(Ordering::Acquire) => (401, r#"{"message":"401 Unauthorized"}"#.into(), Vec::new()),
        ("GET", "/user") => response(r#"{"username":"fixture"}"#),
        ("GET", p) if p == mr => response(&format!(
            r#"{{"iid":7,"title":"Fix","description":"","state":"opened","draft":false,"sha":"{HEAD}","source_branch":"feature","target_branch":"main","detailed_merge_status":"mergeable","author":{{"username":"author"}},"diff_refs":{{"base_sha":"{BASE}","head_sha":"{HEAD}","start_sha":"{BASE}"}}}}"#
        )),
        ("GET", "/projects/team%2Fweb/repository/branches/main") => response(&format!(r#"{{"commit":{{"id":"{BASE}"}}}}"#)),
        ("GET", p) if p == format!("{mr}/approvals") => response(r#"{"approved_by":[]}"#),
        // "401 Unauthorized: This user does not have permission to accept this merge request."
        ("PUT", p) if p == format!("{mr}/merge") => {
            attempted.store(true, Ordering::Release);
            (401, r#"{"message":"401 Unauthorized"}"#.into(), Vec::new())
        }
        // Community Edition "returns only approved, approved_by, user_can_approve, and
        // user_has_approved", with 201 Created.
        ("POST", p) if p == format!("{mr}/approve") => (
            201,
            r#"{"approved":true,"approved_by":[{"user":{"id":1,"username":"fixture","name":"Fixture"}}],"user_can_approve":false,"user_has_approved":true}"#.into(),
            Vec::new(),
        ),
        _ => response("[]"),
    }
}
case!(gitlab_merge_refused_for_permission_keeps_the_session, {
    let f = Fixture::new();
    for (revoked, expected) in [(false, ForgeFailure::Forbidden), (true, ForgeFailure::Unauthorized)] {
        let attempted = Arc::new(AtomicBool::new(false));
        let flag = attempted.clone();
        let server = serve(Box::new(move |r| gitlab(r, &flag, revoked)));
        let config = config(ForgeProvider::GitLab, server.port);
        let cancel = CancellationToken::new();
        let session = f.git.forge_session(config.clone(), "", TOKEN, &cancel).unwrap();
        let repo = ForgeRepository::new(config, "team/web").unwrap();
        let details = session.pull_details(&repo, 7, &cancel).unwrap();
        let preview = session.preview_pull_merge(&details, ForgeMergeMethod::Merge, &cancel).unwrap();
        // A refused merge reads as Forbidden; only a token the account endpoint
        // also rejects is Unauthorized, which signs the user out.
        fail_is(session.merge_pull(&preview, &cancel), expected);
        let seen = server.seen.lock().unwrap();
        let put = seen.iter().position(|r| r.method == "PUT").expect("one merge request");
        assert_eq!(seen.iter().filter(|r| r.method == "PUT").count(), 1);
        assert!(seen[put + 1..].iter().any(|r| r.method == "GET" && r.path.ends_with("/user")));
    }
});
/// A fast-forward merge creates no merge commit: GitLab answers merged with both
/// commit fields empty, and the target branch then points at the reviewed head.
fn gitlab_fast_forward(r: &Request, merged: &AtomicBool, tip_after: &'static str) -> Reply {
    let path = r.path.strip_prefix("/gitlab").unwrap_or(&r.path);
    match (r.method.as_str(), path.split('?').next().unwrap()) {
        ("PUT", "/projects/team%2Fweb/merge_requests/7/merge") => {
            merged.store(true, Ordering::Release);
            response(&format!(
                r#"{{"iid":7,"title":"Fix","description":"","state":"merged","draft":false,"sha":"{HEAD}","source_branch":"feature","target_branch":"main","detailed_merge_status":"not_open","author":{{"username":"author"}},"merge_commit_sha":null,"squash_commit_sha":null,"diff_refs":{{"base_sha":"{BASE}","head_sha":"{HEAD}","start_sha":"{BASE}"}}}}"#
            ))
        }
        ("GET", "/projects/team%2Fweb/repository/branches/main") if merged.load(Ordering::Acquire) => {
            response(&format!(r#"{{"commit":{{"id":"{tip_after}"}}}}"#))
        }
        _ => gitlab(r, &AtomicBool::new(false), false),
    }
}
case!(gitlab_fast_forward_merge_is_confirmed_by_the_target_branch, {
    let f = Fixture::new();
    for (tip_after, expected) in [(HEAD, Some(HEAD)), (MERGE, None)] {
        let merged = Arc::new(AtomicBool::new(false));
        let flag = merged.clone();
        let server = serve(Box::new(move |r| gitlab_fast_forward(r, &flag, tip_after)));
        let config = config(ForgeProvider::GitLab, server.port);
        let cancel = CancellationToken::new();
        let session = f.git.forge_session(config.clone(), "", TOKEN, &cancel).unwrap();
        let repo = ForgeRepository::new(config, "team/web").unwrap();
        let details = session.pull_details(&repo, 7, &cancel).unwrap();
        let preview = session.preview_pull_merge(&details, ForgeMergeMethod::Merge, &cancel).unwrap();
        match expected {
            // The target now points at the reviewed head: that is the result.
            Some(commit) => assert_eq!(session.merge_pull(&preview, &cancel).unwrap().commit, commit),
            // Anything else stays an outcome to inspect.
            None => fail_is(session.merge_pull(&preview, &cancel), ForgeFailure::PullMergeUnknown),
        }
        assert_eq!(server.seen.lock().unwrap().iter().filter(|r| r.method == "PUT").count(), 1);
    }
});
case!(gitlab_community_edition_approval_is_confirmed, {
    let f = Fixture::new();
    let server = serve(Box::new(|r| gitlab(r, &AtomicBool::new(false), false)));
    let config = config(ForgeProvider::GitLab, server.port);
    let cancel = CancellationToken::new();
    let session = f.git.forge_session(config.clone(), "", TOKEN, &cancel).unwrap();
    let repo = ForgeRepository::new(config, "team/web").unwrap();
    let details = session.pull_details(&repo, 7, &cancel).unwrap();
    let preview = session.preview_pull_review(&details, ForgeReviewAction::Approve, "", &cancel).unwrap();
    let review = session.submit_pull_review(&preview, &cancel).unwrap();
    assert_eq!((review.state.as_str(), review.author.as_str()), ("APPROVED", "fixture"));
    let seen = server.seen.lock().unwrap();
    let approve = seen.iter().find(|r| r.method == "POST").unwrap();
    assert!(approve.path.ends_with("/merge_requests/7/approve"));
    assert_eq!(approve.body, format!(r#"{{"sha":"{HEAD}"}}"#));
});
