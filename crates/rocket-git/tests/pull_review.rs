#[allow(dead_code)]
mod common;
use common::Fixture;
use rocket_git::*;
use std::{
    fs,
    path::PathBuf,
    thread,
    time::{Duration, Instant},
};
fn token() -> CancellationToken {
    CancellationToken::new()
}
fn isolated(name: &str, test: fn()) {
    common::isolated("ROCKET_PULL_REVIEW_CASE", name, test)
}
struct Repos {
    f: Fixture,
    client: PathBuf,
    bare: PathBuf,
}
impl Repos {
    fn new(format: &str) -> Self {
        let f = Fixture::with_format(format);
        f.write("検索 🚀.txt", b"base\n");
        f.commit("PRIVATE base");
        let bare = f.root.join("PRIVATE remote 文.git");
        let client = f.root.join("PRIVATE client 文");
        f.run(&[
            "clone",
            "--bare",
            f.repository.to_str().unwrap(),
            bare.to_str().unwrap(),
        ]);
        f.run(&["clone", bare.to_str().unwrap(), client.to_str().unwrap()]);
        for (k, v) in [
            ("user.name", "Fixture"),
            ("user.email", "fixture@example.invalid"),
            ("core.autocrlf", "false"),
            ("commit.gpgsign", "false"),
        ] {
            f.run_at(&client, &["config", k, v]);
        }
        f.run_at(
            &client,
            &[
                "config",
                "core.hooksPath",
                f.root.join("no-hooks").to_str().unwrap(),
            ],
        );
        Self { f, client, bare }
    }
    fn git(&self, a: &[&str]) -> String {
        String::from_utf8(self.f.run_at(&self.client, a))
            .unwrap()
            .trim()
            .into()
    }
    fn repo(&self) -> Repository {
        self.f.git.open(&self.client, &token()).unwrap()
    }
    fn publish(&self, name: &str, contents: &str) {
        self.f.write(name, contents);
        self.f.commit("PRIVATE incoming");
        self.f.run(&["push", self.bare.to_str().unwrap(), "main"]);
    }
    fn local(&self, conflict: bool) {
        fs::write(
            self.client.join(if conflict {
                "検索 🚀.txt"
            } else {
                "local.txt"
            }),
            "PRIVATE local\n",
        )
        .unwrap();
        self.git(&["add", "."]);
        self.git(&["commit", "-m", "PRIVATE local"]);
    }
    fn fetch(&self) -> PullFetched {
        let report = self.repo().fetch_for_pull(None, &token()).unwrap();
        assert!(report.fetch.is_ok(), "{:?}", report.fetch);
        assert!(report.status.is_ok());
        assert!(report.branches.is_ok());
        assert!(report.state.unwrap().is_idle());
        report.prepared.unwrap().unwrap()
    }
    fn snapshot(&self) -> (String, Vec<u8>, Vec<u8>) {
        (
            self.git(&["rev-parse", "HEAD"]),
            fs::read(self.client.join(".git/index")).unwrap(),
            fs::read(self.client.join("検索 🚀.txt")).unwrap(),
        )
    }
}
fn successful(report: PullApplyReport) {
    match report {
        PullApplyReport::Merge(r) => {
            assert!(r.write.result.is_ok(), "{r:?}");
            assert!(r.write.status.is_ok());
            assert!(r.state.unwrap().is_idle());
            assert!(r.branches.is_ok());
        }
        PullApplyReport::Rebase(r) => {
            assert!(r.write.result.is_ok(), "{r:?}");
            assert!(r.write.status.is_ok());
            assert!(r.state.unwrap().is_idle());
            assert!(r.branches.is_ok());
            assert!(r.journal_cleanup.is_ok());
        }
    }
}
#[test]
fn pinned_fetch_review_and_all_strategies_preserve_files() {
    isolated(
        "pinned_fetch_review_and_all_strategies_preserve_files",
        || {
            for format in ["sha1", "sha256"] {
                for strategy in [
                    PullStrategy::FastForward,
                    PullStrategy::Merge,
                    PullStrategy::Rebase,
                ] {
                    let f = Repos::new(format);
                    f.publish("検索 🚀.txt", "incoming\n");
                    fs::write(f.client.join("keep untracked"), "keep").unwrap();
                    let before = f.snapshot();
                    let ticket = f.fetch();
                    assert_eq!(before, f.snapshot());
                    let repo = f.repo();
                    for s in [
                        PullStrategy::FastForward,
                        PullStrategy::Merge,
                        PullStrategy::Rebase,
                    ] {
                        let p = repo.preview_pull(&ticket, s, &token()).unwrap();
                        assert!(p.can_apply());
                        assert_eq!(p.incoming(), 1);
                        assert_eq!(p.current_only(), 0);
                    }
                    // The remote is unavailable now. Reviewing/changing strategy/applying is entirely local.
                    fs::rename(&f.bare, f.bare.with_extension("offline")).unwrap();
                    let p = repo.preview_pull(&ticket, strategy, &token()).unwrap();
                    successful(repo.apply_pull(&p, &token()).unwrap());
                    assert_eq!(f.git(&["rev-parse", "HEAD"]), ticket.target().as_str());
                    assert_eq!(fs::read(f.client.join("keep untracked")).unwrap(), b"keep");
                }
            }
        },
    )
}
#[test]
fn divergence_requires_choice_merge_preserves_parents_rebase_retains_original() {
    isolated(
        "divergence_requires_choice_merge_preserves_parents_rebase_retains_original",
        || {
            for strategy in [PullStrategy::Merge, PullStrategy::Rebase] {
                let f = Repos::new("sha1");
                f.local(false);
                f.publish("検索 🚀.txt", "incoming\n");
                let before = f.snapshot();
                let ticket = f.fetch();
                let repo = f.repo();
                let ff = repo
                    .preview_pull(&ticket, PullStrategy::FastForward, &token())
                    .unwrap();
                assert!(!ff.can_apply());
                assert!(matches!(
                    repo.apply_pull(&ff, &token()),
                    Err(Error::Merge(MergeFailure::NotFastForward))
                ));
                assert_eq!(before, f.snapshot());
                let p = repo.preview_pull(&ticket, strategy, &token()).unwrap();
                assert_eq!(p.relation(), MergeRelation::Diverged);
                assert_eq!((p.incoming(), p.current_only()), (1, 1));
                successful(repo.apply_pull(&p, &token()).unwrap());
                if strategy == PullStrategy::Merge {
                    assert_eq!(f.git(&["rev-parse", "HEAD^1"]), before.0);
                    assert_eq!(f.git(&["rev-parse", "HEAD^2"]), ticket.target().as_str());
                } else {
                    assert_eq!(f.git(&["rev-parse", "HEAD^"]), ticket.target().as_str());
                    assert_eq!(
                        f.git(&["rev-parse", &p.recovery_reference().unwrap()]),
                        before.0
                    );
                }
                assert_eq!(
                    fs::read(f.client.join("local.txt")).unwrap(),
                    b"PRIVATE local\n"
                );
                let ticket = f.fetch();
                let p = repo.preview_pull(&ticket, strategy, &token()).unwrap();
                assert_eq!(p.relation(), MergeRelation::UpToDate);
                let current = f.snapshot();
                successful(repo.apply_pull(&p, &token()).unwrap());
                assert_eq!(current.0, f.snapshot().0);
            }
        },
    )
}

