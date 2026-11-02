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
fn base() -> Fixture {
    let fixture = Fixture::new();
    fixture.write("file", b"base\n");
    fixture.commit("base");
    fixture
}
fn id(repo: &Repository, revision: &str) -> ObjectId {
    repo.resolve_commit(revision, &token()).unwrap()
}
fn stash_ok(report: StashWriteReport) -> Vec<StashEntry> {
    report.write.result.unwrap();
    report.write.status.unwrap();
    report.stashes.unwrap()
}

#[test]
fn stash_previews_are_pinned_read_only_and_separate_index_untracked_and_binary() {
    for format in ["sha1", "sha256"] {
        let f = Fixture::with_format(format);
        f.write("解析.rs", "base\n");
        f.write("image.bin", b"base\0bytes");
        f.write(".gitignore", "ignored\n");
        f.commit("base");
        f.write("解析.rs", "saved index\n");
        f.run(&["add", "解析.rs"]);
        f.write("解析.rs", "saved worktree\n");
        f.write("image.bin", b"changed\0bytes");
        f.write("新 文.txt", "saved untracked\n");
        f.write("ignored", "preserved\n");
        let repo = f.repo();
        let entry = stash_ok(
            repo.push_stash(Some("preview fixture"), true, &token())
                .unwrap(),
        )
        .remove(0);
        let before = repo.status(&token()).unwrap();
        let index = fs::read(f.repository.join(".git/index")).unwrap();
        let tracked = repo
            .stash_diff(&entry, StashPart::Tracked, &token())
            .unwrap();
        let tracked = String::from_utf8_lossy(&tracked.patch);
        assert!(tracked.contains("+saved worktree"));
        assert!(tracked.contains("Binary files"));
        assert!(!tracked.contains("saved untracked"));
        let staged = repo.stash_diff(&entry, StashPart::Index, &token()).unwrap();
        assert!(String::from_utf8_lossy(&staged.patch).contains("+saved index"));
        let untracked = repo
            .stash_diff(&entry, StashPart::Untracked, &token())
            .unwrap();
        assert!(String::from_utf8_lossy(&untracked.patch).contains("+saved untracked"));
        assert_eq!(repo.status(&token()).unwrap(), before);
        assert_eq!(fs::read(f.repository.join(".git/index")).unwrap(), index);
        assert_eq!(
            fs::read(f.repository.join("ignored")).unwrap(),
            b"preserved\n"
        );
        f.write("解析.rs", "newer stash\n");
        stash_ok(repo.push_stash(None, false, &token()).unwrap());
        assert_eq!(
            repo.stash_diff(&entry, StashPart::Untracked, &token())
                .unwrap(),
            untracked
        );
        assert!(repo.drop_stash(&entry, &token()).is_err());
        let other = base();
        assert!(
            other
                .repo()
                .stash_diff(&entry, StashPart::Tracked, &token())
                .is_err()
        );
        let cancel = token();
        cancel.cancel();
        assert!(matches!(
            repo.stash_diff(&entry, StashPart::Index, &cancel),
            Err(Error::Cancelled)
        ));
        let latest = repo.stashes(&token()).unwrap().remove(0);
        assert!(
            repo.stash_diff(&latest, StashPart::Untracked, &token())
                .unwrap()
                .patch
                .is_empty()
        );
    }
}
fn merge_ok(report: MergeWriteReport) -> Status {
    report.write.result.unwrap();
    assert!(report.state.unwrap().is_idle());
    report.branches.unwrap();
    report.write.status.unwrap()
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
    // Allow the runner's own command timeout to diagnose a stalled Git operation;
    // concurrent fixture setup must not expire a shorter hook handshake first.
    let until = Instant::now() + CommandLimits::default().timeout;
    while !path.exists() {
        assert!(
            Instant::now() < until,
            "hook did not reach {}",
            path.display()
        );
        thread::sleep(Duration::from_millis(10));
    }
}
fn diverge(fixture: &Fixture, conflict: bool) {
    fixture.run(&["switch", "-c", "feature"]);
    fixture.write(if conflict { "file" } else { "feature-file" }, "feature\n");
    fixture.commit("feature");
    fixture.run(&["switch", "main"]);
    fixture.write(if conflict { "file" } else { "main-file" }, "main\n");
    fixture.commit("main");
}

