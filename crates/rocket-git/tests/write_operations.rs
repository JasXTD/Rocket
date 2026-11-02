mod common;
use common::Fixture;
use rocket_git::*;
use std::{
    fs,
    path::Path,
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

fn token() -> CancellationToken {
    CancellationToken::new()
}
fn paths(names: &[&str]) -> Vec<RepoPath> {
    names
        .iter()
        .map(|name| RepoPath::new(name.as_bytes().to_vec()).unwrap())
        .collect()
}
fn success(report: WriteReport) -> Status {
    report.result.unwrap();
    report.status.unwrap()
}
fn entry<'a>(status: &'a Status, path: &str) -> &'a StatusEntry {
    status
        .entries
        .iter()
        .find(|entry| entry.path.as_bytes() == path.as_bytes())
        .unwrap_or_else(|| panic!("missing {path}: {status:?}"))
}
fn oid(repo: &Repository) -> ObjectId {
    repo.resolve_commit("HEAD", &token()).unwrap()
}
fn tracked(fixture: &Fixture, revision: &str) -> Vec<u8> {
    fixture.run(&["show", revision])
}

#[test]
fn selected_stage_unstage_and_first_commit_preserve_unstaged_content() {
    let fixture = Fixture::new();
    fixture.write("one.txt", "staged\n");
    fixture.write("two.txt", "leave untracked\n");
    let repo = fixture.repo();
    let status = success(repo.stage(&paths(&["one.txt"]), &token()).unwrap());
    assert_eq!(entry(&status, "one.txt").index, Change::Added);
    assert_eq!(entry(&status, "two.txt").kind, EntryKind::Untracked);
    fixture.write("one.txt", "changed after staging\n");
    let status = success(repo.unstage(&paths(&["one.txt"]), &token()).unwrap());
    assert_eq!(entry(&status, "one.txt").kind, EntryKind::Untracked);
    assert_eq!(
        fs::read(fixture.repository.join("one.txt")).unwrap(),
        b"changed after staging\n"
    );
    fixture.write("one.txt", "staged again\n");
    success(repo.stage(&paths(&["one.txt"]), &token()).unwrap());
    fixture.write("one.txt", "working version\n");
    let report = repo
        .commit(
            "First Rocket commit\n\n# Keep this line\nUnicode: 文",
            &token(),
        )
        .unwrap();
    assert_eq!(report.operation, WriteOperation::Commit);
    let status = success(report);
    assert_eq!(entry(&status, "one.txt").worktree, Change::Modified);
    assert_eq!(entry(&status, "one.txt").index, Change::Unchanged);
    assert_eq!(entry(&status, "two.txt").kind, EntryKind::Untracked);
    assert_eq!(tracked(&fixture, "HEAD:one.txt"), b"staged again\n");
    let page = repo.history(HistoryStart::Head, 1, &token()).unwrap();
    assert_eq!(
        page.commits[0].message,
        "First Rocket commit\n\n# Keep this line\nUnicode: 文\n".as_bytes()
    );
}

#[test]
fn unstage_all_handles_unborn_and_existing_head_without_touching_files() {
    let fixture = Fixture::new();
    let repo = fixture.repo();
    assert!(
        success(repo.unstage_all(&token()).unwrap())
            .entries
            .is_empty()
    );
    fixture.write("directory/a.txt", "a\n");
    fixture.write("b.txt", "b\n");
    success(repo.stage_all(&token()).unwrap());
    fixture.write("directory/a.txt", "changed\n");
    let status = success(repo.unstage_all(&token()).unwrap());
    assert!(
        status
            .entries
            .iter()
            .all(|entry| entry.kind == EntryKind::Untracked)
    );
    assert_eq!(
        fs::read(fixture.repository.join("directory/a.txt")).unwrap(),
        b"changed\n"
    );
    success(repo.stage_all(&token()).unwrap());
    success(repo.commit("Initial", &token()).unwrap());
    let head = oid(&repo);
    fixture.write("directory/a.txt", "another edit\n");
    fs::remove_file(fixture.repository.join("b.txt")).unwrap();
    fixture.write("new.txt", "new\n");
    success(repo.stage_all(&token()).unwrap());
    let status = success(repo.unstage_all(&token()).unwrap());
    assert_eq!(oid(&repo), head);
    assert!(
        status
            .entries
            .iter()
            .all(|entry| entry.index == Change::Unchanged)
    );
    assert_eq!(entry(&status, "b.txt").worktree, Change::Deleted);
    assert_eq!(entry(&status, "new.txt").kind, EntryKind::Untracked);
    assert_eq!(
        fs::read(fixture.repository.join("directory/a.txt")).unwrap(),
        b"another edit\n"
    );
    assert!(!fixture.repository.join("b.txt").exists());
}