#[test]
fn pull_configuration_reads_unicode_names_and_includes_at_every_checkpoint() {
    isolated("pull_configuration_reads_unicode_names_and_includes_at_every_checkpoint", || {
        let f = Repos::new("sha1");
        f.publish("検索 🚀.txt", "incoming\n");
        let branch = "Topic.Fix+(文)";
        f.git(&["branch", "-m", branch]);
        f.git(&["remote", "rename", "origin", "Origin.Team"]);
        let include = f.f.root.join("PRIVATE included config");
        fs::write(&include, "").unwrap();
        for (key, value) in [
            (format!("branch.{branch}.remote"), "Origin.Team"),
            (format!("branch.{branch}.merge"), "refs/heads/main"),
            ("remote.Origin.Team.url".into(), f.bare.to_str().unwrap()),
        ] {
            f.git(&["config", "--unset-all", &key]);
            f.git(&["config", "--file", include.to_str().unwrap(), &key, value]);
        }
        f.git(&["config", "include.path", include.to_str().unwrap()]);
        // Similar case/dotted names are separate, not a regex interpolation.
        f.git(&["config", "branch.topic.Fix+(文).remote", "not-selected"]);
        let repo = f.repo();
        let ticket = f.fetch();
        let preview = repo.preview_pull(&ticket, PullStrategy::Merge, &token()).unwrap();
        assert_eq!(preview.fetched().upstream().branch, branch.as_bytes());
        assert_eq!(preview.fetched().upstream().remote, b"Origin.Team");
        let config = fs::read(f.client.join(".git/config")).unwrap();
        f.git(&["config", "--file", include.to_str().unwrap(), "remote.Origin.Team.url", f.f.repository.to_str().unwrap()]);
        let before = f.snapshot();
        assert!(matches!(repo.preview_pull(&ticket, PullStrategy::Merge, &token()), Err(Error::Pull(PullFailure::StateChanged))));
        assert!(matches!(repo.apply_pull(&preview, &token()), Err(Error::Pull(PullFailure::StateChanged))));
        assert_eq!(before, f.snapshot());
        assert_eq!(config, fs::read(f.client.join(".git/config")).unwrap());
    });
}

