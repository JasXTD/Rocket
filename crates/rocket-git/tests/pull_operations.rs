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
fn isolated(name: &str, test: fn()) {
    common::isolated("ROCKET_PULL_CASE", name, test);
}

struct PullRepos {
    f: Fixture,
    bare: PathBuf,
    client: PathBuf,
}
impl PullRepos {
    fn new(format: &str) -> Self {
        let f = Fixture::with_format(format);
        f.write("file", b"base\n");
        f.commit("base");
        let bare = f.root.join("PRIVATE upstream with spaces 文.git");
        f.run(&[
            "clone",
            "--bare",
            f.repository.to_str().unwrap(),
            bare.to_str().unwrap(),
        ]);
        let client = f.root.join("PRIVATE client with spaces 文");
        f.run(&["clone", bare.to_str().unwrap(), client.to_str().unwrap()]);
        for directory in [&client, &bare] {
            for (key, value) in [
                ("user.name", "Rocket Test"),
                ("user.email", "rocket@example.invalid"),
                ("commit.gpgsign", "false"),
                ("core.autocrlf", "false"),
            ] {
                f.run_at(directory, &["config", key, value]);
            }
            f.run_at(
                directory,
                &[
                    "config",
                    "core.hooksPath",
                    f.root.join("no-hooks").to_str().unwrap(),
                ],
            );
        }
        Self { f, bare, client }
    }
    fn run(&self, args: &[&str]) -> Vec<u8> {
        self.f.run_at(&self.client, args)
    }
    fn repo(&self) -> Repository {
        self.f.git.open(&self.client, &token()).unwrap()
    }
    fn publish(&self) {
        self.f.write("file", b"advanced\n");
        self.f.commit("PRIVATE remote advance");
        self.f.run(&["push", self.bare.to_str().unwrap(), "main"]);
    }
    fn logged(&self, limits: CommandLimits) -> (Repository, DiagnosticLog) {
        let log = DiagnosticLog::open(self.f.root.join("logs")).unwrap();
        let repo = GitClient::new_with_log(self.f.git.executable(), limits, log.clone())
            .unwrap()
            .open(&self.client, &token())
            .unwrap();
        (repo, log)
    }
    fn snapshot(&self) -> (Vec<u8>, Vec<u8>, Vec<u8>) {
        (
            self.run(&["rev-parse", "HEAD"]),
            fs::read(self.client.join(".git/index")).unwrap(),
            fs::read(self.client.join("file")).unwrap(),
        )
    }
}
fn confirmed(report: &PullReport, kind: PullUpdateKind) {
    assert!(report.fetch.is_ok(), "{:?}", report.fetch);
    let update = report.update.as_ref().unwrap().as_ref().unwrap();
    assert_eq!(update.kind, kind);
    assert!(update.verification.is_ok(), "{:?}", update.verification);
    assert!(report.status.is_ok(), "{:?}", report.status);
    assert!(report.branches.is_ok(), "{:?}", report.branches);
    assert!(report.state.as_ref().unwrap().is_idle());
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
    let deadline = Instant::now() + Duration::from_secs(30);
    while !path.is_file() {
        assert!(Instant::now() < deadline, "fixture hook did not start");
        thread::sleep(Duration::from_millis(10));
    }
}
fn fetch_hook(f: &PullRepos, state: &str, body: &str) {
    script(
        &f.f.root.join("no-hooks/reference-transaction"),
        &format!(
            "if [ \"$1\" = '{state}' ]; then\nwhile read old new reference; do\nif [ \"$reference\" = 'refs/remotes/origin/main' ]; then\n{body}\nfi\ndone\nfi"
        ),
    );
}