#[test]
fn stash_round_trip_preserves_staging_binary_untracked_and_ignored_files_in_both_formats() {
    for format in ["sha1", "sha256"] {
        let fixture = Fixture::with_format(format);
        fixture.write("file", b"base\n");
        fixture.write(".gitignore", b"ignored\n");
        fixture.commit("base");
        let repo = fixture.repo();
        fixture.write("file", b"staged\0\xff");
        fixture.run(&["add", "file"]);
        fixture.write("file", b"working\0\xfe");
        fixture.write("new 文", b"untracked\0\xff");
        fixture.write("ignored", b"keep ignored");
        let saved = repo
            .push_stash(Some("save 文 --all"), true, &token())
            .unwrap();
        let entries = stash_ok(saved);
        assert_eq!(entries.len(), 1);
        assert_eq!(
            entries[0].id.as_str().len(),
            if format == "sha1" { 40 } else { 64 }
        );
        assert!(entries[0].subject.ends_with("save 文 --all".as_bytes()));
        let clean = repo.status(&token()).unwrap();
        assert!(clean.entries.is_empty(), "{format}: {clean:#?}");
        assert_eq!(
            fs::read(fixture.repository.join("file")).unwrap(),
            b"base\n"
        );
        assert!(!fixture.repository.join("new 文").exists());
        assert_eq!(
            fs::read(fixture.repository.join("ignored")).unwrap(),
            b"keep ignored"
        );
        let saved = &entries[0];
        assert_eq!(
            stash_ok(repo.apply_stash(&saved.id, true, &token()).unwrap()),
            entries
        );
        assert_eq!(fixture.run(&["show", ":file"]), b"staged\0\xff");
        assert_eq!(
            fs::read(fixture.repository.join("file")).unwrap(),
            b"working\0\xfe"
        );
        assert_eq!(
            fs::read(fixture.repository.join("new 文")).unwrap(),
            b"untracked\0\xff"
        );
        assert!(stash_ok(repo.drop_stash(saved, &token()).unwrap()).is_empty());
        assert_eq!(
            fs::read(fixture.repository.join("file")).unwrap(),
            b"working\0\xfe"
        );
    }
}

#[test]
fn stash_refuses_untracked_or_ignored_replacements_of_tracked_paths() {
    for replacement in ["file", "ignored-file", "directory"] {
        let fixture = base();
        fixture.run(&["rm", "--cached", "file"]);
        if replacement == "ignored-file" {
            fs::write(fixture.repository.join(".git/info/exclude"), "file\n").unwrap();
        }
        let keep = if replacement == "directory" {
            fs::remove_file(fixture.repository.join("file")).unwrap();
            "file/nested"
        } else {
            "file"
        };
        fixture.write(keep, "untracked content must survive\n");
        let repo = fixture.repo();
        assert!(
            repo.push_stash(None, false, &token()).is_err(),
            "{replacement}"
        );
        assert_eq!(
            fs::read(fixture.repository.join(keep)).unwrap(),
            b"untracked content must survive\n"
        );
        assert!(repo.stashes(&token()).unwrap().is_empty());
    }
}

#[test]
fn stash_apply_protects_ignored_files_in_the_way_of_saved_additions() {
    let fixture = base();
    let repo = fixture.repo();
    fixture.write("new", "saved addition\n");
    fixture.run(&["add", "new"]);
    let entry = stash_ok(repo.push_stash(None, false, &token()).unwrap()).remove(0);
    fs::write(fixture.repository.join(".git/info/exclude"), "new\n").unwrap();
    fixture.write("new", "ignored content must survive\n");
    let result = repo.apply_stash(&entry.id, false, &token());
    assert_eq!(
        fs::read(fixture.repository.join("new")).unwrap(),
        b"ignored content must survive\n"
    );
    assert!(matches!(result, Err(Error::InvalidInput(message)) if message.contains("ignored")));
    assert_eq!(repo.stashes(&token()).unwrap()[0].id, entry.id);
}

#[test]
fn stash_apply_rejects_directory_replacement_without_removing_ignored_children() {
    let fixture = base();
    let repo = fixture.repo();
    fixture.write("new", "saved file\n");
    fixture.run(&["add", "new"]);
    let entry = stash_ok(repo.push_stash(None, false, &token()).unwrap()).remove(0);
    fs::write(fixture.repository.join(".git/info/exclude"), "*.ignored\n").unwrap();
    fixture.write("new/notes.ignored", "keep ignored child\n");
    assert!(
        matches!(repo.apply_stash(&entry.id, false, &token()), Err(Error::InvalidInput(message)) if message.contains("directories"))
    );
    assert_eq!(
        fs::read(fixture.repository.join("new/notes.ignored")).unwrap(),
        b"keep ignored child\n"
    );
    assert_eq!(repo.stashes(&token()).unwrap()[0].id, entry.id);
}

#[test]
fn stash_default_leaves_untracked_and_no_changes_does_not_create_an_entry() {
    let fixture = base();
    let repo = fixture.repo();
    fixture.write("untracked", b"keep\n");
    assert!(stash_ok(repo.push_stash(None, false, &token()).unwrap()).is_empty());
    fixture.write("file", b"edit\n");
    assert_eq!(
        stash_ok(repo.push_stash(None, false, &token()).unwrap()).len(),
        1
    );
    assert_eq!(
        fs::read(fixture.repository.join("untracked")).unwrap(),
        b"keep\n"
    );
}