#[test]
fn pull_preview_uses_two_fresh_checkpoints_with_bounded_git_launches() {
    isolated("pull_preview_uses_two_fresh_checkpoints_with_bounded_git_launches", || {
        let f = Repos::new("sha1");
        f.publish("検索 🚀.txt", "incoming\n");
        let ticket = f.fetch();
        let log = DiagnosticLog::open(f.f.root.join("diagnostics")).unwrap();
        let repo = f.repo().with_log(log.clone());
        let before = f.snapshot();
        let preview = repo.preview_pull(&ticket, PullStrategy::FastForward, &token()).unwrap();
        assert_eq!(preview.relation(), MergeRelation::FastForward);
        assert_eq!(before, f.snapshot());
        let text = fs::read_to_string(log.directory().join("rocket.jsonl")).unwrap();
        let commands: Vec<_> = text.lines().filter(|s| s.contains("\"event\":\"git_end\"")).collect();
        assert!(commands.len() <= 32, "pull preview launched {} commands", commands.len());
        for operation in ["status", "ls-files", "config"] {
            let needle = format!("\"operation\":\"{operation}\"");
            assert_eq!(commands.iter().filter(|s| s.contains(&needle)).count(), 2, "fresh before/after {operation}");
        }
        // An external index edit is still rejected by the independent apply guard.
        f.git(&["update-index", "--skip-worktree", "検索 🚀.txt"]);
        let changed = f.snapshot();
        assert!(repo.apply_pull(&preview, &token()).is_err());
        assert_eq!(changed, f.snapshot());
    });
}

