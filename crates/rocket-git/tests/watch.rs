//! Native recursive notifications for worktrees and their Git folders. The UI's
//! refresh tests separately exercise the Windows watcher.
#![cfg(any(target_os = "linux", target_os = "macos"))]
#[allow(dead_code)]
mod common;
use common::Fixture;
use rocket_git::*;
use std::{
    fs,
    sync::Arc,
    thread,
    time::{Duration, Instant},
};

fn quiet() -> Arc<dyn Fn() + Send + Sync> {
    Arc::new(|| {})
}
#[track_caller]
fn notified(watch: &RepositoryWatch) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !watch.take_changed().unwrap() {
        assert!(Instant::now() < deadline, "native notification missing");
        thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn reads_never_notify() {
    let f = Fixture::new();
    f.write("a", "a");
    f.commit("a");
    let watch = RepositoryWatch::new(f.repo().info(), quiet()).unwrap();
    thread::sleep(Duration::from_millis(300));
    assert!(!watch.take_changed().unwrap(), "watch changed without a write");
    let repo = f.repo();
    let token = CancellationToken::new();
    repo.status(&token).unwrap();
    repo.branches(&token).unwrap();
    repo.remotes(&token).unwrap();
    repo.stashes(&token).unwrap();
    repo.operation_state(&token).unwrap();
    repo.history(HistoryStart::Head, 200, &token).unwrap();
    thread::sleep(Duration::from_millis(300));
    assert!(!watch.take_changed().unwrap(), "reads generated notifications");
}

#[test]
fn linked_metadata_is_classified_and_replaced_folders_are_not_current() {
    let f = Fixture::new();
    f.write("a", "base");
    f.commit("base");
    let linked = f.root.join("linked");
    f.run(&["worktree", "add", "-b", "linked", linked.to_str().unwrap()]);
    let repo = f.git.open(&linked, &CancellationToken::new()).unwrap();
    for (path, metadata) in [
        (linked.join("a"), false),
        (repo.info().git_dir.join("refresh-probe"), true),
        (repo.info().common_dir.join("refresh-probe"), true),
    ] {
        let watch = RepositoryWatch::new(repo.info(), quiet()).unwrap();
        assert!(watch.is_current().unwrap());
        fs::write(&path, "changed").unwrap();
        notified(&watch);
        assert_eq!(watch.metadata_generation().unwrap() > 0, metadata, "{}", path.display());
    }
    let watch = RepositoryWatch::new(repo.info(), quiet()).unwrap();
    fs::rename(&linked, f.root.join("retained-linked")).unwrap();
    fs::create_dir(&linked).unwrap();
    assert!(!watch.is_current().unwrap(), "an identical path is not the same watched directory");
}

#[test]
fn edits_in_new_and_moved_folders_notify() {
    let f = Fixture::new();
    f.write("a", "base");
    f.commit("base");
    let watch = RepositoryWatch::new(f.repo().info(), quiet()).unwrap();
    let new = f.repository.join("new");
    fs::create_dir_all(new.join("deeper")).unwrap();
    notified(&watch);
    fs::write(new.join("deeper/file"), "x").unwrap();
    notified(&watch);
    let moved = f.repository.join("moved");
    fs::rename(&new, &moved).unwrap();
    notified(&watch);
    fs::create_dir(moved.join("deeper/inner")).unwrap();
    notified(&watch);
    fs::write(moved.join("deeper/inner/file"), "y").unwrap();
    notified(&watch);
    assert_eq!(watch.metadata_generation(), Some(0), "worktree edits only");
    f.run(&["branch", "late-branch"]);
    notified(&watch);
    assert!(watch.metadata_generation().unwrap() > 0, "a new branch is metadata");
}

#[test]
fn unicode_renames_and_subsequent_edits_notify() {
    let f = Fixture::new();
    f.write("資料/日本語.rs", "base");
    f.commit("base");
    let watch = RepositoryWatch::new(f.repo().info(), quiet()).unwrap();
    fs::rename(f.repository.join("資料"), f.repository.join("한국어 🚀")).unwrap();
    notified(&watch);
    fs::write(f.repository.join("한국어 🚀/日本語.rs"), "changed").unwrap();
    notified(&watch);
    assert!(watch.is_current().unwrap());
    assert_eq!(watch.metadata_generation(), Some(0));
}

#[test]
fn dropping_watch_releases_its_callback_without_more_writes() {
    let f = Fixture::new();
    f.write("a", "base");
    f.commit("base");
    let retained = Arc::new(());
    let marker = retained.clone();
    let wake = Arc::new(move || { let _ = &marker; });
    let watch = RepositoryWatch::new(f.repo().info(), wake).unwrap();
    drop(watch);
    let deadline = Instant::now() + Duration::from_secs(3);
    while Arc::strong_count(&retained) != 1 {
        assert!(Instant::now() < deadline, "watch callback leaked after shutdown");
        thread::sleep(Duration::from_millis(10));
    }
}