fn git_with_corrupted_fetch_stdout(f: &PullRepos) -> PathBuf {
    // Run real Git for every command, then inject one malformed protocol line
    // after a successful fetch. Hook stdout is redirected by Git to stderr, so
    // a hook alone cannot exercise this transport-output/parser boundary.
    let source = f.f.root.join("git-output-fixture.rs");
    let executable = f.f.root.join(if cfg!(windows) {
        "git-output-fixture.exe"
    } else {
        "git-output-fixture"
    });
    fs::write(
        &source,
        format!(
            r#"
use std::{{io::Write, process::{{Command, Stdio}}}};
fn main() {{
    let arguments: Vec<_> = std::env::args_os().skip(1).collect();
    let fetching = arguments.iter().any(|a| a == "fetch");
    let mut command = Command::new({:?});
    command.args(arguments).stdin(Stdio::inherit());
    #[cfg(windows)] {{
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000);
    }}
    let output = command.output().unwrap();
    std::io::stdout().write_all(&output.stdout).unwrap();
    std::io::stderr().write_all(&output.stderr).unwrap();
    if fetching && output.status.success() {{
        std::io::stdout().write_all(b"PRIVATE-corrupt-porcelain\n").unwrap();
    }}
    std::process::exit(output.status.code().unwrap_or(1));
}}
"#,
            f.f.git.executable().to_str().unwrap()
        ),
    )
    .unwrap();
    let mut compiler = std::process::Command::new("rustc");
    compiler
        .arg("--edition=2024")
        .arg(&source)
        .arg("-o")
        .arg(&executable);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        compiler.creation_flags(0x0800_0000);
    }
    let output = compiler.output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    executable
}

#[test]
fn fast_forward_up_to_date_and_local_ahead_support_both_object_formats() {
    isolated(
        "fast_forward_up_to_date_and_local_ahead_support_both_object_formats",
        || {
            for format in ["sha1", "sha256"] {
                let f = PullRepos::new(format);
                let repo = f.repo();
                confirmed(
                    &repo.pull_ff_only(None, &token()).unwrap(),
                    PullUpdateKind::UpToDate,
                );
                fs::write(f.client.join("untracked"), b"keep\n").unwrap();
                f.publish();
                let (sender, receiver) = sync_channel(1);
                drop(receiver); // A disconnected progress consumer must not block the pull.
                let report = repo.pull_ff_only(Some(&sender), &token()).unwrap();
                confirmed(&report, PullUpdateKind::FastForward);
                assert_eq!(report.status.unwrap().ahead_behind, Some((0, 0)));
                assert_eq!(
                    f.run(&["rev-parse", "HEAD"]),
                    f.f.run(&["rev-parse", "HEAD"])
                );
                assert_eq!(fs::read(f.client.join("file")).unwrap(), b"advanced\n");
                assert_eq!(fs::read(f.client.join("untracked")).unwrap(), b"keep\n");
                f.run(&["commit", "--allow-empty", "-m", "local only"]);
                let before = f.run(&["rev-parse", "HEAD"]);
                let report = repo.pull_ff_only(None, &token()).unwrap();
                confirmed(&report, PullUpdateKind::UpToDate);
                assert_eq!(report.status.unwrap().ahead_behind, Some((1, 0)));
                assert_eq!(f.run(&["rev-parse", "HEAD"]), before);
            }
        },
    );
}

#[test]
fn configured_upstream_and_literal_names_override_unrelated_defaults() {
    isolated(
        "configured_upstream_and_literal_names_override_unrelated_defaults",
        || {
            let f = PullRepos::new("sha1");
            f.run(&["branch", "-m", "topic=a"]);
            f.run(&["remote", "rename", "origin", "up=stream.foo"]);
            f.run(&["config", "branch.topic=a.merge", "refs/heads/feature/demo"]);
            f.run(&[
                "config",
                "branch.topic=a.mergeOptions",
                "--squash --no-verify --autostash",
            ]);
            for (key, value) in [
                ("pull.rebase", "true"),
                ("merge.ff", "false"),
                ("merge.autostash", "true"),
                ("submodule.recurse", "true"),
                ("fetch.prune", "true"),
                ("fetch.all", "true"),
            ] {
                f.run(&["config", key, value]);
            }
            f.run(&[
                "remote",
                "add",
                "unselected",
                f.f.root.join("does-not-exist").to_str().unwrap(),
            ]);
            f.run(&["update-ref", "refs/remotes/up=stream.foo/stale", "HEAD"]);
            let configuration = fs::read(f.client.join(".git/config")).unwrap();
            f.publish();
            f.f.run(&["tag", "PRIVATE-tag"]);
            f.f.run(&[
                "push",
                f.bare.to_str().unwrap(),
                "main:refs/heads/feature/demo",
                "refs/tags/PRIVATE-tag",
            ]);
            let report = f.repo().pull_ff_only(None, &token()).unwrap();
            confirmed(&report, PullUpdateKind::FastForward);
            assert_eq!(report.upstream.remote, b"up=stream.foo");
            assert_eq!(report.upstream.branch, b"topic=a");
            assert_eq!(report.upstream.remote_reference, b"refs/heads/feature/demo");
            assert_eq!(
                fs::read(f.client.join(".git/config")).unwrap(),
                configuration
            );
            assert_eq!(f.run(&["rev-list", "--count", "HEAD"]), b"2\n");
            f.run(&["show-ref", "--verify", "refs/remotes/up=stream.foo/stale"]);
            assert!(f.run(&["tag", "--list"]).is_empty());
            assert!(!f.client.join(".git/rebase-merge").exists());
        },
    );
}

