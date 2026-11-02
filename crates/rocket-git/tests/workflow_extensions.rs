mod common;
use common::{Fixture, isolated};
use rocket_git::*;
use std::fs;

#[test]
fn patch_binary_expansion_is_bounded_and_mode_preserves_staged_blob() {
    isolated("ROCKET_GAPS_TEST","patch_binary_expansion_is_bounded_and_mode_preserves_staged_blob",||{
        let f=Fixture::new();f.write("script.sh","base\n");f.commit("base");f.write("script.sh","staged\n");f.run(&["add","script.sh"]);f.write("script.sh","working\n");let r=f.repo();let c=CancellationToken::new();
        let mode=b"diff --git a/script.sh b/script.sh\nold mode 100644\nnew mode 100755\n";let p=r.preview_patch_text(mode,&c).unwrap();r.apply_patch_preview(&p,&c).unwrap().result.unwrap();assert_eq!(f.run(&["show",":script.sh"]),b"staged\n");assert_eq!(fs::read(f.repository.join("script.sh")).unwrap(),b"working\n");assert!(r.preview_patch_text(mode,&c).is_err());
        for body in ["literal 999999999999\n","delta 8\n"] {let patch=format!("diff --git a/data.bin b/data.bin\nnew file mode 100644\nindex {}..{}\nGIT binary patch\n{body}","0".repeat(40),"a".repeat(40));assert!(r.preview_patch_text(patch.as_bytes(),&c).is_err());assert!(!f.repository.join("data.bin").exists());}
    });
}

