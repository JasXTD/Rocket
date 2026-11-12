mod common;
use common::Fixture;
use rocket_git::*;
use std::{fs, path::Path, sync::mpsc::sync_channel};

fn token() -> CancellationToken {
    CancellationToken::new()
}

fn commits(fixture: &Fixture, names: &[&str]) {
    for name in names {
        fixture.write("file", format!("{name}\n"));
        fixture.commit(name);
    }
}

/// Everything a write of the index must leave alone: references, HEAD and the
/// state of the working files.
fn observed(fixture: &Fixture, at: &Path) -> (Vec<u8>, Vec<u8>, Vec<u8>) {
    (
        fixture.run_at(at, &["for-each-ref"]),
        fixture.run_at(at, &["rev-parse", "HEAD"]),
        fixture.run_at(at, &["status", "--porcelain=v2", "--branch"]),
    )
}

fn change(repo: &Repository, change: HistoryIndexChange) -> HistoryIndex {
    let report = repo.change_history_index(change, None, &token()).unwrap();
    report.result.unwrap();
    report.index.unwrap()
}

fn write(on_fetch: bool) -> HistoryIndexChange {
    HistoryIndexChange::Write { on_fetch, commits: None }
}

#[test]
fn writes_updates_and_removes_the_index_and_changes_nothing_else() {
    let f = Fixture::new();
    commits(&f, &["one", "two", "three"]);
    let info = f.repository.join(".git/objects/info");
    let repo = f.repo();
    assert_eq!(
        repo.history_index(&token()).unwrap(),
        HistoryIndex { file: None, unused: None, on_fetch: false }
    );
    assert!(!repo.has_history_index());
    assert_eq!(repo.count_commits(&token()).unwrap(), 3);
    let before = observed(&f, &f.repository);

    // A first write makes the single file and reports the commits Git read.
    let (sender, receiver) = sync_channel(64);
    let report = repo
        .change_history_index(
            HistoryIndexChange::Write { on_fetch: true, commits: Some(3) },
            Some(&sender),
            &token(),
        )
        .unwrap();
    report.result.unwrap();
    let index = report.index.unwrap();
    let file = index.file.clone().unwrap();
    assert_eq!(file.commits, Some(3));
    assert_eq!(file.bytes, fs::metadata(info.join("commit-graph")).unwrap().len());
    assert!(file.written.is_some());
    assert_eq!((index.unused, index.on_fetch), (None, true));
    assert!(repo.has_history_index());
    assert_eq!(f.run(&["config", "--local", "--get", "rocket.commitGraphOnFetch"]), b"true\n");
    // Git prints progress only for work that takes a while, so three commits may
    // report nothing; whatever does arrive counts commits against the total given.
    assert!(receiver
        .try_iter()
        .all(|p| p.phase == FetchPhase::IndexingHistory && p.current <= 3 && p.total == Some(3)));
    assert_eq!(observed(&f, &f.repository), before);
    // Git reads what was written.
    f.run(&["commit-graph", "verify"]);

    // Writing again adds only what the index lacks, as a second layer of a chain.
    commits(&f, &["four"]);
    let before = observed(&f, &f.repository);
    let index = change(&repo, write(true));
    assert_eq!(index.file.as_ref().unwrap().commits, Some(4));
    assert!(!info.join("commit-graph").exists());
    assert!(info.join("commit-graphs/commit-graph-chain").is_file());
    let layers = fs::read_dir(info.join("commit-graphs")).unwrap().count() - 1;
    assert_eq!(layers, 2);
    assert_eq!(observed(&f, &f.repository), before);
    f.run(&["commit-graph", "verify"]);
    // With nothing new, the same request leaves the files as they are.
    let unchanged = change(&repo, write(true));
    assert_eq!(unchanged, index);

    // Removing takes every layer and the chain, and keeps the fetch setting.
    let index = change(&repo, HistoryIndexChange::Remove);
    assert_eq!(index, HistoryIndex { file: None, unused: None, on_fetch: true });
    assert!(!info.join("commit-graph").exists());
    assert!(!info.join("commit-graphs").exists());
    assert_eq!(observed(&f, &f.repository), before);
    // Removing what is not there is not an error.
    assert_eq!(change(&repo, HistoryIndexChange::Remove), index);

    let index = change(&repo, HistoryIndexChange::OnFetch(false));
    assert!(!index.on_fetch);
    let output = f.output(&f.repository, &["config", "--local", "--get", "rocket.commitGraphOnFetch"]);
    assert_eq!(output.status.code(), Some(1));
    f.run(&["fsck", "--no-dangling"]);
}

