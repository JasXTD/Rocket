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

/// The value of one field of a diagnostic record, or "" when the record has none.
fn field<'a>(record: &'a str, name: &str) -> &'a str {
    record
        .split_once(&format!("\"{name}\":"))
        .and_then(|(_, rest)| rest.trim_start_matches('"').split(['"', ',', '}']).next())
        .unwrap_or("")
}

/// Times whole snapshots of the repository named by `ROCKET_LOAD_PROBE_REPOSITORY`, as
/// a refresh takes them, and lists each Git command of the last one: its whole time and
/// the part Git itself ran. Reads only; writes its log under the temporary folder.
#[test]
#[ignore = "manual timing probe; needs ROCKET_LOAD_PROBE_REPOSITORY"]
fn measure_large_repository_refresh() {
    let Some(root) = std::env::var_os("ROCKET_LOAD_PROBE_REPOSITORY").map(std::path::PathBuf::from) else {
        println!("ROCKET_LOAD_PROBE_REPOSITORY is not set");
        return;
    };
    let logs = std::env::temp_dir().join(format!("rocket-load-probe-{}", std::process::id()));
    let backend = Backend::discover(&logs).unwrap();
    let cancel = CancellationToken::new();
    let log = logs.join("rocket.jsonl");
    let written = || fs::metadata(&log).map_or(0, |m| m.len() as usize);
    let commands = |before: usize| {
        for record in fs::read_to_string(&log).unwrap()[before..]
            .lines()
            .filter(|record| record.contains("\"event\":\"git_end\""))
        {
            let part = |name: &str| field(record, name).parse::<u64>().unwrap_or(0) / 1000;
            println!(
                "  git {:<20} whole_ms={:<5} git_ran_ms={:<5} start_ms={:<4} output_bytes={}",
                field(record, "operation"),
                field(record, "elapsed_ms"),
                part("execute_us"),
                part("prepare_us") + part("verify_us") + part("spawn_us") + part("contain_us"),
                field(record, "stdout_bytes"),
            );
        }
    };
    for (label, start) in [("head", HistoryStart::Head), ("all_branches", HistoryStart::AllBranches)] {
        for pass in 1..=3 {
            let before = written();
            let started = std::time::Instant::now();
            let snapshot = backend
                .snapshot_ordered(&root, start.clone(), HistoryOrder::Topological, &cancel, None, false)
                .unwrap();
            println!(
                "refresh_{label} pass={pass} elapsed_ms={} commits={} branches={} changes={} warnings={}",
                started.elapsed().as_millis(),
                snapshot.history.as_ref().map_or(0, |h| h.commits.len()),
                snapshot.branches.len(),
                snapshot.status.as_ref().map_or(0, |s| s.entries.len()),
                snapshot.warnings.len(),
            );
            if pass < 3 {
                continue;
            }
            commands(before);
            if let Some(cursor) = snapshot.history.as_ref().and_then(|h| h.next.clone()) {
                let before = written();
                let started = std::time::Instant::now();
                let page = backend
                    .history(&snapshot.repository, HistoryStart::Continue(cursor), &cancel)
                    .unwrap();
                println!(
                    "load_more_{label} elapsed_ms={} commits={}",
                    started.elapsed().as_millis(),
                    page.commits.len()
                );
                commands(before);
            }
        }
    }
    drop(backend);
    let _ = fs::remove_dir_all(&logs);
}