fn id(f: &Fixture, reference: &str) -> Vec<u8> {
    f.run(&["rev-parse", reference])
}
fn batch_fixture() -> Fixture {
    let f = Fixture::new();
    f.write("共同 文.txt", "base\n");
    f.commit("base");
    for name in ["feature/文", "docs"] {
        f.run(&["branch", name]);
        f.run(&["branch", "--set-upstream-to=main", name]);
    }
    f.write("new.txt", "upstream\n");
    f.commit("advance");
    f
}
#[test]
fn batch_fast_forward_preserves_checkout_index_and_recovery() {
    isolated(
        "ROCKET_GAPS_TEST",
        "batch_fast_forward_preserves_checkout_index_and_recovery",
        || {
            let f = batch_fixture();
            let r = f.repo();
            let c = CancellationToken::new();
            let before = id(&f, "feature/文");
            let head = id(&f, "HEAD");
            f.write("共同 文.txt", "staged\n");
            f.run(&["add", "共同 文.txt"]);
            f.write("共同 文.txt", "working\n");
            let index = fs::read(f.repository.join(".git/index")).unwrap();
            let p = r.preview_fast_forward_branches(&c).unwrap();
            assert!(
                p.entries()
                    .iter()
                    .find(|e| e.branch == b"refs/heads/main")
                    .unwrap()
                    .blocked
                    .is_some()
            );
            let report = r
                .fast_forward_branches(
                    &p,
                    &[
                        "refs/heads/feature/文".as_bytes().to_vec(),
                        b"refs/heads/docs".to_vec(),
                    ],
                    &c,
                )
                .unwrap();
            assert!(report.result.is_ok(), "{:?}", report.result);
            assert!(report.observed.is_ok());
            assert_eq!(id(&f, "feature/文"), head);
            assert_eq!(id(&f, "docs"), head);
            assert_eq!(id(&f, &format!("{}/0", report.recovery)), before);
            assert_eq!(id(&f, "HEAD"), head);
            assert_eq!(fs::read(f.repository.join(".git/index")).unwrap(), index);
            assert_eq!(
                fs::read(f.repository.join("共同 文.txt")).unwrap(),
                b"working\n"
            );
        },
    )
}
#[test]
fn batch_fast_forward_rejects_stale_worktrees_divergence_and_cancel() {
    isolated(
        "ROCKET_GAPS_TEST",
        "batch_fast_forward_rejects_stale_worktrees_divergence_and_cancel",
        || {
            let f = batch_fixture();
            let r = f.repo();
            let c = CancellationToken::new();
            let names = vec![b"refs/heads/docs".to_vec()];
            let p = r.preview_fast_forward_branches(&c).unwrap();
            let before = id(&f, "docs");
            f.write("advance.txt", "next\n");
            f.commit("move upstream");
            assert!(r.fast_forward_branches(&p, &names, &c).is_err());
            assert_eq!(id(&f, "docs"), before);
            let p = r.preview_fast_forward_branches(&c).unwrap();
            let stopped = CancellationToken::new();
            stopped.cancel();
            assert!(r.fast_forward_branches(&p, &names, &stopped).is_err());
            assert_eq!(id(&f, "docs"), before);
            let linked = f.repository.parent().unwrap().join("linked 文");
            f.run(&["worktree", "add", linked.to_str().unwrap(), "docs"]);
            assert!(r.fast_forward_branches(&p, &names, &c).is_err());
            assert_eq!(id(&f, "docs"), before);
            f.run(&["switch", "feature/文"]);
            f.write("topic.txt", "local\n");
            f.commit("diverge");
            f.run(&["switch", "main"]);
            let p = r.preview_fast_forward_branches(&c).unwrap();
            assert!(
                p.entries()
                    .iter()
                    .find(|e| e.branch == "refs/heads/feature/文".as_bytes())
                    .unwrap()
                    .blocked
                    .unwrap()
                    .contains("diverged")
            );
        },
    )
}
fn unrelated(conflict: bool) -> Fixture {
    let f = Fixture::new();
    f.write("main.txt", "main\n");
    f.commit("main root");
    f.run(&["switch", "--orphan", "imported"]);
    f.write(
        if conflict {
            "main.txt"
        } else {
            "imported 文.txt"
        },
        "imported\n",
    );
    f.commit("different root");
    f.run(&["switch", "main"]);
    f
}
#[test]
fn unrelated_merge_requires_opt_in_and_supports_abort() {
    isolated(
        "ROCKET_GAPS_TEST",
        "unrelated_merge_requires_opt_in_and_supports_abort",
        || {
            for conflict in [false, true] {
                let f = unrelated(conflict);
                let r = f.repo();
                let c = CancellationToken::new();
                let original = id(&f, "HEAD");
                assert!(matches!(
                    r.preview_merge("imported", MergeMode::NoFastForward, &c),
                    Err(Error::Merge(MergeFailure::UnrelatedHistories))
                ));
                let ff = r
                    .preview_merge_allow_unrelated("imported", MergeMode::FastForwardOnly, true, &c)
                    .unwrap();
                assert!(!ff.can_apply());
                let p = r
                    .preview_merge_allow_unrelated("imported", MergeMode::NoFastForward, true, &c)
                    .unwrap();
                assert!(p.unrelated());
                let stopped = CancellationToken::new();
                stopped.cancel();
                assert!(r.apply_merge(&p, &stopped).is_err());
                assert_eq!(id(&f, "HEAD"), original);
                let report = r.apply_merge(&p, &c).unwrap();
                if conflict {
                    assert!(report.write.result.is_err());
                    assert!(!r.operation_state(&c).unwrap().merge_heads.is_empty());
                    assert!(r.abort_merge(&c).unwrap().write.result.is_ok());
                    assert_eq!(id(&f, "HEAD"), original);
                } else {
                    assert!(report.write.result.is_ok(), "{:?}", report.write.result);
                    assert_eq!(id(&f, "HEAD^1"), original);
                    assert_eq!(id(&f, "HEAD^2"), id(&f, "imported"));
                }
            }
        },
    )
}

