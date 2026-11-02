mod common;
use common::Fixture;
use rocket_git::*;
use std::{
    fs,
    process::{Command, Stdio},
};
fn token() -> CancellationToken {
    CancellationToken::new()
}

#[test]
fn commit_diff_handles_root_and_uses_first_parent_for_merges() {
    let fixture = Fixture::new();
    fixture.write("root [literal].txt", "root content\n");
    fixture.commit("root subject must not leak into patch");
    let repo = fixture.repo();
    let root = repo.resolve_commit("HEAD", &token()).unwrap();
    let patch = repo
        .diff(
            &DiffOptions {
                target: DiffTarget::Commit { id: root },
                ..Default::default()
            },
            &token(),
        )
        .unwrap();
    let text = String::from_utf8_lossy(&patch.patch);
    assert!(text.contains("+root content"));
    assert!(!text.contains("root subject"));
    fixture.run(&["checkout", "-b", "feature"]);
    fixture.write("feature.txt", "feature content\n");
    fixture.commit("feature commit");
    fixture.run(&["checkout", "main"]);
    fixture.write("main.txt", "main content\n");
    fixture.commit("main commit");
    fixture.run(&["merge", "--no-ff", "--no-edit", "feature"]);
    let tip = repo.resolve_commit("HEAD", &token()).unwrap();
    let patch = repo
        .diff(
            &DiffOptions {
                target: DiffTarget::Commit { id: tip },
                ..Default::default()
            },
            &token(),
        )
        .unwrap();
    let text = String::from_utf8_lossy(&patch.patch);
    assert!(text.contains("+feature content"));
    assert!(!text.contains("+main content"));
}
fn entry<'a>(status: &'a Status, path: &str) -> &'a StatusEntry {
    status
        .entries
        .iter()
        .find(|entry| entry.path.as_bytes() == path.as_bytes())
        .unwrap_or_else(|| panic!("missing {path}: {status:?}"))
}

#[test]
fn empty_repository_and_nested_discovery() {
    let fixture = Fixture::new();
    fs::create_dir_all(fixture.repository.join("nested/deeper")).unwrap();
    let repo = fixture
        .git
        .open(fixture.repository.join("nested/deeper"), &token())
        .unwrap();
    assert_eq!(
        repo.info().worktree.as_ref().unwrap(),
        &fixture.repository.canonicalize().unwrap()
    );
    assert_eq!(repo.info().git_dir, repo.info().common_dir);
    assert_eq!(
        repo.head(&token()).unwrap(),
        Head::Unborn {
            branch: b"main".to_vec()
        }
    );
    let status = repo.status(&token()).unwrap();
    assert!(status.entries.is_empty());
    assert!(matches!(status.head, Head::Unborn { .. }));
    assert!(repo.branches(&token()).unwrap().is_empty());
    assert!(
        repo.history(HistoryStart::Head, 10, &token())
            .unwrap()
            .commits
            .is_empty()
    );
    fixture.write("first.txt", "hello\n");
    fixture.run(&["add", "--", "first.txt"]);
    let diff = repo
        .diff(
            &DiffOptions {
                target: DiffTarget::Staged,
                ..Default::default()
            },
            &token(),
        )
        .unwrap();
    assert!(String::from_utf8_lossy(&diff.patch).contains("+hello"));
}

#[test]
fn status_separates_index_worktree_renames_and_untracked_paths() {
    let fixture = Fixture::new();
    fixture.write("both.txt", "original\n");
    fixture.write("old name.txt", "rename this\n");
    fixture.write("removed.txt", "remove this\n");
    fixture.commit("initial");
    fixture.write("both.txt", "staged\n");
    fixture.run(&["add", "both.txt"]);
    fixture.write("both.txt", "unstaged\n");
    fixture.run(&["mv", "old name.txt", "new name 文.txt"]);
    fs::remove_file(fixture.repository.join("removed.txt")).unwrap();
    fixture.write("-leading [literal].txt", "new\n");
    fixture.write("nested/other.txt", "new\n");
    let status = fixture.repo().status(&token()).unwrap();
    assert_eq!(entry(&status, "both.txt").index, Change::Modified);
    assert_eq!(entry(&status, "both.txt").worktree, Change::Modified);
    let rename = entry(&status, "new name 文.txt");
    assert_eq!(rename.kind, EntryKind::RenameOrCopy);
    assert_eq!(
        rename.original_path.as_ref().unwrap().as_bytes(),
        b"old name.txt"
    );
    assert_eq!(rename.similarity, Some(100));
    assert_eq!(entry(&status, "removed.txt").worktree, Change::Deleted);
    assert_eq!(
        entry(&status, "-leading [literal].txt").kind,
        EntryKind::Untracked
    );
    assert_eq!(
        entry(&status, "nested/other.txt").kind,
        EntryKind::Untracked
    );
}

