mod common;
use common::Fixture;
use rocket_git::{CancellationToken, HistoryIndexChange};
use std::{fs, path::Path};

fn link_directory(target: &Path, link: &Path) {
    #[cfg(unix)]
    std::os::unix::fs::symlink(target, link).unwrap();
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let output = std::process::Command::new("cmd.exe")
            .args(["/d", "/c", "mklink", "/J"])
            .arg(link)
            .arg(target)
            .creation_flags(0x0800_0000)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

fn unlink_directory(link: &Path) {
    #[cfg(unix)]
    fs::remove_file(link).unwrap();
    #[cfg(windows)]
    fs::remove_dir(link).unwrap();
}

#[test]
fn redirected_graph_directories_are_refused_and_outside_files_survive() {
    for (part, suffix) in [
        ("objects", "info/commit-graphs"),
        ("objects/info", "commit-graphs"),
        ("objects/info/commit-graphs", ""),
    ] {
        let f = Fixture::new();
        f.write("keep 文", b"uncommitted\n");
        let repo = f.repo();
        let outside = f.root.join("outside 資料");
        let graphs = outside.join(suffix);
        fs::create_dir_all(&graphs).unwrap();
        let canary = graphs.join(format!("graph-{}.graph", "a".repeat(40)));
        fs::write(&canary, b"outside must survive").unwrap();
        fs::write(
            graphs.join("commit-graph-chain"),
            format!("{}\n", "a".repeat(40)),
        )
        .unwrap();
        let link = f.repository.join(".git").join(part);
        let backup = f.root.join("original directory");
        let existed = link.exists();
        if existed {
            fs::rename(&link, &backup).unwrap();
        }
        link_directory(&outside, &link);
        let config = fs::read(f.repository.join(".git/config")).unwrap();
        assert!(
            repo.history_index(&CancellationToken::new()).is_err(),
            "read followed redirected {part}"
        );
        assert!(!repo.has_history_index());
        let write = repo
            .change_history_index(
                HistoryIndexChange::Write {
                    on_fetch: true,
                    commits: None,
                },
                None,
                &CancellationToken::new(),
            )
            .unwrap();
        assert!(write.result.is_err());
        assert_eq!(fs::read(f.repository.join(".git/config")).unwrap(), config);
        let report = repo
            .change_history_index(HistoryIndexChange::Remove, None, &CancellationToken::new())
            .unwrap();
        let preserved = fs::read(&canary);
        // Remove only this fixture's link, never recursively traverse its target.
        unlink_directory(&link);
        if existed {
            fs::rename(&backup, &link).unwrap();
        }
        assert_eq!(
            preserved.unwrap(),
            b"outside must survive",
            "redirected {part}"
        );
        assert!(report.result.is_err(), "redirected {part} was accepted");
        assert_eq!(
            fs::read(f.repository.join("keep 文")).unwrap(),
            b"uncommitted\n"
        );
    }
}

#[test]
#[cfg_attr(windows, allow(clippy::permissions_set_readonly_false))]
fn remove_preserves_unknown_entries_and_hard_link_peer_permissions() {
    let f = Fixture::new();
    f.write("tracked", b"base\n");
    f.commit("base");
    let repo = f.repo();
    let graphs = f.repository.join(".git/objects/info/commit-graphs");
    fs::create_dir(&graphs).unwrap();
    let outside = f.root.join("outside hard link");
    fs::write(&outside, b"outside data").unwrap();
    let layer = graphs.join(format!("graph-{}.graph", "b".repeat(64)));
    fs::hard_link(&outside, &layer).unwrap();
    let mut permissions = fs::metadata(&outside).unwrap().permissions();
    permissions.set_readonly(true);
    fs::set_permissions(&outside, permissions).unwrap();
    let before = fs::metadata(&outside).unwrap().permissions();
    for name in [
        "graph-personal.graph",
        "graph-abc.graph",
        "tmp_graph_other",
        "notes 文",
    ] {
        fs::write(graphs.join(name), b"keep").unwrap();
    }
    let report = repo
        .change_history_index(HistoryIndexChange::Remove, None, &CancellationToken::new())
        .unwrap();
    assert!(report.result.is_ok(), "{:?}", report.result);
    assert!(!layer.exists());
    assert_eq!(fs::read(&outside).unwrap(), b"outside data");
    assert_eq!(fs::metadata(&outside).unwrap().permissions(), before);
    for name in [
        "graph-personal.graph",
        "graph-abc.graph",
        "tmp_graph_other",
        "notes 文",
    ] {
        assert_eq!(fs::read(graphs.join(name)).unwrap(), b"keep");
    }
    // Fixture cleanup can clear this owned canary after the assertion. Production
    // deletion must not change attributes on its hard-linked peer.
    #[cfg(windows)]
    {
        let mut permissions = before;
        permissions.set_readonly(false);
        fs::set_permissions(outside, permissions).unwrap();
    }
}

#[test]
fn linked_worktree_index_removal_preserves_local_changes_and_honors_cancellation() {
    let f = Fixture::new();
    f.write("tracked", b"base\n");
    f.commit("base");
    f.run(&["commit-graph", "write", "--reachable"]);
    let worktree = f.root.join("linked 工作");
    f.run(&[
        "worktree",
        "add",
        "-b",
        "linked",
        worktree.to_str().unwrap(),
    ]);
    fs::write(worktree.join("tracked"), b"local changes\n").unwrap();
    let repo = f.git.open(&worktree, &CancellationToken::new()).unwrap();
    let index = f.repository.join(".git/objects/info/commit-graph");
    let before = fs::read(&index).unwrap();
    let cancel = CancellationToken::new();
    cancel.cancel();
    assert!(matches!(
        repo.change_history_index(HistoryIndexChange::Remove, None, &cancel),
        Err(rocket_git::Error::Cancelled)
    ));
    assert_eq!(fs::read(&index).unwrap(), before);
    repo.change_history_index(HistoryIndexChange::Remove, None, &CancellationToken::new())
        .unwrap()
        .result
        .unwrap();
    assert!(!index.exists());
    assert_eq!(
        fs::read(worktree.join("tracked")).unwrap(),
        b"local changes\n"
    );
    assert_eq!(fs::read(f.repository.join("tracked")).unwrap(), b"base\n");
}
