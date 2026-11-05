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
fn oid(repo: &Repository) -> ObjectId {
    repo.resolve_commit("HEAD", &token()).unwrap()
}
fn head_name(head: &Head) -> &[u8] {
    match head {
        Head::Branch { name, .. } => name,
        Head::Unborn { branch } => branch,
        Head::Detached { .. } => panic!("unexpected detached HEAD"),
    }
}
fn success(report: BranchWriteReport) -> (Status, Vec<Branch>) {
    report.write.result.unwrap();
    (report.write.status.unwrap(), report.branches.unwrap())
}
fn rejected(report: BranchWriteReport) -> (Status, Vec<Branch>) {
    assert!(
        matches!(report.write.result, Err(Error::Git { .. })),
        "{:?}",
        report.write.result
    );
    (report.write.status.unwrap(), report.branches.unwrap())
}
fn branch<'a>(branches: &'a [Branch], name: &[u8]) -> &'a Branch {
    let full = [b"refs/heads/".as_slice(), name].concat();
    branches
        .iter()
        .find(|item| item.reference == full)
        .unwrap_or_else(|| panic!("missing {name:?}: {branches:?}"))
}
fn base() -> Fixture {
    let fixture = Fixture::new();
    fixture.write("file", "base\n");
    fixture.commit("base");
    fixture
}
fn hook(fixture: &Fixture, name: &str, script: &str) {
    let path = fixture.root.join("no-hooks").join(name);
    fs::write(&path, script).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
    }
}
fn wait_file(path: &Path) {
    let deadline = Instant::now() + Duration::from_secs(15);
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
fn a_remote_branch_checks_out_as_a_tracking_local_branch_and_never_replaces_one() {
    let fixture = base();
    let repo = fixture.repo();
    let base = oid(&repo);
    fixture.run(&["switch", "-c", "topic"]);
    fixture.write("file", "topic\n");
    fixture.commit("topic");
    let topic = oid(&repo);
    fixture.run(&["switch", "-"]);
    // A remote-tracking ref as a fetch would leave it. Git only tracks refs a remote's
    // fetch refspec maps, so the remote is configured; nothing contacts it.
    fixture.run(&["remote", "add", "origin", "https://example.invalid/rocket.git"]);
    fixture.run(&["update-ref", "refs/remotes/origin/topic", topic.as_str()]);
    fixture.run(&["branch", "-D", "topic"]);
    let before = repo.branches(&token()).unwrap();
    assert!(before.iter().all(|b| b.reference != b"refs/heads/topic"));
    let (status, branches) = success(repo.check_out_remote_branch("topic", b"refs/remotes/origin/topic", &token()).unwrap());
    assert_eq!(head_name(&status.head), b"topic");
    assert_eq!(oid(&repo), topic);
    let local = branch(&branches, b"topic");
    assert_eq!(local.upstream.as_deref(), Some(b"refs/remotes/origin/topic".as_slice()));
    assert_eq!(local.ahead_behind, Some((0, 0)));
    assert!(local.committed.is_some_and(|t| t > 0));
    // An existing local branch stays as it was, and so does the checkout.
    fixture.run(&["switch", "-c", "main-copy", base.as_str()]);
    let report = repo.check_out_remote_branch("topic", b"refs/remotes/origin/topic", &token());
    assert!(report.is_err() || report.as_ref().is_ok_and(|r| r.write.result.is_err()), "{report:?}");
    assert_eq!(oid(&repo), base);
    // Only remote-tracking refs are checked out this way.
    assert!(repo.check_out_remote_branch("other", b"refs/heads/topic", &token()).is_err());
}

/// A refused upstream or switch deletes the new branch again, leaving HEAD, files and
/// config as they were, so the same name can be tried again.
#[test]
fn a_refused_remote_checkout_leaves_no_branch_behind() {
    let fixture = base();
    let repo = fixture.repo();
    let base = oid(&repo);
    fixture.run(&["switch", "-c", "topic"]);
    fixture.write("file", "topic\n");
    fixture.commit("topic");
    let topic = oid(&repo);
    fixture.run(&["switch", "-"]);
    fixture.run(&["branch", "-D", "topic"]);
    fixture.run(&["remote", "add", "origin", "https://example.invalid/rocket.git"]);
    fixture.run(&["update-ref", "refs/remotes/origin/topic", topic.as_str()]);
    let check_out = || rejected(repo.check_out_remote_branch("topic", b"refs/remotes/origin/topic", &token()).unwrap());
    let unchanged = |(status, branches): (Status, Vec<Branch>), file: &str| {
        assert!(branches.iter().all(|b| b.reference != b"refs/heads/topic"), "the new branch is gone");
        assert_ne!(head_name(&status.head), b"topic");
        assert_eq!(oid(&repo), base);
        assert_eq!(fs::read_to_string(fixture.repository.join("file")).unwrap(), file);
        let config = fixture.output(&fixture.repository, &["config", "--get-regexp", r"^branch\.topic\."]);
        assert!(!config.status.success(), "its upstream is gone too: {:?}", String::from_utf8_lossy(&config.stdout));
    };
    // No remote's fetch refspec maps the ref: Git refuses the upstream.
    fixture.run(&["config", "remote.origin.fetch", "+refs/heads/main:refs/remotes/origin/main"]);
    unchanged(check_out(), "base\n");
    // Two remotes map it.
    fixture.run(&["config", "remote.origin.fetch", "+refs/heads/*:refs/remotes/origin/*"]);
    fixture.run(&["remote", "add", "mirror", "https://example.invalid/mirror.git"]);
    fixture.run(&["config", "remote.mirror.fetch", "+refs/heads/*:refs/remotes/origin/*"]);
    unchanged(check_out(), "base\n");
    fixture.run(&["config", "--unset", "remote.mirror.fetch"]);
    // A local change the switch would overwrite: the branch and its upstream go again.
    fixture.write("file", "local\n");
    unchanged(check_out(), "local\n");
    // With the way clear, the same name works.
    fixture.write("file", "base\n");
    let (status, branches) = success(repo.check_out_remote_branch("topic", b"refs/remotes/origin/topic", &token()).unwrap());
    assert_eq!(head_name(&status.head), b"topic");
    assert_eq!(branch(&branches, b"topic").upstream.as_deref(), Some(b"refs/remotes/origin/topic".as_slice()));
}

#[test]
fn create_from_head_or_a_commit_preserves_checkout_index_and_working_files() {
    let fixture = base();
    let repo = fixture.repo();
    let first = oid(&repo);
    fixture.write("file", "second\n");
    fixture.commit("second");
    let current = oid(&repo);
    fixture.write("file", "staged\n");
    fixture.run(&["add", "--", "file"]);
    fixture.write("file", "working\n");
    let index = fs::read(repo.info().git_dir.join("index")).unwrap();
    fixture.run(&["config", "branch.autoSetupMerge", "always"]);
    let report = repo.create_branch("feature/文", None, &token()).unwrap();
    assert_eq!(report.write.operation, WriteOperation::CreateBranch);
    let (status, branches) = success(report);
    assert_eq!(head_name(&status.head), b"main");
    assert_eq!(branch(&branches, "feature/文".as_bytes()).target, current);
    assert!(
        branch(&branches, "feature/文".as_bytes())
            .upstream
            .is_none()
    );
    fixture.run(&["tag", "old-start", first.as_str()]);
    let (_, branches) = success(
        repo.create_branch("older", Some("old-start"), &token())
            .unwrap(),
    );
    assert_eq!(branch(&branches, b"older").target, first);
    assert_eq!(oid(&repo), current);
    assert_eq!(fixture.run(&["show", ":file"]), b"staged\n");
    assert_eq!(
        fs::read(fixture.repository.join("file")).unwrap(),
        b"working\n"
    );
    assert_eq!(fs::read(repo.info().git_dir.join("index")).unwrap(), index);
    rejected(
        repo.create_branch("feature/文", Some(first.as_str()), &token())
            .unwrap(),
    );
    assert_eq!(
        repo.resolve_commit("refs/heads/feature/文", &token())
            .unwrap(),
        current
    );
    assert!(
        repo.create_branch("bad-start", Some("missing-revision"), &token())
            .is_err()
    );
    assert!(
        !repo
            .branches(&token())
            .unwrap()
            .iter()
            .any(|item| item.reference == b"refs/heads/bad-start")
    );
}

#[test]
fn switching_updates_head_and_files_and_preserves_compatible_staging() {
    let fixture = base();
    fixture.write("unchanged-on-both", "original\n");
    fixture.commit("common file");
    fixture.run(&["checkout", "-b", "feature"]);
    fixture.write("file", "feature\n");
    fixture.commit("feature");
    fixture.run(&["checkout", "main"]);
    fixture.write("unchanged-on-both", "staged edit\n");
    fixture.run(&["add", "--", "unchanged-on-both"]);
    fixture.write("unchanged-on-both", "later edit\n");
    fixture.write("untracked", "keep\n");
    let repo = fixture.repo();
    let report = repo.switch_branch("feature", &token()).unwrap();
    assert_eq!(report.write.operation, WriteOperation::SwitchBranch);
    let (status, branches) = success(report);
    assert_eq!(head_name(&status.head), b"feature");
    assert!(branch(&branches, b"feature").is_head);
    assert!(!branch(&branches, b"main").is_head);
    assert_eq!(
        fs::read(fixture.repository.join("file")).unwrap(),
        b"feature\n"
    );
    assert_eq!(
        fixture.run(&["show", ":unchanged-on-both"]),
        b"staged edit\n"
    );
    assert_eq!(
        fs::read(fixture.repository.join("unchanged-on-both")).unwrap(),
        b"later edit\n"
    );
    assert_eq!(
        fs::read(fixture.repository.join("untracked")).unwrap(),
        b"keep\n"
    );
}

#[test]
fn switching_refuses_to_overwrite_tracked_staged_or_untracked_content() {
    let fixture = base();
    fixture.run(&["checkout", "-b", "feature"]);
    fixture.write("file", "feature\n");
    fixture.write("new-on-feature", "tracked on feature\n");
    fixture.commit("feature");
    fixture.run(&["checkout", "main"]);
    let repo = fixture.repo();
    let before = oid(&repo);
    fixture.write("file", "my local edit\n");
    let (status, _) = rejected(repo.switch_branch("feature", &token()).unwrap());
    assert_eq!(head_name(&status.head), b"main");
    assert_eq!(
        fs::read(fixture.repository.join("file")).unwrap(),
        b"my local edit\n"
    );
    fixture.run(&["add", "--", "file"]);
    rejected(repo.switch_branch("feature", &token()).unwrap());
    assert_eq!(fixture.run(&["show", ":file"]), b"my local edit\n");
    // Restore only the test's selected file before testing an untracked collision.
    fixture.run(&[
        "restore",
        "--source=HEAD",
        "--staged",
        "--worktree",
        "--",
        "file",
    ]);
    fixture.write("new-on-feature", "my untracked file\n");
    rejected(repo.switch_branch("feature", &token()).unwrap());
    assert_eq!(
        fs::read(fixture.repository.join("new-on-feature")).unwrap(),
        b"my untracked file\n"
    );
    assert_eq!(oid(&repo), before);
    assert_eq!(head_name(&repo.head(&token()).unwrap()), b"main");
}

#[test]
fn switching_preserves_ignored_files_that_collide_with_the_target_branch() {
    let fixture = base();
    fixture.run(&["checkout", "-b", "feature"]);
    fixture.write("local.config", "tracked on feature\n");
    fixture.commit("feature config");
    fixture.run(&["checkout", "main"]);
    fixture.write(".gitignore", "local.config\n");
    fixture.commit("ignore local configuration");
    fixture.write("local.config", "private local configuration\n");
    let repo = fixture.repo();
    let (status, _) = rejected(repo.switch_branch("feature", &token()).unwrap());
    assert_eq!(head_name(&status.head), b"main");
    assert_eq!(
        fs::read(fixture.repository.join("local.config")).unwrap(),
        b"private local configuration\n"
    );
}

#[test]
fn switching_keeps_submodule_head_and_edits_even_when_recursion_is_configured() {
    let component = base();
    component.write("file", "new component\n");
    component.commit("component second");
    let fixture = base();
    fixture.run(&[
        "-c",
        "protocol.file.allow=always",
        "submodule",
        "add",
        component.repository.to_str().unwrap(),
        "module",
    ]);
    fixture.commit("new component pointer");
    fixture.run(&["checkout", "-b", "old"]);
    let module = fixture.repository.join("module");
    fixture.run_at(&module, &["checkout", "--detach", "HEAD~1"]);
    fixture.commit("old component pointer");
    fixture.run(&["config", "submodule.recurse", "true"]);
    let head_before = fixture.run_at(&module, &["rev-parse", "HEAD"]);
    fs::write(module.join("file"), b"local component edit\n").unwrap();
    let repo = fixture.repo();
    let (status, _) = success(repo.switch_branch("main", &token()).unwrap());
    assert_eq!(head_name(&status.head), b"main");
    assert_eq!(fixture.run_at(&module, &["rev-parse", "HEAD"]), head_before);
    assert_eq!(
        fs::read(module.join("file")).unwrap(),
        b"local component edit\n"
    );
    let state = status
        .entries
        .iter()
        .find(|entry| entry.path.as_bytes() == b"module")
        .unwrap()
        .submodule
        .as_ref()
        .unwrap();
    assert!(state.commit_changed && state.tracked_changes);
}

#[test]
fn branch_mutations_do_not_propagate_into_submodules_when_configured() {
    let component = base();
    let fixture = base();
    fixture.run(&[
        "-c",
        "protocol.file.allow=always",
        "submodule",
        "add",
        component.repository.to_str().unwrap(),
        "module",
    ]);
    fixture.commit("component pointer");
    fixture.run(&["config", "submodule.recurse", "true"]);
    fixture.run(&["config", "submodule.propagateBranches", "true"]);
    let module = fixture.repository.join("module");
    let before = fixture.run_at(
        &module,
        &["for-each-ref", "--format=%(refname) %(objectname)"],
    );
    let repo = fixture.repo();
    success(repo.create_branch("parent-only", None, &token()).unwrap());
    assert_eq!(
        fixture.run_at(
            &module,
            &["for-each-ref", "--format=%(refname) %(objectname)"]
        ),
        before
    );
    success(
        repo.rename_branch("parent-only", "renamed-parent", &token())
            .unwrap(),
    );
    success(repo.delete_branch("renamed-parent", &token()).unwrap());
    assert_eq!(
        fixture.run_at(
            &module,
            &["for-each-ref", "--format=%(refname) %(objectname)"]
        ),
        before
    );
    assert_eq!(fs::read(module.join("file")).unwrap(), b"base\n");
    assert_eq!(
        fixture.run(&["config", "--get", "submodule.recurse"]),
        b"true\n"
    );
    assert_eq!(
        fixture.run(&["config", "--get", "submodule.propagateBranches"]),
        b"true\n"
    );
}

#[test]
fn switching_does_not_guess_remote_branches_or_detach_at_a_tag() {
    let fixture = base();
    fixture.run(&["update-ref", "refs/remotes/origin/remote-only", "HEAD"]);
    fixture.run(&["config", "remote.origin.url", "."]);
    fixture.run(&[
        "config",
        "remote.origin.fetch",
        "+refs/heads/*:refs/remotes/origin/*",
    ]);
    fixture.run(&["tag", "tag-only"]);
    let repo = fixture.repo();
    let before = repo.branches(&token()).unwrap();
    for name in ["remote-only", "tag-only", "missing"] {
        let (status, branches) = rejected(repo.switch_branch(name, &token()).unwrap());
        assert_eq!(head_name(&status.head), b"main");
        assert_eq!(branches, before);
    }
}

#[test]
fn rename_preserves_current_head_config_reflog_and_rejects_an_existing_destination() {
    let fixture = base();
    fixture.run(&["branch", "occupied"]);
    fixture.run(&["config", "branch.main.description", "keep branch metadata"]);
    fixture.run(&["config", "branch.main.remote", "."]);
    fixture.run(&["config", "branch.main.merge", "refs/heads/occupied"]);
    fixture.write("file", "local edit\n");
    let repo = fixture.repo();
    let before = oid(&repo);
    rejected(repo.rename_branch("main", "occupied", &token()).unwrap());
    assert_eq!(head_name(&repo.head(&token()).unwrap()), b"main");
    let report = repo.rename_branch("main", "renamed/文", &token()).unwrap();
    assert_eq!(report.write.operation, WriteOperation::RenameBranch);
    let (status, branches) = success(report);
    assert_eq!(head_name(&status.head), "renamed/文".as_bytes());
    assert_eq!(branch(&branches, "renamed/文".as_bytes()).target, before);
    assert_eq!(
        branch(&branches, "renamed/文".as_bytes())
            .upstream
            .as_deref(),
        Some(b"refs/heads/occupied".as_slice())
    );
    assert!(
        !branches
            .iter()
            .any(|item| item.reference == b"refs/heads/main")
    );
    assert_eq!(
        fixture.run(&["config", "--get", "branch.renamed/文.description"]),
        b"keep branch metadata\n"
    );
    assert!(
        fixture
            .run(&["reflog", "show", "--format=%gs", "renamed/文"])
            .windows(b"Branch: renamed".len())
            .any(|part| part == b"Branch: renamed")
    );
    assert_eq!(
        fs::read(fixture.repository.join("file")).unwrap(),
        b"local edit\n"
    );
    rejected(repo.rename_branch("missing", "other", &token()).unwrap());
    assert_eq!(oid(&repo), before);
}

#[test]
fn delete_refuses_current_and_unmerged_branches_and_removes_only_the_selected_local_ref() {
    let fixture = base();
    fixture.run(&["branch", "merged"]);
    fixture.run(&["tag", "merged"]);
    fixture.run(&["update-ref", "refs/remotes/origin/merged", "HEAD"]);
    fixture.run(&["checkout", "-b", "unmerged"]);
    fixture.write("file", "unmerged\n");
    fixture.commit("unmerged");
    fixture.run(&["checkout", "main"]);
    let repo = fixture.repo();
    let before = repo.branches(&token()).unwrap();
    rejected(repo.delete_branch("main", &token()).unwrap());
    rejected(repo.delete_branch("unmerged", &token()).unwrap());
    assert_eq!(repo.branches(&token()).unwrap(), before);
    let report = repo.delete_branch("merged", &token()).unwrap();
    assert_eq!(report.write.operation, WriteOperation::DeleteBranch);
    let (status, branches) = success(report);
    assert_eq!(head_name(&status.head), b"main");
    assert!(
        !branches
            .iter()
            .any(|item| item.reference == b"refs/heads/merged")
    );
    assert!(
        branches
            .iter()
            .any(|item| item.reference == b"refs/remotes/origin/merged")
    );
    assert_eq!(
        repo.resolve_commit("refs/tags/merged", &token()).unwrap(),
        oid(&repo)
    );
    rejected(repo.delete_branch("merged", &token()).unwrap());
    assert_eq!(
        fs::read(fixture.repository.join("file")).unwrap(),
        b"base\n"
    );
}

#[test]
fn nonforced_deletion_obeys_the_configured_upstream_merge_check() {
    let fixture = base();
    fixture.run(&["checkout", "-b", "feature"]);
    fixture.write("file", "feature\n");
    fixture.commit("feature");
    fixture.run(&["branch", "upstream"]);
    fixture.run(&["branch", "protected"]);
    fixture.run(&["config", "branch.feature.remote", "."]);
    fixture.run(&["config", "branch.feature.merge", "refs/heads/upstream"]);
    fixture.run(&["config", "branch.protected.remote", "."]);
    fixture.run(&["config", "branch.protected.merge", "refs/heads/main"]);
    fixture.run(&["checkout", "main"]);
    let repo = fixture.repo();
    // Feature is contained in its upstream, even though HEAD does not contain it.
    success(repo.delete_branch("feature", &token()).unwrap());
    success(repo.switch_branch("upstream", &token()).unwrap());
    // HEAD contains protected, but its configured upstream does not.
    rejected(repo.delete_branch("protected", &token()).unwrap());
    assert_eq!(
        repo.resolve_commit("protected", &token()).unwrap(),
        oid(&repo)
    );
}

#[test]
fn linked_worktrees_block_switch_and_delete_while_rename_updates_their_head() {
    let fixture = base();
    let linked = fixture.root.join("linked");
    fixture.run(&["worktree", "add", "-b", "linked", linked.to_str().unwrap()]);
    fs::write(linked.join("file"), b"linked local edit\n").unwrap();
    let repo = fixture.repo();
    rejected(repo.switch_branch("linked", &token()).unwrap());
    rejected(repo.delete_branch("linked", &token()).unwrap());
    let (status, branches) = success(
        repo.rename_branch("linked", "linked-renamed", &token())
            .unwrap(),
    );
    assert_eq!(head_name(&status.head), b"main");
    assert_eq!(
        branch(&branches, b"linked-renamed")
            .checked_out_at
            .as_ref()
            .unwrap()
            .canonicalize()
            .unwrap(),
        linked.canonicalize().unwrap()
    );
    let linked_repo = fixture.git.open(&linked, &token()).unwrap();
    assert_eq!(
        head_name(&linked_repo.head(&token()).unwrap()),
        b"linked-renamed"
    );
    assert_eq!(
        fs::read(linked.join("file")).unwrap(),
        b"linked local edit\n"
    );
    rejected(repo.delete_branch("linked-renamed", &token()).unwrap());
}

#[test]
fn invalid_names_and_pre_cancelled_calls_leave_refs_index_and_content_unchanged() {
    let fixture = base();
    fixture.write("file", "local edit\n");
    let repo = fixture.repo();
    let before = repo.branches(&token()).unwrap();
    let index = fs::read(repo.info().git_dir.join("index")).unwrap();
    let long = vec![b'x'; 4097];
    for name in [
        b"".as_slice(),
        b"--force",
        b"-",
        b"HEAD",
        b"@",
        b"refs/heads/main",
        b"@{-1}",
        b"bad name",
        b"topic..x",
        b"bad\0name",
        b"topic.lock",
        b"bad/[glob]",
        b"topic/",
        b"a\\b",
        &long,
    ] {
        assert!(matches!(
            repo.create_branch(name, None, &token()),
            Err(Error::InvalidInput(_))
        ));
        assert!(matches!(
            repo.switch_branch(name, &token()),
            Err(Error::InvalidInput(_))
        ));
        assert!(matches!(
            repo.rename_branch("main", name, &token()),
            Err(Error::InvalidInput(_))
        ));
        assert!(matches!(
            repo.rename_branch(name, "valid", &token()),
            Err(Error::InvalidInput(_))
        ));
        assert!(matches!(
            repo.delete_branch(name, &token()),
            Err(Error::InvalidInput(_))
        ));
    }
    let cancel = token();
    cancel.cancel();
    assert!(matches!(
        repo.create_branch("new", None, &cancel),
        Err(Error::Cancelled)
    ));
    assert!(matches!(
        repo.switch_branch("main", &cancel),
        Err(Error::Cancelled)
    ));
    assert!(matches!(
        repo.rename_branch("main", "new", &cancel),
        Err(Error::Cancelled)
    ));
    assert!(matches!(
        repo.delete_branch("main", &cancel),
        Err(Error::Cancelled)
    ));
    assert_eq!(repo.branches(&token()).unwrap(), before);
    assert_eq!(fs::read(repo.info().git_dir.join("index")).unwrap(), index);
    assert_eq!(
        fs::read(fixture.repository.join("file")).unwrap(),
        b"local edit\n"
    );
}

#[test]
fn unborn_head_requires_a_commit_for_creation_and_can_be_renamed_without_losing_staging() {
    let fixture = Fixture::new();
    fixture.write("file", "first staged version\n");
    fixture.run(&["add", "--", "file"]);
    fixture.write("file", "later working version\n");
    let repo = fixture.repo();
    assert!(repo.create_branch("feature", None, &token()).is_err());
    rejected(repo.switch_branch("missing", &token()).unwrap());
    let (status, branches) = success(repo.rename_branch("main", "first", &token()).unwrap());
    assert_eq!(
        status.head,
        Head::Unborn {
            branch: b"first".to_vec()
        }
    );
    assert!(branches.is_empty());
    assert_eq!(fixture.run(&["show", ":file"]), b"first staged version\n");
    assert_eq!(
        fs::read(fixture.repository.join("file")).unwrap(),
        b"later working version\n"
    );
    rejected(repo.delete_branch("first", &token()).unwrap());
}

#[test]
fn detached_sha256_head_can_create_a_branch_and_switch_to_it() {
    let fixture = Fixture::with_format("sha256");
    fixture.write("file", "base\n");
    fixture.commit("base");
    fixture.run(&["checkout", "--detach", "HEAD"]);
    let repo = fixture.repo();
    let before = oid(&repo);
    let (status, branches) = success(repo.create_branch("feature", None, &token()).unwrap());
    assert!(matches!(status.head, Head::Detached { .. }));
    assert_eq!(branch(&branches, b"feature").target, before);
    assert_eq!(before.as_str().len(), 64);
    let (status, _) = success(repo.switch_branch("feature", &token()).unwrap());
    assert_eq!(head_name(&status.head), b"feature");
    assert_eq!(oid(&repo), before);
}

#[test]
fn bare_repositories_reject_all_branch_writes_before_mutation() {
    let fixture = base();
    let bare = fixture.root.join("bare.git");
    fixture.run(&[
        "clone",
        "--bare",
        fixture.repository.to_str().unwrap(),
        bare.to_str().unwrap(),
    ]);
    let repo = fixture.git.open(&bare, &token()).unwrap();
    let before = repo.branches(&token()).unwrap();
    assert!(matches!(
        repo.create_branch("new", None, &token()),
        Err(Error::WorktreeRequired)
    ));
    assert!(matches!(
        repo.switch_branch("main", &token()),
        Err(Error::WorktreeRequired)
    ));
    assert!(matches!(
        repo.rename_branch("main", "new", &token()),
        Err(Error::WorktreeRequired)
    ));
    assert!(matches!(
        repo.delete_branch("main", &token()),
        Err(Error::WorktreeRequired)
    ));
    assert_eq!(repo.branches(&token()).unwrap(), before);
}

#[test]
fn failing_post_checkout_hook_reports_the_already_completed_switch() {
    let fixture = base();
    fixture.run(&["branch", "feature"]);
    hook(
        &fixture,
        "post-checkout",
        "#!/bin/sh\nprintf 'hook edit\\n' > from-hook\nprintf 'post checkout failed\\n' >&2\nexit 1\n",
    );
    let repo = fixture.repo();
    let (status, branches) = rejected(repo.switch_branch("feature", &token()).unwrap());
    assert_eq!(head_name(&status.head), b"feature");
    assert!(branch(&branches, b"feature").is_head);
    assert!(
        status
            .entries
            .iter()
            .any(|entry| entry.path.as_bytes() == b"from-hook")
    );
    assert_eq!(
        fs::read(fixture.repository.join("from-hook")).unwrap(),
        b"hook edit\n"
    );
}

#[test]
fn cancelling_post_checkout_reconciles_head_and_branches_without_retry_or_rollback() {
    let fixture = base();
    fixture.run(&["branch", "feature"]);
    hook(
        &fixture,
        "post-checkout",
        "#!/bin/sh\nprintf 'ready' > .rocket-ready\nsleep 30\n",
    );
    let repo = fixture.repo();
    let worker = repo.clone();
    let task = WriteTask::spawn(move |cancel| worker.switch_branch("feature", &cancel)).unwrap();
    wait_file(&fixture.repository.join(".rocket-ready"));
    task.cancel();
    let report = task.wait().unwrap();
    assert!(matches!(report.write.result, Err(Error::Cancelled)));
    assert_eq!(head_name(&report.write.status.unwrap().head), b"feature");
    assert!(branch(&report.branches.unwrap(), b"feature").is_head);
    let reflog = fixture.run(&["reflog", "--format=%gs", "HEAD"]);
    assert_eq!(
        reflog
            .split(|byte| *byte == b'\n')
            .filter(|line| line.starts_with(b"checkout:"))
            .count(),
        1
    );
}

#[test]
fn successful_creation_survives_branch_list_refresh_failure() {
    let fixture = base();
    for index in 0..12 {
        fixture.run(&["branch", &format!("other-{index}")]);
    }
    let client = GitClient::new(
        fixture.git.executable(),
        CommandLimits {
            stdout_bytes: 512,
            ..Default::default()
        },
    )
    .unwrap();
    let repo = client.open(&fixture.repository, &token()).unwrap();
    let report = repo.create_branch("created", None, &token()).unwrap();
    assert!(report.write.result.is_ok());
    assert_eq!(head_name(&report.write.status.unwrap().head), b"main");
    assert!(matches!(
        report.branches,
        Err(Error::OutputLimit {
            stream: "stdout",
            ..
        })
    ));
    assert_eq!(
        repo.resolve_commit("created", &token()).unwrap(),
        oid(&repo)
    );
}

#[test]
fn branch_list_is_refreshed_even_when_status_refresh_fails() {
    let fixture = base();
    fixture.run(&["branch", "feature"]);
    hook(
        &fixture,
        "post-checkout",
        "#!/bin/sh\nn=0\nwhile test \"$n\" -lt 80; do\n  printf 'hook file\\n' > \"untracked-$n\"\n  n=$((n + 1))\ndone\n",
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
    let report = repo.switch_branch("feature", &token()).unwrap();
    assert!(report.write.result.is_ok());
    assert!(matches!(
        report.write.status,
        Err(Error::OutputLimit {
            stream: "stdout",
            ..
        })
    ));
    assert!(branch(&report.branches.unwrap(), b"feature").is_head);
}

#[test]
fn branch_operations_share_the_commit_queue_across_independent_clients() {
    let fixture = base();
    fixture.write("file", "commit edit\n");
    let repo = fixture.repo();
    repo.stage_all(&token()).unwrap().result.unwrap();
    hook(
        &fixture,
        "post-commit",
        "#!/bin/sh\nprintf 'ready' > .rocket-ready\nwhile ! test -f .rocket-release; do sleep 0.02; done\n",
    );
    let writer = repo.clone();
    let commit = WriteTask::spawn(move |cancel| writer.commit("queue owner", &cancel)).unwrap();
    wait_file(&fixture.repository.join(".rocket-ready"));
    let independent = GitClient::discover()
        .unwrap()
        .open(&fixture.repository, &token())
        .unwrap();
    let pending = independent.clone();
    let (started, ready) = mpsc::channel();
    let queued = WriteTask::spawn(move |cancel| {
        started.send(()).unwrap();
        pending.create_branch("cancelled", None, &cancel)
    })
    .unwrap();
    ready.recv_timeout(Duration::from_secs(5)).unwrap();
    queued.cancel();
    assert!(matches!(queued.wait(), Err(Error::Cancelled)));
    let pending = independent.clone();
    let (finished, completion) = mpsc::channel();
    let create = WriteTask::spawn(move |cancel| {
        let result = pending.create_branch("after-commit", None, &cancel);
        finished.send(()).unwrap();
        result
    })
    .unwrap();
    assert_eq!(
        completion.recv_timeout(Duration::from_secs(1)),
        Err(mpsc::RecvTimeoutError::Timeout)
    );
    assert!(
        !independent
            .branches(&token())
            .unwrap()
            .iter()
            .any(|item| item.reference == b"refs/heads/after-commit"
                || item.reference == b"refs/heads/cancelled")
    );
    fs::write(fixture.repository.join(".rocket-release"), b"release").unwrap();
    commit.wait().unwrap().result.unwrap();
    let (_, branches) = success(create.wait().unwrap());
    assert_eq!(branch(&branches, b"after-commit").target, oid(&repo));
}

#[cfg(unix)]
#[test]
fn non_utf8_branch_names_round_trip_without_lossy_conversion() {
    let fixture = base();
    let repo = fixture.repo();
    let raw = b"feature/\xff";
    success(repo.create_branch(raw, None, &token()).unwrap());
    let (status, _) = success(repo.switch_branch(raw, &token()).unwrap());
    assert_eq!(head_name(&status.head), raw);
    success(repo.rename_branch(raw, b"renamed/\xfe", &token()).unwrap());
    success(repo.switch_branch("main", &token()).unwrap());
    success(repo.delete_branch(b"renamed/\xfe", &token()).unwrap());
}
