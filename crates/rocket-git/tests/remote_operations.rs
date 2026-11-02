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

struct Remotes {
    fixture: Fixture,
    bare: PathBuf,
    client: PathBuf,
}
impl Remotes {
    fn new(format: &str) -> Self {
        let fixture = Fixture::with_format(format);
        fixture.write("file", b"base\n");
        fixture.commit("base");
        let bare = fixture.root.join("upstream with spaces 文.git");
        fixture.run(&[
            "clone",
            "--bare",
            fixture.repository.to_str().unwrap(),
            bare.to_str().unwrap(),
        ]);
        let client = fixture.root.join("client with spaces 文");
        fixture.run(&["clone", bare.to_str().unwrap(), client.to_str().unwrap()]);
        for directory in [&bare, &client] {
            fixture.run_at(
                directory,
                &[
                    "config",
                    "core.hooksPath",
                    fixture.root.join("no-hooks").to_str().unwrap(),
                ],
            );
        }
        Self {
            fixture,
            bare,
            client,
        }
    }
    fn repo(&self) -> Repository {
        self.fixture.git.open(&self.client, &token()).unwrap()
    }
    fn run(&self, args: &[&str]) -> Vec<u8> {
        self.fixture.run_at(&self.client, args)
    }
    fn publish(&self, message: &str) {
        self.fixture.write("file", format!("{message}\n"));
        self.fixture.commit(message);
        // Test setup pushes only between newly owned local fixture repositories.
        self.fixture
            .run(&["push", self.bare.to_str().unwrap(), "main"]);
    }
}

#[test]
fn lists_effective_config_without_contacting_or_rewriting_urls() {
    let f = Fixture::new();
    assert!(f.repo().remotes(&token()).unwrap().is_empty());
    f.run(&[
        "config",
        "remote.Work.foo.url",
        "fixture:PRIVATE\nsecond line",
    ]);
    f.run(&["config", "--add", "remote.Work.foo.url", "second URL"]);
    f.run(&["config", "remote.Work.foo.pushurl", "PRIVATE push URL"]);
    f.run(&[
        "config",
        "remote.Work.foo.fetch",
        "+refs/heads/*:refs/remotes/Work.foo/*",
    ]);
    f.run(&[
        "config",
        "url.https://example.invalid/.insteadOf",
        "fixture:",
    ]);
    let include = f.root.join("included.config");
    f.run(&[
        "config",
        "--file",
        include.to_str().unwrap(),
        "remote.work.url",
        "local path",
    ]);
    f.run(&["config", "include.path", include.to_str().unwrap()]);
    let remotes = f.repo().remotes(&token()).unwrap();
    assert_eq!(remotes.len(), 2);
    assert_eq!(remotes[0].name, b"Work.foo");
    assert_eq!(
        remotes[0].urls,
        [
            b"fixture:PRIVATE\nsecond line".to_vec(),
            b"second URL".to_vec()
        ]
    );
    assert_eq!(remotes[0].push_urls, [b"PRIVATE push URL".to_vec()]);
    assert_eq!(remotes[1].name, b"work");
}