#[test]
fn remote_rename_preserves_urls_upstreams_and_recovery_and_rejects_stale() {
    isolated(
        "ROCKET_GAPS_TEST",
        "remote_rename_preserves_urls_upstreams_and_recovery_and_rejects_stale",
        || {
            let f = Fixture::new();
            f.write("a.txt", "base\n");
            f.commit("base");
            f.run(&["remote", "add", "origin", "https://example.invalid/project"]);
            f.run(&[
                "config",
                "remote.origin.pushurl",
                "ssh://example.invalid/project",
            ]);
            let tip = String::from_utf8(id(&f, "HEAD")).unwrap();
            f.run(&["update-ref", "refs/remotes/origin/main", tip.trim()]);
            f.run(&["branch", "--set-upstream-to=origin/main", "main"]);
            let r = f.repo();
            let c = CancellationToken::new();
            let remote = r.remotes(&c).unwrap().remove(0);
            let p = r.preview_remote_rename(&remote, b"upstream", &c).unwrap();
            assert_eq!(p.affected_branches().len(), 1);
            f.run(&["config", "fixture.changed", "yes"]);
            assert!(r.rename_remote(&p, &c).is_err());
            f.run(&["config", "--unset", "fixture.changed"]);
            let p = r.preview_remote_rename(&remote, b"upstream", &c).unwrap();
            let stopped = CancellationToken::new();
            stopped.cancel();
            assert!(r.rename_remote(&p, &stopped).is_err());
            let before = fs::read(f.repository.join(".git/config")).unwrap();
            let report = r.rename_remote(&p, &c).unwrap();
            assert!(report.report.result.is_ok(), "{:?}", report.report.result);
            assert_eq!(fs::read(report.config_backup).unwrap(), before);
            assert_eq!(f.run(&["config", "branch.main.remote"]), b"upstream\n");
            assert_eq!(id(&f, "refs/remotes/upstream/main"), id(&f, "HEAD"));
            let renamed = r.remotes(&c).unwrap().remove(0);
            assert_eq!(renamed.urls, remote.urls);
            assert_eq!(renamed.push_urls, remote.push_urls);
            assert!(r.preview_remote_rename(&renamed, b"upstream", &c).is_err());
        },
    )
}

#[test]
fn fetch_tag_choices_preserve_existing_tags_and_working_files() {
    isolated(
        "ROCKET_GAPS_TEST",
        "fetch_tag_choices_preserve_existing_tags_and_working_files",
        || {
            let source = Fixture::new();
            source.write("base.txt", "base\n");
            source.commit("source");
            source.run(&["tag", "release"]);
            let f = Fixture::new();
            f.run(&[
                "remote",
                "add",
                "origin",
                source.repository.to_str().unwrap(),
            ]);
            let r = f.repo();
            let c = CancellationToken::new();
            r.fetch_with_tags(b"origin", FetchTags::None, None, &c)
                .unwrap()
                .result
                .unwrap();
            assert!(r.tags(100, &c).unwrap().entries.is_empty());
            r.fetch_with_tags(b"origin", FetchTags::RemoteDefault, None, &c)
                .unwrap()
                .result
                .unwrap();
            assert_eq!(r.tags(100, &c).unwrap().entries.len(), 1);
            source.write("advance.txt", "next\n");
            source.commit("next");
            source.run(&["tag", "other"]);
            r.fetch_with_tags(b"origin", FetchTags::All, None, &c)
                .unwrap()
                .result
                .unwrap();
            assert_eq!(r.tags(100, &c).unwrap().entries.len(), 2);
            let original = id(&f, "refs/tags/release");
            source.run(&["tag", "-f", "release"]);
            assert!(
                r.fetch_with_tags(b"origin", FetchTags::All, None, &c)
                    .unwrap()
                    .result
                    .is_err()
            );
            assert_eq!(id(&f, "refs/tags/release"), original);
        },
    )
}