#[test]
fn reports_merge_conflict_without_hiding_other_changes() {
    let fixture = Fixture::new();
    fixture.write("conflict.txt", "base\n");
    fixture.commit("base");
    fixture.run(&["checkout", "-b", "other"]);
    fixture.write("conflict.txt", "other\n");
    fixture.commit("other");
    fixture.run(&["checkout", "main"]);
    fixture.write("conflict.txt", "main\n");
    fixture.commit("main");
    let merge = fixture.output(&fixture.repository, &["merge", "--no-edit", "other"]);
    assert_eq!(merge.status.code(), Some(1));
    fixture.write("new.txt", "still visible\n");
    let status = fixture.repo().status(&token()).unwrap();
    let conflict = entry(&status, "conflict.txt");
    assert_eq!(conflict.kind, EntryKind::Unmerged);
    assert_eq!(conflict.index, Change::Unmerged);
    assert_eq!(conflict.worktree, Change::Unmerged);
    assert_eq!(entry(&status, "new.txt").kind, EntryKind::Untracked);
}

#[test]
fn local_remote_branches_upstream_and_detached_head() {
    let fixture = Fixture::new();
    fixture.write("file", "first\n");
    fixture.commit("first");
    fixture.run(&["branch", "feature"]);
    fixture.run(&["update-ref", "refs/remotes/origin/main", "HEAD"]);
    fixture.run(&[
        "symbolic-ref",
        "refs/remotes/origin/HEAD",
        "refs/remotes/origin/main",
    ]);
    fixture.run(&["config", "remote.origin.url", "."]);
    fixture.run(&[
        "config",
        "remote.origin.fetch",
        "+refs/heads/*:refs/remotes/origin/*",
    ]);
    fixture.run(&["branch", "--set-upstream-to=origin/main", "main"]);
    fixture.write("file", "second\n");
    fixture.commit("second");
    let repo = fixture.repo();
    let branches = repo.branches(&token()).unwrap();
    let main = branches
        .iter()
        .find(|branch| branch.reference == b"refs/heads/main")
        .unwrap();
    assert!(main.is_head);
    assert_eq!(
        main.upstream.as_deref(),
        Some(b"refs/remotes/origin/main".as_slice())
    );
    let remote_head = branches
        .iter()
        .find(|branch| branch.reference == b"refs/remotes/origin/HEAD")
        .unwrap();
    assert_eq!(remote_head.kind, BranchKind::Remote);
    assert_eq!(
        remote_head.symbolic_target.as_deref(),
        Some(b"refs/remotes/origin/main".as_slice())
    );
    assert_eq!(repo.status(&token()).unwrap().ahead_behind, Some((1, 0)));
    fixture.run(&["checkout", "--detach", "HEAD"]);
    assert!(matches!(
        repo.head(&token()).unwrap(),
        Head::Detached { .. }
    ));
    assert!(matches!(
        repo.status(&token()).unwrap().head,
        Head::Detached { .. }
    ));
    assert!(
        repo.branches(&token())
            .unwrap()
            .iter()
            .all(|branch| !branch.is_head)
    );
}