#[test]
fn fetch_refreshes_tracking_and_behind_without_changing_local_edits_or_pruning() {
    let f = Remotes::new("sha1");
    f.run(&["config", "fetch.all", "true"]);
    f.run(&[
        "remote",
        "add",
        "unselected",
        f.fixture.root.join("missing").to_str().unwrap(),
    ]);
    f.run(&["config", "fetch.prune", "true"]);
    f.run(&["config", "fetch.pruneTags", "true"]);
    f.run(&["config", "remote.origin.tagOpt", "--tags"]);
    f.run(&["update-ref", "refs/remotes/origin/stale", "HEAD"]);
    f.run(&["tag", "local-tag"]);
    f.fixture.run(&["tag", "upstream-tag"]);
    f.fixture
        .run(&["push", f.bare.to_str().unwrap(), "refs/tags/upstream-tag"]);
    fs::write(f.client.join("file"), b"staged locally\n").unwrap();
    f.run(&["add", "file"]);
    fs::write(f.client.join("file"), b"unstaged locally\n").unwrap();
    let before = f.repo().status(&token()).unwrap();
    let index = fs::read(f.client.join(".git/index")).unwrap();
    f.publish("upstream advance");
    f.fixture.run(&[
        "push",
        f.bare.to_str().unwrap(),
        "main:refs/heads/feature/demo",
    ]);
    let (sender, receiver) = sync_channel(1); // Deliberately leave the queue full until completion.
    let report = f.repo().fetch("origin", Some(&sender), &token()).unwrap();
    let output = report.result.unwrap();
    let updates = output.updates.unwrap();
    assert!(
        updates
            .iter()
            .any(|u| u.kind == FetchUpdateKind::FastForward
                && u.reference == b"refs/remotes/origin/main")
    );
    assert!(
        updates.iter().any(|u| u.kind == FetchUpdateKind::New
            && u.reference == b"refs/remotes/origin/feature/demo")
    );
    assert_eq!(receiver.try_iter().count(), 1);
    let status = report.status.unwrap().unwrap();
    assert_eq!(status.head, before.head);
    assert_eq!(status.entries, before.entries);
    assert_eq!(status.ahead_behind, Some((0, 1)));
    assert_eq!(fs::read(f.client.join(".git/index")).unwrap(), index);
    assert_eq!(
        fs::read(f.client.join("file")).unwrap(),
        b"unstaged locally\n"
    );
    assert!(
        report
            .branches
            .unwrap()
            .iter()
            .any(|b| b.reference == b"refs/remotes/origin/stale")
    );
    assert_eq!(f.run(&["tag", "--list"]), b"local-tag\n");
    assert!(f.client.join(".git/FETCH_HEAD").is_file());
}

#[test]
fn up_to_date_and_forced_updates_are_distinguished() {
    let f = Remotes::new("sha1");
    let base = f.fixture.run(&["rev-parse", "HEAD"]);
    let repo = f.repo();
    let updates = repo
        .fetch("origin", None, &token())
        .unwrap()
        .result
        .unwrap()
        .updates
        .unwrap();
    assert_eq!(updates[0].kind, FetchUpdateKind::UpToDate);
    f.publish("advance");
    repo.fetch("origin", None, &token())
        .unwrap()
        .result
        .unwrap();
    f.fixture.run_at(
        &f.bare,
        &[
            "update-ref",
            "refs/heads/main",
            std::str::from_utf8(&base).unwrap().trim(),
        ],
    );
    let updates = repo
        .fetch("origin", None, &token())
        .unwrap()
        .result
        .unwrap()
        .updates
        .unwrap();
    assert_eq!(updates[0].kind, FetchUpdateKind::Forced);
    assert_eq!(
        updates[0].new.as_str(),
        std::str::from_utf8(&base).unwrap().trim()
    );
}

#[test]
fn missing_names_urls_and_cancelled_preflight_never_attempt_fetch() {
    let f = Fixture::new();
    let repo = f.repo();
    for name in [
        "origin",
        "--all",
        "../outside",
        "https://example.invalid/repo",
        "",
    ] {
        assert!(repo.fetch(name, None, &token()).is_err());
    }
    f.run(&[
        "config",
        "remote.empty.fetch",
        "+refs/heads/*:refs/remotes/empty/*",
    ]);
    assert!(matches!(
        repo.fetch("empty", None, &token()),
        Err(Error::Unsupported(_))
    ));
    let cancelled = token();
    cancelled.cancel();
    assert!(matches!(
        repo.fetch("origin", None, &cancelled),
        Err(Error::Cancelled)
    ));
    assert!(!f.repository.join(".git/FETCH_HEAD").exists());
}