#[test]
fn apply_uses_pinned_id_and_drop_rejects_stale_or_foreign_entries() {
    let fixture = base();
    let repo = fixture.repo();
    fixture.write("file", "first\n");
    let first = stash_ok(repo.push_stash(Some("first"), false, &token()).unwrap()).remove(0);
    fixture.write("file", "second\n");
    let current = stash_ok(repo.push_stash(Some("second"), false, &token()).unwrap());
    assert!(matches!(
        repo.drop_stash(&first, &token()),
        Err(Error::InvalidInput(_))
    ));
    assert_eq!(repo.stashes(&token()).unwrap(), current);
    stash_ok(repo.apply_stash(&first.id, false, &token()).unwrap());
    assert_eq!(
        fs::read(fixture.repository.join("file")).unwrap(),
        b"first\n"
    );
    let other = base();
    assert!(matches!(
        other.repo().drop_stash(&first, &token()),
        Err(Error::InvalidInput(_))
    ));
    let left = stash_ok(repo.drop_stash(&current[1], &token()).unwrap());
    assert_eq!(left.len(), 1);
    assert_eq!(left[0].id, current[0].id);
}

#[test]
fn apply_index_choice_overrides_config_and_conflict_keeps_stash() {
    let fixture = base();
    let repo = fixture.repo();
    fixture.write("file", "saved\n");
    fixture.run(&["add", "file"]);
    let entry = stash_ok(repo.push_stash(None, false, &token()).unwrap()).remove(0);
    fixture.run(&["config", "stash.index", "true"]);
    stash_ok(repo.apply_stash(&entry.id, false, &token()).unwrap());
    assert_eq!(fixture.run(&["show", ":file"]), b"base\n");
    fixture.write("file", "divergent\n");
    fixture.commit("divergent");
    let report = repo.apply_stash(&entry.id, false, &token()).unwrap();
    assert!(matches!(report.write.result, Err(Error::Git { .. })));
    assert!(
        report
            .write
            .status
            .unwrap()
            .entries
            .iter()
            .any(|e| e.kind == EntryKind::Unmerged)
    );
    assert_eq!(report.stashes.unwrap()[0].id, entry.id);
    assert!(repo.operation_state(&token()).unwrap().is_idle()); // stash conflicts are index state, not MERGE_HEAD
    assert!(repo.push_stash(None, false, &token()).is_err());
    assert!(repo.abort_merge(&token()).is_err());
}

#[test]
fn failed_stash_apply_preserves_local_and_untracked_collisions() {
    let fixture = base();
    let repo = fixture.repo();
    fixture.write("file", "saved\n");
    fixture.write("new", "saved untracked\n");
    let entry = stash_ok(repo.push_stash(None, true, &token()).unwrap()).remove(0);
    fixture.write("file", "local\n");
    let report = repo.apply_stash(&entry.id, false, &token()).unwrap();
    assert!(report.write.result.is_err());
    assert_eq!(
        fs::read(fixture.repository.join("file")).unwrap(),
        b"local\n"
    );
    fixture.write("file", "base\n");
    fixture.write("new", "must survive\n");
    let report = repo.apply_stash(&entry.id, false, &token()).unwrap();
    assert!(report.write.result.is_err());
    assert_eq!(
        fs::read(fixture.repository.join("new")).unwrap(),
        b"must survive\n"
    );
    assert_eq!(report.stashes.unwrap()[0].id, entry.id);
}

#[test]
fn cancelled_stash_after_ref_update_retains_recoverable_entry_and_refreshes() {
    let fixture = base();
    let repo = fixture.repo();
    fixture.write("file", "saved before cancellation\n");
    hook(
        &fixture,
        "reference-transaction",
        "#!/bin/sh\nif test \"$1\" = committed && grep ' refs/stash$' >/dev/null; then\n  printf ready > .git/stash-ready\n  sleep 30\nfi\n",
    );
    let writer = repo.clone();
    let task = WriteTask::spawn(move |cancel| writer.push_stash(None, false, &cancel)).unwrap();
    wait_file(&fixture.repository.join(".git/stash-ready"));
    task.cancel();
    let report = task.wait().unwrap();
    assert!(matches!(report.write.result, Err(Error::Cancelled)));
    report.write.status.unwrap();
    let stashes = report.stashes.unwrap();
    assert_eq!(stashes.len(), 1);
    assert_eq!(
        fixture.run(&["show", &format!("{}:file", stashes[0].id)]),
        b"saved before cancellation\n"
    );
}