#[test]
fn says_why_git_would_not_read_an_index_and_writes_none_there() {
    let refused = |fixture: &Fixture, expected: HistoryIndexUnused| {
        let repo = fixture.repo();
        assert_eq!(repo.history_index(&token()).unwrap().unused, Some(expected));
        let report = repo.change_history_index(write(true), None, &token()).unwrap();
        assert!(matches!(report.result, Err(Error::Unsupported(_))), "{:?}", report.result);
        let index = report.index.unwrap();
        // Nothing was written and the setting was not stored either.
        assert_eq!((index.file, index.on_fetch), (None, false));
        assert!(!fixture.repository.join(".git/objects/info/commit-graph").exists());
    };

    let f = Fixture::new();
    commits(&f, &["one", "two"]);
    f.run(&["config", "core.commitGraph", "false"]);
    refused(&f, HistoryIndexUnused::TurnedOff);

    let f = Fixture::new();
    commits(&f, &["one", "two"]);
    f.run(&["replace", "HEAD", "HEAD~1"]);
    refused(&f, HistoryIndexUnused::Replaced);

    let f = Fixture::new();
    commits(&f, &["one", "two"]);
    let head = f.run(&["rev-parse", "HEAD"]);
    fs::write(f.repository.join(".git/shallow"), head).unwrap();
    refused(&f, HistoryIndexUnused::Shallow);
}

#[test]
fn another_writers_lock_refuses_the_change_and_is_left_alone() {
    let f = Fixture::new();
    commits(&f, &["one", "two"]);
    let repo = f.repo();
    change(&repo, write(false));
    let info = f.repository.join(".git/objects/info");
    let index = fs::read(info.join("commit-graph")).unwrap();
    for lock in ["commit-graph.lock", "commit-graphs/commit-graph-chain.lock"] {
        let lock = info.join(lock);
        fs::create_dir_all(lock.parent().unwrap()).unwrap();
        fs::write(&lock, b"another writer").unwrap();
        for request in [write(false), HistoryIndexChange::Remove] {
            let report = repo.change_history_index(request, None, &token()).unwrap();
            let Err(Error::Unsupported(message)) = report.result else {
                panic!("{:?}", report.result);
            };
            assert!(message.contains(lock.file_name().unwrap().to_str().unwrap()), "{message}");
        }
        assert_eq!(fs::read(&lock).unwrap(), b"another writer");
        assert_eq!(fs::read(info.join("commit-graph")).unwrap(), index);
        fs::remove_file(&lock).unwrap();
    }
    // The fetch setting touches no index file, so a lock does not stop it.
    fs::write(info.join("commit-graph.lock"), b"another writer").unwrap();
    assert!(change(&repo, HistoryIndexChange::OnFetch(true)).on_fetch);
}

#[test]
fn a_cancelled_request_writes_nothing() {
    let f = Fixture::new();
    commits(&f, &["one", "two"]);
    let repo = f.repo();
    let cancel = token();
    cancel.cancel();
    assert!(matches!(
        repo.change_history_index(write(true), None, &cancel),
        Err(Error::Cancelled)
    ));
    assert_eq!(
        repo.history_index(&token()).unwrap(),
        HistoryIndex { file: None, unused: None, on_fetch: false }
    );
}