#[test]
fn linked_worktree_uses_its_own_index_and_common_repository() {
    let fixture = Fixture::new();
    fixture.write("file.txt", "main\n");
    fixture.commit("initial");
    let linked = fixture.root.join("linked worktree");
    fixture.run(&["worktree", "add", "-b", "linked", linked.to_str().unwrap()]);
    fs::write(linked.join("file.txt"), "linked change\n").unwrap();
    let main = fixture.repo();
    let linked_repo = fixture.git.open(&linked, &token()).unwrap();
    assert_eq!(main.info().common_dir, linked_repo.info().common_dir);
    assert_ne!(main.info().git_dir, linked_repo.info().git_dir);
    fixture.run_at(&linked, &["add", "--", "file.txt"]);
    let marker = fs::read(linked.join(".git")).unwrap();
    // A cached diff must stay with its discovered index even if this marker changes.
    fs::write(
        linked.join(".git"),
        format!("gitdir: {}\n", fixture.repository.join(".git").display()),
    )
    .unwrap();
    let staged = DiffOptions {
        target: DiffTarget::Staged,
        ..Default::default()
    };
    assert!(
        String::from_utf8_lossy(&linked_repo.diff(&staged, &token()).unwrap().patch)
            .contains("+linked change")
    );
    assert!(main.diff(&staged, &token()).unwrap().patch.is_empty());
    fs::write(linked.join(".git"), marker).unwrap();
    fixture.run_at(&linked, &["reset", "--", "file.txt"]);
    assert!(main.status(&token()).unwrap().entries.is_empty());
    assert_eq!(
        entry(&linked_repo.status(&token()).unwrap(), "file.txt").worktree,
        Change::Modified
    );
    let branches = main.branches(&token()).unwrap();
    let branch = branches
        .iter()
        .find(|branch| branch.reference == b"refs/heads/linked")
        .unwrap();
    assert_eq!(
        branch
            .checked_out_at
            .as_ref()
            .unwrap()
            .canonicalize()
            .unwrap(),
        linked.canonicalize().unwrap()
    );
}

#[test]
fn bare_repository_supports_history_branches_and_commit_diffs() {
    let fixture = Fixture::new();
    fixture.write("file.txt", "one\n");
    fixture.commit("one");
    fixture.write("file.txt", "two\n");
    fixture.commit("two");
    let bare = fixture.root.join("bare.git");
    fixture.run(&[
        "clone",
        "--bare",
        fixture.repository.to_str().unwrap(),
        bare.to_str().unwrap(),
    ]);
    let repo = fixture.git.open(&bare, &token()).unwrap();
    assert!(repo.info().worktree.is_none());
    assert_eq!(repo.branches(&token()).unwrap().len(), 1);
    assert_eq!(
        repo.history(HistoryStart::Head, 10, &token())
            .unwrap()
            .commits
            .len(),
        2
    );
    assert!(matches!(
        repo.status(&token()),
        Err(Error::WorktreeRequired)
    ));
    assert!(matches!(
        repo.diff(&DiffOptions::default(), &token()),
        Err(Error::WorktreeRequired)
    ));
    let diff = repo
        .diff(
            &DiffOptions {
                target: DiffTarget::Between {
                    base: "HEAD~1".into(),
                    target: "HEAD".into(),
                },
                ..Default::default()
            },
            &token(),
        )
        .unwrap();
    assert!(String::from_utf8_lossy(&diff.patch).contains("+two"));
}

#[test]
fn retained_diff_handle_does_not_fall_back_to_an_ancestor_repository() {
    let fixture = Fixture::new();
    fixture.write("parent.txt", "base\n");
    fixture.commit("parent");
    fixture.write("parent.txt", "parent edit must not appear\n");
    fixture.run(&["init", "--initial-branch=main", "nested"]);
    let nested = fixture.repository.join("nested");
    let repo = fixture.git.open(&nested, &token()).unwrap();
    let source = nested.join(".git").canonicalize().unwrap();
    let root = fixture.root.canonicalize().unwrap();
    let destination = root.join("moved-nested-git");
    assert!(source.starts_with(&root) && destination.starts_with(&root));
    fs::rename(source, destination).unwrap();
    assert!(matches!(
        repo.diff(&DiffOptions::default(), &token()),
        Err(Error::Git { .. })
    ));
}

