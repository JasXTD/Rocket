//! What Changes keeps and when it reads again.
use super::*;
use crate::{
    agents::model::{Command, Provider},
    test_common as common,
};
use std::{fs, path::Path, thread};

fn record(id: u64, source: &Path) -> Record {
    Record {
        id,
        name: format!("task {id}"),
        source: source.to_owned(),
        destination: source.parent().unwrap().join(format!("agent 資料 {id}")),
        branch: format!("agents/kept-{id}"),
        base: "HEAD".into(),
        provider: Provider::Custom,
        command: Command {
            program: std::env::current_exe().unwrap().to_string_lossy().into_owned(),
            arguments: String::new(),
        },
        ..Default::default()
    }
}

fn file(name: &str) -> File {
    File {
        path: RepoPath::new(name.as_bytes().to_vec()).unwrap(),
        original: None,
        area: Area::Unstaged,
    }
}

fn listed(names: &[&str]) -> Snapshot {
    Snapshot {
        files: names.iter().map(|name| file(name)).collect(),
        limited: false,
        at: Instant::now(),
        watch: None,
        watch_error: None,
    }
}

#[test]
fn a_session_keeps_its_changes_while_only_its_activity_changes() {
    let a = record(1, &std::env::temp_dir().join("source"));
    let mut s = State::default();
    s.select_owner(&a);
    s.patch = Some(Arc::new(Patch::default()));
    s.selected = Some(0);
    s.paused = false;
    // What a provider event changes on the record of the session in view.
    let mut later = a.clone();
    later.attention = true;
    later.conversation = Some("11111111-2222-3333-4444-555555555555".into());
    s.select_owner(&later);
    assert!(s.patch.is_some() && s.selected == Some(0) && !s.paused, "nothing is read again");
    assert_eq!(s.owner, Some(later));
    // Another branch is another worktree to read.
    let mut moved = a.clone();
    moved.branch = "agents/other".into();
    s.select_owner(&moved);
    assert!(s.patch.is_none() && s.selected.is_none() && s.paused);
}

#[test]
fn another_session_and_back_shows_what_it_showed_and_checks_it_again() {
    let source = std::env::temp_dir().join("source");
    let (a, b) = (record(1, &source), record(2, &source));
    let diff = Arc::new(Patch::default());
    let mut s = State::default();
    s.select_owner(&a);
    s.loaded = Some(listed(&["one.txt", "two.txt"]));
    s.selected = Some(1);
    s.patch = Some(diff.clone());
    s.ready = vec![(file("two.txt"), diff.clone())];
    s.error = Some("old error".into());
    s.paused = false;
    s.select_owner(&b);
    assert!(s.loaded.is_none() && s.patch.is_none() && s.selected.is_none());
    assert!(s.ready.is_empty() && s.error.is_none());
    assert_eq!(s.owner, Some(b));
    s.select_owner(&a);
    assert_eq!(s.loaded.as_ref().map(|l| l.files.len()), Some(2));
    assert_eq!(s.selected, Some(1));
    assert!(Arc::ptr_eq(s.patch.as_ref().unwrap(), &diff) && s.ready.len() == 1);
    assert!(
        s.paused && s.watch.is_none() && !s.busy(),
        "it is read again once in view, and not trusted meanwhile"
    );
    assert!(s.error.is_none(), "an old error is not brought back");
    // Only so many sessions are kept: the ones seen last.
    let mut s = State::default();
    for id in 1..=(KEPT_SESSIONS as u64 + 3) {
        s.select_owner(&record(id, &source));
        s.loaded = Some(listed(&["one.txt"]));
    }
    assert_eq!(s.kept.len(), KEPT_SESSIONS);
    assert!(s.kept.iter().all(|k| k.owner.id > 2));
}

/// Poll until nothing has been read and no change has waited for half a second.
fn settle(s: &mut State, backend: &Backend, ctx: &Context) {
    let deadline = Instant::now() + Duration::from_secs(40);
    let mut quiet = Instant::now();
    loop {
        s.poll(backend, ctx);
        if s.busy() || s.dirty_since.is_some() || s.paused || s.loaded.is_none() {
            quiet = Instant::now();
        }
        if quiet.elapsed() >= Duration::from_millis(500) {
            return;
        }
        assert!(Instant::now() < deadline, "still reading: {:?} {:?}", s.error, s.notice);
        thread::sleep(Duration::from_millis(10));
    }
}

