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
fn a_write_stopped_while_git_holds_its_lock_leaves_none_behind() {
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
    assert!(!lock.exists(), "the stopped write left its lock");
    assert_eq!(report.index.unwrap().file, None);
    assert_eq!(observed_plain(&git, &root), before);
    // Nothing is in the way of the next write.
    let started = std::time::Instant::now();
    let index = change(&repo, write(false));
    println!("written in {} ms: {:?}", started.elapsed().as_millis(), index.file);
    assert!(index.file.unwrap().commits.is_some_and(|commits| commits >= 100_000));
    assert_eq!(observed_plain(&git, &root), before);
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