#[test]
fn ambiguous_upstream_and_existing_index_lock_stop_before_pull_fetch() {
    isolated("ambiguous_upstream_and_existing_index_lock_stop_before_pull_fetch", || {
        for change in ["remote", "merge", "lock"] {
            let f = Repos::new("sha1");
            f.publish("検索 🚀.txt", "incoming\n");
            if change == "lock" {
                fs::write(f.client.join(".git/index.lock"), "keep existing lock").unwrap();
            } else {
                let value = if change == "remote" { "origin" } else { "refs/heads/main" };
                f.git(&["config", "--add", &format!("branch.main.{change}"), value]);
            }
            let before = f.snapshot();
            let tracking = f.git(&["rev-parse", "refs/remotes/origin/main"]);
            assert!(f.repo().fetch_for_pull(None, &token()).is_err(), "{change}");
            assert_eq!(before, f.snapshot());
            assert_eq!(tracking, f.git(&["rev-parse", "refs/remotes/origin/main"]));
            if change == "lock" { assert_eq!(fs::read(f.client.join(".git/index.lock")).unwrap(), b"keep existing lock"); }
        }
    });
}
#[test]
fn stale_tickets_dirty_hidden_changes_and_cancel_refuse_integration() {
    isolated(
        "stale_tickets_dirty_hidden_changes_and_cancel_refuse_integration",
        || {
            for change in ["tracking", "config", "head", "dirty", "hidden", "cancel"] {
                let f = Repos::new("sha1");
                f.publish("検索 🚀.txt", "incoming\n");
                let ticket = f.fetch();
                let repo = f.repo();
                let p = repo
                    .preview_pull(&ticket, PullStrategy::Merge, &token())
                    .unwrap();
                match change {
                    "tracking" => {
                        f.git(&[
                            "update-ref",
                            "refs/remotes/origin/main",
                            ticket.original().as_str(),
                        ]);
                    }
                    "config" => {
                        f.git(&[
                            "config",
                            "remote.origin.url",
                            f.f.repository.to_str().unwrap(),
                        ]);
                    }
                    "head" => {
                        f.git(&["commit", "--allow-empty", "-m", "external"]);
                    }
                    "dirty" => {
                        fs::write(f.client.join("検索 🚀.txt"), "keep external").unwrap();
                    }
                    "hidden" => {
                        f.git(&["update-index", "--assume-unchanged", "検索 🚀.txt"]);
                        fs::write(f.client.join("検索 🚀.txt"), "keep hidden").unwrap();
                    }
                    _ => (),
                }
                let before = f.snapshot();
                let cancel = token();
                if change == "cancel" {
                    cancel.cancel();
                }
                assert!(repo.apply_pull(&p, &cancel).is_err(), "{change}");
                assert_eq!(before, f.snapshot(), "{change}");
                if matches!(change, "dirty" | "hidden") {
                    assert!(repo.fetch_for_pull(None, &token()).is_err());
                    assert_eq!(before, f.snapshot());
                }
            }
        },
    )
}
#[test]
fn ignored_and_untracked_collisions_survive_all_strategies() {
    isolated(
        "ignored_and_untracked_collisions_survive_all_strategies",
        || {
            for ignored in [false, true] {
                for strategy in [
                    PullStrategy::FastForward,
                    PullStrategy::Merge,
                    PullStrategy::Rebase,
                ] {
                    let f = Repos::new("sha1");
                    f.publish("collision 文.txt", "incoming");
                    let ticket = f.fetch();
                    if ignored {
                        fs::create_dir_all(f.client.join(".git/info")).unwrap();
                        fs::write(f.client.join(".git/info/exclude"), "collision 文.txt\n")
                            .unwrap();
                    }
                    fs::write(f.client.join("collision 文.txt"), "keep PRIVATE").unwrap();
                    let original = f.git(&["rev-parse", "HEAD"]);
                    let repo = f.repo();
                    match repo.preview_pull(&ticket, strategy, &token()) {
                        Ok(p) => match repo.apply_pull(&p, &token()) {
                            Ok(PullApplyReport::Merge(r)) => assert!(r.write.result.is_err()),
                            Ok(PullApplyReport::Rebase(r)) => assert!(r.write.result.is_err()),
                            Err(_) => (),
                        },
                        Err(e) => assert!(matches!(e, Error::Rebase(_))),
                    }
                    assert_eq!(
                        fs::read(f.client.join("collision 文.txt")).unwrap(),
                        b"keep PRIVATE"
                    );
                    assert_eq!(f.git(&["rev-parse", "HEAD"]), original);
                }
            }
        },
    )
}
#[test]
fn pull_conflicts_continue_skip_abort_with_original_recovery() {
    isolated(
        "pull_conflicts_continue_skip_abort_with_original_recovery",
        || {
            for strategy in [PullStrategy::Merge, PullStrategy::Rebase] {
                for finish in ["continue", "abort", "skip"] {
                    if strategy == PullStrategy::Merge && finish == "skip" {
                        continue;
                    }
                    let f = Repos::new("sha1");
                    f.local(true);
                    f.publish("検索 🚀.txt", "incoming\n");
                    let original = f.git(&["rev-parse", "HEAD"]);
                    let ticket = f.fetch();
                    let repo = f.repo();
                    let p = repo.preview_pull(&ticket, strategy, &token()).unwrap();
                    match repo.apply_pull(&p, &token()).unwrap() {
                        PullApplyReport::Merge(r) => {
                            assert_eq!(r.failure, Some(MergeFailure::Conflicts));
                            assert!(!r.state.unwrap().is_idle());
                        }
                        PullApplyReport::Rebase(r) => {
                            assert!(r.write.result.is_err());
                            assert!(r.state.unwrap().rebase);
                        }
                    }
                    if finish == "continue" {
                        fs::write(f.client.join("検索 🚀.txt"), "resolved\n").unwrap();
                        f.git(&["add", "--", "検索 🚀.txt"]);
                    }
                    if strategy == PullStrategy::Merge {
                        let r = if finish == "abort" {
                            repo.abort_merge(&token())
                        } else {
                            repo.continue_merge("Resolved pull", &token())
                        }
                        .unwrap();
                        assert!(r.write.result.is_ok(), "{r:?}");
                    } else {
                        let action = match finish {
                            "abort" => RebaseAction::Abort,
                            "skip" => RebaseAction::Skip,
                            _ => RebaseAction::Continue,
                        };
                        let preview = repo.preview_rebase(action, "", &token()).unwrap();
                        let r = repo.apply_rebase(&preview, &token()).unwrap();
                        assert!(r.write.result.is_ok(), "{r:?}");
                        assert_eq!(
                            f.git(&["rev-parse", &p.recovery_reference().unwrap()]),
                            original
                        );
                    }
                    assert!(repo.operation_state(&token()).unwrap().is_idle());
                    if finish == "abort" {
                        assert_eq!(f.git(&["rev-parse", "HEAD"]), original);
                        assert_eq!(
                            fs::read(f.client.join("検索 🚀.txt")).unwrap(),
                            b"PRIVATE local\n"
                        );
                    }
                    if finish == "skip" {
                        assert_eq!(f.git(&["rev-parse", "HEAD"]), ticket.target().as_str());
                    }
                }
            }
        },
    )
}
fn hook(f: &Repos, name: &str, body: &str) {
    let p = f.f.root.join("no-hooks").join(name);
    fs::write(&p, format!("#!/bin/sh\n{body}\n")).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(p, fs::Permissions::from_mode(0o755)).unwrap();
    }
}
fn wait_file(path: &std::path::Path) {
    let start = Instant::now();
    while !path.exists() {
        assert!(start.elapsed() < Duration::from_secs(30));
        thread::sleep(Duration::from_millis(10));
    }
}
#[test]
fn live_fetch_and_integration_cancellation_refresh_without_retry() {
    isolated(
        "live_fetch_and_integration_cancellation_refresh_without_retry",
        || {
            for stage in ["fetch", "merge", "rebase"] {
                let f = Repos::new("sha1");
                if stage != "fetch" {
                    f.local(false);
                }
                f.publish("検索 🚀.txt", "incoming\n");
                let marker = f.f.root.join("started");
                let body = format!(
                    "printf started > '{}'\nsleep 20",
                    marker.to_str().unwrap().replace('\\', "/")
                );
                let original = f.snapshot();
                let repo = f.repo();
                if stage == "fetch" {
                    hook(
                        &f,
                        "reference-transaction",
                        &format!(
                            "if [ \"$1\" = 'prepared' ]; then\nwhile read old new ref; do\nif [ \"$ref\" = 'refs/remotes/origin/main' ]; then\n{body}\nfi\ndone\nfi"
                        ),
                    );
                    let task =
                        WriteTask::spawn(move |cancel| repo.fetch_for_pull(None, &cancel)).unwrap();
                    wait_file(&marker);
                    task.cancel();
                    let r = task.wait().unwrap();
                    assert!(r.fetch.is_err());
                    assert!(r.prepared.is_none());
                    assert!(r.status.is_ok());
                    assert!(r.branches.is_ok());
                    assert!(r.state.unwrap().is_idle());
                    assert_eq!(original, f.snapshot());
                } else {
                    let ticket = f.fetch();
                    let strategy = if stage == "merge" {
                        PullStrategy::Merge
                    } else {
                        PullStrategy::Rebase
                    };
                    let p = repo.preview_pull(&ticket, strategy, &token()).unwrap();
                    hook(
                        &f,
                        if stage == "merge" {
                            "pre-merge-commit"
                        } else {
                            "pre-rebase"
                        },
                        &body,
                    );
                    let task =
                        WriteTask::spawn(move |cancel| repo.apply_pull(&p, &cancel)).unwrap();
                    wait_file(&marker);
                    task.cancel();
                    match task.wait().unwrap() {
                        PullApplyReport::Merge(r) => {
                            assert!(r.write.result.is_err());
                            assert!(r.write.status.is_ok());
                            assert!(r.state.is_ok());
                        }
                        PullApplyReport::Rebase(r) => {
                            assert!(r.write.result.is_err());
                            assert!(r.write.status.is_ok());
                            assert!(r.state.is_ok());
                        }
                    }
                    assert_eq!(original.0, f.git(&["rev-parse", "HEAD"]));
                }
            }
        },
    )
}