#[cfg(unix)]
#[test]
fn stopped_write_preserves_locks_created_by_an_external_writer_after_preflight() {
    common::isolated("ROCKET_INDEX_RACE", "stopped_write_preserves_locks_created_by_an_external_writer_after_preflight", || {
        use std::{io::Write, os::unix::fs::PermissionsExt, thread, time::{Duration, Instant}};
        for timeout in [false, true] {
            let f = Fixture::new();
            commits(&f, &["base"]);
            let info = f.repository.join(".git/objects/info");
            fs::create_dir_all(info.join("commit-graphs")).unwrap();
            let marker = f.root.join("write-started");
            let relay = f.root.join("git-relay");
            let quote = |p: &Path| format!("'{}'", p.to_str().unwrap().replace('\'', "'\\''"));
            // Pass every discovery/read through to real Git. Only the graph write
            // waits at a deterministic barrier, after Rocket's lock preflight.
            fs::write(&relay, format!("#!/bin/sh\nfor arg in \"$@\"; do\n if [ \"$arg\" = commit-graph ]; then\n  : > {}\n  while :; do sleep 0.02; done\n fi\ndone\nexec {} \"$@\"\n", quote(&marker), quote(f.git.executable()))).unwrap();
            fs::set_permissions(&relay, fs::Permissions::from_mode(0o700)).unwrap();
            let log = DiagnosticLog::open(f.root.join("logs")).unwrap();
            let git = GitClient::new_with_log(&relay, CommandLimits { timeout: Duration::from_secs(3), ..Default::default() }, log.clone()).unwrap();
            let repo = git.open(&f.repository, &token()).unwrap();
            let before = observed(&f, &f.repository);
            let cancel = token();
            let paths = [info.join("commit-graph.lock"), info.join("commit-graphs/commit-graph-chain.lock"), info.join("commit-graphs/tmp_graph_external")];
            let report = thread::scope(|scope| {
                let writer = scope.spawn(|| repo.change_history_index(write(false), None, &cancel));
                let deadline = Instant::now() + Duration::from_secs(10);
                while !marker.exists() && !writer.is_finished() && Instant::now() < deadline {
                    thread::sleep(Duration::from_millis(5));
                }
                assert!(marker.exists(), "graph write did not reach the barrier");
                // These handles belong to this process, not Rocket's Git child.
                let held: Vec<_> = paths.iter().map(|path| {
                    let mut file = fs::OpenOptions::new().write(true).create_new(true).open(path).unwrap();
                    file.write_all(b"external writer data").unwrap();
                    file
                }).collect();
                if !timeout { cancel.cancel(); }
                let report = writer.join().unwrap().unwrap();
                for path in &paths {
                    assert_eq!(fs::read(path).unwrap(), b"external writer data", "{}", path.display());
                }
                drop(held);
                report
            });
            assert!(if timeout { matches!(report.result, Err(Error::TimedOut)) } else { matches!(report.result, Err(Error::Cancelled)) }, "{:?}", report.result);
            assert_eq!(observed(&f, &f.repository), before);
            let retry = repo.change_history_index(write(false), None, &token()).unwrap();
            let error = retry.result.unwrap_err().to_string();
            assert!(error.contains("Another Git process") && error.contains("commit-graph.lock"), "{error}");
            assert_eq!(log.health().dropped_events, 0);
            // Only this test knows who owns these disposable files.
            for path in paths { fs::remove_file(path).unwrap(); }
        }
    });
}