fn text(s: &State) -> String {
    s.patch
        .as_ref()
        .map(|p| {
            p.files
                .iter()
                .flat_map(|f| f.lines.iter().map(|l| l.text.as_str()))
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default()
}

#[test]
fn diffs_are_read_once_until_the_worktree_changes_and_never_blank_while_read_again() {
    common::isolated(
        "ROCKET_AGENT_KEPT",
        "agents::changes::tests::diffs_are_read_once_until_the_worktree_changes_and_never_blank_while_read_again",
        || {
            let f = common::Fixture::new();
            fs::write(f.repository.join("one.txt"), "base one\n").unwrap();
            fs::write(f.repository.join("two.txt"), "base two\n").unwrap();
            f.commit("base");
            let r = record(1, &f.repository);
            f.run(&["worktree", "add", "-b", &r.branch, r.destination.to_str().unwrap()]);
            fs::write(r.destination.join("one.txt"), "first edit\n").unwrap();
            fs::write(r.destination.join("two.txt"), "second edit\n").unwrap();
            let backend = Backend::discover(&f.root.join("logs")).unwrap();
            let ctx = Context::default();
            let mut s = State::default();
            s.select_owner(&r);
            settle(&mut s, &backend, &ctx);
            let files = &s.loaded.as_ref().unwrap().files;
            assert_eq!(files.len(), 2);
            assert!(s.watch.is_some(), "the worktree is watched: {:?}", s.notice);
            let one = files.iter().position(|f| f.path.as_bytes() == b"one.txt").unwrap();
            let two = 1 - one;
            // Each file is read when it is first chosen.
            s.select(two, &backend, &ctx);
            settle(&mut s, &backend, &ctx);
            assert!(text(&s).contains("+second edit"), "{}", text(&s));
            s.select(one, &backend, &ctx);
            settle(&mut s, &backend, &ctx);
            let first = s.patch.clone().unwrap();
            assert!(text(&s).contains("+first edit"));
            // Chosen again with nothing changed: the kept diff, and no read.
            s.select(two, &backend, &ctx);
            assert!(!s.busy() && text(&s).contains("+second edit"));
            s.select(one, &backend, &ctx);
            assert!(!s.busy() && Arc::ptr_eq(s.patch.as_ref().unwrap(), &first));
            // Out of view and back with nothing changed: no read either.
            s.suspend();
            s.poll(&backend, &ctx);
            assert!(!s.busy() && !s.paused && Arc::ptr_eq(s.patch.as_ref().unwrap(), &first));
            // A session dialog or another page lets go of the folder. Back in view the list
            // is read again, with the diff still on screen meanwhile.
            s.release();
            assert!(s.watch.is_none() && s.ready.is_empty());
            assert!(Arc::ptr_eq(s.patch.as_ref().unwrap(), &first));
            s.poll(&backend, &ctx);
            assert!(s.busy() && s.patch.is_some(), "unwatched meanwhile, so it is read again");
            settle(&mut s, &backend, &ctx);
            assert!(s.watch.is_some() && text(&s).contains("+first edit"));
            let first = s.patch.clone().unwrap();
            // The worktree changes: the file is read again, and its earlier diff stays on
            // screen until the new one is there.
            fs::write(r.destination.join("one.txt"), "edited again\n").unwrap();
            let deadline = Instant::now() + Duration::from_secs(40);
            while !text(&s).contains("+edited again") {
                s.poll(&backend, &ctx);
                assert!(s.patch.is_some(), "the diff went blank while it was read again");
                assert!(
                    Instant::now() < deadline,
                    "the change was never read: {:?} {:?}",
                    s.error,
                    s.notice
                );
                thread::sleep(Duration::from_millis(10));
            }
            assert!(!Arc::ptr_eq(s.patch.as_ref().unwrap(), &first));
            // The other file's kept diff went with the change: choosing it reads it.
            settle(&mut s, &backend, &ctx);
            s.select(two, &backend, &ctx);
            assert!(s.busy(), "a diff read before the change is not trusted");
            settle(&mut s, &backend, &ctx);
            assert!(text(&s).contains("+second edit"));
            // Stop ends the watch, so from then on nothing is kept.
            s.select(one, &backend, &ctx);
            assert!(!s.busy());
            s.stop();
            assert!(s.ready.is_empty() && s.patch.is_some());
            s.select(two, &backend, &ctx);
            assert!(s.busy(), "unwatched, every choice reads");
        },
    );
}
