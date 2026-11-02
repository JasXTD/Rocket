mod common;
use common::Fixture;
use rocket_git::*;
use std::{
    fs,
    path::{Path, PathBuf},
    sync::mpsc::sync_channel,
    thread,
    time::{Duration, Instant},
};

fn token() -> CancellationToken {
    CancellationToken::new()
}

// Re-exec each case with an empty Git configuration and file-only transports.
// Environment changes are child-only, including when the harness runs in parallel.
fn isolated(name: &str, test: fn()) {
    common::isolated("ROCKET_PUSH_CASE", name, test);
}

struct PushRepos {
    f: Fixture,
    bare: PathBuf,
    server_hooks: PathBuf,
}
impl PushRepos {
    fn new(format: &str) -> Self {
        let f = Fixture::with_format(format);
        f.write("file", b"base\n");
        f.commit("base");
        let bare = f.root.join("PRIVATE destination with spaces 文.git");
        f.run(&[
            "init",
            "--bare",
            "--initial-branch=main",
            &format!("--object-format={format}"),
            bare.to_str().unwrap(),
        ]);
        let server_hooks = f.root.join("server-hooks");
        fs::create_dir(&server_hooks).unwrap();
        f.run_at(
            &bare,
            &["config", "core.hooksPath", server_hooks.to_str().unwrap()],
        );
        f.run(&["remote", "add", "origin", bare.to_str().unwrap()]);
        Self {
            f,
            bare,
            server_hooks,
        }
    }
    fn push(&self, upstream: bool) -> PushReport {
        self.f
            .repo()
            .push(
                "origin",
                "main",
                "main",
                PushOptions {
                    set_upstream: upstream,
                },
                &token(),
            )
            .unwrap()
    }
    fn remote_head(&self) -> Vec<u8> {
        self.f.run_at(&self.bare, &["rev-parse", "refs/heads/main"])
    }
    fn advance(&self) {
        self.f.write("file", b"advance\n");
        self.f.commit("PRIVATE advance");
    }
    fn logged(&self, limits: CommandLimits) -> (Repository, DiagnosticLog) {
        let log = DiagnosticLog::open(self.f.root.join("logs")).unwrap();
        let repo = GitClient::new_with_log(self.f.git.executable(), limits, log.clone())
            .unwrap()
            .open(&self.f.repository, &token())
            .unwrap();
        (repo, log)
    }
}

fn confirmed(report: &PushReport, kind: PushUpdateKind) {
    assert!(report.result.is_ok(), "{:?}", report.result);
    assert_eq!(report.remote_outcome, PushRemoteOutcome::Confirmed);
    assert_eq!(report.update.as_ref().unwrap().as_ref().unwrap().kind, kind);
    assert!(report.branches.is_ok(), "{:?}", report.branches);
    if let Some(status) = &report.status {
        assert!(status.is_ok(), "{status:?}");
    }
}

fn script(path: &Path, body: &str) {
    fs::write(path, format!("#!/bin/sh\n{body}\n")).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
    }
}
fn quote(path: &Path) -> String {
    format!(
        "'{}'",
        path.to_str()
            .unwrap()
            .replace('\\', "/")
            .replace('\'', "'\\''")
    )
}
fn wait_file(path: &Path) {
    let deadline = Instant::now() + Duration::from_secs(20);
    while !path.is_file() {
        assert!(Instant::now() < deadline, "fixture hook did not start");
        thread::sleep(Duration::from_millis(10));
    }
}
fn blocking_hook(f: &PushRepos, server: bool, name: &str) -> PathBuf {
    let marker = f.f.root.join("hook-ready");
    let directory = if server {
        f.server_hooks.clone()
    } else {
        f.f.root.join("no-hooks")
    };
    script(
        &directory.join(name),
        &format!("printf ready > {}\nsleep 30", quote(&marker)),
    );
    marker
}