#[test]
fn rejects_mappings_to_local_branches_tags_other_remotes_and_symbolic_exact_targets() {
    let f = Remotes::new("sha1");
    let head = f.run(&["rev-parse", "HEAD"]);
    let before = f.run(&["show-ref"]);
    let repo = f.repo();
    for spec in [
        "+refs/*:refs/*",
        "+refs/heads/*:refs/heads/*",
        "refs/tags/*:refs/tags/*",
        "+refs/heads/*:refs/remotes/other/*",
        "refs/heads/main:refs/remotes/origin/renamed",
        "^refs/heads/main",
        "+refs/heads/**:refs/remotes/origin/**",
    ] {
        f.run(&["config", "remote.origin.fetch", spec]);
        assert!(
            repo.fetch("origin", None, &token()).is_err(),
            "accepted {spec}"
        );
        assert_eq!(f.run(&["show-ref"]), before);
    }
    f.run(&[
        "config",
        "remote.origin.fetch",
        "+refs/heads/main:refs/remotes/origin/main",
    ]);
    f.run(&[
        "symbolic-ref",
        "refs/remotes/origin/main",
        "refs/heads/dangling",
    ]);
    assert!(matches!(
        repo.fetch("origin", None, &token()),
        Err(Error::Unsupported(_))
    ));
    assert!(
        !f.fixture
            .output(&f.client, &["show-ref", "--verify", "refs/heads/dangling"])
            .status
            .success()
    );
    assert_eq!(f.run(&["rev-parse", "HEAD"]), head);
    f.run(&[
        "symbolic-ref",
        "refs/remotes/origin/main",
        "refs/heads/main",
    ]);
    assert!(matches!(
        repo.fetch("origin", None, &token()),
        Err(Error::Unsupported(_))
    ));
    assert_eq!(f.run(&["rev-parse", "HEAD"]), head);
}

#[test]
fn nonexistent_local_transport_returns_failure_with_independent_refreshes() {
    let f = Remotes::new("sha1");
    f.run(&[
        "remote",
        "set-url",
        "origin",
        f.fixture.root.join("missing.git").to_str().unwrap(),
    ]);
    let report = f.repo().fetch("origin", None, &token()).unwrap();
    assert!(matches!(report.result, Err(Error::Git { .. })));
    assert!(report.branches.is_ok());
    assert!(report.status.unwrap().is_ok());
}

#[test]
fn selective_mappings_exclusions_and_local_url_rewriting_work_with_literal_names() {
    let f = Remotes::new("sha1");
    let name = "Work.文=team";
    f.run(&["remote", "rename", "origin", name]);
    f.run(&["remote", "set-url", name, "fixture-local:upstream"]);
    f.run(&[
        "config",
        &format!("url.{}.insteadOf", f.bare.to_str().unwrap()),
        "fixture-local:upstream",
    ]);
    f.run(&[
        "config",
        &format!("remote.{name}.fetch"),
        &format!("+refs/heads/feature/*:refs/remotes/{name}/feature/*"),
    ]);
    f.run(&[
        "config",
        "--add",
        &format!("remote.{name}.fetch"),
        "^refs/heads/feature/excluded",
    ]);
    let before = f.run(&["rev-parse", &format!("refs/remotes/{name}/main")]);
    f.publish("advance");
    for branch in ["feature/included", "feature/excluded", "outside"] {
        f.fixture.run(&[
            "push",
            f.bare.to_str().unwrap(),
            &format!("main:refs/heads/{branch}"),
        ]);
    }
    let updates = f
        .repo()
        .fetch(name, None, &token())
        .unwrap()
        .result
        .unwrap()
        .updates
        .unwrap();
    assert_eq!(updates.len(), 1);
    assert_eq!(
        updates[0].reference,
        format!("refs/remotes/{name}/feature/included").as_bytes()
    );
    assert_eq!(
        f.run(&["rev-parse", &format!("refs/remotes/{name}/main")]),
        before
    );
    assert_eq!(
        f.repo().remotes(&token()).unwrap()[0].urls,
        [b"fixture-local:upstream".to_vec()]
    );
}

#[test]
fn exact_branch_mapping_works_but_a_colliding_remote_group_is_rejected() {
    let f = Remotes::new("sha1");
    f.run(&[
        "config",
        "remote.origin.fetch",
        "+refs/heads/main:refs/remotes/origin/main",
    ]);
    f.publish("advance");
    assert_eq!(
        f.repo()
            .fetch("origin", None, &token())
            .unwrap()
            .result
            .unwrap()
            .updates
            .unwrap()[0]
            .kind,
        FetchUpdateKind::FastForward
    );
    f.run(&["config", "remotes.origin", "other"]);
    let before = f.run(&["show-ref"]);
    assert!(matches!(
        f.repo().fetch("origin", None, &token()),
        Err(Error::Unsupported(_))
    ));
    assert_eq!(f.run(&["show-ref"]), before);
}