#[test]
fn preflight_rejects_local_edits_detached_unborn_bare_and_active_operations() {
    isolated(
        "preflight_rejects_local_edits_detached_unborn_bare_and_active_operations",
        || {
            let f = PullRepos::new("sha1");
            let repo = f.repo();
            f.publish();
            let untouched = f.snapshot();
            for staged in [false, true] {
                fs::write(f.client.join("file"), b"local edit\n").unwrap();
                if staged {
                    f.run(&["add", "file"]);
                }
                let before = f.snapshot();
                assert_eq!(
                    repo.pull_ff_only(None, &token())
                        .unwrap_err()
                        .pull_failure(),
                    Some(PullFailure::DirtyWorktree)
                );
                assert_eq!(f.snapshot(), before);
                f.run(&["reset", "--hard", "HEAD"]); // Owned fixture only.
            }
            f.run(&["checkout", "--detach"]);
            assert_eq!(
                repo.pull_ff_only(None, &token())
                    .unwrap_err()
                    .pull_failure(),
                Some(PullFailure::DetachedHead)
            );
            f.run(&["checkout", "main"]);
            for marker in ["MERGE_HEAD", "CHERRY_PICK_HEAD", "REVERT_HEAD"] {
                let path = repo.info().git_dir.join(marker);
                fs::write(&path, &untouched.0).unwrap();
                assert_eq!(
                    repo.pull_ff_only(None, &token())
                        .unwrap_err()
                        .pull_failure(),
                    Some(PullFailure::OperationInProgress)
                );
                fs::remove_file(path).unwrap();
            }
            let cancelled = token();
            cancelled.cancel();
            assert!(matches!(
                repo.pull_ff_only(None, &cancelled),
                Err(Error::Cancelled)
            ));
            assert!(!f.client.join(".git/FETCH_HEAD").exists());
            assert_eq!(f.run(&["rev-parse", "HEAD"]), untouched.0);
            let empty = Fixture::new();
            assert_eq!(
                empty
                    .repo()
                    .pull_ff_only(None, &token())
                    .unwrap_err()
                    .pull_failure(),
                Some(PullFailure::UnbornHead)
            );
            let bare = f.f.git.open(&f.bare, &token()).unwrap();
            assert!(matches!(
                bare.pull_ff_only(None, &token()),
                Err(Error::WorktreeRequired)
            ));
        },
    );
}

