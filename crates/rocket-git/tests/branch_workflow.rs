mod common;
use common::{Fixture, isolated};
use rocket_git::*;
use std::fs;
fn setup(conflict: bool) -> Fixture {
    let f = Fixture::new();
    f.write("共同 文.txt", "base\n");
    f.commit("base");
    f.run(&["switch", "-c", "feature/検索"]);
    f.write(
        if conflict {
            "共同 文.txt"
        } else {
            "feature 文.txt"
        },
        "feature\n",
    );
    f.commit("feature");
    f.run(&["switch", "main"]);
    f.write(
        if conflict {
            "共同 文.txt"
        } else {
            "base 文.txt"
        },
        "main\n",
    );
    f.commit("main");
    f.run(&["switch", "feature/検索"]);
    f
}
fn oid(f: &Fixture, revision: &str) -> String {
    String::from_utf8(f.run(&["rev-parse", revision]))
        .unwrap()
        .trim()
        .into()
}
#[test]
fn sync_opt_in_restores_staging_edits_and_retains_recovery() {
    isolated("ROCKET_BRANCH_HELPER_TEST", "sync_opt_in_restores_staging_edits_and_retains_recovery", || {
        let f=setup(false);let repo=f.repo();let c=CancellationToken::new();
        let tip=oid(&f,"HEAD");f.write("共同 文.txt","staged\n");f.run(&["add","--","共同 文.txt"]);
        f.write("共同 文.txt","working\n");f.write("keep.txt","untracked");
        assert!(repo.preview_branch_workflow(BranchWorkflowKind::Sync,"main",&c).is_err());
        let p=repo.preview_branch_workflow_with_options(BranchWorkflowKind::Sync,"main",BranchWorkflowOptions{save_changes:true,..Default::default()},&c).unwrap();
        assert_eq!(p.saved_changes(),1);let r=repo.apply_branch_workflow(&p,&c).unwrap();r.result.unwrap();
        assert_eq!(f.run(&["show",":共同 文.txt"]),b"staged\n");
        assert_eq!(fs::read(f.repository.join("共同 文.txt")).unwrap(),b"working\n");
        assert_eq!(fs::read(f.repository.join("keep.txt")).unwrap(),b"untracked");
        assert_eq!(repo.stashes(&c).unwrap()[0].id,r.recovery_stash.unwrap());
        assert!(r.saved_work_journal.unwrap().is_file());assert_eq!(oid(&f,&format!("{}/feature",r.recovery_prefix)),tip);
        assert_eq!(f.run(&["branch","--show-current"]),"feature/検索\n".as_bytes());
    })
}
#[test]
fn sync_saved_review_rejects_external_edits_and_precancel() {
    isolated("ROCKET_BRANCH_HELPER_TEST", "sync_saved_review_rejects_external_edits_and_precancel", || {
        let f=setup(false);let repo=f.repo();let c=CancellationToken::new();f.write("共同 文.txt","reviewed\n");
        let p=repo.preview_branch_workflow_with_options(BranchWorkflowKind::Sync,"main",BranchWorkflowOptions{save_changes:true,..Default::default()},&c).unwrap();
        let stop=CancellationToken::new();stop.cancel();assert!(matches!(repo.apply_branch_workflow(&p,&stop),Err(Error::Cancelled)));
        f.write("共同 文.txt","external\n");let head=oid(&f,"HEAD");
        let r=repo.apply_branch_workflow(&p,&c).unwrap();assert!(r.result.is_err());assert!(r.recovery_stash.is_none());
        assert_eq!(oid(&f,"HEAD"),head);assert!(repo.stashes(&c).unwrap().is_empty());
        assert_eq!(fs::read(f.repository.join("共同 文.txt")).unwrap(),b"external\n");
    })
}
#[test]
fn sync_conflict_keeps_saved_edits_for_abort_and_restore() {
    isolated("ROCKET_BRANCH_HELPER_TEST", "sync_conflict_keeps_saved_edits_for_abort_and_restore", || {
        let f=setup(true);f.write("draft.txt","committed\n");f.commit("draft base");
        f.write("draft.txt","private edit\n");let repo=f.repo();let c=CancellationToken::new();let head=oid(&f,"HEAD");
        let p=repo.preview_branch_workflow_with_options(BranchWorkflowKind::Sync,"main",BranchWorkflowOptions{save_changes:true,..Default::default()},&c).unwrap();
        let r=repo.apply_branch_workflow(&p,&c).unwrap();assert!(r.result.is_err());assert!(!r.state.unwrap().is_idle());
        let stash=r.recovery_stash.unwrap();assert!(r.saved_work_journal.unwrap().is_file());
        let abort=repo.preview_rebase(RebaseAction::Abort,"",&c).unwrap();repo.apply_rebase(&abort,&c).unwrap().write.result.unwrap();
        assert_eq!(oid(&f,"HEAD"),head);repo.apply_stash(&stash,true,&c).unwrap().write.result.unwrap();
        assert_eq!(fs::read(f.repository.join("draft.txt")).unwrap(),b"private edit\n");assert_eq!(repo.stashes(&c).unwrap()[0].id,stash);
    })
}
#[test]
fn finish_fast_forward_choice_and_diverged_merge_keep_both_branches() {
    isolated("ROCKET_BRANCH_HELPER_TEST", "finish_fast_forward_choice_and_diverged_merge_keep_both_branches", || {
        for diverged in [false,true] {
            let f=if diverged {setup(false)} else {let f=Fixture::new();f.write("base.txt","base");f.commit("base");f.run(&["switch","-c","feature/検索"]);f.write("new.txt","feature");f.commit("feature");f};
            let repo=f.repo();let c=CancellationToken::new();let tip=oid(&f,"HEAD");let base=oid(&f,"main");
            let p=repo.preview_branch_workflow_with_options(BranchWorkflowKind::Finish,"main",BranchWorkflowOptions{allow_fast_forward:true,..Default::default()},&c).unwrap();
            assert_eq!(p.can_fast_forward(),!diverged);repo.apply_branch_workflow(&p,&c).unwrap().result.unwrap();
            assert_eq!(oid(&f,"feature/検索"),tip);assert_eq!(f.run(&["branch","--show-current"]),b"main\n");
            if diverged {assert_eq!(oid(&f,"HEAD^1"),base);assert_eq!(oid(&f,"HEAD^2"),tip)}else{assert_eq!(oid(&f,"HEAD"),tip)}
        }
    })
}
#[test]
fn sync_restore_conflict_retains_stash_and_rebased_checkout() {
    isolated("ROCKET_BRANCH_HELPER_TEST","sync_restore_conflict_retains_stash_and_rebased_checkout",|| {
        let f=setup(false);f.run(&["switch","main"]);f.write("共同 文.txt","upstream\n");f.commit("upstream edit");f.run(&["switch","feature/検索"]);
        f.write("共同 文.txt","local edit\n");let repo=f.repo();let c=CancellationToken::new();
        let p=repo.preview_branch_workflow_with_options(BranchWorkflowKind::Sync,"main",BranchWorkflowOptions{save_changes:true,..Default::default()},&c).unwrap();
        let r=repo.apply_branch_workflow(&p,&c).unwrap();assert!(r.result.is_err());assert_eq!(r.phase,"reapplying");
        assert!(r.state.unwrap().is_idle());assert!(repo.status(&c).unwrap().entries.iter().any(|e|e.kind==EntryKind::Unmerged));
        let stash=r.recovery_stash.unwrap();assert_eq!(repo.stashes(&c).unwrap()[0].id,stash);
        assert_eq!(f.run(&["show",&format!("{stash}:共同 文.txt")]),b"local edit\n");
    });
}
#[test]
fn sync_saved_work_refuses_untracked_and_ignored_overwrite() {
    isolated("ROCKET_BRANCH_HELPER_TEST","sync_saved_work_refuses_untracked_and_ignored_overwrite",|| {
        for ignored in [false,true] {
            let f=setup(false);if ignored {f.write(".git/info/exclude","base 文.txt\n");}
            f.write("base 文.txt","KEEP PRIVATE\n");f.write("共同 文.txt","local edit\n");let repo=f.repo();let c=CancellationToken::new();let head=oid(&f,"HEAD");
            let p=repo.preview_branch_workflow_with_options(BranchWorkflowKind::Sync,"main",BranchWorkflowOptions{save_changes:true,..Default::default()},&c).unwrap();
            let r=repo.apply_branch_workflow(&p,&c).unwrap();assert!(r.result.is_err());assert_eq!(oid(&f,"HEAD"),head);
            assert_eq!(fs::read(f.repository.join("base 文.txt")).unwrap(),b"KEEP PRIVATE\n");
            let stash=r.recovery_stash.unwrap();assert_eq!(f.run(&["show",&format!("{stash}:共同 文.txt")]),b"local edit\n");
        }
    });
}
#[test]
fn sync_finish_keep_recovery_tips_and_both_branches() {
    isolated(
        "ROCKET_BRANCH_HELPER_TEST",
        "sync_finish_keep_recovery_tips_and_both_branches",
        || {
            let f = setup(false);
            let r = f.repo();
            let c = CancellationToken::new();
            let original = oid(&f, "HEAD");
            let p = r
                .preview_branch_workflow(BranchWorkflowKind::Sync, "main", &c)
                .unwrap();
            assert_eq!(p.feature(), "feature/検索");
            assert_eq!(p.commits(), 1);
            let report = r.apply_branch_workflow(&p, &c).unwrap();
            assert!(report.result.is_ok(), "{:?}", report.result);
            assert_eq!(report.phase, "finished");
            assert_eq!(
                oid(&f, &format!("{}/feature", report.recovery_prefix)),
                original
            );
            assert!(report.journal.is_file());
            assert!(f.repository.join("base 文.txt").is_file());
            let feature = oid(&f, "HEAD");
            let base = oid(&f, "main");
            let p = r
                .preview_branch_workflow(BranchWorkflowKind::Finish, "main", &c)
                .unwrap();
            let report = r.apply_branch_workflow(&p, &c).unwrap();
            assert!(report.result.is_ok(), "{:?}", report.result);
            assert_eq!(f.run(&["branch", "--show-current"]), b"main\n");
            assert_eq!(oid(&f, "feature/検索"), feature);
            assert_eq!(oid(&f, "HEAD^1"), base);
            assert_eq!(oid(&f, "HEAD^2"), feature);
            assert_eq!(oid(&f, &format!("{}/base", report.recovery_prefix)), base);
        },
    )
}
#[test]
fn helpers_reject_edits_stale_base_and_cancel_without_checkout_changes() {
    isolated(
        "ROCKET_BRANCH_HELPER_TEST",
        "helpers_reject_edits_stale_base_and_cancel_without_checkout_changes",
        || {
            let f = setup(false);
            let r = f.repo();
            let c = CancellationToken::new();
            let original = f.run(&["rev-parse", "HEAD"]);
            let index = fs::read(f.repository.join(".git/index")).unwrap();
            for kind in [BranchWorkflowKind::Sync, BranchWorkflowKind::Finish] {
                let p = r.preview_branch_workflow(kind, "main", &c).unwrap();
                let stop = CancellationToken::new();
                stop.cancel();
                assert!(matches!(
                    r.apply_branch_workflow(&p, &stop),
                    Err(Error::Cancelled)
                ));
                f.write("untracked.keep", "keep");
                assert!(r.apply_branch_workflow(&p, &c).is_err());
                assert_eq!(
                    fs::read_to_string(f.repository.join("untracked.keep")).unwrap(),
                    "keep"
                );
                fs::remove_file(f.repository.join("untracked.keep")).unwrap();
                assert_eq!(f.run(&["rev-parse", "HEAD"]), original);
                assert_eq!(fs::read(f.repository.join(".git/index")).unwrap(), index);
            }
            let p = r
                .preview_branch_workflow(BranchWorkflowKind::Finish, "main", &c)
                .unwrap();
            f.run(&["update-ref", "refs/heads/main", "HEAD"]);
            assert!(r.apply_branch_workflow(&p, &c).is_err());
            assert_eq!(f.run(&["rev-parse", "HEAD"]), original);
            f.run(&["update-index", "--assume-unchanged", "共同 文.txt"]);
            assert!(
                r.preview_branch_workflow(BranchWorkflowKind::Sync, "main", &c)
                    .is_err()
            );
        },
    )
}
#[test]
fn finish_conflict_keeps_original_tips_and_abort_keeps_feature() {
    isolated(
        "ROCKET_BRANCH_HELPER_TEST",
        "finish_conflict_keeps_original_tips_and_abort_keeps_feature",
        || {
            let f = setup(true);
            let r = f.repo();
            let c = CancellationToken::new();
            let feature = oid(&f, "HEAD");
            let base = oid(&f, "main");
            let p = r
                .preview_branch_workflow(BranchWorkflowKind::Finish, "main", &c)
                .unwrap();
            let report = r.apply_branch_workflow(&p, &c).unwrap();
            assert!(report.result.is_err());
            assert_eq!(report.phase, "merging");
            assert!(!report.state.unwrap().merge_heads.is_empty());
            assert_eq!(
                oid(&f, &format!("{}/feature", report.recovery_prefix)),
                feature
            );
            assert_eq!(oid(&f, "main"), base);
            let abort = r.abort_merge(&c).unwrap();
            assert!(abort.write.result.is_ok());
            assert_eq!(oid(&f, "HEAD"), base);
            assert_eq!(oid(&f, "feature/検索"), feature);
            assert_eq!(
                fs::read_to_string(f.repository.join("共同 文.txt")).unwrap(),
                "main\n"
            );
        },
    )
}