#[test]
fn stash_success_survives_list_refresh_limit() {
    let fixture = base();
    let client = GitClient::new(
        fixture.git.executable(),
        CommandLimits {
            stdout_bytes: 512,
            ..Default::default()
        },
    )
    .unwrap();
    let repo = client.open(&fixture.repository, &token()).unwrap();
    for index in 0..8 {
        fixture.write("file", format!("{index}\n"));
        fixture.run(&[
            "stash",
            "push",
            "-m",
            "longer stash subject used to exceed a bounded list response",
        ]);
    }
    // The write output itself is short; only its independent list refresh exceeds the limit.
    let latest = fixture.repo().stashes(&token()).unwrap().remove(0);
    let report = repo.apply_stash(&latest.id, false, &token()).unwrap();
    report.write.result.unwrap();
    report.write.status.unwrap();
    assert!(matches!(
        report.stashes,
        Err(Error::OutputLimit {
            stream: "stdout",
            ..
        })
    ));
    assert_eq!(fs::read(fixture.repository.join("file")).unwrap(), b"7\n");
}

#[test]
fn merge_fast_forward_and_forced_merge_commit_have_correct_parents() {
    for mode in [MergeMode::FastForwardOnly, MergeMode::NoFastForward] {
        let fixture = base();
        fixture.run(&["switch", "-c", "feature"]);
        fixture.write("file", "feature\n");
        fixture.commit("feature");
        fixture.run(&["switch", "main"]);
        let repo = fixture.repo();
        let before = id(&repo, "HEAD");
        let target = id(&repo, "feature");
        merge_ok(repo.merge("feature", mode, &token()).unwrap());
        assert_eq!(
            fs::read(fixture.repository.join("file")).unwrap(),
            b"feature\n"
        );
        if mode == MergeMode::FastForwardOnly {
            assert_eq!(id(&repo, "HEAD"), target);
        } else {
            let commit = repo
                .history(HistoryStart::Head, 1, &token())
                .unwrap()
                .commits
                .remove(0);
            assert_eq!(commit.parents, vec![before, target]);
        }
        let merged = id(&repo, "HEAD");
        merge_ok(repo.merge("feature", mode, &token()).unwrap());
        assert_eq!(id(&repo, "HEAD"), merged); // already up to date
    }
}

#[test]
fn divergent_merge_refuses_ff_only_then_creates_two_parent_commit() {
    let fixture = base();
    diverge(&fixture, false);
    let repo = fixture.repo();
    let before = id(&repo, "HEAD");
    let target = id(&repo, "feature");
    let failed = repo
        .merge("feature", MergeMode::FastForwardOnly, &token())
        .unwrap();
    assert!(failed.write.result.is_err());
    assert!(failed.state.unwrap().is_idle());
    assert_eq!(id(&repo, "HEAD"), before);
    merge_ok(
        repo.merge("feature", MergeMode::AllowFastForward, &token())
            .unwrap(),
    );
    let commit = repo
        .history(HistoryStart::Head, 1, &token())
        .unwrap()
        .commits
        .remove(0);
    assert_eq!(commit.parents, vec![before, target]);
    assert_eq!(
        fs::read(fixture.repository.join("main-file")).unwrap(),
        b"main\n"
    );
    assert_eq!(
        fs::read(fixture.repository.join("feature-file")).unwrap(),
        b"feature\n"
    );
}

#[test]
fn merge_conflict_is_reported_then_resolved_staged_and_committed() {
    let fixture = base();
    diverge(&fixture, true);
    let repo = fixture.repo();
    let before = id(&repo, "HEAD");
    let target = id(&repo, "feature");
    let report = repo
        .merge("feature", MergeMode::AllowFastForward, &token())
        .unwrap();
    assert!(report.write.result.is_err());
    assert_eq!(report.state.unwrap().merge_heads, vec![target.clone()]);
    assert!(
        report
            .write
            .status
            .unwrap()
            .entries
            .iter()
            .any(|e| e.kind == EntryKind::Unmerged)
    );
    assert_eq!(id(&repo, "HEAD"), before);
    assert!(
        repo.merge("feature", MergeMode::AllowFastForward, &token())
            .is_err()
    );
    assert!(repo.push_stash(None, false, &token()).is_err());
    let unresolved = repo.continue_merge("cannot finish yet", &token()).unwrap();
    assert!(unresolved.write.result.is_err());
    assert!(!unresolved.state.unwrap().is_idle());
    fixture.write("file", "resolved\n");
    repo.stage(&[RepoPath::new(b"file".to_vec()).unwrap()], &token())
        .unwrap()
        .result
        .unwrap();
    merge_ok(
        repo.continue_merge("Resolve both changes\n\n# literal line", &token())
            .unwrap(),
    );
    let commit = repo
        .history(HistoryStart::Head, 1, &token())
        .unwrap()
        .commits
        .remove(0);
    assert_eq!(commit.parents, vec![before, target]);
    assert_eq!(commit.message, b"Resolve both changes\n\n# literal line\n");
}