#[test]
fn missing_ambiguous_excluded_and_symbolic_upstreams_are_rejected_before_fetch() {
    isolated(
        "missing_ambiguous_excluded_and_symbolic_upstreams_are_rejected_before_fetch",
        || {
            let f = PullRepos::new("sha1");
            let config = fs::read(f.client.join(".git/config")).unwrap();
            let cases: &[(&[&str], PullFailure)] = &[
                (
                    &["config", "--unset", "branch.main.remote"],
                    PullFailure::MissingUpstream,
                ),
                (
                    &["config", "--unset", "branch.main.merge"],
                    PullFailure::MissingUpstream,
                ),
                (
                    &["config", "--add", "branch.main.merge", "refs/heads/other"],
                    PullFailure::UnsupportedUpstream,
                ),
                (
                    &["config", "--add", "branch.main.remote", "origin"],
                    PullFailure::UnsupportedUpstream,
                ),
                (
                    &["config", "branch.main.remote", "."],
                    PullFailure::UnsupportedUpstream,
                ),
                (
                    &["config", "branch.main.merge", "refs/tags/main"],
                    PullFailure::UnsupportedUpstream,
                ),
                (
                    &["config", "--add", "remote.origin.fetch", "^refs/heads/main"],
                    PullFailure::UnsupportedUpstream,
                ),
                (
                    &[
                        "config",
                        "remote.origin.fetch",
                        "+refs/heads/other:refs/remotes/origin/other",
                    ],
                    PullFailure::UnsupportedUpstream,
                ),
            ];
            for (arguments, expected) in cases {
                fs::write(f.client.join(".git/config"), &config).unwrap();
                f.run(arguments);
                assert_eq!(
                    f.repo()
                        .pull_ff_only(None, &token())
                        .unwrap_err()
                        .pull_failure(),
                    Some(*expected),
                    "{arguments:?}"
                );
                assert!(!f.client.join(".git/FETCH_HEAD").exists());
            }
            fs::write(f.client.join(".git/config"), config).unwrap();
            f.run(&[
                "symbolic-ref",
                "refs/remotes/origin/main",
                "refs/heads/main",
            ]);
            let before = f.snapshot();
            assert_eq!(
                f.repo()
                    .pull_ff_only(None, &token())
                    .unwrap_err()
                    .pull_failure(),
                Some(PullFailure::UnsupportedUpstream)
            );
            assert_eq!(f.snapshot(), before);
            assert!(!f.client.join(".git/FETCH_HEAD").exists());
        },
    );
}

#[test]
fn divergence_keeps_local_history_and_reports_successful_fetch_separately() {
    isolated(
        "divergence_keeps_local_history_and_reports_successful_fetch_separately",
        || {
            let f = PullRepos::new("sha1");
            f.run(&["commit", "--allow-empty", "-m", "local divergence"]);
            f.publish();
            let before = f.snapshot();
            let (repo, log) = f.logged(CommandLimits::default());
            let report = log
                .request("pull", |_| repo.pull_ff_only(None, &token()))
                .unwrap();
            assert!(report.fetch.is_ok());
            let error = report.update.unwrap().unwrap_err();
            assert_eq!(error.pull_failure(), Some(PullFailure::Diverged));
            assert!(error.to_string().contains("explicit merge or rebase"));
            assert_eq!(f.snapshot(), before);
            assert_eq!(report.status.unwrap().ahead_behind, Some((1, 1)));
            assert!(report.state.unwrap().is_idle());
            assert!(report.branches.is_ok());
            let logs = fs::read_to_string(log.directory().join("rocket.jsonl")).unwrap();
            assert!(logs.contains("\"fetch_outcome\":\"succeeded\",\"update_outcome\":\"failed\""));
            assert!(logs.contains("\"failure_code\":\"pull_diverged\""));
            assert!(!logs.contains("PRIVATE"));
            assert!(!logs.contains(f.client.to_str().unwrap()));
            assert_eq!(log.health().dropped_events, 0);
        },
    );
}

#[test]
fn deleted_upstream_never_uses_a_stale_tracking_ref() {
    isolated("deleted_upstream_never_uses_a_stale_tracking_ref", || {
        let f = PullRepos::new("sha1");
        f.publish();
        f.repo()
            .fetch("origin", None, &token())
            .unwrap()
            .result
            .unwrap();
        f.f.run_at(&f.bare, &["update-ref", "-d", "refs/heads/main"]);
        let before = f.snapshot();
        let report = f.repo().pull_ff_only(None, &token()).unwrap();
        assert!(report.fetch.is_ok());
        assert_eq!(
            report.update.unwrap().unwrap_err().pull_failure(),
            Some(PullFailure::UpstreamNotFetched)
        );
        assert_eq!(f.snapshot(), before);
        assert_eq!(report.status.unwrap().ahead_behind, Some((0, 1)));
    });
}

#[test]
fn first_fetch_can_create_the_missing_tracking_ref() {
    isolated("first_fetch_can_create_the_missing_tracking_ref", || {
        let f = PullRepos::new("sha1");
        f.run(&["update-ref", "-d", "refs/remotes/origin/main"]);
        f.publish();
        confirmed(
            &f.repo().pull_ff_only(None, &token()).unwrap(),
            PullUpdateKind::FastForward,
        );
    });
}