#[test]
fn new_fast_forward_and_up_to_date_preserve_all_local_edits() {
    isolated(
        "new_fast_forward_and_up_to_date_preserve_all_local_edits",
        || {
            let f = PushRepos::new("sha1");
            let first = f.push(false);
            confirmed(&first, PushUpdateKind::New);
            assert!(first.upstream.is_none());
            assert!(first.status.unwrap().unwrap().upstream.is_none());
            assert_eq!(f.remote_head(), f.f.run(&["rev-parse", "HEAD"]));
            f.advance();
            f.f.write("file", b"staged\n");
            f.f.run(&["add", "file"]);
            f.f.write("file", b"unstaged\n");
            f.f.write("untracked", b"keep\n");
            let before = f.f.repo().status(&token()).unwrap();
            let index = fs::read(f.f.repository.join(".git/index")).unwrap();
            let refs = f.f.run(&["for-each-ref", "refs/heads", "refs/tags"]);
            let report = f.push(true);
            confirmed(&report, PushUpdateKind::FastForward);
            assert!(report.upstream.unwrap().is_ok());
            let status = report.status.unwrap().unwrap();
            assert_eq!(status.head, before.head);
            assert_eq!(status.entries, before.entries);
            assert_eq!(status.ahead_behind, Some((0, 0)));
            assert_eq!(f.f.run(&["for-each-ref", "refs/heads", "refs/tags"]), refs);
            assert_eq!(fs::read(f.f.repository.join(".git/index")).unwrap(), index);
            assert_eq!(
                fs::read(f.f.repository.join("file")).unwrap(),
                b"unstaged\n"
            );
            assert_eq!(
                fs::read(f.f.repository.join("untracked")).unwrap(),
                b"keep\n"
            );
            assert_eq!(f.remote_head(), f.f.run(&["rev-parse", "HEAD"]));
            let report = f.push(true);
            confirmed(&report, PushUpdateKind::UpToDate);
            assert!(report.upstream.unwrap().is_ok());
        },
    );
}

#[test]
fn config_cannot_broaden_push_or_implicitly_set_upstream() {
    isolated(
        "config_cannot_broaden_push_or_implicitly_set_upstream",
        || {
            let f = PushRepos::new("sha1");
            f.f.run(&["branch", "extra"]);
            f.f.run(&["tag", "-a", "extra-tag", "-m", "tag"]);
            for (key, value) in [
                ("remote.origin.mirror", "true"),
                ("remote.origin.push", "+refs/heads/*:refs/heads/*"),
                ("push.default", "matching"),
                ("push.followTags", "true"),
                ("push.autoSetupRemote", "true"),
                ("push.negotiate", "true"),
                ("push.recurseSubmodules", "on-demand"),
                ("push.pushOption", "PRIVATE-option"),
            ] {
                f.f.run(&["config", key, value]);
            }
            let config = fs::read(f.f.repository.join(".git/config")).unwrap();
            confirmed(&f.push(false), PushUpdateKind::New);
            assert_eq!(
                f.f.run_at(&f.bare, &["for-each-ref", "--format=%(refname)"]),
                b"refs/heads/main\n"
            );
            assert_eq!(
                fs::read(f.f.repository.join(".git/config")).unwrap(),
                config
            );
            assert!(f.f.repo().status(&token()).unwrap().upstream.is_none());
        },
    );
}

#[test]
fn explicit_noncurrent_branch_pushurl_rewriting_and_literal_names_work() {
    isolated(
        "explicit_noncurrent_branch_pushurl_rewriting_and_literal_names_work",
        || {
            let f = PushRepos::new("sha1");
            let remote = "Work.foo=文";
            // A leading + is legal inside a literal branch name; full refs prevent
            // it from being interpreted as a forced refspec.
            let local = "+feature/文=value.part";
            let destination = "+release/文=value.part";
            f.f.run(&["remote", "rename", "origin", remote]);
            f.f.run(&["branch", local]);
            f.f.run(&["tag", local]); // A tag with the same short name must not be pushed.
            f.f.run(&[
                "config",
                &format!("remote.{remote}.url"),
                f.f.root.join("missing").to_str().unwrap(),
            ]);
            f.f.run(&[
                "config",
                &format!("remote.{remote}.pushurl"),
                "fixture-push:destination",
            ]);
            f.f.run(&[
                "config",
                &format!("url.{}.insteadOf", f.bare.to_str().unwrap()),
                "fixture-push:destination",
            ]);
            f.f.run(&["config", &format!("remote.{remote}.mirror"), "true"]);
            let head = f.f.run(&["rev-parse", "HEAD"]);
            let report =
                f.f.repo()
                    .push(
                        remote,
                        local,
                        destination,
                        PushOptions { set_upstream: true },
                        &token(),
                    )
                    .unwrap();
            confirmed(&report, PushUpdateKind::New);
            assert!(report.upstream.unwrap().is_ok());
            assert_eq!(
                f.f.run_at(
                    &f.bare,
                    &["rev-parse", &format!("refs/heads/{destination}")]
                ),
                head
            );
            assert_eq!(f.f.run(&["symbolic-ref", "HEAD"]), b"refs/heads/main\n");
            assert_eq!(f.f.run_at(&f.bare, &["tag", "--list"]), b"");
        },
    );
}