#[test]
fn commit_diff_checks_object_type_and_peels_tags_in_the_patch_command() {
    let fixture = Fixture::new();
    fixture.write("file.txt", "content\n");
    fixture.commit("base");
    fixture.run(&["tag", "-a", "test-tag", "-m", "tag metadata"]);
    let repo = fixture.repo();
    let object = |revision| {
        let bytes = fixture.run(&["rev-parse", revision]);
        ObjectId::parse(String::from_utf8(bytes).unwrap().trim()).unwrap()
    };
    let diff = |id| {
        repo.diff(
            &DiffOptions {
                target: DiffTarget::Commit { id },
                ..Default::default()
            },
            &token(),
        )
    };
    let patch = diff(object("test-tag")).unwrap();
    assert!(String::from_utf8_lossy(&patch.patch).contains("+content"));
    assert!(!String::from_utf8_lossy(&patch.patch).contains("tag metadata"));
    assert!(matches!(
        diff(object("HEAD:file.txt")),
        Err(Error::Git { .. })
    ));
    assert!(matches!(
        diff(object("HEAD^{tree}")),
        Err(Error::Git { .. })
    ));
    assert!(matches!(
        diff(ObjectId::parse(&"0".repeat(40)).unwrap()),
        Err(Error::Git { .. })
    ));
}

#[test]
fn history_pages_pin_tip_and_preserve_commit_bytes_and_parents() {
    let fixture = Fixture::new();
    for index in 0..5 {
        fixture.write("file", format!("{index}\n"));
        fixture.commit(&format!("commit {index}\n\nbody {index}"));
    }
    let repo = fixture.repo();
    let first = repo.history(HistoryStart::Head, 2, &token()).unwrap();
    assert_eq!(first.commits.len(), 2);
    assert_eq!(first.commits[0].subject_lossy(), "commit 4");
    assert!(first.commits[0].message.ends_with(b"body 4\n"));
    assert_eq!(first.commits[0].author.offset_minutes, 480);
    assert_eq!(first.commits[0].parents, vec![first.commits[1].id.clone()]);
    fixture.write("file", "new tip\n");
    fixture.commit("new tip");
    let second = repo
        .history(HistoryStart::Continue(first.next.unwrap()), 2, &token())
        .unwrap();
    assert_eq!(second.commits[0].subject_lossy(), "commit 2");
    let third = repo
        .history(HistoryStart::Continue(second.next.unwrap()), 2, &token())
        .unwrap();
    assert_eq!(third.commits.len(), 1);
    assert_eq!(third.commits[0].subject_lossy(), "commit 0");
    assert!(third.commits[0].parents.is_empty());
    assert!(third.next.is_none());
}

#[test]
fn batch_history_preserves_non_utf8_and_nul_in_message() {
    let fixture = Fixture::new();
    fixture.write("file", "value");
    fixture.commit("base");
    let tree = String::from_utf8(fixture.run(&["rev-parse", "HEAD^{tree}"])).unwrap();
    let mut raw = format!("tree {}\nauthor A <a@invalid> 1 +0000\ncommitter A <a@invalid> 1 +0000\nencoding ISO-8859-1\n\n", tree.trim()).into_bytes();
    raw.extend_from_slice(b"subject\n\nraw \xff\0 body\n");
    let mut command = fixture.command(
        &fixture.repository,
        &[
            "hash-object",
            "-t",
            "commit",
            "-w",
            "--stdin",
            "--literally",
        ],
    );
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn().unwrap();
    use std::io::Write;
    child.stdin.take().unwrap().write_all(&raw).unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success());
    let id = String::from_utf8(output.stdout).unwrap();
    let history = fixture
        .repo()
        .history(HistoryStart::Revision(id.trim().into()), 1, &token())
        .unwrap();
    assert_eq!(history.commits[0].message, b"subject\n\nraw \xff\0 body\n");
    assert_eq!(
        history.commits[0].encoding.as_deref(),
        Some(b"ISO-8859-1".as_slice())
    );
}

