use super::*;

macro_rules! case {
    ($name:ident, $body:block) => {
        #[test]
        fn $name() {
            isolated(
                "ROCKET_PULL_REVIEW_CASE",
                concat!("pull_review_tests::", stringify!($name)),
                || $body,
            );
        }
    };
}
fn summary(mode: usize) -> String {
    let head = if mode == 1 { "c" } else { "a" }.repeat(40);
    let base = if mode == 16 { "d" } else { "b" }.repeat(40);
    let author = if mode == 12 { "fixture" } else { "contributor" };
    let state = if mode == 13 { "closed" } else { "open" };
    let files = if mode == 6 { 250 } else { 1 };
    format!(
        r#"{{"number":42,"title":"PRIVATE Unicode 界\u202e","body":"PRIVATE description\nline two","user":{{"login":"{author}"}},"state":"{state}","draft":false,"merged":false,"head":{{"sha":"{head}","ref":"feature/界"}},"base":{{"sha":"{base}","ref":"main"}},"commits":1,"changed_files":{files},"additions":2,"deletions":1}}"#
    )
}
fn review_result(event: &str, sha: &str) -> String {
    let state = match event {
        "APPROVE" => "APPROVED",
        "REQUEST_CHANGES" => "CHANGES_REQUESTED",
        _ => "COMMENTED",
    };
    format!(
        r#"{{"id":81,"user":{{"login":"fixture"}},"body":"PRIVATE review","state":"{state}","commit_id":"{sha}","submitted_at":"2026-10-27T00:00:00Z"}}"#
    )
}
fn api(req: &Request, mode: usize) -> (u16, String, Vec<(String, String)>) {
    if req.path.ends_with("/user") {
        return response(r#"{"login":"fixture"}"#);
    }
    if req.method == "POST" {
        if mode == 8 {
            return (422, format!(r#"{{"message":"{TOKEN}"}}"#), vec![]);
        }
        if mode == 9 {
            return response("{}");
        }
        if mode == 10 {
            thread::sleep(Duration::from_millis(800));
        }
        let event = if req.body.contains("REQUEST_CHANGES") {
            "REQUEST_CHANGES"
        } else if req.body.contains("APPROVE") {
            "APPROVE"
        } else {
            "COMMENT"
        };
        return (
            201,
            review_result(event, &if mode == 15 { "c" } else { "a" }.repeat(40)),
            vec![],
        );
    }
    if req.path.ends_with("/pulls/42") {
        if mode == 7 {
            return response("{}");
        }
        if mode == 11 {
            thread::sleep(Duration::from_millis(800));
        }
        return response(&summary(mode));
    }
    if req.path.contains("/pulls/42/commits?") {
        return response(&format!(
            r#"[{{"sha":"{}","commit":{{"message":"PRIVATE title\nbody","author":{{"name":"作者"}}}}}}]"#,
            "a".repeat(40)
        ));
    }
    if req.path.contains("/files?") {
        let json = r#"[{"filename":"資料/新.rs","previous_filename":"資料/旧.rs","status":"renamed","additions":2,"deletions":1,"patch":"@@ -1 +1 @@\n-old\n+new 界\u202e"}]"#;
        if mode == 19 {
            let page: u32 = req.path.rsplit("page=").next().unwrap().parse().unwrap();
            let endpoint = req.path.split('?').next().unwrap();
            return (
                200,
                format!(
                    "[{}]",
                    std::iter::repeat_n(&json[1..json.len() - 1], 50)
                        .collect::<Vec<_>>()
                        .join(",")
                ),
                vec![(
                    "Link".into(),
                    format!(
                        "<{}{endpoint}?per_page=50&page={}>; rel=\"next\"",
                        fixture_origin(req),
                        page + 1
                    ),
                )],
            );
        }
        if mode == 4 {
            return (
                200,
                json.into(),
                vec![(
                    "Link".into(),
                    "<http://127.0.0.1:1/stolen>; rel=\"next\"".into(),
                )],
            );
        }
        if mode == 5 {
            let endpoint = req.path.split('?').next().unwrap();
            return (
                200,
                json.into(),
                vec![(
                    "Link".into(),
                    format!(
                        "<{}{endpoint}?per_page=50&page=2>; rel=\"next\"",
                        fixture_origin(req)
                    ),
                )],
            );
        }
        if mode == 6 {
            return response(&format!(
                "[{}]",
                std::iter::repeat_n(&json[1..json.len() - 1], 50)
                    .collect::<Vec<_>>()
                    .join(",")
            ));
        }
        if mode == 17 {
            return response(
                &json.replace("@@ -1 +1 @@\\n-old\\n+new 界\\u202e", &"x".repeat(65537)),
            );
        }
        return response(json);
    }
    if req.path.contains("/issues/42/comments?") {
        return response(
            r#"[{"id":2,"user":{"login":"author"},"body":"PRIVATE discussion","created_at":"today"}]"#,
        );
    }
    if req.path.contains("/pulls/42/comments?") {
        return response(
            r#"[{"id":3,"user":{"login":"author"},"body":"PRIVATE inline","created_at":"today","path":"資料/新.rs","line":2,"in_reply_to_id":1}]"#,
        );
    }
    if req.path.contains("/reviews?") {
        if mode == 3 {
            return (429, "{}".into(), vec![]);
        }
        return response(&format!("[{}]", review_result("COMMENT", &"a".repeat(40))));
    }
    if req.path.contains("/check-runs?") {
        if mode == 2 {
            return (403, format!(r#"{{"message":"{TOKEN}"}}"#), vec![]);
        }
        return response(&format!(
            r#"{{"total_count":1,"check_runs":[{{"name":"build","head_sha":"{}","status":"completed","conclusion":"failure"}}]}}"#,
            "a".repeat(40)
        ));
    }
    if req.path.contains("/status?") {
        return response(&format!(
            r#"{{"sha":"{}","total_count":1,"statuses":[{{"context":"lint","state":"pending"}}]}}"#,
            if mode == 18 { "c" } else { "a" }.repeat(40)
        ));
    }
    panic!("unexpected {} {}", req.method, req.path);
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
        let repo = ForgeRepository::new(cfg, "team/資料").unwrap();
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
    fn posts(&self) -> usize {
        self.server
            .seen
            .lock()
            .unwrap()
            .iter()
            .filter(|r| r.method == "POST")
            .count()
    }
}
case!(details_unicode_rename_comments_and_exact_head_checks, {
    let h = Harness::new();
    let d = h.details();
    assert!(d.summary().title.contains("\\u{202e}"));
    assert_eq!(d.files.items[0].previous.as_deref(), Some("資料/旧.rs"));
    assert!(
        d.files.items[0]
            .patch
            .as_ref()
            .unwrap()
            .contains("\\u{202e}")
    );
    assert_eq!(d.discussion.items.len(), 1);
    assert_eq!(d.inline_comments.items[0].reply_to, Some(1));
    assert_eq!(d.commits.items[0].author, "作者");
    assert_eq!(d.checks.items[0].conclusion.as_deref(), Some("failure"));
    assert_eq!(d.statuses.items[0].status, "pending");
    assert_eq!(d.url(), h.repo.item_url(ForgeKind::Pulls, 42));
    assert_eq!(h.posts(), 0);
});
case!(
    collection_limits_stop_after_four_pages_and_remain_explicit,
    {
        let h = Harness::new();
        h.mode(19);
        let d = h.details();
        assert_eq!(d.files.items.len(), 200);
        assert!(d.files.limited);
        assert!(d.files.error.is_none());
        assert_eq!(
            h.server
                .seen
                .lock()
                .unwrap()
                .iter()
                .filter(|r| r.path.contains("/files?"))
                .count(),
            4
        );
    }
);
case!(concurrent_submission_of_one_preview_posts_once, {
    let h = Harness::new();
    let c = CancellationToken::new();
    let d = h.details();
    let p = h
        .session
        .preview_pull_review(&d, ForgeReviewAction::Comment, "draft", &c)
        .unwrap();
    let barrier = std::sync::Barrier::new(2);
    let run = || {
        barrier.wait();
        h.session.submit_pull_review(&p, &c)
    };
    let outcomes = thread::scope(|scope| {
        let a = scope.spawn(run);
        let b = scope.spawn(run);
        [a.join().unwrap(), b.join().unwrap()]
    });
    assert_eq!(outcomes.iter().filter(|r| r.is_ok()).count(), 1);
    assert_eq!(
        outcomes
            .iter()
            .filter(|r| matches!(r, Err(Error::Forge(ForgeFailure::PullReviewStale))))
            .count(),
        1
    );
    assert_eq!(h.posts(), 1);
});
case!(partial_unavailable_and_bounded_patches_are_honest, {
    let h = Harness::new();
    h.mode(2);
    assert_eq!(h.details().checks.error, Some(ForgeFailure::Forbidden));
    h.mode(3);
    assert_eq!(h.details().reviews.error, Some(ForgeFailure::RateLimited));
    h.mode(6);
    let d = h.details();
    assert!(d.files.limited);
    assert_eq!(d.files.items.len(), 50);
    h.mode(17);
    assert!(h.details().files.items[0].patch.is_none());
    h.mode(18);
    assert_eq!(
        h.details().statuses.error,
        Some(ForgeFailure::InvalidResponse)
    );
});
case!(unsafe_and_repeated_pagination_never_forwards_credentials, {
    let h = Harness::new();
    for mode in [4, 5] {
        h.mode(mode);
        assert_eq!(
            h.details().files.error,
            Some(ForgeFailure::InvalidPagination)
        );
    }
    assert!(
        !h.server
            .seen
            .lock()
            .unwrap()
            .iter()
            .any(|r| r.path.contains("stolen"))
    );
});
case!(comment_approve_changes_send_exact_commit_and_only_once, {
    let h = Harness::new();
    let c = CancellationToken::new();
    let d = h.details();
    for action in [
        ForgeReviewAction::Comment,
        ForgeReviewAction::Approve,
        ForgeReviewAction::RequestChanges,
    ] {
        let p = h
            .session
            .preview_pull_review(&d, action, "PRIVATE \"review\"\n界", &c)
            .unwrap();
        let result = h.session.submit_pull_review(&p, &c).unwrap();
        assert_eq!(result.commit, Some("a".repeat(40)));
        assert_eq!(
            failure(h.session.submit_pull_review(&p, &c)),
            ForgeFailure::PullReviewStale
        );
    }
    assert_eq!(h.posts(), 3);
    for req in h
        .server
        .seen
        .lock()
        .unwrap()
        .iter()
        .filter(|r| r.method == "POST")
    {
        assert!(
            req.body
                .contains(&format!("\"commit_id\":\"{}\"", "a".repeat(40)))
        );
        assert!(req.body.contains(r#"PRIVATE \"review\"\n界"#));
    }
});
case!(
    head_and_base_changes_reject_before_post_and_keep_preview_message,
    {
        let h = Harness::new();
        let c = CancellationToken::new();
        let d = h.details();
        for mode in [1, 16] {
            h.mode(0);
            let p = h
                .session
                .preview_pull_review(&d, ForgeReviewAction::Comment, "PRIVATE draft", &c)
                .unwrap();
            h.mode(mode);
            assert!(matches!(
                h.session
                    .preview_pull_review(&d, ForgeReviewAction::Comment, "draft", &c),
                Err(Error::Forge(ForgeFailure::PullChanged))
            ));
            assert_eq!(
                failure(h.session.submit_pull_review(&p, &c)),
                ForgeFailure::PullChanged
            );
            assert_eq!(p.body(), "PRIVATE draft");
        }
        assert_eq!(h.posts(), 0);
    }
);
case!(mixed_revision_snapshot_is_not_published, {
    let f = Fixture::new();
    let count = Arc::new(AtomicUsize::new(0));
    let seen = count.clone();
    let server = serve(Box::new(move |r| {
        let mode = if r.path.ends_with("/pulls/42") && seen.fetch_add(1, Ordering::AcqRel) > 0 {
            1
        } else {
            0
        };
        api(r, mode)
    }));
    let cfg = config(ForgeProvider::GitHub, server.port);
    let c = CancellationToken::new();
    let s = f.git.forge_session(cfg.clone(), "", TOKEN, &c).unwrap();
    let repo = ForgeRepository::new(cfg, "team/repo").unwrap();
    assert_eq!(
        failure(s.pull_details(&repo, 42, &c)),
        ForgeFailure::PullChanged
    );
});
case!(
    closed_own_pr_and_empty_review_are_rejected_without_writes,
    {
        let h = Harness::new();
        let c = CancellationToken::new();
        let d = h.details();
        assert!(
            h.session
                .preview_pull_review(&d, ForgeReviewAction::Comment, "  ", &c)
                .is_err()
        );
        assert!(
            h.session
                .preview_pull_review(&d, ForgeReviewAction::RequestChanges, "", &c)
                .is_err()
        );
        assert!(
            h.session
                .preview_pull_review(&d, ForgeReviewAction::Approve, "", &c)
                .is_ok()
        );
        h.mode(12);
        let d = h.details();
        assert!(
            h.session
                .preview_pull_review(&d, ForgeReviewAction::Approve, "", &c)
                .is_err()
        );
        assert!(
            h.session
                .preview_pull_review(&d, ForgeReviewAction::Comment, "hi", &c)
                .is_ok()
        );
        h.mode(13);
        assert!(
            h.session
                .preview_pull_review(&h.details(), ForgeReviewAction::Comment, "hi", &c)
                .is_err()
        );
        assert_eq!(h.posts(), 0);
    }
);
case!(
    rejection_and_unverifiable_success_are_distinct_without_retry,
    {
        let h = Harness::new();
        let c = CancellationToken::new();
        let d = h.details();
        for (mode, expected) in [
            (8, ForgeFailure::PullReviewRejected),
            (9, ForgeFailure::PullReviewUnknown),
            (15, ForgeFailure::PullReviewUnknown),
        ] {
            h.mode(0);
            let p = h
                .session
                .preview_pull_review(&d, ForgeReviewAction::Comment, "draft", &c)
                .unwrap();
            h.mode(mode);
            assert_eq!(failure(h.session.submit_pull_review(&p, &c)), expected);
            assert_eq!(
                failure(h.session.submit_pull_review(&p, &c)),
                ForgeFailure::PullReviewStale
            );
        }
        assert_eq!(h.posts(), 3);
    }
);
case!(
    cancel_before_submit_does_not_post_and_cancel_in_flight_is_unknown,
    {
        let h = Harness::new();
        let c = CancellationToken::new();
        let d = h.details();
        let p = h
            .session
            .preview_pull_review(&d, ForgeReviewAction::Comment, "draft", &c)
            .unwrap();
        let stopped = CancellationToken::new();
        stopped.cancel();
        assert!(matches!(
            h.session.submit_pull_review(&p, &stopped),
            Err(Error::Cancelled)
        ));
        assert_eq!(h.posts(), 0);
        h.mode(10);
        let seen = h.server.seen.clone();
        let cancel = c.clone();
        let worker = thread::spawn(move || {
            let start = Instant::now();
            while !seen.lock().unwrap().iter().any(|r| r.method == "POST") {
                assert!(start.elapsed() < Duration::from_secs(10));
                thread::sleep(Duration::from_millis(5));
            }
            cancel.cancel();
        });
        assert_eq!(
            failure(h.session.submit_pull_review(&p, &c)),
            ForgeFailure::PullReviewUnknown
        );
        worker.join().unwrap();
        assert_eq!(h.posts(), 1);
    }
);
case!(
    cancel_read_and_invalid_response_do_not_return_new_details,
    {
        let h = Harness::new();
        let c = CancellationToken::new();
        h.mode(7);
        assert_eq!(
            failure(h.session.pull_details(&h.repo, 42, &c)),
            ForgeFailure::InvalidResponse
        );
        h.mode(11);
        let seen = h.server.seen.clone();
        seen.lock().unwrap().clear();
        let cancel = c.clone();
        let worker = thread::spawn(move || {
            let start = Instant::now();
            while !seen
                .lock()
                .unwrap()
                .iter()
                .any(|r| r.path.ends_with("/pulls/42"))
            {
                assert!(start.elapsed() < Duration::from_secs(10));
                thread::sleep(Duration::from_millis(5));
            }
            cancel.cancel();
        });
        assert!(matches!(
            h.session.pull_details(&h.repo, 42, &c),
            Err(Error::Cancelled)
        ));
        worker.join().unwrap();
    }
);
case!(sessions_and_providers_are_bound_to_review_previews, {
    let h = Harness::new();
    let c = CancellationToken::new();
    let d = h.details();
    let p = h
        .session
        .preview_pull_review(&d, ForgeReviewAction::Comment, "draft", &c)
        .unwrap();
    let s = h
        .fixture
        .git
        .forge_session(h.repo.config().clone(), "", TOKEN, &c)
        .unwrap();
    assert_eq!(
        failure(s.submit_pull_review(&p, &c)),
        ForgeFailure::PullReviewStale
    );
    assert!(matches!(
        s.preview_pull_review(&d, ForgeReviewAction::Comment, "draft", &c),
        Err(Error::Forge(ForgeFailure::PullReviewStale))
    ));
    assert_eq!(h.posts(), 0);
});
case!(
    diagnostics_record_steps_without_tokens_paths_or_review_text,
    {
        let h = Harness::new();
        let c = CancellationToken::new();
        let dir = h.fixture.root.join("pull-review-logs");
        let log = DiagnosticLog::open(&dir).unwrap();
        let d = log
            .request("hosting-pull-details", |l| {
                h.session.with_log(l.clone()).pull_details(&h.repo, 42, &c)
            })
            .unwrap();
        let p = log
            .request("hosting-review-preview", |l| {
                h.session.with_log(l.clone()).preview_pull_review(
                    &d,
                    ForgeReviewAction::Comment,
                    "PRIVATE review",
                    &c,
                )
            })
            .unwrap();
        log.request("hosting-review-submit", |l| {
            h.session.with_log(l.clone()).submit_pull_review(&p, &c)
        })
        .unwrap();
        drop(log);
        let logs = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| std::fs::read_to_string(e.unwrap().path()).ok())
            .collect::<String>();
        for step in [
            "hosting-pull-details",
            "read_pull_files",
            "read_pull_checks",
            "validate_pull_review",
            "publish_pull_review",
        ] {
            assert!(logs.contains(step), "missing {step}");
        }
        for secret in [TOKEN, "PRIVATE", "資料", "Authorization", "contributor"] {
            assert!(!logs.contains(secret), "leak {secret}");
        }
    }
);