#[test]
fn invalid_selections_and_cancelled_preflight_never_contact_remote() {
    isolated(
        "invalid_selections_and_cancelled_preflight_never_contact_remote",
        || {
            let f = PushRepos::new("sha1");
            let (repo, log) = f.logged(CommandLimits::default());
            for bad in [
                "",
                "--all",
                "../outside",
                "HEAD",
                "main:other",
                "refs/heads/main",
                "@{-1}",
            ] {
                assert!(
                    repo.push("origin", bad, "main", PushOptions::default(), &token())
                        .is_err(),
                    "{bad}"
                );
                assert!(
                    repo.push("origin", "main", bad, PushOptions::default(), &token())
                        .is_err(),
                    "{bad}"
                );
            }
            for remote in ["", "missing", "--all", "../outside", "file:///somewhere"] {
                assert!(
                    repo.push(remote, "main", "main", PushOptions::default(), &token())
                        .is_err()
                );
            }
            assert!(
                repo.push(
                    "origin",
                    "missing",
                    "main",
                    PushOptions::default(),
                    &token()
                )
                .is_err()
            );
            let cancelled = token();
            cancelled.cancel();
            assert!(matches!(
                repo.push("origin", "main", "main", PushOptions::default(), &cancelled),
                Err(Error::Cancelled)
            ));
            assert!(f.f.run_at(&f.bare, &["for-each-ref"]).is_empty());
            let text = fs::read_to_string(log.directory().join("rocket.jsonl")).unwrap();
            assert!(
                !text
                    .lines()
                    .any(|line| line.contains("\"event\":\"git_start\"")
                        && line.contains("\"operation\":\"push\""))
            );
        },
    );
}

#[test]
fn unborn_symbolic_and_unsafe_tracking_destinations_are_rejected() {
    isolated(
        "unborn_symbolic_and_unsafe_tracking_destinations_are_rejected",
        || {
            let f = PushRepos::new("sha1");
            let head = f.f.run(&["rev-parse", "HEAD"]);
            for mapping in [
                "+refs/heads/*:refs/heads/*",
                "+refs/heads/*:refs/tags/*",
                "+refs/heads/*:refs/remotes/other/*",
                "+refs/heads/main:refs/remotes/origin/renamed",
            ] {
                f.f.run(&["config", "remote.origin.fetch", mapping]);
                assert!(
                    f.f.repo()
                        .push("origin", "main", "main", PushOptions::default(), &token())
                        .is_err()
                );
            }
            f.f.run(&[
                "config",
                "remote.origin.fetch",
                "+refs/heads/*:refs/remotes/origin/*",
            ]);
            f.f.run(&[
                "symbolic-ref",
                "refs/remotes/origin/main",
                "refs/heads/main",
            ]);
            assert!(
                f.f.repo()
                    .push("origin", "main", "main", PushOptions::default(), &token())
                    .is_err()
            );
            f.f.run(&["symbolic-ref", "--delete", "refs/remotes/origin/main"]);
            f.f.run(&["symbolic-ref", "refs/heads/alias", "refs/heads/main"]);
            assert!(
                f.f.repo()
                    .push("origin", "alias", "main", PushOptions::default(), &token())
                    .is_err()
            );
            assert_eq!(f.f.run(&["rev-parse", "HEAD"]), head);
            f.f.run(&["switch", "--orphan", "unborn"]);
            assert!(
                f.f.repo()
                    .push("origin", "unborn", "main", PushOptions::default(), &token())
                    .is_err()
            );
            assert!(f.f.run_at(&f.bare, &["for-each-ref"]).is_empty());
        },
    );
}