#[test]
fn malformed_fetch_output_preserves_fetch_success_but_skips_the_branch_update() {
    isolated(
        "malformed_fetch_output_preserves_fetch_success_but_skips_the_branch_update",
        || {
            let f = PullRepos::new("sha1");
            f.publish();
            fetch_hook(&f, "committed", "echo PRIVATE-corrupt-porcelain");
            let before = f.snapshot();
            let log = DiagnosticLog::open(f.f.root.join("logs")).unwrap();
            let repo = GitClient::new_with_log(
                git_with_corrupted_fetch_stdout(&f),
                CommandLimits::default(),
                log.clone(),
            )
            .unwrap()
            .open(&f.client, &token())
            .unwrap();
            let report = repo.pull_ff_only(None, &token()).unwrap();
            let fetched = report.fetch.unwrap();
            assert!(
                String::from_utf8_lossy(&fetched.output.stderr)
                    .contains("PRIVATE-corrupt-porcelain")
            );
            assert!(matches!(fetched.updates, Err(Error::InvalidOutput { .. })));
            assert!(report.update.is_none());
            assert_eq!(f.snapshot(), before);
            assert_eq!(report.status.unwrap().ahead_behind, Some((0, 1)));
            assert!(report.branches.is_ok() && report.state.is_ok());
            let logs = fs::read_to_string(log.directory().join("rocket.jsonl")).unwrap();
            assert!(logs.lines().any(|l| l.contains("\"event\":\"parse\"")
                && l.contains("\"outcome\":\"invalid_output\"")));
            assert!(
                logs.contains("\"fetch_outcome\":\"succeeded\",\"update_outcome\":\"skipped\"")
            );
            assert!(!logs.contains("PRIVATE"));
        },
    );
}

#[test]
fn incoming_paths_do_not_overwrite_untracked_or_ignored_files() {
    isolated(
        "incoming_paths_do_not_overwrite_untracked_or_ignored_files",
        || {
            for ignored in [false, true] {
                let f = PullRepos::new("sha1");
                fs::write(f.client.join("collision"), b"PRIVATE local file\n").unwrap();
                if ignored {
                    fs::create_dir_all(f.client.join(".git/info")).unwrap();
                    fs::write(f.client.join(".git/info/exclude"), b"collision\n").unwrap();
                }
                f.f.write("collision", b"incoming\n");
                f.publish();
                let before = f.snapshot();
                let report = f.repo().pull_ff_only(None, &token()).unwrap();
                assert!(report.fetch.is_ok());
                assert_eq!(
                    report.update.unwrap().unwrap_err().pull_failure(),
                    Some(PullFailure::UpdateFailed)
                );
                assert_eq!(f.snapshot(), before);
                assert_eq!(
                    fs::read(f.client.join("collision")).unwrap(),
                    b"PRIVATE local file\n"
                );
                assert!(report.state.unwrap().is_idle());
            }
        },
    );
}

#[test]
fn fetch_failure_keeps_authentication_guidance_and_skips_branch_update() {
    isolated(
        "fetch_failure_keeps_authentication_guidance_and_skips_branch_update",
        || {
            let f = PullRepos::new("sha1");
            f.publish();
            fetch_hook(
                &f,
                "prepared",
                "echo \"fatal: Authentication failed for 'PRIVATE-token'\" >&2\nexit 1",
            );
            let before = f.snapshot();
            let (repo, log) = f.logged(CommandLimits::default());
            let report = repo.pull_ff_only(None, &token()).unwrap();
            let error = report.fetch.unwrap_err();
            assert_eq!(
                error.fetch_failure(),
                Some(FetchFailure::AuthenticationRejected)
            );
            assert!(!error.to_string().contains("PRIVATE"));
            assert!(report.update.is_none());
            assert!(report.status.is_ok() && report.branches.is_ok() && report.state.is_ok());
            assert_eq!(f.snapshot(), before);
            let logs = fs::read_to_string(log.directory().join("rocket.jsonl")).unwrap();
            assert!(logs.contains("\"fetch_outcome\":\"failed\",\"update_outcome\":\"skipped\""));
            assert!(logs.contains("\"failure_code\":\"authentication_rejected\""));
            assert!(!logs.contains("PRIVATE"));
        },
    );
}