#[test]
fn diffs_are_literal_and_separate_staged_unstaged_and_binary() {
    let fixture = Fixture::new();
    for name in ["[literal].txt", "l.txt", "-leading.txt"] {
        fixture.write(name, "base\n");
    }
    fixture.write("binary.dat", b"\0original");
    fixture.commit("base");
    fixture.write("[literal].txt", "staged\n");
    fixture.run(&["--literal-pathspecs", "add", "--", "[literal].txt"]);
    fixture.write("[literal].txt", "unstaged\n");
    fixture.write("l.txt", "not selected\n");
    fixture.write("-leading.txt", "leading\n");
    fixture.write("binary.dat", b"\0changed");
    let repo = fixture.repo();
    let mut options = DiffOptions {
        paths: vec![RepoPath::new(b"[literal].txt".to_vec()).unwrap()],
        ..Default::default()
    };
    let diff = repo.diff(&options, &token()).unwrap();
    let text = String::from_utf8_lossy(&diff.patch);
    assert!(text.contains("-staged") && text.contains("+unstaged"));
    assert!(!text.contains("not selected"));
    options.target = DiffTarget::Staged;
    let diff = repo.diff(&options, &token()).unwrap();
    assert!(String::from_utf8_lossy(&diff.patch).contains("+staged"));
    options.target = DiffTarget::Unstaged;
    options.paths = vec![RepoPath::new(b"-leading.txt".to_vec()).unwrap()];
    assert!(
        String::from_utf8_lossy(&repo.diff(&options, &token()).unwrap().patch).contains("+leading")
    );
    options.paths = vec![RepoPath::new(b"binary.dat".to_vec()).unwrap()];
    assert!(
        String::from_utf8_lossy(&repo.diff(&options, &token()).unwrap().patch)
            .contains("Binary files")
    );
}

#[test]
fn previews_do_not_execute_external_diff_textconv_or_fsmonitor() {
    let fixture = Fixture::new();
    fixture.write(".gitattributes", "*.txt diff=rocket-test\n");
    fixture.write("file.txt", "base\n");
    fixture.commit("base");
    fixture.write("file.txt", "changed\n");
    // Any attempted helper execution fails the command; a valid raw patch must still be produced.
    fixture.run(&[
        "config",
        "diff.external",
        "rocket-must-not-execute-missing-helper",
    ]);
    fixture.run(&[
        "config",
        "diff.rocket-test.command",
        "rocket-must-not-execute-missing-helper",
    ]);
    fixture.run(&[
        "config",
        "diff.rocket-test.textconv",
        "rocket-must-not-execute-missing-helper",
    ]);
    fixture.run(&[
        "config",
        "core.fsmonitor",
        "rocket-must-not-execute-missing-helper",
    ]);
    let repo = fixture.repo();
    assert_eq!(repo.status(&token()).unwrap().entries.len(), 1);
    assert!(
        String::from_utf8_lossy(&repo.diff(&DiffOptions::default(), &token()).unwrap().patch)
            .contains("+changed")
    );
}

#[test]
fn sha256_objects_are_not_assumed_to_be_forty_characters() {
    let fixture = Fixture::with_format("sha256");
    fixture.write("file", "base\n");
    fixture.commit("base");
    let repo = fixture.repo();
    assert_eq!(repo.info().object_format, ObjectFormat::Sha256);
    let page = repo.history(HistoryStart::Head, 1, &token()).unwrap();
    assert_eq!(page.commits[0].id.as_str().len(), 64);
    assert_eq!(
        repo.branches(&token()).unwrap()[0].target.as_str().len(),
        64
    );
    assert!(matches!(
        repo.status(&token()).unwrap().head,
        Head::Branch { .. }
    ));
}

#[test]
fn read_commands_leave_index_refs_and_worktree_content_unchanged() {
    let fixture = Fixture::new();
    fixture.write("file.txt", "base\n");
    fixture.commit("base");
    fixture.write("file.txt", "modified\n");
    let repo = fixture.repo();
    let index_before = fs::read(repo.info().git_dir.join("index")).unwrap();
    let head_before = fs::read(repo.info().git_dir.join("HEAD")).unwrap();
    let ref_before = fs::read(repo.info().common_dir.join("refs/heads/main")).unwrap();
    let config_before = fs::read(repo.info().common_dir.join("config")).unwrap();
    repo.status(&token()).unwrap();
    repo.head(&token()).unwrap();
    repo.branches(&token()).unwrap();
    repo.history(HistoryStart::Head, 10, &token()).unwrap();
    repo.diff(&DiffOptions::default(), &token()).unwrap();
    assert_eq!(
        fs::read(repo.info().git_dir.join("index")).unwrap(),
        index_before
    );
    assert_eq!(
        fs::read(repo.info().git_dir.join("HEAD")).unwrap(),
        head_before
    );
    assert_eq!(
        fs::read(repo.info().common_dir.join("refs/heads/main")).unwrap(),
        ref_before
    );
    assert_eq!(
        fs::read(repo.info().common_dir.join("config")).unwrap(),
        config_before
    );
    assert_eq!(
        fs::read(fixture.repository.join("file.txt")).unwrap(),
        b"modified\n"
    );
    assert!(!repo.info().git_dir.join("index.lock").exists());
}

