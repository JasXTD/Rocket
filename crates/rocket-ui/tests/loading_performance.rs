#![cfg(windows)]
#[path = "../../rocket-git/tests/common/mod.rs"]
mod common;
use common::Fixture;
use rocket_git::*;
use rocket_ui::backend::Backend;
use std::{fs, sync::Arc};

fn commands_since(logs: &std::path::Path, before: usize) -> usize {
    fs::read_to_string(logs.join("rocket.jsonl")).unwrap()[before..]
        .lines()
        .filter(|line| line.contains("\"event\":\"git_start\""))
        .count()
}

#[test]
fn watched_loading_reuses_config_and_retains_every_snapshot_section() {
    common::isolated(
        "ROCKET_LOADING_UI_CASE",
        "watched_loading_reuses_config_and_retains_every_snapshot_section",
        || {
            let f = Fixture::new();
            f.write("a", "base\n");
            f.commit("base");
            let include = f.root.join("included.config");
            fs::write(&include, b"[remote \"Work.foo\"]\n url = ../first\n url = ../second\n pushurl = \"push\\nline\"\n fetch = +refs/heads/*:refs/remotes/Work.foo/*\n[remote]\n url = ignored\n").unwrap();
            f.run(&["config", "include.path", include.to_str().unwrap()]);
            f.run(&["config", "extensions.worktreeConfig", "true"]);
            f.run(&[
                "config",
                "--worktree",
                "--add",
                "remote.Work.foo.url",
                "../worktree",
            ]);
            let cancel = CancellationToken::new();
            let expected = f.repo().remotes(&cancel).unwrap();
            let logs = f.root.join("loading-logs");
            let backend = Backend::discover(&logs).unwrap();
            let before = fs::metadata(logs.join("rocket.jsonl")).unwrap().len() as usize;
            let snap = backend
                .snapshot_observed(
                    &f.repository,
                    HistoryStart::Head,
                    &cancel,
                    Some(Arc::new(|| {})),
                    false,
                )
                .unwrap();
            assert!(snap.warnings.is_empty(), "{:?}", snap.warnings);
            assert!(snap.watcher.is_some());
            assert_eq!(
                commands_since(&logs, before),
                10,
                "the guard and remote list should share one config read"
            );
            assert_eq!(snap.remotes, expected);
            assert_eq!(snap.remotes[0].urls.len(), 3);
            assert!(snap.status.is_some());
            assert!(snap.head.is_some());
            assert_eq!(snap.branches.len(), 1);
            assert_eq!(snap.history.as_ref().unwrap().commits.len(), 1);
            assert!(snap.state.as_ref().unwrap().is_idle());
            assert!(snap.stashes.is_empty());

            f.run(&["config", "remote.new.url", "../fresh"]);
            let fresh = backend
                .snapshot_observed(
                    &f.repository,
                    HistoryStart::Head,
                    &cancel,
                    Some(Arc::new(|| {})),
                    false,
                )
                .unwrap();
            assert!(fresh.warnings.is_empty(), "{:?}", fresh.warnings);
            assert_eq!(fresh.remotes, f.repo().remotes(&cancel).unwrap());
            assert!(fresh.remotes.iter().any(|remote| remote.name == b"new"));
            assert!(!snap.remotes.iter().any(|remote| remote.name == b"new"));
        },
    );
}

#[test]
fn welcome_heads_reuse_git_discovery_but_read_current_state() {
    common::isolated(
        "ROCKET_LOADING_UI_CASE",
        "welcome_heads_reuse_git_discovery_but_read_current_state",
        || {
            let f = Fixture::new();
            f.write("a", "base\n");
            f.commit("base");
            let logs = f.root.join("welcome-logs");
            let backend = Backend::discover(&logs).unwrap();
            let cancel = CancellationToken::new();
            let before = fs::metadata(logs.join("rocket.jsonl")).unwrap().len() as usize;
            assert!(
                matches!(backend.head(&f.repository, &cancel).unwrap(), Head::Branch { name, .. } if name == b"main")
            );
            assert_eq!(
                commands_since(&logs, before),
                2,
                "one repository discovery and one HEAD read"
            );
            f.run(&["switch", "-c", "fresh"]);
            assert!(
                matches!(backend.head(&f.repository, &cancel).unwrap(), Head::Branch { name, .. } if name == b"fresh")
            );
            cancel.cancel();
            assert!(matches!(
                backend.head(&f.repository, &cancel),
                Err(Error::Cancelled)
            ));
        },
    );
}