#[test]
fn index_lock_failure_preserves_the_existing_lock_and_fetched_refs() {
    isolated(
        "index_lock_failure_preserves_the_existing_lock_and_fetched_refs",
        || {
            let f = PullRepos::new("sha1");
            f.publish();
            let lock = f.client.join(".git/index.lock");
            fs::write(&lock, b"owned test lock\n").unwrap();
            let before = f.snapshot();
            let report = f.repo().pull_ff_only(None, &token()).unwrap();
            assert!(report.fetch.is_ok());
            let error = report.update.unwrap().unwrap_err();
            assert_eq!(error.pull_failure(), Some(PullFailure::UpdateFailed));
            assert!(!error.to_string().contains(f.client.to_str().unwrap()));
            assert_eq!(f.snapshot(), before);
            assert_eq!(fs::read(lock).unwrap(), b"owned test lock\n");
            assert_eq!(report.status.unwrap().ahead_behind, Some((0, 1)));
        },
    );
}

#[test]
fn fetch_side_effects_are_rechecked_before_touching_the_branch() {
    isolated(
        "fetch_side_effects_are_rechecked_before_touching_the_branch",
        || {
            for change in ["dirty", "upstream", "branch"] {
                let f = PullRepos::new("sha1");
                f.run(&["branch", "other"]);
                f.publish();
                let before = f.run(&["rev-parse", "HEAD"]);
                fetch_hook(
                    &f,
                    "committed",
                    match change {
                        "dirty" => "printf 'local edit during fetch\\n' > file",
                        "branch" => "git symbolic-ref HEAD refs/heads/other",
                        _ => "git config branch.main.merge refs/heads/changed",
                    },
                );
                let report = f.repo().pull_ff_only(None, &token()).unwrap();
                assert!(report.fetch.is_ok());
                assert_eq!(
                    report.update.unwrap().unwrap_err().pull_failure(),
                    Some(if change == "dirty" {
                        PullFailure::DirtyWorktree
                    } else {
                        PullFailure::StateChanged
                    })
                );
                assert_eq!(f.run(&["rev-parse", "HEAD"]), before);
                if change == "dirty" {
                    assert_eq!(
                        fs::read(f.client.join("file")).unwrap(),
                        b"local edit during fetch\n"
                    );
                }
            }
        },
    );
}

#[test]
fn update_uses_the_fetched_object_even_if_tracking_ref_is_changed_by_a_hook() {
    isolated(
        "update_uses_the_fetched_object_even_if_tracking_ref_is_changed_by_a_hook",
        || {
            let f = PullRepos::new("sha1");
            let base = String::from_utf8(f.run(&["rev-parse", "HEAD"])).unwrap();
            f.publish();
            let marker = f.f.root.join("once");
            fetch_hook(
                &f,
                "committed",
                &format!(
                    "if [ ! -f {0} ]; then\nprintf ready > {0}\ngit update-ref refs/remotes/origin/main {1}\nfi",
                    quote(&marker),
                    base.trim()
                ),
            );
            let report = f.repo().pull_ff_only(None, &token()).unwrap();
            confirmed(&report, PullUpdateKind::FastForward);
            assert_eq!(
                f.run(&["rev-parse", "HEAD"]),
                f.f.run(&["rev-parse", "HEAD"])
            );
            assert_eq!(
                f.run(&["rev-parse", "refs/remotes/origin/main"]),
                base.as_bytes()
            );
        },
    );
}

#[test]
fn cancelling_fetch_after_tracking_changes_never_starts_the_merge() {
    isolated(
        "cancelling_fetch_after_tracking_changes_never_starts_the_merge",
        || {
            let f = PullRepos::new("sha1");
            f.publish();
            let marker = f.f.root.join("fetch-ready");
            fetch_hook(
                &f,
                "committed",
                &format!("printf ready > {}\nsleep 30", quote(&marker)),
            );
            let before = f.snapshot();
            let repo = f.repo();
            let task = WriteTask::spawn(move |cancel| repo.pull_ff_only(None, &cancel)).unwrap();
            wait_file(&marker);
            task.cancel();
            let report = task.wait().unwrap();
            assert!(matches!(report.fetch, Err(Error::Cancelled)));
            assert!(report.update.is_none());
            assert_eq!(f.snapshot(), before);
            assert_eq!(report.status.unwrap().ahead_behind, Some((0, 1)));
            assert!(report.branches.is_ok() && report.state.is_ok());
        },
    );
}