#[test]
fn cancellation_errors_limits_and_background_reads_are_explicit() {
    let fixture = Fixture::new();
    fixture.write("file", "base\n");
    fixture.commit("base");
    let repo = fixture.repo();
    let cancelled = token();
    cancelled.cancel();
    assert!(matches!(repo.status(&cancelled), Err(Error::Cancelled)));
    assert!(matches!(
        repo.diff(&DiffOptions::default(), &cancelled),
        Err(Error::Cancelled)
    ));
    assert!(matches!(
        repo.resolve_commit("--output=some-file", &token()),
        Err(Error::Git { .. })
    ));
    assert!(matches!(
        repo.history(HistoryStart::Head, 0, &token()),
        Err(Error::InvalidInput(_))
    ));
    assert!(matches!(
        fixture.git.open(&fixture.root, &token()),
        Err(Error::Git { .. })
    ));
    let task_repo = repo.clone();
    assert!(
        ReadTask::spawn(move |cancel| task_repo.status(&cancel))
            .unwrap()
            .wait()
            .unwrap()
            .entries
            .is_empty()
    );
    let limited = GitClient::new(
        fixture.git.executable(),
        CommandLimits {
            stdout_bytes: 512,
            ..Default::default()
        },
    )
    .unwrap();
    let limited_repo = limited.open(&fixture.repository, &token()).unwrap();
    fixture.write("file", "a very long diff line ".repeat(100));
    assert!(matches!(
        limited_repo.diff(&DiffOptions::default(), &token()),
        Err(Error::OutputLimit {
            stream: "stdout",
            ..
        })
    ));
}

#[test]
fn detects_submodule_worktree_changes() {
    let fixture = Fixture::new();
    fixture.write("file", "base\n");
    fixture.commit("base");
    let child = fixture.root.join("submodule-source");
    fixture.run(&[
        "clone",
        fixture.repository.to_str().unwrap(),
        child.to_str().unwrap(),
    ]);
    fixture.run(&[
        "-c",
        "protocol.file.allow=always",
        "submodule",
        "add",
        child.to_str().unwrap(),
        "module",
    ]);
    fixture.commit("add module");
    fs::write(fixture.repository.join("module/file"), "dirty\n").unwrap();
    fs::write(fixture.repository.join("module/new-file"), "untracked\n").unwrap();
    let status = fixture.repo().status(&token()).unwrap();
    let state = entry(&status, "module").submodule.unwrap();
    assert!(state.tracked_changes && state.untracked_changes);
    assert!(!state.commit_changed);
}

#[test]
#[ignore = "subprocess fixture invoked by environment-isolation test"]
fn poisoned_environment_fixture() {
    let directory = std::env::var_os("ROCKET_TEST_REPOSITORY").unwrap();
    let repo = GitClient::discover()
        .unwrap()
        .open(directory, &token())
        .unwrap();
    assert!(repo.info().worktree.is_some());
    let status = repo.status(&token()).unwrap();
    assert_eq!(status.entries.len(), 1);
    assert_eq!(status.entries[0].path.as_bytes(), b"wanted.txt");
}