#[test]
fn multiple_urls_remote_groups_and_missing_upstream_mappings_are_rejected() {
    isolated(
        "multiple_urls_remote_groups_and_missing_upstream_mappings_are_rejected",
        || {
            let f = PushRepos::new("sha1");
            for key in ["remote.origin.url", "remote.origin.pushurl"] {
                if key.ends_with("pushurl") {
                    f.f.run(&["config", key, f.bare.to_str().unwrap()]);
                }
                f.f.run(&["config", "--add", key, f.bare.to_str().unwrap()]);
                assert!(
                    f.f.repo()
                        .push("origin", "main", "main", PushOptions::default(), &token())
                        .is_err()
                );
                f.f.run(&["config", "--unset-all", key]);
                if key.ends_with(".url") {
                    f.f.run(&["config", key, f.bare.to_str().unwrap()]);
                }
            }
            f.f.run(&["config", "remotes.origin", "origin other"]);
            assert!(
                f.f.repo()
                    .push("origin", "main", "main", PushOptions::default(), &token())
                    .is_err()
            );
            f.f.run(&["config", "--unset-all", "remotes.origin"]);
            f.f.run(&["config", "--add", "remote.origin.fetch", "^refs/heads/main"]);
            assert!(
                f.f.repo()
                    .push(
                        "origin",
                        "main",
                        "main",
                        PushOptions { set_upstream: true },
                        &token()
                    )
                    .is_err()
            );
            f.f.run(&["config", "--unset-all", "remote.origin.fetch"]);
            assert!(
                f.f.repo()
                    .push(
                        "origin",
                        "main",
                        "main",
                        PushOptions { set_upstream: true },
                        &token()
                    )
                    .is_err()
            );
            assert!(f.f.run_at(&f.bare, &["for-each-ref"]).is_empty());
            // Tracking is optional when upstream setup was not requested.
            confirmed(&f.push(false), PushUpdateKind::New);
        },
    );
}

#[test]
fn divergence_and_fetch_first_are_distinct_and_never_force_history() {
    isolated(
        "divergence_and_fetch_first_are_distinct_and_never_force_history",
        || {
            let f = PushRepos::new("sha1");
            confirmed(&f.push(false), PushUpdateKind::New);
            let peer = f.f.root.join("peer");
            f.f.run(&["clone", f.bare.to_str().unwrap(), peer.to_str().unwrap()]);
            fs::write(peer.join("peer"), b"peer edit\n").unwrap();
            f.f.run_at(&peer, &["add", "peer"]);
            f.f.run_at(
                &peer,
                &[
                    "-c",
                    "user.name=Rocket",
                    "-c",
                    "user.email=rocket@example.invalid",
                    "-c",
                    "commit.gpgsign=false",
                    "commit",
                    "-m",
                    "peer advance",
                ],
            );
            f.f.run_at(&peer, &["push", "origin", "main"]);
            let remote_before = f.remote_head();
            f.advance();
            let local_before = f.f.run(&["rev-parse", "HEAD"]);
            for expected in [PushFailure::FetchFirst, PushFailure::NonFastForward] {
                let report = f.push(false);
                assert_eq!(report.result.unwrap_err().push_failure(), Some(expected));
                assert_eq!(report.remote_outcome, PushRemoteOutcome::Rejected);
                assert!(report.branches.is_ok());
                assert!(report.status.unwrap().is_ok());
                assert_eq!(f.remote_head(), remote_before);
                assert_eq!(f.f.run(&["rev-parse", "HEAD"]), local_before);
                f.f.repo()
                    .fetch("origin", None, &token())
                    .unwrap()
                    .result
                    .unwrap();
            }
        },
    );
}