#[test]
fn abort_merge_restores_original_tree_and_keeps_unrelated_untracked_file() {
    let fixture = base();
    diverge(&fixture, true);
    let repo = fixture.repo();
    let before = id(&repo, "HEAD");
    assert!(
        repo.merge("feature", MergeMode::AllowFastForward, &token())
            .unwrap()
            .write
            .result
            .is_err()
    );
    fixture.write("notes", "keep\n");
    fixture.write("file", "staged resolution\n");
    repo.stage(&[RepoPath::new(b"file".to_vec()).unwrap()], &token())
        .unwrap()
        .result
        .unwrap();
    merge_ok(repo.abort_merge(&token()).unwrap());
    assert_eq!(id(&repo, "HEAD"), before);
    assert_eq!(
        fs::read(fixture.repository.join("file")).unwrap(),
        b"main\n"
    );
    assert_eq!(
        fs::read(fixture.repository.join("notes")).unwrap(),
        b"keep\n"
    );
    assert!(repo.abort_merge(&token()).is_err());
    assert!(repo.continue_merge("no merge", &token()).is_err());
}

#[test]
fn dirty_tracked_preflight_disables_autostash_without_losing_edits() {
    let fixture = base();
    diverge(&fixture, false);
    let repo = fixture.repo();
    fixture.run(&["config", "merge.autoStash", "true"]);
    for staged in [false, true] {
        fixture.write("file", "local edit\n");
        if staged {
            fixture.run(&["add", "file"]);
        }
        assert!(matches!(
            repo.merge("feature", MergeMode::AllowFastForward, &token()),
            Err(Error::InvalidInput(_))
        ));
        assert_eq!(
            fs::read(fixture.repository.join("file")).unwrap(),
            b"local edit\n"
        );
        assert!(repo.stashes(&token()).unwrap().is_empty());
        assert!(repo.operation_state(&token()).unwrap().is_idle());
    }
}

#[test]
fn merge_protects_untracked_and_ignored_collisions() {
    for ignored in [false, true] {
        let fixture = base();
        fixture.run(&["switch", "-c", "feature"]);
        fixture.write("collision", "from feature\n");
        fixture.commit("feature");
        fixture.run(&["switch", "main"]);
        if ignored {
            fs::write(fixture.repository.join(".git/info/exclude"), "collision\n").unwrap();
        }
        fixture.write("collision", "must survive\n");
        let repo = fixture.repo();
        let before = id(&repo, "HEAD");
        let report = repo
            .merge("feature", MergeMode::AllowFastForward, &token())
            .unwrap();
        assert!(report.write.result.is_err());
        assert_eq!(
            fs::read(fixture.repository.join("collision")).unwrap(),
            b"must survive\n"
        );
        assert_eq!(id(&repo, "HEAD"), before);
    }
}

#[test]
fn branch_merge_defaults_cannot_bypass_hooks_or_change_merge_strategy() {
    let fixture = base();
    diverge(&fixture, false);
    let repo = fixture.repo();
    fixture.run(&[
        "config",
        "branch.main.mergeOptions",
        "--squash --no-commit --no-verify --autostash -sours",
    ]);
    hook(
        &fixture,
        "pre-merge-commit",
        "#!/bin/sh\necho rejected >&2\nexit 1\n",
    );
    let report = repo
        .merge("feature", MergeMode::AllowFastForward, &token())
        .unwrap();
    assert!(report.write.result.is_err());
    assert!(!report.state.unwrap().merge_heads.is_empty());
    assert_eq!(
        fs::read(fixture.repository.join("feature-file")).unwrap(),
        b"feature\n"
    );
    merge_ok(repo.abort_merge(&token()).unwrap());
    fs::remove_file(fixture.root.join("no-hooks/pre-merge-commit")).unwrap();
    merge_ok(
        repo.merge("feature", MergeMode::AllowFastForward, &token())
            .unwrap(),
    );
    assert_eq!(
        repo.history(HistoryStart::Head, 1, &token())
            .unwrap()
            .commits[0]
            .parents
            .len(),
        2
    );
    assert_eq!(
        fixture.run(&["config", "branch.main.mergeOptions"]),
        b"--squash --no-commit --no-verify --autostash -sours\n"
    );
}

#[test]
fn cancelled_post_merge_returns_changed_head_and_reconciliation() {
    let fixture = base();
    fixture.run(&["switch", "-c", "feature"]);
    fixture.write("file", "feature\n");
    fixture.commit("feature");
    fixture.run(&["switch", "main"]);
    let repo = fixture.repo();
    let target = id(&repo, "feature");
    hook(
        &fixture,
        "post-merge",
        "#!/bin/sh\nprintf ready > .git/merge-ready\nsleep 30\n",
    );
    let writer = repo.clone();
    let task = WriteTask::spawn(move |cancel| {
        writer.merge("feature", MergeMode::AllowFastForward, &cancel)
    })
    .unwrap();
    wait_file(&fixture.repository.join(".git/merge-ready"));
    task.cancel();
    let report = task.wait().unwrap();
    assert!(matches!(report.write.result, Err(Error::Cancelled)));
    assert_eq!(id(&repo, "HEAD"), target);
    report.write.status.unwrap();
    assert_eq!(
        report.state.unwrap(),
        repo.operation_state(&token()).unwrap()
    );
    assert!(
        report
            .branches
            .unwrap()
            .iter()
            .any(|b| b.is_head && b.target == target)
    );
}