#[test]
fn wildcard_fetch_skips_symbolic_destinations_without_updating_their_local_targets() {
    let f = Remotes::new("sha1");
    let before = f.run(&["rev-parse", "HEAD"]);
    f.run(&[
        "symbolic-ref",
        "refs/remotes/origin/main",
        "refs/heads/main",
    ]);
    f.publish("advance");
    let report = f.repo().fetch("origin", None, &token()).unwrap();
    assert!(report.result.unwrap().updates.unwrap().is_empty());
    assert_eq!(f.run(&["rev-parse", "HEAD"]), before);
}

#[test]
fn fetch_supports_sha256_and_bare_destinations() {
    let f = Remotes::new("sha256");
    f.publish("sha256 advance");
    let updates = f
        .repo()
        .fetch("origin", None, &token())
        .unwrap()
        .result
        .unwrap()
        .updates
        .unwrap();
    assert_eq!(updates[0].new.as_str().len(), 64);
    let bare_client = f.fixture.root.join("bare-client.git");
    f.fixture.run(&[
        "init",
        "--bare",
        "--object-format=sha256",
        bare_client.to_str().unwrap(),
    ]);
    f.fixture.run_at(
        &bare_client,
        &["remote", "add", "origin", f.bare.to_str().unwrap()],
    );
    f.fixture.run_at(
        &bare_client,
        &[
            "config",
            "core.hooksPath",
            f.fixture.root.join("no-hooks").to_str().unwrap(),
        ],
    );
    let repo = f.fixture.git.open(&bare_client, &token()).unwrap();
    let report = repo.fetch("origin", None, &token()).unwrap();
    let updates = report.result.unwrap().updates.unwrap();
    assert_eq!(updates[0].kind, FetchUpdateKind::New);
    assert_eq!(updates[0].new.as_str().len(), 64);
    assert!(report.status.is_none());
    assert!(
        report
            .branches
            .unwrap()
            .iter()
            .any(|b| b.reference == b"refs/remotes/origin/main")
    );
}

#[test]
fn an_empty_local_remote_can_be_fetched_before_the_first_commit() {
    let f = Fixture::new();
    let bare = f.root.join("empty.git");
    f.run(&["init", "--bare", bare.to_str().unwrap()]);
    f.run(&["remote", "add", "origin", bare.to_str().unwrap()]);
    let report = f.repo().fetch("origin", None, &token()).unwrap();
    assert!(report.result.unwrap().updates.unwrap().is_empty());
    assert!(report.branches.unwrap().is_empty());
    assert!(matches!(
        report.status.unwrap().unwrap().head,
        Head::Unborn { .. }
    ));
}

#[test]
fn a_locked_ref_fails_the_entire_atomic_update_and_preserves_the_lock_file() {
    let f = Remotes::new("sha1");
    f.publish("advance");
    for branch in ["blocked", "good"] {
        f.fixture.run(&[
            "push",
            f.bare.to_str().unwrap(),
            &format!("main:refs/heads/{branch}"),
        ]);
    }
    let before = f.run(&["show-ref"]);
    let lock = f.client.join(".git/refs/remotes/origin/blocked.lock");
    fs::write(&lock, b"owned test lock\n").unwrap();
    let report = f.repo().fetch("origin", None, &token()).unwrap();
    assert!(matches!(report.result, Err(Error::Git { .. })));
    assert!(report.branches.is_ok());
    assert!(report.status.unwrap().is_ok());
    assert_eq!(f.run(&["show-ref"]), before);
    assert_eq!(fs::read(lock).unwrap(), b"owned test lock\n");
}