#[test]
fn server_and_local_hook_rejections_have_private_errors_and_complete_logs() {
    isolated(
        "server_and_local_hook_rejections_have_private_errors_and_complete_logs",
        || {
            let f = PushRepos::new("sha1");
            let (repo, log) = f.logged(CommandLimits::default());
            script(
                &f.server_hooks.join("pre-receive"),
                "echo 'PRIVATE-server-token-and-policy' >&2\nexit 1",
            );
            let report = log
                .request("push", |_| {
                    repo.push("origin", "main", "main", PushOptions::default(), &token())
                })
                .unwrap();
            let error = report.result.unwrap_err();
            assert_eq!(error.push_failure(), Some(PushFailure::RemoteRejected));
            assert!(!error.to_string().contains("PRIVATE"));
            assert_eq!(report.remote_outcome, PushRemoteOutcome::Rejected);
            assert!(report.branches.is_ok());
            assert!(report.status.unwrap().is_ok());
            if let Error::Git { stderr, .. } = error {
                assert!(String::from_utf8_lossy(&stderr).contains("PRIVATE-server-token"));
            }
            fs::remove_file(f.server_hooks.join("pre-receive")).unwrap();
            script(
                &f.f.root.join("no-hooks/pre-push"),
                "echo 'PRIVATE-local-hook' >&2\nexit 1",
            );
            let report = repo
                .push("origin", "main", "main", PushOptions::default(), &token())
                .unwrap();
            assert_eq!(
                report.result.unwrap_err().push_failure(),
                Some(PushFailure::Other)
            );
            assert_eq!(report.remote_outcome, PushRemoteOutcome::Unknown);
            assert!(f.f.run_at(&f.bare, &["for-each-ref"]).is_empty());
            let text = fs::read_to_string(log.directory().join("rocket.jsonl")).unwrap();
            for event in [
                "step_start",
                "step_end",
                "write_queue",
                "git_start",
                "git_end",
                "write_result",
                "parse",
                "push_outcome",
                "refresh",
            ] {
                assert!(
                    text.contains(&format!("\"event\":\"{event}\"")),
                    "missing {event}"
                );
            }
            assert!(text.contains("\"failure_code\":\"push_remote_rejected\""));
            assert!(text.contains("\"remote_outcome\":\"rejected\""));
            assert!(text.contains("\"remote_outcome\":\"unknown\""));
            assert!(!text.contains("PRIVATE"));
            assert!(!text.contains(f.f.repository.to_str().unwrap()));
            assert_eq!(log.health().dropped_events, 0);
        },
    );
}

#[test]
fn cancellation_after_remote_commit_reports_unknown_without_rollback() {
    isolated(
        "cancellation_after_remote_commit_reports_unknown_without_rollback",
        || {
            let f = PushRepos::new("sha1");
            let marker = blocking_hook(&f, true, "post-receive");
            let (repo, log) = f.logged(CommandLimits::default());
            let head = f.f.run(&["rev-parse", "HEAD"]);
            let task = WriteTask::spawn(move |cancel| {
                repo.push(
                    "origin",
                    "main",
                    "main",
                    PushOptions { set_upstream: true },
                    &cancel,
                )
            })
            .unwrap();
            wait_file(&marker);
            assert_eq!(f.remote_head(), head); // Remote write happened before cancellation.
            // External changes made during the write must be observed after it
            // stops, even though the write's cancellation token remains set.
            f.f.run(&["branch", "PRIVATE-external-分支"]);
            f.f.write("PRIVATE-外部 🚀 é.txt", b"external edit\n");
            f.f.write("file", b"staged edit\n");
            f.f.run(&["add", "--", "file"]);
            let index = f.f.run(&["ls-files", "--stage", "-z"]);
            f.f.write("file", b"unstaged edit\n");
            task.cancel();
            let report = task.wait().unwrap();
            assert!(matches!(report.result, Err(Error::Cancelled)));
            assert_eq!(report.remote_outcome, PushRemoteOutcome::Unknown);
            assert!(report.upstream.is_none());
            assert!(report.branches.unwrap().iter().any(|branch| {
                branch.reference == "refs/heads/PRIVATE-external-分支".as_bytes()
            }));
            let status = report.status.unwrap().unwrap();
            assert!(status.entries.iter().any(|entry| {
                entry.path.as_bytes() == "PRIVATE-外部 🚀 é.txt".as_bytes()
            }));
            assert_eq!(f.f.run(&["ls-files", "--stage", "-z"]), index);
            assert_eq!(
                fs::read(f.f.repository.join("file")).unwrap(),
                b"unstaged edit\n"
            );
            assert_eq!(f.f.run(&["rev-parse", "HEAD"]), head);
            assert_eq!(f.remote_head(), head);
            let text = fs::read_to_string(log.directory().join("rocket.jsonl")).unwrap();
            assert!(text.contains("\"remote_outcome\":\"unknown\""));
            assert!(text.contains("\"outcome\":\"cancelled\""));
            assert_eq!(
                text.lines()
                    .filter(|l| l.contains("\"event\":\"git_start\"")
                        && l.contains("\"operation\":\"push\""))
                    .count(),
                1
            );
            assert!(!text.contains("PRIVATE"));
        },
    );
}