#[test]
fn stage_and_unstage_are_literal_handle_renames_and_leave_other_staging_alone() {
    let fixture = Fixture::new();
    for name in [
        "[literal].txt",
        "l.txt",
        "-option.txt",
        "old 文.txt",
        "deleted.txt",
    ] {
        fixture.write(name, format!("{name} original\n"));
    }
    fixture.commit("base");
    fixture.write("[literal].txt", "selected\n");
    fixture.write("l.txt", "not selected\n");
    fixture.write("-option.txt", "selected option\n");
    fs::rename(
        fixture.repository.join("old 文.txt"),
        fixture.repository.join("new 文.txt"),
    )
    .unwrap();
    fs::remove_file(fixture.repository.join("deleted.txt")).unwrap();
    let repo = fixture.repo();
    let selection = paths(&[
        "[literal].txt",
        "-option.txt",
        "old 文.txt",
        "new 文.txt",
        "deleted.txt",
    ]);
    let status = success(repo.stage(&selection, &token()).unwrap());
    assert_eq!(entry(&status, "l.txt").index, Change::Unchanged);
    assert_eq!(entry(&status, "deleted.txt").index, Change::Deleted);
    assert_eq!(entry(&status, "new 文.txt").index, Change::Renamed);
    success(
        repo.unstage(
            &paths(&["old 文.txt", "new 文.txt", "[literal].txt"]),
            &token(),
        )
        .unwrap(),
    );
    let status = repo.status(&token()).unwrap();
    assert_eq!(entry(&status, "-option.txt").index, Change::Modified);
    assert_eq!(entry(&status, "[literal].txt").index, Change::Unchanged);
    assert_eq!(entry(&status, "new 文.txt").kind, EntryKind::Untracked);
    assert!(fixture.repository.join("new 文.txt").exists());
    assert!(!fixture.repository.join("old 文.txt").exists());
}

#[test]
fn empty_selections_invalid_messages_and_cancelled_calls_do_not_write() {
    let fixture = Fixture::new();
    fixture.write("file", "base\n");
    fixture.commit("base");
    fixture.write("file", "changed\n");
    let repo = fixture.repo();
    let index_before = fs::read(repo.info().git_dir.join("index")).unwrap();
    let head_before = oid(&repo);
    assert!(matches!(
        repo.stage(&[], &token()),
        Err(Error::InvalidInput(_))
    ));
    assert!(matches!(
        repo.unstage(&[], &token()),
        Err(Error::InvalidInput(_))
    ));
    for message in ["", " \n\t", "no\0nul"] {
        assert!(matches!(
            repo.commit(message, &token()),
            Err(Error::InvalidInput(_))
        ));
    }
    let cancelled = token();
    cancelled.cancel();
    assert!(matches!(repo.stage_all(&cancelled), Err(Error::Cancelled)));
    assert!(matches!(
        repo.unstage_all(&cancelled),
        Err(Error::Cancelled)
    ));
    assert!(matches!(
        repo.commit("must not commit", &cancelled),
        Err(Error::Cancelled)
    ));
    assert_eq!(
        fs::read(repo.info().git_dir.join("index")).unwrap(),
        index_before
    );
    assert_eq!(oid(&repo), head_before);
}

#[test]
fn ignored_files_are_not_forced_and_failed_commands_include_refreshed_status() {
    let fixture = Fixture::new();
    fixture.write(".gitignore", "secret.txt\n");
    fixture.write("secret.txt", "ignored\n");
    fixture.write("visible.txt", "visible\n");
    let repo = fixture.repo();
    let report = repo.stage(&paths(&["secret.txt"]), &token()).unwrap();
    assert!(matches!(report.result, Err(Error::Git { .. })));
    assert_eq!(
        entry(&report.status.unwrap(), "visible.txt").kind,
        EntryKind::Untracked
    );
    let status = success(repo.stage_all(&token()).unwrap());
    assert!(
        !status
            .entries
            .iter()
            .any(|entry| entry.path.as_bytes() == b"secret.txt")
    );
    success(repo.commit("ordinary files only", &token()).unwrap());
    let no_changes = repo.commit("nothing to commit", &token()).unwrap();
    match no_changes.result {
        Err(Error::Git {
            code: Some(1),
            stdout,
            ..
        }) => {
            assert!(String::from_utf8_lossy(&stdout).contains("nothing to commit"));
        }
        other => panic!("unexpected result: {other:?}"),
    }
    assert!(no_changes.status.unwrap().entries.is_empty());
}