#[test]
fn ignores_repository_overrides_in_the_parent_environment() {
    let fixture = Fixture::new();
    fixture.write("wanted.txt", "untracked\n");
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args([
            "--exact",
            "poisoned_environment_fixture",
            "--ignored",
            "--nocapture",
        ])
        .env("ROCKET_TEST_REPOSITORY", &fixture.repository)
        .env("GIT_DIR", fixture.root.join("not-a-repository"))
        .env("GIT_WORK_TREE", &fixture.root)
        .env("GIT_INDEX_FILE", fixture.root.join("wrong-index"))
        .env("GIT_CONFIG_COUNT", "1")
        .env("GIT_CONFIG_KEY_0", "core.bare")
        .env("GIT_CONFIG_VALUE_0", "true");
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000);
    }
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn merged_history_matches_git_topological_order_and_cursor_stays_in_its_worktree() {
    let fixture = Fixture::new();
    fixture.write("base", "base\n");
    fixture.commit("base");
    fixture.run(&["checkout", "-b", "side"]);
    fixture.write("side", "side\n");
    fixture.commit("side");
    fixture.run(&["checkout", "main"]);
    fixture.write("main", "main\n");
    fixture.commit("main");
    fixture.run(&["merge", "--no-ff", "--no-edit", "side"]);
    let repo = fixture.repo();
    let first = repo.history(HistoryStart::Head, 2, &token()).unwrap();
    assert_eq!(first.commits[0].parents.len(), 2);
    let linked = fixture.root.join("linked");
    fixture.run(&["worktree", "add", "--detach", linked.to_str().unwrap()]);
    let linked_repo = fixture.git.open(linked, &token()).unwrap();
    assert!(matches!(
        linked_repo.history(
            HistoryStart::Continue(first.next.clone().unwrap()),
            2,
            &token()
        ),
        Err(Error::InvalidInput(_))
    ));
    let second = repo
        .history(HistoryStart::Continue(first.next.unwrap()), 2, &token())
        .unwrap();
    let ids: Vec<_> = first
        .commits
        .iter()
        .chain(second.commits.iter())
        .map(|commit| commit.id.to_string())
        .collect();
    let expected =
        String::from_utf8(fixture.run(&["rev-list", "--topo-order", "HEAD", "--"])).unwrap();
    assert_eq!(ids, expected.lines().collect::<Vec<_>>());
    assert!(second.next.is_none());
}

#[test]
fn shallow_history_does_not_require_unavailable_parents() {
    let fixture = Fixture::new();
    for index in 0..3 {
        fixture.write("file", format!("{index}\n"));
        fixture.commit(&format!("commit {index}"));
    }
    let shallow = fixture.root.join("shallow");
    fixture.run(&[
        "clone",
        "--no-local",
        "--depth=1",
        fixture.repository.to_str().unwrap(),
        shallow.to_str().unwrap(),
    ]);
    let repo = fixture.git.open(shallow, &token()).unwrap();
    assert!(repo.info().is_shallow);
    let page = repo.history(HistoryStart::Head, 5, &token()).unwrap();
    assert_eq!(page.commits.len(), 1);
    assert_eq!(page.commits[0].parents.len(), 1); // The object keeps its real parent ID.
    assert!(page.next.is_none());
}

#[cfg(unix)]
#[test]
fn non_utf8_and_newline_paths_round_trip() {
    use std::os::unix::ffi::OsStringExt;
    let fixture = Fixture::new();
    let name = b"literal\n\xff.txt";
    fs::write(
        fixture
            .repository
            .join(std::ffi::OsString::from_vec(name.to_vec())),
        "base\n",
    )
    .unwrap();
    fixture.commit("base");
    fs::write(
        fixture
            .repository
            .join(std::ffi::OsString::from_vec(name.to_vec())),
        "changed\n",
    )
    .unwrap();
    let repo = fixture.repo();
    assert_eq!(
        repo.status(&token()).unwrap().entries[0].path.as_bytes(),
        name
    );
    let options = DiffOptions {
        paths: vec![RepoPath::new(name.to_vec()).unwrap()],
        ..Default::default()
    };
    assert!(
        String::from_utf8_lossy(&repo.diff(&options, &token()).unwrap().patch).contains("+changed")
    );
}

#[test]
fn observed_history_id_matches_head_and_rejects_non_commit_objects() {
    let fixture = Fixture::new();
    fixture.write("file", "base\n");
    fixture.commit("base");
    let repo = fixture.repo();
    let head = repo.resolve_commit("HEAD", &token()).unwrap();
    let page = repo
        .history(HistoryStart::Commit(head.clone()), 10, &token())
        .unwrap();
    assert_eq!(page.commits.len(), 1);
    assert_eq!(page.commits[0].id, head);
    for revision in ["HEAD:file", "HEAD^{tree}"] {
        let bytes = fixture.run(&["rev-parse", revision]);
        let id = ObjectId::parse(std::str::from_utf8(&bytes).unwrap().trim()).unwrap();
        assert!(
            repo.history(HistoryStart::Commit(id), 10, &token())
                .is_err()
        );
    }
}