#[test]
fn merge_success_survives_status_refresh_failure_and_still_reads_state_and_branches() {
    let fixture = base();
    fixture.run(&["switch", "-c", "feature"]);
    fixture.write("file", "feature\n");
    fixture.commit("feature");
    fixture.run(&["switch", "main"]);
    hook(
        &fixture,
        "post-merge",
        "#!/bin/sh\nn=0\nwhile test \"$n\" -lt 80; do\n  printf new > \"untracked-$n\"\n  n=$((n + 1))\ndone\n",
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
    let report = repo
        .merge("feature", MergeMode::AllowFastForward, &token())
        .unwrap();
    report.write.result.unwrap();
    assert!(matches!(
        report.write.status,
        Err(Error::OutputLimit { .. })
    ));
    assert!(report.state.unwrap().is_idle());
    assert_eq!(
        report
            .branches
            .unwrap()
            .iter()
            .filter(|b| b.is_head)
            .count(),
        1
    );
}

#[test]
fn operation_state_is_worktree_local_and_rejects_active_or_malformed_markers() {
    let fixture = base();
    let repo = fixture.repo();
    let linked = fixture.root.join("linked");
    fixture.run(&["worktree", "add", "-b", "linked", linked.to_str().unwrap()]);
    let other = fixture.git.open(&linked, &token()).unwrap();
    for name in [
        "CHERRY_PICK_HEAD",
        "REVERT_HEAD",
        "sequencer",
        "rebase-merge",
        "rebase-apply",
    ] {
        fs::write(
            other.info().git_dir.join(name),
            format!("{}\n", id(&repo, "HEAD")),
        )
        .unwrap();
        assert!(!other.operation_state(&token()).unwrap().is_idle());
        assert!(repo.operation_state(&token()).unwrap().is_idle());
        assert!(
            other
                .merge("main", MergeMode::AllowFastForward, &token())
                .is_err()
        );
        assert!(other.push_stash(None, false, &token()).is_err());
        fs::remove_file(other.info().git_dir.join(name)).unwrap();
    }
    fs::write(other.info().git_dir.join("MERGE_HEAD"), b"corrupt\n").unwrap();
    assert!(matches!(
        other.operation_state(&token()),
        Err(Error::InvalidOutput { .. })
    ));
    fs::write(other.info().git_dir.join("MERGE_HEAD"), vec![b'a'; 65537]).unwrap();
    assert!(other.operation_state(&token()).is_err());
    fs::remove_file(other.info().git_dir.join("MERGE_HEAD")).unwrap();
    fs::write(
        other.info().git_dir.join("MERGE_AUTOSTASH"),
        format!("{}\n", id(&repo, "HEAD")),
    )
    .unwrap();
    fs::write(
        other.info().git_dir.join("MERGE_HEAD"),
        format!("{}\n", id(&repo, "HEAD")),
    )
    .unwrap();
    assert!(matches!(
        other.abort_merge(&token()),
        Err(Error::Unsupported(_))
    ));
    assert!(matches!(
        other.continue_merge("message", &token()),
        Err(Error::Unsupported(_))
    ));
}

#[test]
fn merge_handles_equals_in_branch_name_without_reinterpreting_config_keys() {
    let fixture = base();
    diverge(&fixture, false);
    fixture.run(&["branch", "-m", "main=文"]);
    fixture.run(&[
        "config",
        "branch.main=文.mergeOptions",
        "--squash --no-commit -sours",
    ]);
    let repo = fixture.repo();
    merge_ok(
        repo.merge("feature", MergeMode::AllowFastForward, &token())
            .unwrap(),
    );
    let commit = repo
        .history(HistoryStart::Head, 1, &token())
        .unwrap()
        .commits
        .remove(0);
    assert_eq!(commit.parents.len(), 2);
    assert_eq!(
        fs::read(fixture.repository.join("feature-file")).unwrap(),
        b"feature\n"
    );
}

#[test]
fn operation_refs_and_stash_list_work_with_reftable_storage() {
    let fixture = base();
    fixture.run(&["refs", "migrate", "--ref-format=reftable"]);
    let repo = fixture.repo();
    let head = id(&repo, "HEAD");
    for name in ["CHERRY_PICK_HEAD", "REVERT_HEAD", "MERGE_AUTOSTASH"] {
        fixture.run(&["update-ref", name, head.as_str()]);
        assert!(!repo.operation_state(&token()).unwrap().is_idle());
        assert!(repo.push_stash(None, false, &token()).is_err());
        fixture.run(&["update-ref", "-d", name]);
    }
    fixture.write("file", "stash in reftable\n");
    let entry = stash_ok(repo.push_stash(None, false, &token()).unwrap()).remove(0);
    stash_ok(repo.apply_stash(&entry.id, false, &token()).unwrap());
    assert_eq!(
        fs::read(fixture.repository.join("file")).unwrap(),
        b"stash in reftable\n"
    );
    assert!(stash_ok(repo.drop_stash(&entry, &token()).unwrap()).is_empty());
}

#[test]
fn broken_operation_refs_cannot_make_a_repository_look_idle() {
    let fixture = base();
    let repo = fixture.repo();
    for name in ["CHERRY_PICK_HEAD", "REVERT_HEAD", "MERGE_AUTOSTASH"] {
        for contents in [
            "not an object id\n",
            "ref: refs/heads/missing\n",
            "0000000000000000000000000000000000000001\n",
        ] {
            let marker = repo.info().git_dir.join(name);
            fs::write(&marker, contents).unwrap();
            assert!(
                !matches!(repo.operation_state(&token()), Ok(state) if state.is_idle()),
                "{name}: {contents}"
            );
            assert!(repo.push_stash(None, false, &token()).is_err());
            fs::remove_file(marker).unwrap();
        }
    }
    assert!(repo.operation_state(&token()).unwrap().is_idle());
}

#[test]
fn branches_and_tags_named_like_operation_refs_do_not_report_an_active_operation() {
    let fixture = base();
    for name in ["MERGE_AUTOSTASH", "CHERRY_PICK_HEAD", "REVERT_HEAD"] {
        fixture.run(&["branch", name]);
        fixture.run(&["tag", name]);
    }
    let repo = fixture.repo();
    assert!(repo.operation_state(&token()).unwrap().is_idle());
    fixture.write("file", "save despite ref names\n");
    let entry = stash_ok(repo.push_stash(None, false, &token()).unwrap()).remove(0);
    stash_ok(repo.apply_stash(&entry.id, false, &token()).unwrap());
    assert_eq!(
        fs::read(fixture.repository.join("file")).unwrap(),
        b"save despite ref names\n"
    );
}

#[test]
fn stash_and_merge_share_existing_write_queue_and_queued_cancellation() {
    let fixture = base();
    let repo = fixture.repo();
    fixture.write("file", "commit edit\n");
    repo.stage_all(&token()).unwrap().result.unwrap();
    hook(
        &fixture,
        "post-commit",
        "#!/bin/sh\nprintf ready > .git/queue-ready\nwhile ! test -f .git/queue-release; do sleep 0.02; done\n",
    );
    let writer = repo.clone();
    let commit = WriteTask::spawn(move |cancel| writer.commit("owner", &cancel)).unwrap();
    wait_file(&fixture.repository.join(".git/queue-ready"));
    let independent = GitClient::discover()
        .unwrap()
        .open(&fixture.repository, &token())
        .unwrap();
    let pending = independent.clone();
    let (started, start) = mpsc::channel();
    let queued = WriteTask::spawn(move |cancel| {
        started.send(()).unwrap();
        pending.push_stash(None, false, &cancel)
    })
    .unwrap();
    start.recv_timeout(Duration::from_secs(5)).unwrap();
    queued.cancel();
    assert!(matches!(queued.wait(), Err(Error::Cancelled)));
    let (done, completion) = mpsc::channel();
    let queued = WriteTask::spawn(move |cancel| {
        let result = independent.merge("HEAD", MergeMode::FastForwardOnly, &cancel);
        done.send(()).unwrap();
        result
    })
    .unwrap();
    assert_eq!(
        completion.recv_timeout(Duration::from_secs(1)),
        Err(mpsc::RecvTimeoutError::Timeout)
    );
    fs::write(fixture.repository.join(".git/queue-release"), "release").unwrap();
    commit.wait().unwrap().result.unwrap();
    merge_ok(queued.wait().unwrap());
    assert!(repo.stashes(&token()).unwrap().is_empty());
}

#[test]
fn invalid_inputs_unborn_bare_and_precancelled_calls_do_not_mutate() {
    let fixture = Fixture::new();
    let repo = fixture.repo();
    assert!(repo.stashes(&token()).unwrap().is_empty());
    assert!(
        repo.push_stash(None, false, &token())
            .unwrap()
            .write
            .result
            .is_err()
    );
    assert!(
        repo.merge("HEAD", MergeMode::AllowFastForward, &token())
            .is_err()
    );
    fixture.write("file", "base\n");
    fixture.commit("base");
    for message in ["", " \n", "bad\0message"] {
        assert!(repo.push_stash(Some(message), false, &token()).is_err());
        assert!(repo.continue_merge(message, &token()).is_err());
    }
    for target in ["", "--abort", "missing", "bad\0ref"] {
        assert!(
            repo.merge(target, MergeMode::AllowFastForward, &token())
                .is_err()
        );
    }
    let cancel = token();
    cancel.cancel();
    assert!(matches!(
        repo.push_stash(None, true, &cancel),
        Err(Error::Cancelled)
    ));
    assert!(matches!(
        repo.apply_stash(&id(&repo, "HEAD"), false, &cancel),
        Err(Error::Cancelled)
    ));
    assert!(matches!(
        repo.merge("HEAD", MergeMode::AllowFastForward, &cancel),
        Err(Error::Cancelled)
    ));
    assert!(matches!(repo.abort_merge(&cancel), Err(Error::Cancelled)));
    assert!(matches!(
        repo.continue_merge("merge", &cancel),
        Err(Error::Cancelled)
    ));
    assert!(repo.status(&token()).unwrap().entries.is_empty());
    let bare = fixture.root.join("bare.git");
    fixture.run(&[
        "clone",
        "--bare",
        "--no-hardlinks",
        fixture.repository.to_str().unwrap(),
        bare.to_str().unwrap(),
    ]);
    let bare = fixture.git.open(bare, &token()).unwrap();
    assert!(matches!(
        bare.push_stash(None, false, &token()),
        Err(Error::WorktreeRequired)
    ));
    assert!(matches!(
        bare.apply_stash(&id(&repo, "HEAD"), false, &token()),
        Err(Error::WorktreeRequired)
    ));
    assert!(matches!(
        bare.merge("HEAD", MergeMode::AllowFastForward, &token()),
        Err(Error::WorktreeRequired)
    ));
    assert!(matches!(
        bare.abort_merge(&token()),
        Err(Error::WorktreeRequired)
    ));
    assert!(matches!(
        bare.continue_merge("merge", &token()),
        Err(Error::WorktreeRequired)
    ));
}

#[test]
fn merge_does_not_recurse_into_submodules_and_stash_rejects_their_dirty_contents() {
    let component = base();
    let old = id(&component.repo(), "HEAD");
    component.write("file", "new component\n");
    component.commit("component second");
    let new = id(&component.repo(), "HEAD");
    let fixture = base();
    fixture.run(&[
        "-c",
        "protocol.file.allow=always",
        "submodule",
        "add",
        component.repository.to_str().unwrap(),
        "module",
    ]);
    let module = fixture.repository.join("module");
    fixture.run_at(&module, &["checkout", "--detach", old.as_str()]);
    fixture.commit("old pointer");
    fixture.run(&["switch", "-c", "feature"]);
    fixture.run_at(&module, &["checkout", "--detach", new.as_str()]);
    fixture.commit("new pointer");
    fixture.run(&["switch", "main"]);
    fixture.run_at(&module, &["checkout", "--detach", old.as_str()]);
    fixture.run(&["config", "submodule.recurse", "true"]);
    let repo = fixture.repo();
    merge_ok(
        repo.merge("feature", MergeMode::AllowFastForward, &token())
            .unwrap(),
    );
    assert_eq!(
        fixture.run_at(&module, &["rev-parse", "HEAD"]),
        format!("{old}\n").as_bytes()
    );
    assert_eq!(fs::read(module.join("file")).unwrap(), b"base\n");
    fixture.run_at(&module, &["checkout", "--detach", new.as_str()]);
    fs::write(module.join("file"), "local component edit\n").unwrap();
    fixture.write("file", "parent edit\n");
    assert!(matches!(
        repo.push_stash(None, true, &token()),
        Err(Error::InvalidInput(_))
    ));
    assert!(repo.stashes(&token()).unwrap().is_empty());
    assert_eq!(
        fs::read(module.join("file")).unwrap(),
        b"local component edit\n"
    );
    assert_eq!(
        fs::read(fixture.repository.join("file")).unwrap(),
        b"parent edit\n"
    );
}

#[test]
fn sha256_linked_worktree_can_merge_without_moving_the_main_checkout() {
    let fixture = Fixture::with_format("sha256");
    fixture.write("file", "base\n");
    fixture.commit("base");
    let repo = fixture.repo();
    let before = id(&repo, "HEAD");
    fixture.run(&["switch", "-c", "feature"]);
    fixture.write("file", "feature\n");
    fixture.commit("feature");
    fixture.run(&["switch", "main"]);
    let linked = fixture.root.join("linked");
    fixture.run(&["worktree", "add", "-b", "linked", linked.to_str().unwrap()]);
    let other = fixture.git.open(&linked, &token()).unwrap();
    merge_ok(
        other
            .merge("feature", MergeMode::NoFastForward, &token())
            .unwrap(),
    );
    let commit = other
        .history(HistoryStart::Head, 1, &token())
        .unwrap()
        .commits
        .remove(0);
    assert_eq!(commit.id.as_str().len(), 64);
    assert_eq!(commit.parents, vec![before.clone(), id(&repo, "feature")]);
    assert_eq!(id(&repo, "HEAD"), before);
    assert_eq!(
        fs::read(fixture.repository.join("file")).unwrap(),
        b"base\n"
    );
    assert_eq!(fs::read(linked.join("file")).unwrap(), b"feature\n");
}