fn hook(fixture: &Fixture, name: &str, script: &str) {
    let path = fixture.root.join("no-hooks").join(name);
    fs::write(&path, script).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    }
}

#[test]
fn commit_hooks_run_rejections_are_reported_and_message_hooks_can_edit() {
    let fixture = Fixture::new();
    fixture.write("file", "base\n");
    fixture.commit("base");
    fixture.write("file", "changed\n");
    let repo = fixture.repo();
    success(repo.stage_all(&token()).unwrap());
    let before = oid(&repo);
    hook(
        &fixture,
        "pre-commit",
        "#!/bin/sh\nprintf 'hook worktree change\\n' > from-hook.txt\nprintf 'Rocket hook rejected\\n' >&2\nexit 1\n",
    );
    let rejected = repo.commit("Rejected", &token()).unwrap();
    match rejected.result {
        Err(Error::Git { stderr, .. }) => {
            assert!(String::from_utf8_lossy(&stderr).contains("Rocket hook rejected"))
        }
        other => panic!("hook should reject: {other:?}"),
    }
    assert_eq!(
        entry(&rejected.status.unwrap(), "from-hook.txt").kind,
        EntryKind::Untracked
    );
    assert_eq!(oid(&repo), before);
    fs::remove_file(fixture.root.join("no-hooks/pre-commit")).unwrap();
    hook(
        &fixture,
        "commit-msg",
        "#!/bin/sh\nprintf '\\nValidated-by: Rocket test\\n' >> \"$1\"\n",
    );
    success(repo.commit("Accepted", &token()).unwrap());
    assert!(
        repo.history(HistoryStart::Head, 1, &token())
            .unwrap()
            .commits[0]
            .message
            .ends_with(b"Validated-by: Rocket test\n")
    );
    assert_eq!(
        fs::read(fixture.repository.join("from-hook.txt")).unwrap(),
        b"hook worktree change\n"
    );
}

#[test]
fn missing_identity_and_signing_failure_do_not_fall_back_or_lose_staging() {
    let fixture = Fixture::new();
    fixture.write("file", "base\n");
    fixture.commit("base");
    fixture.write("file", "changed\n");
    let repo = fixture.repo();
    success(repo.stage_all(&token()).unwrap());
    let before = oid(&repo);
    fixture.run(&["config", "user.name", ""]);
    fixture.run(&["config", "user.email", ""]);
    let report = repo.commit("needs identity", &token()).unwrap();
    assert!(matches!(report.result, Err(Error::Git { .. })));
    assert_eq!(
        entry(&report.status.unwrap(), "file").index,
        Change::Modified
    );
    assert_eq!(oid(&repo), before);
    fixture.run(&["config", "user.name", "Rocket Test"]);
    fixture.run(&["config", "user.email", "rocket@example.invalid"]);
    fixture.run(&["config", "commit.gpgsign", "true"]);
    fixture.run(&["config", "gpg.format", "openpgp"]);
    fixture.run(&["config", "gpg.program", "rocket-missing-signing-helper"]);
    let report = repo.commit("must be signed", &token()).unwrap();
    assert!(matches!(report.result, Err(Error::Git { .. })));
    assert_eq!(oid(&repo), before);
    assert_eq!(
        entry(&report.status.unwrap(), "file").index,
        Change::Modified
    );
}

#[test]
fn preexisting_git_lock_is_preserved_without_retrying_or_deleting_it() {
    let fixture = Fixture::new();
    fixture.write("file", "base\n");
    fixture.commit("base");
    fixture.write("file", "changed\n");
    let repo = fixture.repo();
    let index = fs::read(repo.info().git_dir.join("index")).unwrap();
    let lock = repo.info().git_dir.join("index.lock");
    fs::write(&lock, "another client's lock").unwrap();
    let report = repo.stage_all(&token()).unwrap();
    assert!(matches!(report.result, Err(Error::Git { .. })));
    assert_eq!(
        entry(&report.status.unwrap(), "file").worktree,
        Change::Modified
    );
    assert_eq!(fs::read(&lock).unwrap(), b"another client's lock");
    assert_eq!(fs::read(repo.info().git_dir.join("index")).unwrap(), index);
}