fn quote_path(path: &Path) -> String {
    format!(
        "'{}'",
        path.to_str()
            .unwrap()
            .replace('\\', "/")
            .replace('\'', "'\\''")
    )
}
fn hook(f: &Remotes, state: &str, body: &str) -> PathBuf {
    let marker = f.fixture.root.join("hook-ready");
    let path = f.fixture.root.join("no-hooks/reference-transaction");
    fs::write(
        &path,
        format!(
            "#!/bin/sh\nif [ \"$1\" = '{state}' ]; then\n{body}\nprintf ready > {}\nsleep 30\nfi\n",
            quote_path(&marker)
        ),
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
    }
    marker
}
fn wait_file(path: &Path) {
    let deadline = Instant::now() + Duration::from_secs(20);
    while !path.is_file() {
        assert!(Instant::now() < deadline, "fixture hook did not start");
        thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn cancellation_after_refs_commit_keeps_the_report_refreshes_and_private_logs() {
    let f = Remotes::new("sha1");
    f.run(&["remote", "rename", "origin", "PRIVATE-remote"]);
    f.publish("PRIVATE-advance");
    let target = f.fixture.run(&["rev-parse", "HEAD"]);
    let marker = hook(
        &f,
        "committed",
        "printf 'PRIVATE hook output\\nReceiving objects: 100%% (2/2)\\r' >&2",
    );
    let log = DiagnosticLog::open(f.fixture.root.join("logs")).unwrap();
    let client = f.client.clone();
    let git = f.fixture.git.executable().to_path_buf();
    let logger = log.clone();
    let (sender, receiver) = sync_channel(32);
    let task = WriteTask::spawn(move |cancel| {
        logger.request("fetch", |context| {
            let repo = GitClient::new_with_log(git, CommandLimits::default(), context.clone())?
                .open(client, &cancel)?;
            repo.fetch("PRIVATE-remote", Some(&sender), &cancel)
        })
    })
    .unwrap();
    wait_file(&marker);
    task.cancel();
    let report = task.wait().unwrap();
    assert!(matches!(report.result, Err(Error::Cancelled)));
    assert_eq!(report.status.unwrap().unwrap().ahead_behind, Some((0, 1)));
    assert!(
        report
            .branches
            .unwrap()
            .iter()
            .any(|b| b.reference == b"refs/remotes/PRIVATE-remote/main"
                && b.target.as_str().as_bytes() == target.trim_ascii())
    );
    assert!(
        receiver
            .try_iter()
            .any(|p| p.phase == FetchPhase::Receiving)
    );
    let text = fs::read_to_string(log.directory().join("rocket.jsonl")).unwrap();
    assert!(!text.contains("PRIVATE"));
    assert!(!text.contains(f.bare.to_str().unwrap()));
    assert!(!text.contains(std::str::from_utf8(target.trim_ascii()).unwrap()));
    assert!(
        text.lines()
            .any(|l| l.contains("\"event\":\"write_result\"")
                && l.contains("\"operation\":\"fetch\"")
                && l.contains("\"outcome\":\"cancelled\""))
    );
    assert_eq!(
        text.lines()
            .filter(|l| l.contains("\"event\":\"refresh\"") && l.contains("\"outcome\":\"ok\""))
            .count(),
        2
    );
    assert_eq!(log.health().dropped_events, 0);
}

#[test]
fn timeout_terminates_a_fetch_hook_and_refreshes_without_retrying() {
    let f = Remotes::new("sha1");
    f.publish("advance");
    let before = f.run(&["show-ref"]);
    let marker = hook(&f, "prepared", "");
    let git = GitClient::new(
        f.fixture.git.executable(),
        CommandLimits {
            timeout: Duration::from_secs(3),
            ..CommandLimits::default()
        },
    )
    .unwrap();
    let repo = git.open(&f.client, &token()).unwrap();
    let started = Instant::now();
    let report = repo.fetch("origin", None, &token()).unwrap();
    assert!(matches!(report.result, Err(Error::TimedOut)));
    assert!(started.elapsed() < Duration::from_secs(15));
    assert!(marker.is_file());
    assert!(report.branches.is_ok());
    assert!(report.status.unwrap().is_ok());
    assert_eq!(f.run(&["show-ref"]), before);
}

#[test]
fn fetch_uses_the_shared_write_gate_across_linked_worktrees() {
    let f = Remotes::new("sha1");
    let linked = f.fixture.root.join("linked");
    f.run(&["worktree", "add", "-b", "linked", linked.to_str().unwrap()]);
    fs::write(linked.join("unstaged"), b"preserve me\n").unwrap();
    f.publish("advance");
    let repo = f.repo();
    let linked_repo = f.fixture.git.open(&linked, &token()).unwrap();
    let marker = hook(&f, "prepared", "");
    let fetch = WriteTask::spawn(move |cancel| repo.fetch("origin", None, &cancel)).unwrap();
    wait_file(&marker);
    let before = fs::read(linked_repo.info().git_dir.join("index")).unwrap();
    let (started, ready) = sync_channel(0);
    let mut stage = WriteTask::spawn(move |cancel| {
        started.send(()).unwrap();
        linked_repo.stage_all(&cancel)
    })
    .unwrap();
    ready.recv_timeout(Duration::from_secs(5)).unwrap();
    // The fetch's prepared transaction remains blocked in its fixture hook.
    // Observe that staging stays pending while that independent Git process runs.
    let deadline = Instant::now() + Duration::from_secs(1);
    while Instant::now() < deadline {
        assert!(
            stage.try_finish().is_none(),
            "stage bypassed the shared fetch gate"
        );
        thread::sleep(Duration::from_millis(10));
    }
    stage.cancel();
    assert!(matches!(stage.wait(), Err(Error::Cancelled)));
    fetch.cancel();
    assert!(matches!(
        fetch.wait().unwrap().result,
        Err(Error::Cancelled)
    ));
    let linked_repo = f.fixture.git.open(&linked, &token()).unwrap();
    assert_eq!(
        fs::read(linked_repo.info().git_dir.join("index")).unwrap(),
        before
    );
    // The fetch permit is released even after cancellation.
    linked_repo.stage_all(&token()).unwrap().result.unwrap();
    assert!(
        linked_repo
            .status(&token())
            .unwrap()
            .entries
            .iter()
            .any(|e| e.index == Change::Added)
    );
}

#[test]
fn successful_fetch_survives_a_failed_status_refresh() {
    let f = Remotes::new("sha1");
    f.publish("advance");
    for i in 0..30 {
        fs::write(
            f.client.join(format!("{i}-{}", "x".repeat(90))),
            b"untracked",
        )
        .unwrap();
    }
    let git = GitClient::new(
        f.fixture.git.executable(),
        CommandLimits {
            stdout_bytes: 1024,
            ..CommandLimits::default()
        },
    )
    .unwrap();
    let report = git
        .open(&f.client, &token())
        .unwrap()
        .fetch("origin", None, &token())
        .unwrap();
    assert!(report.result.unwrap().updates.is_ok());
    assert!(report.branches.is_ok());
    assert!(matches!(
        report.status.unwrap(),
        Err(Error::OutputLimit {
            stream: "stdout",
            ..
        })
    ));
    assert_eq!(
        f.run(&["rev-parse", "refs/remotes/origin/main"]),
        f.fixture.run(&["rev-parse", "HEAD"])
    );
}

#[test]
fn excessive_fetch_stderr_is_bounded_even_with_a_disconnected_progress_consumer() {
    let f = Remotes::new("sha1");
    f.publish("advance");
    hook(
        &f,
        "prepared",
        "i=0\nwhile [ \"$i\" -lt 4096 ]; do printf 'PRIVATE-flood-record\\n' >&2; i=$((i + 1)); done",
    );
    let git = GitClient::new(
        f.fixture.git.executable(),
        CommandLimits {
            stderr_bytes: 4096,
            ..CommandLimits::default()
        },
    )
    .unwrap();
    let (sender, receiver) = sync_channel(1);
    drop(receiver);
    let report = git
        .open(&f.client, &token())
        .unwrap()
        .fetch("origin", Some(&sender), &token())
        .unwrap();
    assert!(matches!(
        report.result,
        Err(Error::OutputLimit {
            stream: "stderr",
            limit: 4096
        })
    ));
    assert!(report.branches.is_ok());
    assert!(report.status.unwrap().is_ok());
}
