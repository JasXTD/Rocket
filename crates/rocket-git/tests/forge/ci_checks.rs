//! CI marks, commit checks and reviewed re-runs against provider-shaped mock APIs.
use super::*;

macro_rules! case {
    ($name:ident, $body:block) => {
        #[test]
        fn $name() {
            isolated("ROCKET_CI_CHECKS_CASE", concat!("ci_checks_tests::", stringify!($name)), || $body);
        }
    };
}
const PASS: &str = "1111111111111111111111111111111111111111";
const FAIL: &str = "2222222222222222222222222222222222222222";
const NONE: &str = "3333333333333333333333333333333333333333";
const UNKNOWN: &str = "4444444444444444444444444444444444444444";
fn ago(seconds: u64) -> String {
    // RFC 3339 for "seconds ago", from the same clock the module reads.
    let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() - seconds;
    let days = t / 86400;
    let (h, m, s) = ((t % 86400) / 3600, (t % 3600) / 60, t % 60);
    // Civil from days, Howard Hinnant's algorithm.
    let z = days as i64 + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let mo = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + if mo <= 2 { 1 } else { 0 };
    format!("{y:04}-{mo:02}-{d:02}T{h:02}:{m:02}:{s:02}Z")
}
type Api = fn(&Request, usize) -> (u16, String, Vec<(String, String)>);
struct Host {
    _fixture: Fixture,
    server: Server,
    session: ForgeSession,
    repo: ForgeRepository,
    mode: Arc<AtomicUsize>,
}
impl Host {
    fn new(p: ForgeProvider, api: Api) -> Self {
        let fixture = Fixture::new();
        let mode = Arc::new(AtomicUsize::new(0));
        let m = mode.clone();
        let server = serve(Box::new(move |r| {
            if r.path.ends_with("/user") || r.path.contains("connectionData") {
                return response(match p {
                    ForgeProvider::GitLab => r#"{"username":"fixture"}"#,
                    ForgeProvider::Bitbucket => r#"{"display_name":"fixture"}"#,
                    ForgeProvider::Azure => r#"{"authenticatedUser":{"providerDisplayName":"fixture"}}"#,
                    _ => r#"{"login":"fixture"}"#,
                });
            }
            api(r, m.load(Ordering::Acquire))
        }));
        let cfg = config(p, server.port);
        let session = fixture.git.forge_session(cfg.clone(), "", TOKEN, &CancellationToken::new()).unwrap();
        let repo = ForgeRepository::new(cfg, "team/sample").unwrap();
        Self { _fixture: fixture, server, session, repo, mode }
    }
    fn mode(&self, mode: usize) {
        self.mode.store(mode, Ordering::Release);
    }
    fn seen(&self, method: &str) -> Vec<Request> {
        self.server.seen.lock().unwrap().iter().filter(|r| r.method == method).cloned().collect()
    }
    fn marks(&self, commits: &[&str]) -> Vec<(String, ForgeCiState)> {
        let ids: Vec<String> = commits.iter().map(|s| s.to_string()).collect();
        self.session.ci_marks(&self.repo, &ids, &CancellationToken::new()).unwrap().marks
    }
    fn checks(&self, commit: &str) -> ForgeCommitChecks {
        self.session.commit_checks(&self.repo, commit, &CancellationToken::new()).unwrap()
    }
}
fn github(req: &Request, mode: usize) -> (u16, String, Vec<(String, String)>) {
    let path = req.path.split('?').next().unwrap();
    if req.method == "POST" && path == "/github/graphql" {
        if mode == 1 || mode == 10 {
            return response(r#"{"data":null,"errors":[{"type":"FORBIDDEN","message":"Resource not accessible by personal access token"}]}"#);
        }
        if mode == 2 {
            return response(r#"{"data":null,"errors":[{"type":"RATE_LIMITED","message":"API rate limit exceeded"}]}"#);
        }
        assert!(req.body.contains(PASS) && req.body.contains("statusCheckRollup") && !req.body.contains(TOKEN));
        return response(&format!(
            r#"{{"data":{{"repository":{{"nameWithOwner":"team/sample","c0":{{"oid":"{PASS}","statusCheckRollup":{{"state":"SUCCESS"}}}},"c1":{{"oid":"{FAIL}","statusCheckRollup":{{"state":"FAILURE"}}}},"c2":{{"oid":"{NONE}","statusCheckRollup":null}},"c3":null}}}}}}"#
        ));
    }
    if req.method == "POST" {
        assert_eq!(path, "/github/repos/team/sample/actions/runs/812/rerun-failed-jobs");
        assert_eq!(req.body, r#"{"enable_debug_logging":false}"#);
        return match mode {
            5 => (403, r#"{"message":"Resource not accessible by personal access token"}"#.into(), vec![("X-Accepted-GitHub-Permissions".into(), "actions=write".into())]),
            6 => (500, "{}".into(), vec![]),
            _ => (201, String::new(), vec![]),
        };
    }
    let commit = FAIL;
    let recent = ago(3600);
    if path.ends_with("/check-runs") {
        if mode == 3 {
            return (403, r#"{"message":"Resource not accessible by personal access token"}"#.into(), vec![("X-Accepted-GitHub-Permissions".into(), "checks=read".into())]);
        }
        let windows = if mode == 4 { r#""status":"in_progress","conclusion":null"# } else { r#""status":"completed","conclusion":"failure""# };
        return response(&format!(
            r#"{{"total_count":4,"check_runs":[
            {{"name":"build (windows-latest)","head_sha":"{commit}",{windows},"started_at":"2026-11-04T09:12:03Z","completed_at":"2026-11-04T09:16:44Z","html_url":"https://github.com/team/sample/actions/runs/812/job/101","app":{{"name":"GitHub Actions"}},"check_suite":{{"id":7001}}}},
            {{"name":"build (ubuntu-latest)","head_sha":"{commit}","status":"completed","conclusion":"success","started_at":"2026-11-04T09:12:03Z","completed_at":"2026-11-04T09:14:31Z","html_url":"https://github.com/team/sample/actions/runs/812/job/102","app":{{"name":"GitHub Actions"}},"check_suite":{{"id":7001}}}},
            {{"name":"docs","head_sha":"{commit}","status":"completed","conclusion":"skipped","html_url":"https://github.com/team/sample/actions/runs/812/job/105","app":{{"name":"GitHub Actions"}},"check_suite":{{"id":7001}}}},
            {{"name":"CodeQL","head_sha":"{commit}","status":"queued","conclusion":null,"html_url":"https://github.com/team/sample/runs/9","app":{{"name":"GitHub Code Scanning"}},"check_suite":{{"id":7002}}}}]}}"#
        ));
    }
    if path.ends_with("/status") {
        return response(&format!(
            r#"{{"sha":"{commit}","state":"pending","total_count":2,"statuses":[
            {{"context":"netlify/deploy-preview","state":"success","description":"Deploy preview ready","target_url":"https://app.netlify.com/sites/sample/deploys/1"}},
            {{"context":"codecov/project","state":"pending","description":"PRIVATE \u202e coverage","target_url":"javascript:alert(1)"}}]}}"#
        ));
    }
    if path.ends_with("/actions/runs") {
        if mode == 10 {
            return (403, r#"{"message":"Resource not accessible by personal access token"}"#.into(), vec![("X-Accepted-GitHub-Permissions".into(), "actions=read".into())]);
        }
        if mode == 1 {
            // The fallback for marks lists recent runs for every commit.
            return response(&format!(r#"{{"total_count":2,"workflow_runs":[{{"id":812,"name":"CI","head_sha":"{FAIL}","status":"completed","conclusion":"failure","run_number":812,"run_attempt":1,"check_suite_id":7001,"created_at":"{recent}","html_url":"https://github.com/team/sample/actions/runs/812"}},{{"id":813,"name":"CI","head_sha":"{PASS}","status":"completed","conclusion":"success","run_number":813,"run_attempt":1,"check_suite_id":7003,"created_at":"{recent}","html_url":"https://github.com/team/sample/actions/runs/813"}}]}}"#));
        }
        let created = if mode == 7 { ago(31 * 86400) } else { recent };
        let status = if mode == 4 { r#""status":"in_progress","conclusion":null"# } else { r#""status":"completed","conclusion":"failure""# };
        return response(&format!(r#"{{"total_count":1,"workflow_runs":[{{"id":812,"name":"CI","head_sha":"{commit}",{status},"run_number":812,"run_attempt":1,"check_suite_id":7001,"created_at":"{created}","html_url":"https://github.com/team/sample/actions/runs/812"}}]}}"#));
    }
    if path == "/github/repos/team/sample" {
        let push = mode != 8;
        let scopes = if mode == 9 { vec![("X-OAuth-Scopes".into(), "read:org, public_repo".into())] } else { vec![("X-OAuth-Scopes".into(), "repo, workflow".into())] };
        return (200, format!(r#"{{"full_name":"team/sample","archived":false,"permissions":{{"push":{push},"pull":true}}}}"#), scopes);
    }
    (404, "{}".into(), vec![])
}
case!(github_marks_come_from_one_rollup_query, {
    let h = Host::new(ForgeProvider::GitHub, github);
    let marks = h.marks(&[PASS, FAIL, NONE, UNKNOWN, PASS]);
    assert_eq!(marks, vec![(PASS.into(), ForgeCiState::Passed), (FAIL.into(), ForgeCiState::Failed), (NONE.into(), ForgeCiState::NotChecked)]);
    let posts = h.seen("POST");
    assert_eq!(posts.len(), 1, "one GraphQL request answers every commit");
    assert!(posts[0].body.matches("GitObjectID").count() == 4, "duplicates are asked once");
    // A token that can't read the rollup still gets marks from Actions runs.
    h.mode(1);
    assert_eq!(h.marks(&[PASS, FAIL, NONE]), vec![(PASS.into(), ForgeCiState::Passed), (FAIL.into(), ForgeCiState::Failed)]);
    h.mode(2);
    let ids = vec![PASS.to_string()];
    assert_eq!(failure(h.session.ci_marks(&h.repo, &ids, &CancellationToken::new())), ForgeFailure::RateLimited);
    // Refused twice (rollup and Actions runs): hidden, with the permission GitHub named.
    h.mode(10);
    let hidden = h.session.ci_marks(&h.repo, &ids, &CancellationToken::new()).unwrap();
    assert!(hidden.hidden && hidden.marks.is_empty());
    assert_eq!(hidden.missing.as_deref(), Some("Actions: Read"));
    let bad = vec!["not-a-commit".to_string()];
    assert!(h.session.ci_marks(&h.repo, &bad, &CancellationToken::new()).is_err());
});
case!(github_checks_list_failures_first_with_runs_links_and_rerun, {
    let h = Host::new(ForgeProvider::GitHub, github);
    let d = h.checks(FAIL);
    let names: Vec<_> = d.checks.iter().map(|c| (c.state, c.name.as_str())).collect();
    assert_eq!(names[0], (ForgeCheckState::Failed, "build (windows-latest)"));
    assert_eq!(d.checks[0].detail, "Failed after 4m 41s");
    assert_eq!(d.checks[0].group, "CI · run 812");
    assert_eq!(d.checks[0].link.as_ref().unwrap().external, None);
    assert!(names.contains(&(ForgeCheckState::Skipped, "docs")));
    assert!(names.contains(&(ForgeCheckState::Queued, "CodeQL")));
    let netlify = d.checks.iter().find(|c| c.name == "netlify/deploy-preview").unwrap();
    assert_eq!(netlify.link.as_ref().unwrap().external.as_deref(), Some("app.netlify.com"));
    let codecov = d.checks.iter().find(|c| c.name == "codecov/project").unwrap();
    assert!(codecov.link.is_none(), "a non-HTTPS link is dropped");
    assert!(codecov.detail.contains("\\u{202e}"), "hidden characters are escaped");
    assert_eq!(d.state(), ForgeCiState::Failed);
    assert_eq!(d.runs.len(), 1);
    assert_eq!(d.failed_runs().next().unwrap().failed, vec!["build (windows-latest)".to_string()]);
    assert_eq!(d.rerun, None);
    assert!(d.url.ends_with(&format!("/team/sample/commit/{FAIL}")));
    // Review, then one POST; the preview can't be sent twice.
    let preview = h.session.preview_rerun(&d, &CancellationToken::new()).unwrap();
    assert_eq!(preview.runs()[0].title(), "CI · run 812");
    assert!(h.seen("POST").iter().all(|r| r.path.ends_with("/graphql")));
    let result = h.session.submit_rerun(&preview, &CancellationToken::new()).unwrap();
    assert_eq!(result.started, vec![("CI · run 812".to_string(), Some(2))]);
    assert_eq!(failure(h.session.submit_rerun(&preview, &CancellationToken::new())), ForgeFailure::StaleReview);
    assert_eq!(h.seen("POST").iter().filter(|r| r.path.contains("rerun")).count(), 1);
});
case!(github_rerun_is_offered_only_when_it_can_work, {
    let h = Host::new(ForgeProvider::GitHub, github);
    h.mode(4);
    assert_eq!(h.checks(FAIL).rerun, None, "a running job that hasn't failed offers nothing to re-run");
    h.mode(7);
    assert_eq!(h.checks(FAIL).rerun, Some(ForgeRerunBlock::TooOld));
    h.mode(8);
    assert_eq!(h.checks(FAIL).rerun, Some(ForgeRerunBlock::NoWriteAccess));
    h.mode(9);
    assert_eq!(h.checks(FAIL).rerun, Some(ForgeRerunBlock::TokenScope));
    h.mode(8);
    let d = h.checks(FAIL);
    assert!(h.session.preview_rerun(&d, &CancellationToken::new()).unwrap_err().to_string().contains("write access"));
    assert!(h.seen("POST").iter().all(|r| !r.path.contains("rerun")));
    // A refused check-runs read keeps the statuses and names the permission.
    h.mode(3);
    let d = h.checks(FAIL);
    assert_eq!(d.error, Some(ForgeFailure::Forbidden));
    assert_eq!(d.missing.as_deref(), Some("Checks: Read"));
    assert!(d.checks.iter().any(|c| c.name == "netlify/deploy-preview"));
    // A review made while the read was complete can't be sent after it becomes partial.
    h.mode(0);
    let preview = h.session.preview_rerun(&h.checks(FAIL), &CancellationToken::new()).unwrap();
    h.mode(3);
    assert_eq!(failure(h.session.submit_rerun(&preview, &CancellationToken::new())), ForgeFailure::StaleReview);
    assert!(h.seen("POST").iter().all(|r| !r.path.contains("rerun")), "nothing sent unchecked");
});
case!(github_rerun_refusal_and_lost_answer_never_retry, {
    let h = Host::new(ForgeProvider::GitHub, github);
    let d = h.checks(FAIL);
    let preview = h.session.preview_rerun(&d, &CancellationToken::new()).unwrap();
    h.mode(5);
    let refused = h.session.submit_rerun(&preview, &CancellationToken::new()).unwrap_err().to_string();
    assert!(refused.contains("Actions: Read and write") && refused.contains("Nothing was re-run"), "{refused}");
    h.mode(0);
    let preview = h.session.preview_rerun(&h.checks(FAIL), &CancellationToken::new()).unwrap();
    h.mode(6);
    assert_eq!(failure(h.session.submit_rerun(&preview, &CancellationToken::new())), ForgeFailure::OutcomeUnknown);
    assert_eq!(h.seen("POST").iter().filter(|r| r.path.contains("rerun")).count(), 2, "one POST per reviewed preview, no retry");
});
fn gitlab(req: &Request, mode: usize) -> (u16, String, Vec<(String, String)>) {
    let path = req.path.split('?').next().unwrap();
    let prefix = "/gitlab/projects/team%2Fsample";
    if req.method == "POST" {
        assert_eq!(path, format!("{prefix}/pipelines/77/retry"));
        return (201, format!(r#"{{"id":77,"sha":"{FAIL}","status":"pending"}}"#), vec![]);
    }
    if path == format!("{prefix}/pipelines") {
        if req.path.contains("sha=") {
            return response(&format!(r#"[{{"id":77,"iid":12,"sha":"{FAIL}","ref":"main","status":"failed","web_url":"https://gitlab.com/team/sample/-/pipelines/77"}}]"#));
        }
        return response(&format!(r#"[{{"id":78,"sha":"{PASS}","status":"success"}},{{"id":77,"sha":"{FAIL}","status":"failed"}},{{"id":70,"sha":"{FAIL}","status":"success"}},{{"id":69,"sha":"{NONE}","status":"running"}}]"#));
    }
    if path == format!("{prefix}/pipelines/77/jobs") {
        return response(r#"[{"id":1,"name":"build","stage":"build","status":"failed","duration":281.4,"web_url":"https://gitlab.com/team/sample/-/jobs/1"},{"id":2,"name":"lint","stage":"test","status":"failed","allow_failure":true},{"id":3,"name":"unit","stage":"test","status":"success","duration":55}]"#);
    }
    if path == prefix {
        let level = if mode == 1 { 20 } else { 30 };
        return response(&format!(r#"{{"archived":false,"permissions":{{"project_access":{{"access_level":{level}}},"group_access":null}}}}"#));
    }
    if path == "/gitlab/personal_access_tokens/self" {
        return response(if mode == 2 { r#"{"scopes":["read_api"]}"# } else { r#"{"scopes":["api"]}"# });
    }
    (404, "{}".into(), vec![])
}
case!(gitlab_pipelines_jobs_and_retry, {
    let h = Host::new(ForgeProvider::GitLab, gitlab);
    assert_eq!(h.marks(&[PASS, FAIL, NONE, UNKNOWN]), vec![(PASS.into(), ForgeCiState::Passed), (FAIL.into(), ForgeCiState::Failed), (NONE.into(), ForgeCiState::Running)]);
    assert_eq!(h.seen("GET").iter().filter(|r| r.path.contains("/pipelines?")).count(), 1, "one list answers every commit");
    let d = h.checks(FAIL);
    assert_eq!(d.checks[0].name, "build · build");
    assert_eq!(d.checks[0].detail, "Failed after 4m 41s");
    assert_eq!(d.checks[0].group, "Pipeline · run 12");
    assert!(d.checks.iter().any(|c| c.state == ForgeCheckState::Neutral && c.name == "test · lint"), "allowed failures don't fail the run");
    assert_eq!(d.failed_runs().next().unwrap().failed, vec!["build · build".to_string()]);
    assert_eq!(d.rerun, None);
    let preview = h.session.preview_rerun(&d, &CancellationToken::new()).unwrap();
    assert_eq!(h.session.submit_rerun(&preview, &CancellationToken::new()).unwrap().started, vec![("Pipeline · run 12".to_string(), None)]);
    h.mode(1);
    assert_eq!(h.checks(FAIL).rerun, Some(ForgeRerunBlock::NoWriteAccess));
    h.mode(2);
    assert_eq!(h.checks(FAIL).rerun, Some(ForgeRerunBlock::TokenScope));
});
fn gitea(req: &Request, mode: usize) -> (u16, String, Vec<(String, String)>) {
    let path = req.path.split('?').next().unwrap();
    if req.method == "POST" {
        assert_eq!(path, "/gitea/repos/team/sample/actions/runs/5/rerun-failed-jobs");
        return (201, String::new(), vec![]);
    }
    if let Some(sha) = path.strip_prefix("/gitea/repos/team/sample/commits/").and_then(|p| p.strip_suffix("/status")) {
        return match sha {
            PASS => response(r#"{"state":"success","total_count":1,"statuses":[{"context":"CI / build (push)","status":"success","target_url":"https://gitea.com/team/sample/actions/runs/4/jobs/0"}]}"#),
            FAIL => response(r#"{"state":"failure","total_count":2,"statuses":[{"context":"CI / build (push)","status":"failure","description":"Failing after 1m","target_url":"https://gitea.com/team/sample/actions/runs/5/jobs/0"},{"context":"CI / lint (push)","status":"warning"}]}"#),
            NONE => response(r#"{"state":"","total_count":0,"statuses":[]}"#),
            _ => (404, "{}".into(), vec![]),
        };
    }
    if path == "/gitea/repos/team/sample/actions/runs" {
        if mode == 1 {
            return (404, "{}".into(), vec![]);
        }
        return response(&format!(r#"{{"total_count":1,"workflow_runs":[{{"id":5,"name":"CI","head_sha":"{FAIL}","status":"completed","conclusion":"failure","run_number":5,"run_attempt":1,"created_at":"{}","html_url":"https://gitea.com/team/sample/actions/runs/5"}}]}}"#, ago(60)));
    }
    if path == "/gitea/repos/team/sample/actions/runs/5/jobs" {
        return response(r#"{"total_count":2,"jobs":[{"name":"build","status":"completed","conclusion":"failure"},{"name":"lint","status":"completed","conclusion":"success"}]}"#);
    }
    if path == "/gitea/repos/team/sample" {
        return response(r#"{"full_name":"team/sample","archived":false,"permissions":{"push":true}}"#);
    }
    (404, "{}".into(), vec![])
}
case!(gitea_statuses_runs_and_rerun, {
    let h = Host::new(ForgeProvider::Gitea, gitea);
    assert_eq!(h.marks(&[PASS, FAIL, NONE, UNKNOWN]), vec![(PASS.into(), ForgeCiState::Passed), (FAIL.into(), ForgeCiState::Failed), (NONE.into(), ForgeCiState::NotChecked)]);
    let d = h.checks(FAIL);
    assert_eq!(d.checks[0].name, "CI / build (push)");
    assert_eq!(d.failed_runs().next().unwrap().failed, vec!["build".to_string()]);
    let preview = h.session.preview_rerun(&d, &CancellationToken::new()).unwrap();
    assert_eq!(h.session.submit_rerun(&preview, &CancellationToken::new()).unwrap().started, vec![("CI · run 5".to_string(), Some(2))]);
    // An older Gitea without the Actions API: results, but no re-run.
    h.mode(1);
    assert_eq!(h.checks(FAIL).rerun, Some(ForgeRerunBlock::Unsupported));
});
fn bitbucket(req: &Request, _mode: usize) -> (u16, String, Vec<(String, String)>) {
    let path = req.path.split('?').next().unwrap();
    assert_eq!(req.method, "GET", "Bitbucket is never asked to re-run");
    if let Some(sha) = path.strip_prefix("/bitbucket/repositories/team/sample/commit/").and_then(|p| p.strip_suffix("/statuses")) {
        return match sha {
            FAIL => response(r#"{"values":[{"key":"pipeline","name":"Pipeline #31","state":"FAILED","url":"https://bitbucket.org/team/sample/pipelines/results/31"},{"key":"deploy","state":"INPROGRESS"}]}"#),
            PASS => response(r#"{"values":[{"key":"pipeline","name":"Pipeline #30","state":"SUCCESSFUL"}]}"#),
            NONE => response(r#"{"values":[]}"#),
            _ => (404, "{}".into(), vec![]),
        };
    }
    (404, "{}".into(), vec![])
}
case!(bitbucket_and_azure_show_results_without_rerun, {
    let h = Host::new(ForgeProvider::Bitbucket, bitbucket);
    assert_eq!(h.marks(&[PASS, FAIL, NONE, UNKNOWN]), vec![(PASS.into(), ForgeCiState::Passed), (FAIL.into(), ForgeCiState::Failed), (NONE.into(), ForgeCiState::NotChecked)]);
    let d = h.checks(FAIL);
    assert_eq!(d.checks[0].name, "Pipeline #31");
    assert_eq!(d.checks[1].name, "deploy");
    assert_eq!(d.rerun, Some(ForgeRerunBlock::Unsupported));
    let a = Host::new(ForgeProvider::Azure, |req, _| {
        let path = req.path.split('?').next().unwrap();
        assert!(req.path.contains("api-version=7.1"));
        if path == format!("/azure/team/_apis/git/repositories/sample/commits/{FAIL}/statuses") {
            return response(r#"{"value":[{"state":"failed","description":"Build failed","targetUrl":"https://dev.azure.com/fixture/team/_build/results?buildId=9","context":{"name":"build","genre":"continuous-integration"}}]}"#);
        }
        if path == "/azure/team/_apis/git/repositories/sample" {
            return response(r#"{"id":"0c9e3f2a-4f1b-4d1e-9a7b-3f2c1d0e9b8a","name":"sample"}"#);
        }
        if path == "/azure/team/_apis/build/builds" {
            assert!(req.path.contains("repositoryId=0c9e3f2a-4f1b-4d1e-9a7b-3f2c1d0e9b8a") && req.path.contains("repositoryType=TfsGit"));
            return response(&format!(r#"{{"value":[{{"id":9,"buildNumber":"20261104.3","status":"completed","result":"failed","sourceVersion":"{FAIL}","startTime":"2026-11-04T09:12:03Z","finishTime":"2026-11-04T09:16:44Z","definition":{{"name":"CI"}},"_links":{{"web":{{"href":"https://dev.azure.com/fixture/team/_build/results?buildId=9"}}}}}},{{"id":8,"buildNumber":"20261104.2","status":"inProgress","sourceVersion":"{PASS}","definition":{{"name":"CI"}}}}]}}"#));
        }
        (404, "{}".into(), vec![])
    });
    let d = a.checks(FAIL);
    assert!(d.checks.iter().any(|c| c.name == "continuous-integration/build" && c.link.as_ref().unwrap().external.is_none()));
    let build = d.checks.iter().find(|c| c.group == "Build 20261104.3").unwrap();
    assert_eq!((build.name.as_str(), build.state, build.detail.as_str()), ("CI", ForgeCheckState::Failed, "Failed after 4m 41s"));
    assert_eq!(d.rerun, Some(ForgeRerunBlock::Unsupported));
    assert_eq!(a.marks(&[FAIL, PASS, UNKNOWN]), vec![(FAIL.into(), ForgeCiState::Failed), (PASS.into(), ForgeCiState::Running)]);
});

// ------------------------------------------------------------------ Actions page
fn list(h: &Host, branch: Option<&str>, page: usize) -> ForgeRunList {
    h.session.ci_runs(&h.repo, branch, page, &CancellationToken::new()).unwrap()
}
fn query(req: &Request) -> &str {
    req.path.split_once('?').map_or("", |q| q.1)
}
fn github_runs(req: &Request, mode: usize) -> (u16, String, Vec<(String, String)>) {
    let path = req.path.split('?').next().unwrap();
    if req.method == "POST" {
        assert_eq!(path, "/github/repos/team/sample/actions/runs/812/rerun-failed-jobs");
        return (201, String::new(), vec![]);
    }
    let recent = ago(3600);
    let failed = |attempt: u64| format!(r#"{{"id":812,"name":"CI","run_number":812,"run_attempt":{attempt},"event":"pull_request","status":"completed","conclusion":"failure","head_branch":"feature/sign-in","head_sha":"{FAIL}","pull_requests":[{{"number":42}}],"head_commit":{{"message":"Handle an expired token\n\nThe body."}},"created_at":"{recent}","run_started_at":"2026-11-04T09:12:03Z","updated_at":"2026-11-04T09:16:44Z","html_url":"https://github.com/team/sample/actions/runs/812"}}"#);
    if path == "/github/repos/team/sample/actions/runs" {
        let q = query(req);
        assert!(q.contains("per_page=30") && !q.contains("head_sha"), "{q}");
        if mode == 2 {
            return (403, r#"{"message":"Resource not accessible by personal access token"}"#.into(), vec![("X-Accepted-GitHub-Permissions".into(), "actions=read".into())]);
        }
        if q.contains("branch=feature%2Fsign-in") {
            return response(&format!(r#"{{"total_count":1,"workflow_runs":[{}]}}"#, failed(1)));
        }
        if q.contains("page=2") {
            return response(&format!(r#"{{"total_count":31,"workflow_runs":[{{"id":700,"name":"CI","run_number":700,"event":"push","status":"completed","conclusion":"success","head_branch":"main","head_sha":"{PASS}","created_at":"{recent}"}}]}}"#));
        }
        assert!(q.contains("page=1"), "{q}");
        return response(&format!(
            r#"{{"total_count":31,"workflow_runs":[{{"id":815,"name":"CI","run_number":815,"run_attempt":1,"event":"push","status":"in_progress","conclusion":null,"head_branch":"main","head_sha":"{PASS}","display_title":"Update the changelog","created_at":"{recent}","run_started_at":"{recent}","html_url":"https://github.com/team/sample/actions/runs/815"}},{},{{"id":77,"name":"Docs","run_number":77,"event":"workflow_dispatch","status":"completed","conclusion":"success","head_branch":"main","head_sha":"{NONE}","created_at":"{recent}"}}]}}"#,
            failed(1)
        ));
    }
    if path == "/github/repos/team/sample/actions/runs/812" {
        // Mode 3: it was cancelled after a job failed.
        if mode == 3 {
            return response(&failed(1).replace(r#""conclusion":"failure""#, r#""conclusion":"cancelled""#));
        }
        // Mode 1: someone re-ran it meanwhile, so it's a new attempt.
        return response(&failed(if mode == 1 { 2 } else { 1 }));
    }
    if path == "/github/repos/team/sample/actions/runs/812/jobs" {
        assert!(query(req).contains("filter=latest"));
        return response(r#"{"total_count":3,"jobs":[
            {"name":"build (ubuntu-latest)","status":"completed","conclusion":"success","started_at":"2026-11-04T09:12:03Z","completed_at":"2026-11-04T09:14:31Z","html_url":"https://github.com/team/sample/actions/runs/812/job/102"},
            {"name":"build (windows-latest)","status":"completed","conclusion":"failure","started_at":"2026-11-04T09:12:03Z","completed_at":"2026-11-04T09:16:44Z","html_url":"https://github.com/team/sample/actions/runs/812/job/101"},
            {"name":"docs","status":"completed","conclusion":"skipped"}]}"#);
    }
    if path == "/github/repos/team/sample" {
        let name = if mode == 4 { "team/other" } else { "team/sample" };
        return (200, format!(r#"{{"full_name":"{name}","archived":false,"permissions":{{"push":true}}}}"#), vec![("X-OAuth-Scopes".into(), "repo".into())]);
    }
    (404, "{}".into(), vec![])
}
case!(github_runs_list_pages_and_rerun_one_run, {
    let h = Host::new(ForgeProvider::GitHub, github_runs);
    let l = list(&h, None, 1);
    assert_eq!((l.runs.len(), l.more, l.page), (3, true, 1));
    let running = &l.runs[0];
    assert_eq!((running.state, running.event.as_deref(), running.branch.as_deref(), running.took), (ForgeCheckState::Running, Some("push"), Some("main"), None));
    assert_eq!(running.subject.as_deref(), Some("Update the changelog"));
    let failed = &l.runs[1];
    assert_eq!(failed.title(), "CI · run 812");
    assert_eq!((failed.event.as_deref(), failed.branch.as_deref(), failed.commit.as_deref()), (Some("pull request #42"), Some("feature/sign-in"), Some(FAIL)));
    assert_eq!((failed.subject.as_deref(), failed.took), (Some("Handle an expired token"), Some(281)));
    assert_eq!(l.runs[2].event.as_deref(), Some("manual"));
    let second = list(&h, None, 2);
    assert_eq!((second.runs.len(), second.more), (1, false), "31 runs fill two pages");
    let one = list(&h, Some("feature/sign-in"), 1);
    assert_eq!((one.runs.len(), one.more), (1, false));
    for (branch, page) in [(None, 0), (None, CI_RUN_PAGES + 1), (Some("../main"), 1), (Some("-x"), 1)] {
        assert!(h.session.ci_runs(&h.repo, branch, page, &CancellationToken::new()).is_err(), "{branch:?} {page}");
    }
    // One run: its jobs, failures first, and a reviewed re-run of that run only.
    let d = h.session.ci_run(&h.repo, failed, &CancellationToken::new()).unwrap();
    assert_eq!((d.jobs[0].name.as_str(), d.jobs[0].detail.as_str(), d.jobs[0].group.as_str()), ("build (windows-latest)", "Failed after 4m 41s", "CI · run 812"));
    assert_eq!(d.run.failed, vec!["build (windows-latest)".to_string()]);
    assert_eq!((d.rerun, d.error, d.count(ForgeCheckState::Skipped)), (None, None, 1));
    let preview = h.session.preview_run_rerun(&d, &CancellationToken::new()).unwrap();
    assert_eq!(preview.runs().len(), 1);
    assert_eq!(preview.commit(), FAIL);
    assert!(h.seen("POST").is_empty(), "the review sends nothing");
    h.mode(1);
    assert_eq!(failure(h.session.submit_rerun(&preview, &CancellationToken::new())), ForgeFailure::StaleReview);
    assert!(h.seen("POST").is_empty(), "a new attempt since the review sends nothing");
    h.mode(0);
    let preview = h.session.preview_run_rerun(&h.session.ci_run(&h.repo, failed, &CancellationToken::new()).unwrap(), &CancellationToken::new()).unwrap();
    assert_eq!(h.session.submit_rerun(&preview, &CancellationToken::new()).unwrap().started, vec![("CI · run 812".to_string(), Some(2))]);
    assert_eq!(failure(h.session.submit_rerun(&preview, &CancellationToken::new())), ForgeFailure::StaleReview);
    assert_eq!(h.seen("POST").len(), 1);
    assert!(h.repo.runs_url().ends_with("/team/sample/actions"));
    // A token that can't read Actions: an answer, with the permission GitHub named.
    h.mode(2);
    let hidden = list(&h, None, 1);
    assert!(hidden.hidden && hidden.runs.is_empty() && !hidden.more);
    assert_eq!(hidden.missing.as_deref(), Some("Actions: Read"));
    // Cancelled after a job failed: the job shows failed, nothing is offered to re-run.
    h.mode(3);
    let cancelled = h.session.ci_run(&h.repo, failed, &CancellationToken::new()).unwrap();
    assert_eq!(cancelled.run.state, ForgeCheckState::Cancelled);
    assert_eq!(cancelled.jobs[0].state, ForgeCheckState::Failed);
    assert!(cancelled.run.failed.is_empty() && cancelled.rerun.is_none());
    // An answer for another repository fails the read rather than hiding Re-run.
    h.mode(4);
    assert_eq!(failure(h.session.ci_run(&h.repo, failed, &CancellationToken::new())), ForgeFailure::InvalidResponse);
    assert_eq!(h.seen("POST").len(), 1, "still the one reviewed POST");
});
fn gitlab_runs(req: &Request, _mode: usize) -> (u16, String, Vec<(String, String)>) {
    let path = req.path.split('?').next().unwrap();
    let prefix = "/gitlab/projects/team%2Fsample";
    if req.method == "POST" {
        assert_eq!(path, format!("{prefix}/pipelines/77/retry"));
        return (201, format!(r#"{{"id":77,"sha":"{FAIL}","status":"pending"}}"#), vec![]);
    }
    let recent = ago(600);
    let failed = format!(r#"{{"id":77,"iid":12,"sha":"{FAIL}","ref":"refs/merge-requests/42/head","status":"failed","source":"merge_request_event","created_at":"{recent}","web_url":"https://gitlab.com/team/sample/-/pipelines/77""#);
    if path == format!("{prefix}/pipelines") {
        let q = query(req);
        assert!(q.contains("per_page=30") && q.contains("order_by=id") && !q.contains("sha="), "{q}");
        if q.contains("ref=main") {
            return response(&format!(r#"[{{"id":78,"iid":13,"sha":"{PASS}","ref":"main","status":"success","source":"push"}}]"#));
        }
        let rows = format!(r#"[{{"id":78,"iid":13,"sha":"{PASS}","ref":"main","status":"success","source":"push","created_at":"{recent}"}},{failed}}},{{"id":76,"iid":11,"sha":"{NONE}","ref":"v1.0","status":"canceled","source":"push"}},{{"id":75,"iid":10,"sha":"{NONE}","ref":"main","status":"canceling","source":"push"}}]"#);
        return (200, rows, vec![("X-Next-Page".into(), "2".into())]);
    }
    if path == format!("{prefix}/pipelines/77") {
        return response(&format!(r#"{failed},"duration":281,"started_at":"{recent}"}}"#));
    }
    if path == format!("{prefix}/pipelines/76") {
        return response(&format!(r#"{{"id":76,"iid":11,"sha":"{NONE}","ref":"v1.0","tag":true,"status":"canceled","source":"push","duration":12}}"#));
    }
    if path == format!("{prefix}/pipelines/76/jobs") {
        return response("[]");
    }
    if path == format!("{prefix}/pipelines/77/jobs") {
        return response(r#"[{"id":1,"name":"build","stage":"build","status":"failed","duration":281.4},{"id":2,"name":"lint","stage":"test","status":"failed","allow_failure":true},{"id":3,"name":"unit","stage":"test","status":"success","duration":55}]"#);
    }
    if path == prefix {
        return response(r#"{"archived":false,"permissions":{"project_access":{"access_level":30},"group_access":null}}"#);
    }
    if path == "/gitlab/personal_access_tokens/self" {
        return response(r#"{"scopes":["api"]}"#);
    }
    (404, "{}".into(), vec![])
}
case!(gitlab_pipelines_list_and_retry_one, {
    let h = Host::new(ForgeProvider::GitLab, gitlab_runs);
    let l = list(&h, None, 1);
    assert_eq!((l.runs.len(), l.more), (4, true));
    assert_eq!((l.runs[0].branch.as_deref(), l.runs[0].event.as_deref()), (Some("main"), Some("push")));
    let merge = &l.runs[1];
    assert_eq!((merge.title().as_str(), merge.event.as_deref(), merge.branch.as_deref()), ("Pipeline · run 12", Some("merge request !42"), None));
    assert_eq!(l.runs[2].state, ForgeCheckState::Cancelled);
    assert_eq!(l.runs[3].state, ForgeCheckState::Running, "a pipeline still being canceled runs");
    // The list doesn't say a pipeline ran for a tag; the pipeline read alone does.
    let tag = h.session.ci_run(&h.repo, &l.runs[2], &CancellationToken::new()).unwrap();
    assert_eq!((tag.run.event.as_deref(), tag.run.branch.as_deref(), tag.run.took), (Some("tag"), None, Some(12)));
    assert_eq!(list(&h, Some("main"), 1).runs.len(), 1);
    let d = h.session.ci_run(&h.repo, merge, &CancellationToken::new()).unwrap();
    assert_eq!(d.run.took, Some(281), "the pipeline read alone has its duration");
    assert_eq!(d.run.failed, vec!["build · build".to_string()]);
    assert_eq!(d.rerun, None);
    let preview = h.session.preview_run_rerun(&d, &CancellationToken::new()).unwrap();
    assert_eq!(h.session.submit_rerun(&preview, &CancellationToken::new()).unwrap().started, vec![("Pipeline · run 12".to_string(), None)]);
    assert!(h.repo.runs_url().ends_with("/team/sample/-/pipelines"));
});
fn gitea_runs(req: &Request, _mode: usize) -> (u16, String, Vec<(String, String)>) {
    let path = req.path.split('?').next().unwrap();
    if req.method == "POST" {
        assert_eq!(path, "/gitea/repos/team/sample/actions/runs/5/rerun-failed-jobs");
        return (201, String::new(), vec![]);
    }
    let failed = format!(r#"{{"id":5,"path":"ci.yml@refs/heads/main","run_number":5,"run_attempt":1,"event":"push","status":"completed","conclusion":"failure","head_branch":"main","head_sha":"{FAIL}","created_at":"{}","started_at":"2026-11-04T09:12:03Z","completed_at":"2026-11-04T09:13:03Z","html_url":"https://gitea.com/team/sample/actions/runs/5"}}"#, ago(60));
    if path == "/gitea/repos/team/sample/actions/runs" {
        assert!(query(req).contains("limit=30"));
        return response(&format!(r#"{{"total_count":1,"workflow_runs":[{failed}]}}"#));
    }
    if path == "/gitea/repos/team/sample/actions/runs/5" {
        return response(&failed);
    }
    if path == "/gitea/repos/team/sample/actions/runs/5/jobs" {
        return response(r#"{"total_count":2,"jobs":[{"name":"build","status":"completed","conclusion":"failure"},{"name":"lint","status":"completed","conclusion":"success"}]}"#);
    }
    if path == "/gitea/repos/team/sample" {
        return response(r#"{"full_name":"team/sample","permissions":{"push":true}}"#);
    }
    (404, "{}".into(), vec![])
}
case!(gitea_runs_list_and_rerun_one, {
    let h = Host::new(ForgeProvider::Gitea, gitea_runs);
    let l = list(&h, None, 1);
    let run = &l.runs[0];
    assert_eq!((run.title().as_str(), run.took, l.more), ("ci.yml · run 5", Some(60), false), "a run named only by its workflow file");
    let d = h.session.ci_run(&h.repo, run, &CancellationToken::new()).unwrap();
    assert_eq!((d.run.failed.clone(), d.rerun), (vec!["build".to_string()], None));
    let preview = h.session.preview_run_rerun(&d, &CancellationToken::new()).unwrap();
    assert_eq!(h.session.submit_rerun(&preview, &CancellationToken::new()).unwrap().started, vec![("ci.yml · run 5".to_string(), Some(2))]);
});
const PIPELINE: &str = "{0f8e0a22-6a9e-4b5e-8a8f-2d1c3b4a5e6f}";
fn bitbucket_runs(req: &Request, _mode: usize) -> (u16, String, Vec<(String, String)>) {
    let path = req.path.split('?').next().unwrap();
    assert_eq!(req.method, "GET", "Bitbucket is never asked to re-run");
    assert!(!req.path.contains("page=2"), "the next page's URL is never followed");
    let failed = format!(r#"{{"uuid":"{PIPELINE}","build_number":31,"state":{{"name":"COMPLETED","result":{{"name":"FAILED"}}}},"target":{{"type":"pipeline_pullrequest_target","source":"feature/sign-in","destination":"main","pullrequest":{{"id":42}},"commit":{{"hash":"{FAIL}"}}}},"trigger":{{"name":"PUSH"}},"created_on":"{}","duration_in_seconds":281}}"#, ago(60));
    if path == "/bitbucket/repositories/team/sample/pipelines" {
        assert!(query(req).contains("sort=-created_on") && query(req).contains("pagelen=30"));
        return response(&format!(
            r#"{{"values":[{failed},{{"uuid":"{{1f8e0a22-6a9e-4b5e-8a8f-2d1c3b4a5e6f}}","build_number":30,"state":{{"name":"IN_PROGRESS","stage":{{"name":"RUNNING"}}}},"target":{{"type":"pipeline_ref_target","ref_type":"branch","ref_name":"main","commit":{{"hash":"{PASS}"}}}},"trigger":{{"name":"MANUAL"}},"duration_in_seconds":40}}],"next":"https://api.bitbucket.org/2.0/repositories/team/sample/pipelines?page=2"}}"#
        ));
    }
    if path == "/bitbucket/repositories/team/sample/pipelines/%7B0f8e0a22-6a9e-4b5e-8a8f-2d1c3b4a5e6f%7D" {
        return response(&failed);
    }
    if path == "/bitbucket/repositories/team/sample/pipelines/%7B0f8e0a22-6a9e-4b5e-8a8f-2d1c3b4a5e6f%7D/steps" {
        return response(r#"{"values":[{"state":{"name":"COMPLETED","result":{"name":"SUCCESSFUL"}}},{"name":"Build","state":{"name":"COMPLETED","result":{"name":"FAILED"}},"started_on":"2026-11-04T09:12:03Z","completed_on":"2026-11-04T09:16:44Z"}]}"#);
    }
    (404, "{}".into(), vec![])
}
case!(bitbucket_pipelines_and_steps_without_rerun, {
    let h = Host::new(ForgeProvider::Bitbucket, bitbucket_runs);
    let l = list(&h, None, 1);
    assert!(l.more);
    let pr = &l.runs[0];
    assert_eq!((pr.title().as_str(), pr.event.as_deref(), pr.branch.as_deref(), pr.took), ("Pipeline · run 31", Some("pull request #42"), Some("feature/sign-in"), Some(281)));
    let web = pr.link.as_ref().unwrap();
    assert!(web.url.ends_with("/team/sample/pipelines/results/31") && web.external.is_none(), "{}", web.url);
    assert_eq!((l.runs[1].state, l.runs[1].event.as_deref(), l.runs[1].took), (ForgeCheckState::Running, Some("manual"), None));
    let d = h.session.ci_run(&h.repo, pr, &CancellationToken::new()).unwrap();
    assert_eq!((d.jobs[0].name.as_str(), d.jobs[0].detail.as_str()), ("Build", "Failed after 4m 41s"));
    assert_eq!(d.jobs[1].name, "Step", "a step without a name");
    assert_eq!(d.rerun, Some(ForgeRerunBlock::Unsupported));
    assert!(h.session.preview_run_rerun(&d, &CancellationToken::new()).unwrap_err().to_string().contains("can't re-run"));
});
case!(azure_builds_and_timeline_jobs_without_rerun, {
    let h = Host::new(ForgeProvider::Azure, |req, _| {
        let path = req.path.split('?').next().unwrap();
        assert!(req.path.contains("api-version=7.1") && req.method == "GET", "{}", req.path);
        let failed = format!(r#"{{"id":9,"buildNumber":"20261104.3","status":"completed","result":"failed","sourceBranch":"refs/heads/main","sourceVersion":"{FAIL}","reason":"individualCI","queueTime":"{}","startTime":"2026-11-04T09:12:03Z","finishTime":"2026-11-04T09:16:44Z","definition":{{"name":"CI"}},"_links":{{"web":{{"href":"https://dev.azure.com/fixture/team/_build/results?buildId=9"}}}}}}"#, ago(60));
        if path == "/azure/team/_apis/git/repositories/sample" {
            return response(r#"{"id":"0c9e3f2a-4f1b-4d1e-9a7b-3f2c1d0e9b8a","name":"sample"}"#);
        }
        if path == "/azure/team/_apis/build/builds" {
            // Page 2 asks for the newest 60; there are only two.
            assert!(req.path.contains("$top=30") || req.path.contains("$top=60"));
            return response(&format!(r#"{{"value":[{failed},{{"id":8,"buildNumber":"20261104.2","status":"inProgress","sourceBranch":"refs/pull/7/merge","sourceVersion":"{PASS}","reason":"pullRequest","definition":{{"name":"CI"}}}}]}}"#));
        }
        if path == "/azure/team/_apis/build/builds/9" {
            return response(&failed);
        }
        if path == "/azure/team/_apis/build/builds/9/timeline" {
            return response(r#"{"records":[{"type":"Stage","name":"Build","state":"completed","result":"failed"},{"type":"Job","name":"Windows","state":"completed","result":"failed","startTime":"2026-11-04T09:12:03Z","finishTime":"2026-11-04T09:16:44Z"},{"type":"Task","name":"Checkout","state":"completed","result":"succeeded"},{"type":"Job","name":"Linux","state":"completed","result":"succeeded"}]}"#);
        }
        (404, "{}".into(), vec![])
    });
    let l = list(&h, None, 1);
    assert_eq!((l.runs.len(), l.more), (2, false));
    let build = &l.runs[0];
    assert_eq!((build.title().as_str(), build.event.as_deref(), build.branch.as_deref(), build.took), ("CI · run 20261104.3", Some("push"), Some("main"), Some(281)));
    assert_eq!((l.runs[1].event.as_deref(), l.runs[1].branch.as_deref()), (Some("pull request !7"), None));
    assert!(list(&h, None, 2).runs.is_empty());
    let d = h.session.ci_run(&h.repo, build, &CancellationToken::new()).unwrap();
    let jobs: Vec<_> = d.jobs.iter().map(|j| (j.name.as_str(), j.state)).collect();
    assert_eq!(jobs, vec![("Windows", ForgeCheckState::Failed), ("Linux", ForgeCheckState::Passed)], "only the timeline's jobs");
    assert_eq!(d.rerun, Some(ForgeRerunBlock::Unsupported));
    assert!(h.repo.runs_url().ends_with("/team/_build"));
});