#[test]
fn sha256_and_detached_commits_use_the_existing_repository_format_and_head() {
    let fixture = Fixture::with_format("sha256");
    fixture.write("file", "base\n");
    let repo = fixture.repo();
    success(repo.stage_all(&token()).unwrap());
    success(repo.commit("root", &token()).unwrap());
    let branch_tip = oid(&repo);
    assert_eq!(branch_tip.as_str().len(), 64);
    fixture.run(&["checkout", "--detach", "HEAD"]);
    fixture.write("file", "detached change\n");
    success(repo.stage_all(&token()).unwrap());
    let status = success(repo.commit("detached", &token()).unwrap());
    assert!(matches!(status.head, Head::Detached { .. }));
    assert_ne!(oid(&repo), branch_tip);
    assert_eq!(repo.resolve_commit("main", &token()).unwrap(), branch_tip);
}

#[test]
fn unresolved_merge_is_rejected_then_can_be_resolved_with_stage_and_commit() {
    let fixture = Fixture::new();
    fixture.write("file", "base\n");
    fixture.commit("base");
    fixture.run(&["checkout", "-b", "side"]);
    fixture.write("file", "side\n");
    fixture.commit("side");
    fixture.run(&["checkout", "main"]);
    fixture.write("file", "main\n");
    fixture.commit("main");
    assert_eq!(
        fixture
            .output(&fixture.repository, &["merge", "--no-edit", "side"])
            .status
            .code(),
        Some(1)
    );
    let repo = fixture.repo();
    let before = oid(&repo);
    let rejected = repo.commit("unresolved", &token()).unwrap();
    assert!(matches!(rejected.result, Err(Error::Git { .. })));
    assert_eq!(
        entry(&rejected.status.unwrap(), "file").kind,
        EntryKind::Unmerged
    );
    assert_eq!(oid(&repo), before);
    fixture.write("file", "resolved\n");
    success(repo.stage(&paths(&["file"]), &token()).unwrap());
    success(repo.commit("resolve merge", &token()).unwrap());
    let commit = &repo
        .history(HistoryStart::Head, 1, &token())
        .unwrap()
        .commits[0];
    assert_eq!(commit.parents.len(), 2);
    assert_eq!(tracked(&fixture, "HEAD:file"), b"resolved\n");
}

#[test]
fn writes_reject_bare_repositories_before_attempting_mutation() {
    let fixture = Fixture::new();
    let bare = fixture.root.join("bare.git");
    fixture.run(&["init", "--bare", bare.to_str().unwrap()]);
    let repo = fixture.git.open(bare, &token()).unwrap();
    assert!(matches!(
        repo.stage_all(&token()),
        Err(Error::WorktreeRequired)
    ));
    assert!(matches!(
        repo.unstage_all(&token()),
        Err(Error::WorktreeRequired)
    ));
    assert!(matches!(
        repo.commit("unsupported", &token()),
        Err(Error::WorktreeRequired)
    ));
}

#[test]
fn successful_commit_is_preserved_when_followup_status_exceeds_its_limit() {
    let fixture = Fixture::new();
    fixture.write("file", "base\n");
    fixture.commit("base");
    let full = fixture.repo();
    let before = oid(&full);
    fixture.write("file", "changed\n");
    success(full.stage_all(&token()).unwrap());
    hook(
        &fixture,
        "post-commit",
        "#!/bin/sh\nn=0\nwhile test \"$n\" -lt 80; do\n  printf 'hook file\\n' > \"untracked-$n.txt\"\n  n=$((n + 1))\ndone\n",
    );
    let client = GitClient::new(
        fixture.git.executable(),
        CommandLimits {
            stdout_bytes: 512,
            ..Default::default()
        },
    )
    .unwrap();
    let repo = client.open(&fixture.repository, &token()).unwrap();
    let report = repo.commit("confirmed write", &token()).unwrap();
    assert!(report.result.is_ok(), "{:?}", report.result);
    assert!(matches!(
        report.status,
        Err(Error::OutputLimit {
            stream: "stdout",
            ..
        })
    ));
    assert_ne!(oid(&full), before);
    assert_eq!(tracked(&fixture, "HEAD:file"), b"changed\n");
}