#[test]
fn timeout_and_output_limit_kill_hooks_and_keep_refreshes_separate() {
    isolated(
        "timeout_and_output_limit_kill_hooks_and_keep_refreshes_separate",
        || {
            let f = PushRepos::new("sha1");
            let marker = blocking_hook(&f, false, "pre-push");
            let (repo, _) = f.logged(CommandLimits {
                timeout: Duration::from_secs(3),
                ..CommandLimits::default()
            });
            let started = Instant::now();
            let report = repo
                .push("origin", "main", "main", PushOptions::default(), &token())
                .unwrap();
            assert!(marker.is_file());
            assert!(started.elapsed() < Duration::from_secs(15));
            assert!(matches!(report.result, Err(Error::TimedOut)));
            assert_eq!(report.remote_outcome, PushRemoteOutcome::Unknown);
            assert!(report.branches.is_ok());
            assert!(report.status.unwrap().is_ok());
            script(
                &f.f.root.join("no-hooks/pre-push"),
                "i=0\nwhile [ \"$i\" -lt 4096 ]; do printf 'PRIVATE-output-flood\\n' >&2; i=$((i + 1)); done\nsleep 30",
            );
            let (repo, _) = f.logged(CommandLimits {
                stderr_bytes: 1024,
                ..CommandLimits::default()
            });
            let report = repo
                .push("origin", "main", "main", PushOptions::default(), &token())
                .unwrap();
            assert!(matches!(
                report.result,
                Err(Error::OutputLimit {
                    stream: "stderr",
                    ..
                })
            ));
            assert_eq!(report.remote_outcome, PushRemoteOutcome::Unknown);
            assert!(report.branches.is_ok());
            assert!(report.status.unwrap().is_ok());
            assert!(f.f.run_at(&f.bare, &["for-each-ref"]).is_empty());
            fs::remove_file(f.f.root.join("no-hooks/pre-push")).unwrap();
            confirmed(&f.push(false), PushUpdateKind::New);
        },
    );
}

#[test]
fn linked_worktree_writes_wait_for_push_and_cancel_without_staging() {
    isolated(
        "linked_worktree_writes_wait_for_push_and_cancel_without_staging",
        || {
            let f = PushRepos::new("sha1");
            let linked = f.f.root.join("linked");
            f.f.run(&["worktree", "add", "-b", "linked", linked.to_str().unwrap()]);
            fs::write(linked.join("untracked"), b"keep me\n").unwrap();
            let marker = blocking_hook(&f, false, "pre-push");
            let repo = f.f.repo();
            let linked_repo = f.f.git.open(&linked, &token()).unwrap();
            let index_path = linked_repo.info().git_dir.join("index");
            let index = fs::read(&index_path).unwrap();
            let push = WriteTask::spawn(move |cancel| {
                repo.push("origin", "main", "main", PushOptions::default(), &cancel)
            })
            .unwrap();
            wait_file(&marker);
            let (started, ready) = sync_channel(0);
            let mut stage = WriteTask::spawn(move |cancel| {
                started.send(()).unwrap();
                linked_repo.stage_all(&cancel)
            })
            .unwrap();
            ready.recv_timeout(Duration::from_secs(5)).unwrap();
            let deadline = Instant::now() + Duration::from_secs(1);
            while Instant::now() < deadline {
                assert!(stage.try_finish().is_none(), "stage bypassed push gate");
                thread::sleep(Duration::from_millis(10));
            }
            stage.cancel();
            assert!(matches!(stage.wait(), Err(Error::Cancelled)));
            push.cancel();
            assert!(matches!(push.wait().unwrap().result, Err(Error::Cancelled)));
            assert_eq!(fs::read(index_path).unwrap(), index);
            f.f.git
                .open(&linked, &token())
                .unwrap()
                .stage_all(&token())
                .unwrap()
                .result
                .unwrap();
        },
    );
}

