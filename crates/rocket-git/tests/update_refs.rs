mod common;
use common::{Fixture, isolated};
use rocket_git::*;

fn token() -> CancellationToken {
    CancellationToken::new()
}
fn text(f: &Fixture, args: &[&str]) -> String {
    String::from_utf8(f.run(args)).unwrap().trim().into()
}
fn rev(f: &Fixture, name: &str) -> String {
    text(f, &["rev-parse", name])
}
fn subjects(f: &Fixture, range: &str) -> String {
    text(f, &["log", "--reverse", "--format=%s", range])
}
/// main gets a new commit; stack-1 points at A and stack-2 (checked out) at C,
/// on top of A and B.
fn stack() -> Fixture {
    let f = Fixture::new();
    f.write("base.txt", "base\n");
    f.commit("base");
    f.run(&["switch", "-c", "stack-2"]);
    for name in ["A", "B", "C"] {
        f.write(&format!("{name}.txt"), format!("{name}\n"));
        f.commit(&format!("PRIVATE {name}"));
        if name == "A" {
            f.run(&["branch", "stack-1"]);
        }
    }
    f.run(&["switch", "main"]);
    f.write("main.txt", "main\n");
    f.commit("main moves");
    f.run(&["switch", "stack-2"]);
    f
}
fn success(r: &RebaseWriteReport) {
    assert!(r.write.result.is_ok(), "{r:?}");
    assert!(r.state.as_ref().unwrap().is_idle());
    assert_eq!(r.failure, None);
}

#[test]
fn update_refs_moves_dependent_branches_only_when_chosen() {
    isolated(
        "ROCKET_UPDATE_REFS_CASE",
        "update_refs_moves_dependent_branches_only_when_chosen",
        || {
            for chosen in [false, true] {
                let f = stack();
                let a = rev(&f, "stack-1");
                let repo = f.repo();
                let p = repo
                    .preview_rebase(RebaseAction::Start, "main", &token())
                    .unwrap();
                let dependents = p.dependents();
                assert_eq!(dependents.len(), 1);
                assert_eq!(dependents[0].reference, b"refs/heads/stack-1");
                assert_eq!(dependents[0].target.as_str(), a);
                assert!(dependents[0].moves() && !p.update_refs());
                let p = p.with_update_refs(chosen).unwrap();
                let report = repo.apply_rebase(&p, &token()).unwrap();
                success(&report);
                assert_eq!(
                    subjects(&f, "main..stack-2"),
                    "PRIVATE A\nPRIVATE B\nPRIVATE C"
                );
                if chosen {
                    // stack-1 now points at the rewritten A on top of main.
                    assert_eq!(rev(&f, "stack-1"), rev(&f, "stack-2~2"));
                    assert_eq!(rev(&f, "stack-1~1"), rev(&f, "main"));
                } else {
                    assert_eq!(rev(&f, "stack-1"), a);
                }
            }
        },
    );
}

#[test]
fn a_branch_checked_out_elsewhere_stays_and_cannot_be_the_only_update() {
    isolated(
        "ROCKET_UPDATE_REFS_CASE",
        "a_branch_checked_out_elsewhere_stays_and_cannot_be_the_only_update",
        || {
            let f = stack();
            let a = rev(&f, "stack-1");
            let worktree = f.root.join("stack-1 worktree");
            f.run(&["worktree", "add", worktree.to_str().unwrap(), "stack-1"]);
            let p = f
                .repo()
                .preview_rebase(RebaseAction::Start, "main", &token())
                .unwrap();
            assert_eq!(p.dependents().len(), 1);
            assert!(!p.dependents()[0].moves());
            assert!(p.dependents()[0].checked_out_at.is_some());
            assert!(matches!(
                p.with_update_refs(true),
                Err(Error::Rebase(RebaseFailure::InvalidPlan))
            ));
            // With another branch that can move, the checked-out one still stays.
            f.run(&["branch", "stack-b", "stack-2~1"]);
            let p = f
                .repo()
                .preview_rebase(RebaseAction::Start, "main", &token())
                .unwrap()
                .with_update_refs(true)
                .unwrap();
            success(&f.repo().apply_rebase(&p, &token()).unwrap());
            assert_eq!(rev(&f, "stack-1"), a);
            assert_eq!(rev(&f, "stack-b"), rev(&f, "stack-2~1"));
        },
    );
}