fn wait_file(path: &Path) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !path.is_file() {
        assert!(
            Instant::now() < deadline,
            "hook did not start: {}",
            path.display()
        );
        thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn cancelled_commit_reconciles_status_without_automatic_retry_or_rollback() {
    let fixture = Fixture::new();
    fixture.write("file", "base\n");
    fixture.commit("base");
    fixture.write("file", "changed\n");
    let repo = fixture.repo();
    success(repo.stage_all(&token()).unwrap());
    let before = oid(&repo);
    hook(
        &fixture,
        "post-commit",
        "#!/bin/sh\nprintf 'ready' > .rocket-post-ready\nsleep 30\n",
    );
    let worker_repo = repo.clone();
    let task =
        WriteTask::spawn(move |cancel| worker_repo.commit("commit before cancellation", &cancel))
            .unwrap();
    wait_file(&fixture.repository.join(".rocket-post-ready"));
    task.cancel();
    let report = task.wait().unwrap();
    assert!(matches!(report.result, Err(Error::Cancelled)));
    let after = report.status.unwrap();
    match after.head {
        Head::Branch { target, .. } => assert_ne!(target, before),
        other => panic!("expected branch: {other:?}"),
    }
    assert_eq!(
        repo.history(HistoryStart::Head, 10, &token())
            .unwrap()
            .commits
            .len(),
        2
    );
    assert_eq!(tracked(&fixture, "HEAD:file"), b"changed\n");
}

#[test]
fn linked_worktrees_and_independent_clients_share_a_cancellable_write_queue() {
    let fixture = Fixture::new();
    fixture.write("file", "base\n");
    fixture.commit("base");
    let linked = fixture.root.join("linked");
    fixture.run(&["worktree", "add", "-b", "linked", linked.to_str().unwrap()]);
    fs::write(linked.join("file"), "linked change\n").unwrap();
    fixture.write("file", "main change\n");
    let repo = fixture.repo();
    success(repo.stage_all(&token()).unwrap());
    hook(
        &fixture,
        "pre-commit",
        "#!/bin/sh\nprintf 'ready' > .rocket-ready\nwhile ! test -f .rocket-release; do sleep 0.02; done\n",
    );
    let main_repo = repo.clone();
    let commit =
        WriteTask::spawn(move |cancel| main_repo.commit("serialized commit", &cancel)).unwrap();
    wait_file(&fixture.repository.join(".rocket-ready"));
    let independent = GitClient::discover()
        .unwrap()
        .open(&linked, &token())
        .unwrap();
    let pending_repo = independent.clone();
    let (started, ready) = mpsc::channel();
    let queued = WriteTask::spawn(move |cancel| {
        started.send(()).unwrap();
        pending_repo.stage(&paths(&["file"]), &cancel)
    })
    .unwrap();
    ready.recv_timeout(Duration::from_secs(5)).unwrap();
    queued.cancel();
    assert!(matches!(queued.wait(), Err(Error::Cancelled)));
    assert_eq!(
        entry(&independent.status(&token()).unwrap(), "file").index,
        Change::Unchanged
    );
    let staged_repo = independent.clone();
    let mut staged =
        WriteTask::spawn(move |cancel| staged_repo.stage(&paths(&["file"]), &cancel)).unwrap();
    // Main commit holds the shared gate. A read observes that the other index is unchanged.
    assert_eq!(
        entry(&independent.status(&token()).unwrap(), "file").index,
        Change::Unchanged
    );
    assert!(staged.try_finish().is_none());
    fs::write(fixture.repository.join(".rocket-release"), "release").unwrap();
    success(commit.wait().unwrap());
    let linked_status = success(staged.wait().unwrap());
    assert_eq!(entry(&linked_status, "file").index, Change::Modified);
    assert_eq!(tracked(&fixture, "HEAD:file"), b"main change\n");
}

#[cfg(unix)]
#[test]
fn write_paths_preserve_newlines_non_utf8_and_pathspec_magic_characters() {
    use std::os::unix::ffi::OsStringExt;
    let fixture = Fixture::new();
    let raw = b":(glob) literal\n\xff";
    let name = std::ffi::OsString::from_vec(raw.to_vec());
    fs::write(fixture.repository.join(&name), b"content").unwrap();
    let repo = fixture.repo();
    let selected = vec![RepoPath::new(raw.to_vec()).unwrap()];
    success(repo.stage(&selected, &token()).unwrap());
    let status = success(repo.unstage(&selected, &token()).unwrap());
    assert_eq!(status.entries[0].path.as_bytes(), raw);
    assert_eq!(fs::read(fixture.repository.join(name)).unwrap(), b"content");
}
