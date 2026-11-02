use super::*;

macro_rules! case {
    ($name:ident, $body:block) => {
        #[test]
        fn $name() {
            isolated(
                "ROCKET_PR_WORKFLOW_CASE",
                concat!("pr_workflow_tests::", stringify!($name)),
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
fn summary(mode: usize) -> String {
    let head = if mode == 1 { "c" } else { "a" }.repeat(40);
    let base = if mode == 2 { "d" } else { "b" }.repeat(40);
    let mergeable = if mode == 3 { "null" } else { "true" };
    let state = if mode == 4 { "blocked" } else { "clean" };
    let draft = mode == 5;
    format!(
        r#"{{"number":42,"title":"PRIVATE PR","body":"PRIVATE description","user":{{"login":"contributor"}},"state":"open","draft":{draft},"merged":false,"mergeable":{mergeable},"mergeable_state":"{state}","head":{{"sha":"{head}","ref":"main"}},"base":{{"sha":"{base}","ref":"main"}},"commits":1,"changed_files":3,"additions":2,"deletions":1}}"#
    )
}
pub(super) fn api(r: &Request, mode: usize) -> (u16, String, Vec<(String, String)>) {
    let p = r.path.strip_prefix("/github-enterprise/api/v3").unwrap_or_else(||r.path.trim_start_matches("/github"));
    if p == "/user" {
        return response(r#"{"login":"fixture"}"#);
    }
    if p == "/repos/team/sample" || p == "/repos/team/fork" || p == "/repos/author/fork" {
        let fork = !p.ends_with("sample");
        let root = if mode == 16 && fork { 99 } else { 1 };
        let permission = mode != 6;
        let allowed = mode != 7;
        let archived = mode == 8;
        return response(&format!(
            r#"{{"id":{root},"full_name":"{}","fork":{fork},"source":{{"id":{root}}},"permissions":{{"push":{permission}}},"archived":{archived},"allow_merge_commit":{allowed},"allow_squash_merge":{allowed},"allow_rebase_merge":{allowed}}}"#,
            p.trim_start_matches("/repos/")
        ));
    }
    if p.contains("/branches/") {
        let head = if p.contains("/fork/") {
            if mode == 17 { "c" } else { "a" }
        } else if mode == 18 {
            "d"
        } else {
            "b"
        };
        return response(&format!(r#"{{"commit":{{"sha":"{}"}}}}"#, head.repeat(40)));
    }
    if r.method == "PUT" {
        assert_eq!(p, "/repos/team/sample/pulls/42/merge");
        if (400..600).contains(&mode) {
            return (mode as u16, format!(r#"{{"message":"{TOKEN}"}}"#), vec![]);
        }
        if mode == 9 {
            return response(r#"{"merged":false}"#);
        }
        if mode == 10 {
            return response("{}");
        }
        if mode == 11 {
            thread::sleep(Duration::from_millis(900));
        }
        return response(&format!(r#"{{"merged":true,"sha":"{}"}}"#, "e".repeat(40)));
    }
    if r.method == "POST" && p.ends_with("/reviews") {
        return (
            201,
            format!(
                r#"{{"id":81,"user":{{"login":"fixture"}},"body":"PRIVATE review","state":"COMMENTED","commit_id":"{}","submitted_at":"today"}}"#,
                "a".repeat(40)
            ),
            vec![],
        );
    }
    if r.method == "POST" && p.ends_with("/pulls") {
        if mode == 20 {
            thread::sleep(Duration::from_millis(900));
        }
        let source = if mode == 19 {
            "someone/unrelated"
        } else if r.body.contains("team:main") {
            "team/fork"
        } else {
            "author/fork"
        };
        return (
            201,
            format!(
                r#"{{"number":43,"title":"PRIVATE new PR","state":"open","user":{{"login":"fixture"}},"head":{{"ref":"main","sha":"{}","repo":{{"full_name":"{source}"}}}},"base":{{"ref":"main","sha":"{}","repo":{{"full_name":"team/sample"}}}}}}"#,
                "a".repeat(40),
                "b".repeat(40)
            ),
            vec![],
        );
    }
    if p.ends_with("/pulls/42") {
        if mode == 12 {
            thread::sleep(Duration::from_millis(900));
        }
        return response(&summary(mode));
    }
    if p.contains("/files?") {
        let patch = if mode == 15 {
            "@@ -1,2 +1,2 @@\\n-old\\n+new"
        } else {
            "@@ -1,2 +1,3 @@\\n context\\n-old\\n+new 界\\n+extra"
        };
        return response(&format!(
            r#"[{{"filename":"資料/é 🚀.rs","previous_filename":"資料/旧.rs","status":"renamed","additions":2,"deletions":1,"patch":"{patch}"}},{{"filename":"binary.png","status":"modified","additions":0,"deletions":0}},{{"filename":"bad\u202epath","status":"modified","additions":2,"deletions":1,"patch":"{patch}"}}]"#
        ));
    }
    if p.contains("/check-runs?") {
        return response(r#"{"total_count":0,"check_runs":[]}"#);
    }
    if p.contains("/status?") {
        return response(&format!(
            r#"{{"sha":"{}","total_count":0,"statuses":[]}}"#,
            if mode == 1 { "c" } else { "a" }.repeat(40)
        ));
    }
    if ["/commits?", "/comments?", "/reviews?"]
        .iter()
        .any(|s| p.contains(s))
    {
        return response("[]");
    }
    panic!("Unexpected {} {}", r.method, r.path);
}
struct Harness {
    fixture: Fixture,
    server: Server,
    session: ForgeSession,
    repo: ForgeRepository,
    mode: Arc<AtomicUsize>,
}
impl Harness {
    fn new() -> Self {
        let fixture = Fixture::new();
        let mode = Arc::new(AtomicUsize::new(0));
        let m = mode.clone();
        let server = serve(Box::new(move |r| api(r, m.load(Ordering::Acquire))));
        let cfg = config(ForgeProvider::GitHub, server.port);
        let session = fixture
            .git
            .forge_session(cfg.clone(), "", TOKEN, &CancellationToken::new())
            .unwrap();
        let repo = ForgeRepository::new(cfg, "team/sample").unwrap();
        Self {
            fixture,
            server,
            session,
            repo,
            mode,
        }
    }
    fn mode(&self, mode: usize) {
        self.mode.store(mode, Ordering::Release);
    }
    fn details(&self) -> ForgePullDetails {
        self.session
            .pull_details(&self.repo, 42, &CancellationToken::new())
            .unwrap()
    }
    fn writes(&self) -> Vec<Request> {
        self.server
            .seen
            .lock()
            .unwrap()
            .iter()
            .filter(|r| r.method == "POST" || r.method == "PUT")
            .cloned()
            .collect()
    }
    fn fork_preview(&self, source: &str) -> Result<ForgePreview> {
        self.session.preview_from_fork(
            ForgeDraft {
                repository: self.repo.clone(),
                source: "main".into(),
                target: "main".into(),
                title: "PRIVATE new PR".into(),
                body: "PRIVATE body".into(),
            },
            ForgeRepository::new(self.repo.config().clone(), source).unwrap(),
            &CancellationToken::new(),
        )
    }
}

case!(
    inline_unicode_rename_sides_edit_and_atomic_review_payload,
    {
        let h = Harness::new();
        let d = h.details();
        let c = CancellationToken::new();
        assert_eq!(d.files.items[0].lines.len(), 5);
        let left = d
            .inline_comment(0, 2, ForgeDiffSide::Left, "PRIVATE old")
            .unwrap();
        let mut right = d
            .inline_comment(0, 2, ForgeDiffSide::Right, "first")
            .unwrap();
        right.edit_body("PRIVATE updated 界").unwrap();
        assert!(
            d.inline_comment(0, 3, ForgeDiffSide::Left, "invalid")
                .is_err()
        );
        assert!(
            d.inline_comment(1, 1, ForgeDiffSide::Right, "binary")
                .is_err()
        );
        assert!(
            d.inline_comment(2, 1, ForgeDiffSide::Right, "hidden path")
                .is_err()
        );
        assert!(right.edit_body("").is_err());
        assert!(right.edit_body(&"x".repeat(8193)).is_err());
        let p = h
            .session
            .preview_pull_review_with_comments(
                &d,
                ForgeReviewAction::Comment,
                "PRIVATE summary",
                &[left, right],
                &c,
            )
            .unwrap();
        h.session.submit_pull_review(&p, &c).unwrap();
        let writes = h.writes();
        assert_eq!(writes.len(), 1);
        let body = &writes[0].body;
        assert!(body.contains("\"commit_id\":\"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\""));
        assert!(body.contains(
            "\"path\":\"資料/é 🚀.rs\",\"line\":2,\"side\":\"LEFT\",\"body\":\"PRIVATE old\""
        ));
        assert!(body.contains("\"side\":\"RIGHT\",\"body\":\"PRIVATE updated 界\""));
        fail_is(
            h.session.submit_pull_review(&p, &c),
            ForgeFailure::PullReviewStale,
        );
    }
);
case!(
    inline_refuses_stale_locations_duplicates_and_partial_patches,
    {
        let h = Harness::new();
        let d = h.details();
        let c = CancellationToken::new();
        let comment = d
            .inline_comment(0, 2, ForgeDiffSide::Right, "PRIVATE")
            .unwrap();
        assert!(
            h.session
                .preview_pull_review_with_comments(
                    &d,
                    ForgeReviewAction::Comment,
                    "review",
                    &[comment.clone(), comment.clone()],
                    &c
                )
                .is_err()
        );
        h.mode(1);
        let new = h.details();
        fail_is(
            h.session.preview_pull_review_with_comments(
                &new,
                ForgeReviewAction::Comment,
                "review",
                std::slice::from_ref(&comment),
                &c,
            ),
            ForgeFailure::PullReviewStale,
        );
        h.mode(2);
        let new = h.details();
        fail_is(
            h.session.preview_pull_review_with_comments(
                &new,
                ForgeReviewAction::Comment,
                "review",
                &[comment],
                &c,
            ),
            ForgeFailure::PullReviewStale,
        );
        h.mode(15);
        assert!(h.details().files.items[0].lines.is_empty());
        assert!(h.writes().is_empty());
    }
);
case!(merge_methods_pin_head_and_use_previews_once, {
    let h = Harness::new();
    let c = CancellationToken::new();
    let before = std::fs::read(h.fixture.repository.join(".git/config")).unwrap();
    for method in [
        ForgeMergeMethod::Merge,
        ForgeMergeMethod::Squash,
        ForgeMergeMethod::Rebase,
    ] {
        let p = h
            .session
            .preview_pull_merge(&h.details(), method, &c)
            .unwrap();
        assert_eq!(h.session.merge_pull(&p, &c).unwrap().commit, "e".repeat(40));
        fail_is(h.session.merge_pull(&p, &c), ForgeFailure::PullReviewStale);
        assert_eq!(
            h.writes().last().unwrap().body,
            format!(
                "{{\"sha\":\"{}\",\"merge_method\":\"{}\"}}",
                "a".repeat(40),
                method.key()
            )
        );
    }
    assert_eq!(h.writes().len(), 3);
    assert_eq!(
        before,
        std::fs::read(h.fixture.repository.join(".git/config")).unwrap()
    );
});
case!(
    merge_blocks_missing_mergeability_permissions_methods_and_drafts,
    {
        let h = Harness::new();
        let c = CancellationToken::new();
        for mode in 3..=8 {
            h.mode(mode);
            fail_is(
                h.session
                    .preview_pull_merge(&h.details(), ForgeMergeMethod::Merge, &c),
                ForgeFailure::PullMergeBlocked,
            );
        }
        assert!(h.writes().is_empty());
    }
);
case!(
    merge_revalidates_head_base_permissions_and_session_before_write,
    {
        let h = Harness::new();
        let c = CancellationToken::new();
        for (mode, failure) in [
            (1, ForgeFailure::PullChanged),
            (2, ForgeFailure::PullChanged),
            (4, ForgeFailure::PullMergeBlocked),
            (6, ForgeFailure::PullMergeBlocked),
        ] {
            h.mode(0);
            let p = h
                .session
                .preview_pull_merge(&h.details(), ForgeMergeMethod::Merge, &c)
                .unwrap();
            h.mode(mode);
            fail_is(h.session.merge_pull(&p, &c), failure);
        }
        h.mode(0);
        let d = h.details();
        let other = h
            .fixture
            .git
            .forge_session(h.repo.config().clone(), "", TOKEN, &c)
            .unwrap();
        fail_is(
            other.preview_pull_merge(&d, ForgeMergeMethod::Merge, &c),
            ForgeFailure::PullReviewStale,
        );
        assert!(h.writes().is_empty());
    }
);
case!(
    merge_rejections_and_unknown_responses_are_distinct_and_private,
    {
        let h = Harness::new();
        let c = CancellationToken::new();
        for (mode, expected) in [
            (405, ForgeFailure::PullMergeRejected),
            (409, ForgeFailure::PullMergeRejected),
            (422, ForgeFailure::PullMergeRejected),
            (403, ForgeFailure::Forbidden),
            (9, ForgeFailure::PullMergeRejected),
            (10, ForgeFailure::PullMergeUnknown),
        ] {
            h.mode(0);
            let p = h
                .session
                .preview_pull_merge(&h.details(), ForgeMergeMethod::Merge, &c)
                .unwrap();
            h.mode(mode);
            let err = h.session.merge_pull(&p, &c).unwrap_err();
            assert!(!err.to_string().contains(TOKEN));
            fail_is::<()>(Err(err), expected);
        }
        assert_eq!(h.writes().len(), 6);
    }
);
fn cancel_after_request<T: Send>(
    h: &Harness,
    method: &str,
    run: impl FnOnce(&CancellationToken) -> Result<T> + Send,
) -> Result<T> {
    let c = CancellationToken::new();
    let count = h.server.seen.lock().unwrap().len();
    thread::scope(|scope| {
        let handle = scope.spawn(|| run(&c));
        let start = Instant::now();
        while !h
            .server
            .seen
            .lock()
            .unwrap()
            .iter()
            .skip(count)
            .any(|r| r.method == method)
        {
            assert!(start.elapsed() < Duration::from_secs(10));
            thread::sleep(Duration::from_millis(5));
        }
        c.cancel();
        handle.join().unwrap()
    })
}
case!(
    merge_cancel_before_send_is_cancelled_after_send_is_unknown,
    {
        let h = Harness::new();
        let c = CancellationToken::new();
        let p = h
            .session
            .preview_pull_merge(&h.details(), ForgeMergeMethod::Merge, &c)
            .unwrap();
        c.cancel();
        assert!(matches!(
            h.session.merge_pull(&p, &c),
            Err(Error::Cancelled)
        ));
        assert!(h.writes().is_empty());
        h.mode(11);
        fail_is(
            cancel_after_request(&h, "PUT", |cancel| h.session.merge_pull(&p, cancel)),
            ForgeFailure::PullMergeUnknown,
        );
        assert_eq!(h.writes().len(), 1);
        fail_is(
            h.session.merge_pull(&p, &CancellationToken::new()),
            ForgeFailure::PullReviewStale,
        );
    }
);
case!(
    fork_same_named_branches_and_same_org_send_explicit_source_repository,
    {
        let h = Harness::new();
        let c = CancellationToken::new();
        for source in ["author/fork", "team/fork"] {
            let p = h.fork_preview(source).unwrap();
            assert_eq!(p.source_repository().path(), source);
            assert_ne!(p.source_id(), p.target_id());
            assert_eq!(h.session.create(&p, &c).unwrap().number, 43);
            let r = h.writes().pop().unwrap();
            assert!(r.path.ends_with("/repos/team/sample/pulls"));
            assert!(r.body.contains(&format!(
                "\"head\":\"{}:main\",\"head_repo\":\"fork\"",
                source.split('/').next().unwrap()
            )));
            assert!(r.body.contains("\"maintainer_can_modify\":false"));
            fail_is(h.session.create(&p, &c), ForgeFailure::StaleReview);
        }
    }
);
case!(fork_network_source_and_target_changes_refuse_before_post, {
    let h = Harness::new();
    let c = CancellationToken::new();
    h.mode(16);
    assert!(h.fork_preview("author/fork").is_err());
    for mode in [16, 17, 18] {
        h.mode(0);
        let p = h.fork_preview("author/fork").unwrap();
        h.mode(mode);
        assert!(h.session.create(&p, &c).is_err());
    }
    assert!(h.writes().is_empty());
});
case!(
    fork_unexpected_response_and_cancelled_submission_do_not_retry,
    {
        let h = Harness::new();
        let c = CancellationToken::new();
        let p = h.fork_preview("author/fork").unwrap();
        h.mode(19);
        fail_is(h.session.create(&p, &c), ForgeFailure::OutcomeUnknown);
        fail_is(h.session.create(&p, &c), ForgeFailure::StaleReview);
        h.mode(0);
        let p = h.fork_preview("author/fork").unwrap();
        h.mode(20);
        fail_is(
            cancel_after_request(&h, "POST", |cancel| h.session.create(&p, cancel)),
            ForgeFailure::OutcomeUnknown,
        );
        assert_eq!(h.writes().len(), 2);
    }
);