#[test]
fn cancelling_after_fast_forward_retains_changed_head_and_holds_the_shared_gate() {
    isolated(
        "cancelling_after_fast_forward_retains_changed_head_and_holds_the_shared_gate",
        || {
            let f = PullRepos::new("sha1");
            let linked = f.f.root.join("linked");
            f.run(&["worktree", "add", "-b", "linked", linked.to_str().unwrap()]);
            fs::write(linked.join("local"), b"keep\n").unwrap();
            let linked_repo = f.f.git.open(&linked, &token()).unwrap();
            let index_path = linked_repo.info().git_dir.join("index");
            let index = fs::read(&index_path).unwrap();
            f.publish();
            let marker = f.f.root.join("merge-ready");
            let hook = f.f.root.join("no-hooks/post-merge");
            script(
                &hook,
                &format!("printf ready > {}\nsleep 30", quote(&marker)),
            );
            let (repo, log) = f.logged(CommandLimits::default());
            let task = WriteTask::spawn(move |cancel| repo.pull_ff_only(None, &cancel)).unwrap();
            wait_file(&marker);
            assert_eq!(
                f.run(&["rev-parse", "HEAD"]),
                f.f.run(&["rev-parse", "HEAD"])
            );
            let (started, ready) = sync_channel(0);
            let mut stage = WriteTask::spawn(move |cancel| {
                started.send(()).unwrap();
                linked_repo.stage_all(&cancel)
            })
            .unwrap();
            ready.recv_timeout(Duration::from_secs(5)).unwrap();
            let deadline = Instant::now() + Duration::from_millis(500);
            while Instant::now() < deadline {
                assert!(stage.try_finish().is_none());
                thread::sleep(Duration::from_millis(10));
            }
            stage.cancel();
            assert!(matches!(stage.wait(), Err(Error::Cancelled)));
            assert_eq!(fs::read(index_path).unwrap(), index);
            task.cancel();
            let report = task.wait().unwrap();
            assert!(report.fetch.is_ok());
            assert!(matches!(report.update, Some(Err(Error::Cancelled))));
            assert_eq!(report.status.unwrap().ahead_behind, Some((0, 0)));
            assert!(report.branches.is_ok() && report.state.unwrap().is_idle());
            let logs = fs::read_to_string(log.directory().join("rocket.jsonl")).unwrap();
            assert_eq!(
                logs.lines()
                    .filter(|l| l.contains("\"event\":\"git_start\"")
                        && l.contains("\"operation\":\"merge\""))
                    .count(),
                1
            );
            fs::remove_file(hook).unwrap();
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
fn fetch_timeout_preserves_state_and_leaves_transaction_locks_alone() {
    isolated(
        "fetch_timeout_preserves_state_and_leaves_transaction_locks_alone",
        || {
            let f = PullRepos::new("sha1");
            f.publish();
            fetch_hook(&f, "prepared", "sleep 30");
            let (repo, _) = f.logged(CommandLimits {
                timeout: Duration::from_secs(3),
                ..CommandLimits::default()
            });
            let before = f.snapshot();
            let report = repo.pull_ff_only(None, &token()).unwrap();
            assert!(matches!(report.fetch, Err(Error::TimedOut)));
            assert!(report.update.is_none());
            assert_eq!(f.snapshot(), before);
            assert!(report.status.is_ok() && report.branches.is_ok() && report.state.is_ok());
            fs::remove_file(f.f.root.join("no-hooks/reference-transaction")).unwrap();
            let lock = f.client.join(".git/refs/remotes/origin/main.lock");
            let lock_bytes = fs::read(&lock).unwrap();
            // An explicit second request must report the retained lock, not
            // silently delete it or proceed to updating the local branch.
            let report = repo.pull_ff_only(None, &token()).unwrap();
            assert!(matches!(report.fetch, Err(Error::Git { .. })));
            assert!(report.update.is_none());
            assert_eq!(fs::read(lock).unwrap(), lock_bytes);
            assert_eq!(f.snapshot(), before);
        },
    );
}

#[test]
fn merge_output_limit_keeps_completed_fast_forward_and_refreshes() {
    isolated(
        "merge_output_limit_keeps_completed_fast_forward_and_refreshes",
        || {
            let f = PullRepos::new("sha1");
            f.publish();
            script(
                &f.f.root.join("no-hooks/post-merge"),
                "i=0\nwhile [ \"$i\" -lt 4096 ]; do printf 'PRIVATE-output-flood\\n' >&2; i=$((i + 1)); done\nsleep 30",
            );
            let (repo, _) = f.logged(CommandLimits {
                stderr_bytes: 2048,
                ..CommandLimits::default()
            });
            let report = repo.pull_ff_only(None, &token()).unwrap();
            assert!(report.fetch.is_ok(), "{:?}", report.fetch);
            assert!(matches!(
                report.update,
                Some(Err(Error::OutputLimit {
                    stream: "stderr",
                    ..
                }))
            ));
            assert_eq!(report.status.unwrap().ahead_behind, Some((0, 0)));
            assert!(report.branches.is_ok() && report.state.is_ok());
        },
    );
}

#[test]
fn successful_merge_with_hook_error_is_a_visible_warning_not_a_retry() {
    isolated(
        "successful_merge_with_hook_error_is_a_visible_warning_not_a_retry",
        || {
            let f = PullRepos::new("sha1");
            f.publish();
            script(
                &f.f.root.join("no-hooks/post-merge"),
                "echo 'error: PRIVATE-hook-failed' >&2\nexit 1",
            );
            let (repo, log) = f.logged(CommandLimits::default());
            let report = repo.pull_ff_only(None, &token()).unwrap();
            confirmed(&report, PullUpdateKind::FastForward);
            let update = report.update.unwrap().unwrap();
            assert!(update.error_diagnostics);
            assert!(String::from_utf8_lossy(&update.output.stderr).contains("PRIVATE-hook-failed"));
            let logs = fs::read_to_string(log.directory().join("rocket.jsonl")).unwrap();
            assert!(
                logs.lines()
                    .any(|l| l.contains("\"event\":\"pull_outcome\"")
                        && l.contains("\"error_diagnostics\":true")
                        && l.contains("\"level\":\"warn\""))
            );
            assert!(!logs.contains("PRIVATE"));
            assert_eq!(
                logs.lines()
                    .filter(|l| l.contains("\"event\":\"git_start\"")
                        && l.contains("\"operation\":\"merge\""))
                    .count(),
                1
            );
        },
    );
}

#[test]
fn successful_update_is_not_replaced_by_verification_or_refresh_failures() {
    isolated(
        "successful_update_is_not_replaced_by_verification_or_refresh_failures",
        || {
            for move_head in [false, true] {
                let f = PullRepos::new("sha1");
                let base = String::from_utf8(f.run(&["rev-parse", "HEAD"])).unwrap();
                f.publish();
                let body = if move_head {
                    format!("git update-ref refs/heads/main {}", base.trim())
                } else {
                    "i=0\nwhile [ \"$i\" -lt 100 ]; do printf x > PRIVATE-long-untracked-file-name-for-status-limit-$i; i=$((i + 1)); done".into()
                };
                script(&f.f.root.join("no-hooks/post-merge"), &body);
                let (repo, _) = f.logged(CommandLimits {
                    stdout_bytes: 2048,
                    ..CommandLimits::default()
                });
                let report = repo.pull_ff_only(None, &token()).unwrap();
                assert!(report.fetch.is_ok());
                let update = report.update.unwrap().unwrap();
                assert_eq!(update.kind, PullUpdateKind::FastForward);
                if move_head {
                    assert_eq!(
                        update.verification.unwrap_err().pull_failure(),
                        Some(PullFailure::VerificationFailed)
                    );
                    assert_eq!(f.run(&["rev-parse", "HEAD"]), base.as_bytes());
                } else {
                    assert!(update.verification.is_ok());
                    assert!(matches!(
                        report.status,
                        Err(Error::OutputLimit {
                            stream: "stdout",
                            ..
                        })
                    ));
                    assert_eq!(
                        f.run(&["rev-parse", "HEAD"]),
                        f.f.run(&["rev-parse", "HEAD"])
                    );
                }
                assert!(report.branches.is_ok() && report.state.is_ok());
            }
        },
    );
}