#[test]
fn submodule_deinitialize_retains_checkout_and_protects_dirty_and_stale_state() {
    isolated(
        "ROCKET_GAPS_TEST",
        "submodule_deinitialize_retains_checkout_and_protects_dirty_and_stale_state",
        || {
            let source = Fixture::new();
            source.write("content 文.txt", "keep\n");
            source.commit("child");
            let f = Fixture::new();
            f.write("a.txt", "parent\n");
            f.commit("parent");
            let name = "vendor/子";
            f.run(&[
                "-c",
                "protocol.file.allow=always",
                "submodule",
                "add",
                source.repository.to_str().unwrap(),
                name,
            ]);
            f.commit("module");
            let child = f.repository.join(name);
            let r = f.repo();
            let c = CancellationToken::new();
            let action =
                SubmoduleAction::Deinitialize(RepoPath::new(name.as_bytes().to_vec()).unwrap());
            fs::write(child.join("untracked.txt"), b"keep").unwrap();
            assert!(r.preview_submodule_action(&action, &c).is_err());
            fs::remove_file(child.join("untracked.txt")).unwrap();
            let p = r.preview_submodule_action(&action, &c).unwrap();
            f.run(&["config", "fixture.changed", "yes"]);
            assert!(r.apply_submodule_action(&p, &c).is_err());
            f.run(&["config", "--unset", "fixture.changed"]);
            let p = r.preview_submodule_action(&action, &c).unwrap();
            let stopped = CancellationToken::new();
            stopped.cancel();
            assert!(r.apply_submodule_action(&p, &stopped).is_err());
            assert!(child.join("content 文.txt").is_file());
            let index = fs::read(f.repository.join(".git/index")).unwrap();
            let modules = fs::read(f.repository.join(".gitmodules")).unwrap();
            let report = r.apply_submodule_action(&p, &c).unwrap();
            assert!(report.result.is_ok(), "{:?}", report.result);
            assert!(report.verification.is_ok(), "{:?}", report.verification);
            assert_eq!(fs::read(f.repository.join(".git/index")).unwrap(), index);
            assert_eq!(fs::read(f.repository.join(".gitmodules")).unwrap(), modules);
            assert_eq!(
                fs::read(
                    report
                        .recovery_directory
                        .unwrap()
                        .join("checkout/content 文.txt")
                )
                .unwrap(),
                b"keep\n"
            );
            assert!(!r.submodules(&c).unwrap()[0].initialized);
        },
    )
}

#[test]
fn submodule_batch_updates_selected_clean_children() {
    isolated(
        "ROCKET_GAPS_TEST",
        "submodule_batch_updates_selected_clean_children",
        || {
            let source = Fixture::new();
            source.write("content.txt", "old\n");
            source.commit("child");
            let f = Fixture::new();
            f.write("a.txt", "parent\n");
            f.commit("parent");
            for name in ["vendor/one", "vendor/二"] {
                f.run(&[
                    "-c",
                    "protocol.file.allow=always",
                    "submodule",
                    "add",
                    source.repository.to_str().unwrap(),
                    name,
                ]);
            }
            f.commit("modules");
            source.write("content.txt", "new\n");
            source.commit("advance");
            let next = String::from_utf8(id(&source, "HEAD")).unwrap();
            for name in ["vendor/one", "vendor/二"] {
                f.run(&[
                    "update-index",
                    "--cacheinfo",
                    &format!("160000,{},{}", next.trim(), name),
                ]);
            }
            f.run(&["commit", "-m", "new gitlinks"]);
            let r = f.repo();
            let c = CancellationToken::new();
            let paths =
                ["vendor/one", "vendor/二"].map(|p| RepoPath::new(p.as_bytes().to_vec()).unwrap());
            let p = r.preview_submodule_batch(&paths, None, &c).unwrap();
            let stopped = CancellationToken::new();
            stopped.cancel();
            assert!(r.apply_submodule_batch(&p, &stopped).is_err());
            let report = r.apply_submodule_batch(&p, &c).unwrap();
            assert!(report.skipped.is_empty());
            assert_eq!(report.results.len(), 2);
            for (_, result) in report.results {
                let result = result.unwrap();
                assert!(result.result.is_ok(), "{:?}", result.result);
                assert!(result.verification.is_ok(), "{:?}", result.verification);
            }
            for name in ["vendor/one", "vendor/二"] {
                assert_eq!(
                    fs::read(f.repository.join(name).join("content.txt")).unwrap(),
                    b"new\n"
                );
            }
        },
    )
}

