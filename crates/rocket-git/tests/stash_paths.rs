//! Stashing selected files: the stash holds exactly their changes, and other
//! unstaged and untracked changes stay. Git records the whole index in a stash,
//! so every staged change must be selected.
#[allow(dead_code)]
mod common;
use common::Fixture;
use rocket_git::*;

fn token() -> CancellationToken {
    CancellationToken::new()
}
fn path(text: &str) -> RepoPath {
    RepoPath::new(text.as_bytes().to_vec()).unwrap()
}
fn lines(bytes: Vec<u8>) -> Vec<String> {
    let mut lines: Vec<String> = String::from_utf8(bytes)
        .unwrap()
        .lines()
        .map(Into::into)
        .collect();
    lines.sort();
    lines
}

#[test]
fn partial_stash_saves_exactly_the_selected_files() {
    let f = Fixture::new();
    for (name, body) in [
        ("a.txt", "a1\n"),
        ("b.txt", "b1\n"),
        ("c.txt", "c1\n"),
        ("d.txt", "d1\n"),
        ("old 文.txt", "old\n"),
    ] {
        f.write(name, body);
    }
    f.commit("base");
    f.write("a.txt", "a2\n");
    f.write("b.txt", "b2\n");
    f.write("c.txt", "c2\n");
    f.run(&["add", "c.txt"]);
    f.write("u.txt", "untracked\n");
    let repo = f.repo();
    let read = |name: &str| std::fs::read_to_string(f.repository.join(name)).unwrap();
    // What `git stash show` lists: the stash against the commit it was saved on.
    let saved = |stash: &str| {
        lines(f.run(&[
            "-c",
            "core.quotepath=false",
            "diff",
            "--name-only",
            "--no-renames",
            &format!("{stash}^1"),
            stash,
        ]))
    };
    let staged = || {
        lines(f.run(&[
            "-c",
            "core.quotepath=false",
            "diff",
            "--cached",
            "--name-only",
            "--no-renames",
        ]))
    };
    let changed = || {
        lines(f.run(&[
            "-c",
            "core.quotepath=false",
            "diff",
            "--name-only",
            "--no-renames",
        ]))
    };

    // Refusals change nothing: an untracked file without include_untracked, an
    // unchanged path, and a selection that leaves the staged c.txt out.
    for (paths, untracked) in [
        (vec![path("u.txt")], false),
        (vec![path("unchanged.txt")], false),
        (vec![path("a.txt")], false),
    ] {
        assert!(matches!(
            repo.push_stash_paths(None, untracked, &paths, &token()),
            Err(Error::InvalidInput(_))
        ));
    }
    assert!(repo.stashes(&token()).unwrap().is_empty());

    // One unstaged and the staged file: the stash holds exactly those two.
    let report = repo
        .push_stash_paths(
            Some("PRIVATE a and c"),
            false,
            &[path("a.txt"), path("c.txt")],
            &token(),
        )
        .unwrap();
    assert!(report.write.result.is_ok(), "{:?}", report.write.result);
    assert_eq!(saved("stash@{0}"), ["a.txt", "c.txt"]);
    assert_eq!(
        [read("a.txt"), read("b.txt"), read("c.txt")],
        ["a1\n", "b2\n", "c1\n"]
    );
    assert!(staged().is_empty());
    assert_eq!(changed(), ["b.txt"]);

    // A staged deletion or rename can only be stashed with every change.
    f.run(&["rm", "--quiet", "d.txt"]);
    f.run(&["mv", "old 文.txt", "new 文.txt"]);
    for paths in [vec![path("b.txt")], vec![path("b.txt"), path("new 文.txt")]] {
        assert!(matches!(
            repo.push_stash_paths(None, false, &paths, &token()),
            Err(Error::InvalidInput(_))
        ));
    }
    assert_eq!(repo.stashes(&token()).unwrap().len(), 1);

    // Unstaged, they stay in the working tree; a selected untracked file is saved
    // in the stash's untracked commit and removed.
    f.run(&[
        "reset",
        "--quiet",
        "--",
        "d.txt",
        "old 文.txt",
        "new 文.txt",
    ]);
    let report = repo
        .push_stash_paths(None, true, &[path("b.txt"), path("u.txt")], &token())
        .unwrap();
    assert!(report.write.result.is_ok(), "{:?}", report.write.result);
    assert_eq!(saved("stash@{0}"), ["b.txt"]);
    assert_eq!(
        lines(f.run(&["show", "--name-only", "--format=", "stash@{0}^3"])),
        ["u.txt"]
    );
    assert_eq!(read("b.txt"), "b1\n");
    assert!(!f.repository.join("u.txt").exists());
    assert!(f.repository.join("new 文.txt").exists());
    assert!(staged().is_empty());
    assert_eq!(changed(), ["d.txt", "old 文.txt"]);
    assert_eq!(repo.stashes(&token()).unwrap().len(), 2);
}

/// Git keeps stashes with a working folder and refuses `git stash` in a bare
/// repository, so listing says so instead of failing with Git's exit code.
#[test]
fn listing_stashes_needs_a_working_tree() {
    let f = Fixture::new();
    f.write("a.txt", "a1\n");
    f.commit("base");
    let bare = f.root.canonicalize().unwrap().join("bare.git");
    f.run(&["clone", "--quiet", "--bare", ".", bare.to_str().unwrap().trim_start_matches(r"\\?\")]);
    let repo = f.git.open(&bare, &token()).unwrap();
    let listed = repo.stashes(&token());
    assert!(matches!(listed, Err(Error::WorktreeRequired)), "{listed:?}");
}