#[test]
fn remote_success_survives_parse_refresh_and_upstream_config_failures() {
    isolated(
        "remote_success_survives_parse_refresh_and_upstream_config_failures",
        || {
            let f = PushRepos::new("sha1");
            script(
                &f.f.root.join("no-hooks/pre-push"),
                "echo 'PRIVATE-hook-stdout'",
            );
            let report = f.push(false);
            assert!(report.result.is_ok());
            assert_eq!(report.remote_outcome, PushRemoteOutcome::Confirmed);
            assert!(report.update.unwrap().is_err());
            assert_eq!(f.remote_head(), f.f.run(&["rev-parse", "HEAD"]));
            fs::remove_file(f.f.root.join("no-hooks/pre-push")).unwrap();
            let lock = f.f.repository.join(".git/config.lock");
            fs::write(&lock, b"owned test lock").unwrap();
            let report = f.push(true);
            assert_eq!(report.remote_outcome, PushRemoteOutcome::Confirmed);
            assert!(report.error_diagnostics);
            assert!(report.upstream.unwrap().is_err());
            assert_eq!(fs::read(&lock).unwrap(), b"owned test lock");
            fs::remove_file(lock).unwrap();
            for i in 0..30 {
                f.f.write(&format!("{i}-{}", "x".repeat(90)), b"untracked");
            }
            let (repo, _) = f.logged(CommandLimits {
                stdout_bytes: 1024,
                ..CommandLimits::default()
            });
            let report = repo
                .push("origin", "main", "main", PushOptions::default(), &token())
                .unwrap();
            assert!(report.result.is_ok());
            assert!(report.update.unwrap().is_ok());
            assert_eq!(report.remote_outcome, PushRemoteOutcome::Confirmed);
            assert!(report.branches.is_ok());
            assert!(matches!(
                report.status.unwrap(),
                Err(Error::OutputLimit {
                    stream: "stdout",
                    ..
                })
            ));
        },
    );
}

#[test]
fn sha256_and_bare_sources_use_the_same_explicit_push_contract() {
    isolated(
        "sha256_and_bare_sources_use_the_same_explicit_push_contract",
        || {
            let f = PushRepos::new("sha256");
            confirmed(&f.push(true), PushUpdateKind::New);
            assert_eq!(f.remote_head().trim_ascii().len(), 64);
            let source = f.f.root.join("bare source.git");
            f.f.run(&[
                "clone",
                "--bare",
                f.f.repository.to_str().unwrap(),
                source.to_str().unwrap(),
            ]);
            f.f.run_at(
                &source,
                &["remote", "set-url", "origin", f.bare.to_str().unwrap()],
            );
            f.f.run_at(
                &source,
                &[
                    "config",
                    "core.hooksPath",
                    f.f.root.join("no-hooks").to_str().unwrap(),
                ],
            );
            let report =
                f.f.git
                    .open(&source, &token())
                    .unwrap()
                    .push(
                        "origin",
                        "main",
                        "from-bare",
                        PushOptions::default(),
                        &token(),
                    )
                    .unwrap();
            confirmed(&report, PushUpdateKind::New);
            assert!(report.status.is_none());
            assert_eq!(
                f.f.run_at(&f.bare, &["rev-parse", "refs/heads/from-bare"]),
                f.remote_head()
            );
        },
    );
}

#[test]
fn unavailable_destination_and_tracking_lock_do_not_hide_remote_outcomes() {
    isolated(
        "unavailable_destination_and_tracking_lock_do_not_hide_remote_outcomes",
        || {
            let f = PushRepos::new("sha1");
            f.f.run(&[
                "remote",
                "set-url",
                "origin",
                f.f.root.join("PRIVATE-missing.git").to_str().unwrap(),
            ]);
            let report = f.push(false);
            let error = report.result.unwrap_err();
            assert_eq!(
                error.push_failure(),
                Some(PushFailure::Transport(FetchFailure::RepositoryUnavailable))
            );
            assert!(!error.to_string().contains("PRIVATE"));
            assert_eq!(report.remote_outcome, PushRemoteOutcome::Unknown);
            assert!(report.branches.is_ok());
            assert!(report.status.unwrap().is_ok());
            f.f.run(&["remote", "set-url", "origin", f.bare.to_str().unwrap()]);
            let lock = f.f.repository.join(".git/refs/remotes/origin/main.lock");
            fs::create_dir_all(lock.parent().unwrap()).unwrap();
            fs::write(&lock, b"owned tracking lock").unwrap();
            let report = f.push(false);
            assert_eq!(report.remote_outcome, PushRemoteOutcome::Confirmed);
            assert!(report.error_diagnostics);
            assert_eq!(report.update.unwrap().unwrap().kind, PushUpdateKind::New);
            assert_eq!(f.remote_head(), f.f.run(&["rev-parse", "HEAD"]));
            assert_eq!(fs::read(lock).unwrap(), b"owned tracking lock");
            assert!(
                !f.f.output(
                    &f.f.repository,
                    &["show-ref", "--verify", "refs/remotes/origin/main"]
                )
                .status
                .success()
            );
        },
    );
}