#[test]
fn interactive_plans_place_updates_after_squashes_and_drops() {
    isolated(
        "ROCKET_UPDATE_REFS_CASE",
        "interactive_plans_place_updates_after_squashes_and_drops",
        || {
            let f = stack();
            // stack-1 points at A, into which C is squashed; stack-b points at B,
            // which is dropped.
            f.run(&["branch", "stack-b", "stack-2~1"]);
            let p = f
                .repo()
                .preview_rebase(RebaseAction::Interactive, "main", &token())
                .unwrap()
                .with_update_refs(true)
                .unwrap();
            let mut rows = p.plan().unwrap().to_vec();
            // A, C (squashed into A), B (dropped).
            rows.swap(1, 2);
            rows[1].instruction = RebaseInstruction::Squash;
            rows[1].message = "PRIVATE A and C".into();
            rows[2].instruction = RebaseInstruction::Drop;
            let p = p.with_plan(rows).unwrap();
            success(&f.repo().apply_rebase(&p, &token()).unwrap());
            assert_eq!(subjects(&f, "main..stack-2"), "PRIVATE A and C");
            // stack-1 (at A) gets the combined commit, after the squash.
            assert_eq!(rev(&f, "stack-1"), rev(&f, "stack-2"));
            // stack-b (at the dropped B) gets the commit before it.
            assert_eq!(rev(&f, "stack-b"), rev(&f, "stack-2"));
            assert!(!f.repository.join("B.txt").exists());
        },
    );
}

#[test]
fn moved_dependents_make_the_review_stale() {
    isolated(
        "ROCKET_UPDATE_REFS_CASE",
        "moved_dependents_make_the_review_stale",
        || {
            let f = stack();
            let head = rev(&f, "HEAD");
            let p = f
                .repo()
                .preview_rebase(RebaseAction::Start, "main", &token())
                .unwrap()
                .with_update_refs(true)
                .unwrap();
            f.run(&["branch", "-f", "stack-1", "stack-2~1"]);
            assert!(matches!(
                f.repo().apply_rebase(&p, &token()),
                Err(Error::Rebase(RebaseFailure::StalePreview))
            ));
            assert_eq!(rev(&f, "HEAD"), head);
            // Without --update-refs the other branches do not matter.
            let p = f
                .repo()
                .preview_rebase(RebaseAction::Start, "main", &token())
                .unwrap();
            f.run(&["branch", "-f", "stack-1", "stack-2~2"]);
            success(&f.repo().apply_rebase(&p, &token()).unwrap());
        },
    );
}

#[test]
fn a_conflict_keeps_the_updates_for_continue_and_abort_drops_them() {
    isolated(
        "ROCKET_UPDATE_REFS_CASE",
        "a_conflict_keeps_the_updates_for_continue_and_abort_drops_them",
        || {
            for finish in [RebaseAction::Continue, RebaseAction::Abort] {
                let f = stack();
                let a = rev(&f, "stack-1");
                // main changes B.txt too, so replaying B conflicts.
                f.run(&["switch", "main"]);
                f.write("B.txt", "main's B\n");
                f.commit("main writes B");
                f.run(&["switch", "stack-2"]);
                let repo = f.repo();
                let p = repo
                    .preview_rebase(RebaseAction::Start, "main", &token())
                    .unwrap()
                    .with_update_refs(true)
                    .unwrap();
                let report = repo.apply_rebase(&p, &token()).unwrap();
                assert!(report.state.as_ref().unwrap().rebase);
                if finish == RebaseAction::Continue {
                    f.write("B.txt", "resolved\n");
                    f.run(&["add", "B.txt"]);
                }
                // The rebase in progress lists the branch it will move.
                let resume = repo.preview_rebase(finish, "", &token()).unwrap();
                assert!(resume.update_refs());
                assert_eq!(resume.dependents()[0].reference, b"refs/heads/stack-1");
                assert_eq!(resume.dependents()[0].target.as_str(), a);
                if finish == RebaseAction::Continue {
                    success(&repo.apply_rebase(&resume, &token()).unwrap());
                    assert_eq!(rev(&f, "stack-1"), rev(&f, "stack-2~2"));
                    assert_ne!(rev(&f, "stack-1"), a);
                } else {
                    success(&repo.apply_rebase(&resume, &token()).unwrap());
                    assert_eq!(rev(&f, "stack-1"), a);
                }
            }
        },
    );
}