#[test]
fn a_fetch_adds_what_it_brought_only_where_the_repository_asks() {
    let f = Fixture::new();
    commits(&f, &["base"]);
    let bare = f.root.join("upstream.git");
    f.run(&["clone", "--bare", f.repository.to_str().unwrap(), bare.to_str().unwrap()]);
    let client = f.root.join("client");
    f.run(&["clone", bare.to_str().unwrap(), client.to_str().unwrap()]);
    f.run_at(&client, &["config", "core.hooksPath", f.root.join("no-hooks").to_str().unwrap()]);
    let publish = |name: &str| {
        commits(&f, &[name]);
        f.run(&["push", bare.to_str().unwrap(), "main"]);
    };
    let repo = f.git.open(&client, &token()).unwrap();
    let fetch = || {
        repo.fetch("origin", None, &token()).unwrap().result.unwrap();
        repo.history_index(&token()).unwrap().file.and_then(|file| file.commits)
    };

    // No index: a fetch makes none, whatever the setting says.
    change(&repo, HistoryIndexChange::OnFetch(true));
    publish("first");
    assert_eq!(fetch(), None);
    assert!(!repo.has_history_index());

    // With an index and the setting on, the fetched commit joins it.
    assert_eq!(change(&repo, write(true)).file.unwrap().commits, Some(2));
    publish("second");
    assert_eq!(fetch(), Some(3));
    f.run_at(&client, &["commit-graph", "verify"]);
    let before = observed(&f, &client);
    // A fetch that brings nothing leaves the index files untouched.
    let chain = client.join(".git/objects/info/commit-graphs/commit-graph-chain");
    let written = fs::metadata(&chain).unwrap().modified().unwrap();
    assert_eq!(fetch(), Some(3));
    assert_eq!(fs::metadata(&chain).unwrap().modified().unwrap(), written);
    assert_eq!(observed(&f, &client), before);

    // With the setting off the index stays as it was; it is still valid, only older.
    change(&repo, HistoryIndexChange::OnFetch(false));
    publish("third");
    assert_eq!(fetch(), Some(3));
    assert_eq!(repo.count_commits(&token()).unwrap(), 4);
    f.run_at(&client, &["commit-graph", "verify"]);
}

/// Stops a real write while Git holds its lock. Small fixtures finish too fast to
/// catch, so this runs by hand against a large repository:
/// `ROCKET_LOAD_PROBE_REPOSITORY=<folder> cargo test -p rocket-git --test history_index -- --ignored`
#[test]
#[ignore = "manual check; needs ROCKET_LOAD_PROBE_REPOSITORY, a repository with 100,000 commits or more"]
fn a_stopped_real_write_preserves_work_and_reports_a_retained_lock() {
    let Some(root) = std::env::var_os("ROCKET_LOAD_PROBE_REPOSITORY").map(std::path::PathBuf::from) else {
        println!("ROCKET_LOAD_PROBE_REPOSITORY is not set");
        return;
    };
    let git = GitClient::new(
        GitClient::discover().unwrap().executable(),
        CommandLimits { timeout: std::time::Duration::from_secs(600), ..Default::default() },
    )
    .unwrap();
    let repo = git.open(&root, &token()).unwrap();
    let info = repo.info().common_dir.join("objects/info");
    change(&repo, HistoryIndexChange::Remove);
    let before = observed_plain(&git, &root);
    let lock = info.join("commit-graph.lock");
    let cancel = token();
    let started = std::time::Instant::now();
    let report = std::thread::scope(|scope| {
        let writer = scope.spawn(|| repo.change_history_index(write(false), None, &cancel));
        while !lock.exists() && !writer.is_finished() {
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        let seen = lock.exists();
        cancel.cancel();
        println!("lock seen={seen} after {} ms", started.elapsed().as_millis());
        assert!(seen, "the write finished before its lock was seen; use a larger repository");
        writer.join().unwrap().unwrap()
    });
    println!("stopped after {} ms: {:?}", started.elapsed().as_millis(), report.result);
    assert!(matches!(report.result, Err(Error::Cancelled)), "{:?}", report.result);
    assert_eq!(report.index.unwrap().file, None);
    assert_eq!(observed_plain(&git, &root), before);
    // Do not assume a surviving lock is ours. This opt-in probe does not own its
    // input repository, so it leaves recovery to the operator rather than unlinking.
    if lock.exists() {
        let contents = fs::read(&lock).unwrap();
        let retry = repo.change_history_index(write(false), None, &token()).unwrap();
        assert!(matches!(retry.result, Err(Error::Unsupported(_))));
        assert_eq!(fs::read(&lock).unwrap(), contents);
        assert_eq!(observed_plain(&git, &root), before);
    }
}

fn observed_plain(git: &GitClient, at: &Path) -> Vec<Vec<u8>> {
    [&["for-each-ref"][..], &["rev-parse", "HEAD"]]
        .into_iter()
        .map(|arguments| {
            let output = std::process::Command::new(git.executable())
                .current_dir(at)
                .args(arguments)
                .output()
                .unwrap();
            assert!(output.status.success());
            output.stdout
        })
        .collect()
}