#[test]
fn stash_pop_conflict_keeps_entry_and_clear_retains_recovery() {
    isolated(
        "ROCKET_GAPS_TEST",
        "stash_pop_conflict_keeps_entry_and_clear_retains_recovery",
        || {
            let f = Fixture::new();
            f.write("file.txt", "base\n");
            f.commit("base");
            let r = f.repo();
            let c = CancellationToken::new();
            f.write("file.txt", "saved\n");
            f.run(&["stash", "push", "-m", "saved"]);
            let entry = r.stashes(&c).unwrap().remove(0);
            let p = r
                .preview_stash_control(StashControl::Pop(entry.clone()), &c)
                .unwrap();
            let stopped = CancellationToken::new();
            stopped.cancel();
            assert!(r.apply_stash_control(&p, &stopped).is_err());
            assert_eq!(r.stashes(&c).unwrap().len(), 1);
            let report = r.apply_stash_control(&p, &c).unwrap();
            assert!(report.result.is_ok(), "{:?}", report.result);
            assert!(report.removed);
            assert!(r.stashes(&c).unwrap().is_empty());
            assert_eq!(fs::read(f.repository.join("file.txt")).unwrap(), b"saved\n");
            assert_eq!(
                String::from_utf8(id(&f, &format!("{}/0", report.recovery)))
                    .unwrap()
                    .trim(),
                entry.id.as_str()
            );
            f.run(&["stash", "push", "-m", "conflicting"]);
            let entry = r.stashes(&c).unwrap().remove(0);
            f.write("file.txt", "different\n");
            f.commit("different");
            let p = r
                .preview_stash_control(StashControl::Pop(entry), &c)
                .unwrap();
            let report = r.apply_stash_control(&p, &c).unwrap();
            assert!(report.result.is_err());
            assert!(!report.removed);
            assert_eq!(r.stashes(&c).unwrap().len(), 1);
            f.run(&["reset", "--hard", "HEAD"]);
            let p = r.preview_stash_control(StashControl::Clear, &c).unwrap();
            f.write("other.txt", "another");
            f.run(&["stash", "push", "-u", "-m", "newer"]);
            assert!(r.apply_stash_control(&p, &c).is_err());
            let p = r.preview_stash_control(StashControl::Clear, &c).unwrap();
            f.write("dirty.txt", "keep");
            let report = r.apply_stash_control(&p, &c).unwrap();
            assert!(report.result.is_ok());
            assert!(r.stashes(&c).unwrap().is_empty());
            assert_eq!(fs::read(f.repository.join("dirty.txt")).unwrap(), b"keep");
            for (i, e) in p.entries().iter().enumerate() {
                assert_eq!(
                    String::from_utf8(id(&f, &format!("{}/{i}", report.recovery)))
                        .unwrap()
                        .trim(),
                    e.id.as_str()
                );
            }
        },
    )
}
#[test]
fn selected_hunk_stash_preserves_other_hunks_and_staging() {
    isolated(
        "ROCKET_GAPS_TEST",
        "selected_hunk_stash_preserves_other_hunks_and_staging",
        || {
            let f = Fixture::new();
            let base = (0..30).map(|i| format!("line {i}\n")).collect::<String>();
            f.write("文.txt", &base);
            f.write("other.txt", "old\n");
            f.commit("base");
            let changed = base
                .replace("line 1\n", "selected\n")
                .replace("line 25\n", "keep\n");
            f.write("文.txt", &changed);
            f.write("other.txt", "staged\n");
            f.run(&["add", "other.txt"]);
            let r = f.repo();
            let c = CancellationToken::new();
            let path = RepoPath::new("文.txt".as_bytes().to_vec()).unwrap();
            let review = r.review_change(&path, false, &c).unwrap();
            let selected = review.changed_lines()[..2].to_vec();
            let p = r
                .preview_stash_selection(&review, &selected, "Selected 文", &c)
                .unwrap();
            let before = fs::read(f.repository.join(".git/index")).unwrap();
            let report = r.stash_selection(&p, &c).unwrap();
            assert!(report.result.is_ok(), "{:?}", report.result);
            assert_eq!(fs::read(f.repository.join(".git/index")).unwrap(), before);
            assert_eq!(
                fs::read(f.repository.join("文.txt")).unwrap(),
                base.replace("line 25\n", "keep\n").as_bytes()
            );
            let stash = report.stash.unwrap();
            let show = f.run(&["show", &format!("{stash}:文.txt")]);
            assert_eq!(show, base.replace("line 1\n", "selected\n").as_bytes());
            assert_eq!(f.run(&["show", &format!("{stash}:other.txt")]), b"old\n");
        },
    )
}
#[test]
fn extended_patch_binary_rename_mode_and_mail_preview_apply() {
    isolated(
        "ROCKET_GAPS_TEST",
        "extended_patch_binary_rename_mode_and_mail_preview_apply",
        || {
            let f = Fixture::new();
            fs::write(f.repository.join("image.bin"), [0, 1, 2, 3, 4]).unwrap();
            f.write("old 文.txt", "rename\n");
            f.write("script.sh", "echo hello\n");
            f.commit("base");
            let base = String::from_utf8(id(&f, "HEAD")).unwrap();
            fs::write(f.repository.join("image.bin"), [0, 5, 6, 7, 8, 9]).unwrap();
            f.run(&["mv", "old 文.txt", "new 文.txt"]);
            f.run(&["update-index", "--chmod=+x", "script.sh"]);
            f.commit("extended");
            let mail = f.run(&["format-patch", "-1", "--stdout", "--binary", "--full-index"]);
            f.run(&["reset", "--hard", base.trim()]);
            let r = f.repo();
            let c = CancellationToken::new();
            let p = r.preview_patch_text(&mail, &c).unwrap();
            assert_eq!(p.mode_changes().len(), 1);
            assert_eq!(p.files().len(), 4);
            let stopped = CancellationToken::new();
            stopped.cancel();
            assert!(r.apply_patch_preview(&p, &stopped).is_err());
            assert!(f.repository.join("old 文.txt").is_file());
            let report = r.apply_patch_preview(&p, &c).unwrap();
            assert!(report.result.is_ok(), "{:?}", report.result);
            assert!(report.verification.is_ok(), "{:?}", report.verification);
            assert!(!report.recovery.is_empty());
            assert_eq!(
                fs::read(f.repository.join("image.bin")).unwrap(),
                [0, 5, 6, 7, 8, 9]
            );
            assert!(!f.repository.join("old 文.txt").exists());
            assert_eq!(
                fs::read(f.repository.join("new 文.txt")).unwrap(),
                b"rename\n"
            );
            assert!(
                f.run(&["ls-files", "--stage", "script.sh"])
                    .starts_with(b"100755 ")
            );
            assert_eq!(String::from_utf8(id(&f, "HEAD")).unwrap(), base);
        },
    )
}
#[test]
fn extended_patch_export_and_external_changes_are_guarded() {
    isolated(
        "ROCKET_GAPS_TEST",
        "extended_patch_export_and_external_changes_are_guarded",
        || {
            let f = Fixture::new();
            fs::write(f.repository.join("binary.bin"), [0, 1, 2]).unwrap();
            f.commit("base");
            fs::write(f.repository.join("binary.bin"), [0, 9, 8]).unwrap();
            let r = f.repo();
            let c = CancellationToken::new();
            let target = f
                .repository
                .parent()
                .unwrap()
                .canonicalize()
                .unwrap()
                .join("out.patch");
            let p = r
                .preview_patch_export(PatchExportSource::Unstaged, &target, &c)
                .unwrap();
            assert!(String::from_utf8_lossy(p.patch()).contains("GIT binary patch"));
            r.export_patch(&p, &c).unwrap();
            f.run(&["restore", "binary.bin"]);
            let import = r.preview_patch_import(&target, &c).unwrap();
            fs::write(f.repository.join("binary.bin"), [0, 6, 5]).unwrap();
            assert!(r.apply_patch_preview(&import, &c).is_err());
            assert_eq!(
                fs::read(f.repository.join("binary.bin")).unwrap(),
                [0, 6, 5]
            );
            let hostile =
                b"diff --git a/../outside b/../outside\nold mode 100644\nnew mode 100755\n";
            assert!(r.preview_patch_text(hostile, &c).is_err());
        },
    )
}
