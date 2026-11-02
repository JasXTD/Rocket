mod common;
use common::{Fixture, isolated};
use rocket_git::*;
use std::fs;

fn text(f: &Fixture, args: &[&str]) -> String {
    String::from_utf8(f.run(args)).unwrap().trim().into()
}
fn oid(f: &Fixture) -> ObjectId {
    ObjectId::parse(&text(f, &["rev-parse", "HEAD"])).unwrap()
}
fn preview(f: &Fixture) -> RebasePreview {
    f.repo()
        .preview_rebase(RebaseAction::Interactive, "main", &CancellationToken::new())
        .unwrap()
}
fn apply(f: &Fixture, p: &RebasePreview) -> RebaseWriteReport {
    f.repo().apply_rebase(p, &CancellationToken::new()).unwrap()
}
fn success(r: RebaseWriteReport) {
    assert!(r.write.result.is_ok(), "{r:?}");
    assert!(r.state.unwrap().is_idle());
    assert_eq!(r.failure, None);
}
fn setup(format: &str) -> (Fixture, Vec<ObjectId>) {
    let f = Fixture::with_format(format);
    f.write("base", "base\n");
    f.commit("base");
    f.run(&["switch", "-c", "feature/検索"]);
    let mut ids = Vec::new();
    for name in ["A", "B", "C", "D", "E", "F"] {
        f.write(&format!("検索/{name}.txt"), format!("{name}\n"));
        f.commit(&format!("PRIVATE_MESSAGE {name}"));
        ids.push(oid(&f));
    }
    (f, ids)
}
#[test]
fn reorder_reword_squash_fixup_drop_sha1_sha256() {
    isolated(
        "ROCKET_IR_CASE",
        "reorder_reword_squash_fixup_drop_sha1_sha256",
        || {
            for format in ["sha1", "sha256"] {
                let (f, ids) = setup(format);
                let original = oid(&f);
                f.run(&["branch", "other"]);
                f.write("untracked", "keep\n");
                for (key, value) in [
                    (
                        "rebase.instructionFormat",
                        "%s%nexec touch PRIVATE_INJECTION",
                    ),
                    ("rebase.abbreviateCommands", "true"),
                    ("rebase.autosquash", "true"),
                    ("rebase.updateRefs", "true"),
                ] {
                    f.run(&["config", key, value]);
                }
                let config = fs::read(f.repository.join(".git/config")).unwrap();
                let p = preview(&f);
                assert_eq!(p.commits().len(), 6);
                let mut rows = p.plan().unwrap().to_vec();
                rows.swap(0, 1);
                rows[0].instruction = RebaseInstruction::Reword;
                rows[0].message =
                    "PRIVATE_REWORD 中文 🚀\n\n# literal line\n$(touch PRIVATE_INJECTION)".into();
                rows[2].instruction = RebaseInstruction::Squash;
                rows[2].message = "PRIVATE_COMBINED 中文\n\nCombined A and C".into();
                rows[3].instruction = RebaseInstruction::Fixup;
                rows[4].instruction = RebaseInstruction::Drop;
                let p = p.with_plan(rows).unwrap();
                success(apply(&f, &p));
                assert_eq!(text(&f, &["rev-list", "--count", "main..HEAD"]), "3");
                assert_eq!(
                    text(&f, &["log", "--reverse", "--format=%s", "main..HEAD"]),
                    "PRIVATE_REWORD 中文 🚀\nPRIVATE_COMBINED 中文\nPRIVATE_MESSAGE F"
                );
                assert!(
                    text(&f, &["show", "-s", "--format=%B", "HEAD~2"]).contains("# literal line")
                );
                assert_eq!(text(&f, &["rev-parse", "other"]), original.as_str());
                assert_eq!(
                    text(&f, &["rev-parse", &p.recovery_reference()]),
                    original.as_str()
                );
                assert!(!f.repository.join("検索/E.txt").exists());
                for name in ["A", "B", "C", "D", "F"] {
                    assert!(f.repository.join(format!("検索/{name}.txt")).exists());
                }
                assert_eq!(fs::read(f.repository.join("untracked")).unwrap(), b"keep\n");
                assert_eq!(fs::read(f.repository.join(".git/config")).unwrap(), config);
                assert!(!f.repository.join("PRIVATE_INJECTION").exists());
                assert_eq!(ids.len(), 6);
            }
        },
    );
}

#[test]
fn invalid_plans_and_stale_preview_do_not_rewrite() {
    isolated(
        "ROCKET_IR_CASE",
        "invalid_plans_and_stale_preview_do_not_rewrite",
        || {
            let (f, _) = setup("sha1");
            let head = oid(&f);
            let p = preview(&f);
            let rows = p.plan().unwrap().to_vec();
            for invalid in 0..9 {
                let mut r = rows.clone();
                match invalid {
                    0 => {
                        r.pop();
                    }
                    1 => r[1] = r[0].clone(),
                    2 => r[0].instruction = RebaseInstruction::Fixup,
                    3 => r[0].instruction = RebaseInstruction::Squash,
                    4 => r[0].instruction = RebaseInstruction::Reword,
                    5 => {
                        r[0].instruction = RebaseInstruction::Reword;
                        r[0].message = "invalid\0message".into();
                    }
                    6 => {
                        r[0].instruction = RebaseInstruction::Reword;
                        r[0].message = "界".repeat(21846); // byte limit, not character count
                    }
                    7 => r[0].message = "x".repeat(512 * 1024 + 1),
                    _ => {
                        r[0].instruction = RebaseInstruction::Drop;
                        r[1].instruction = RebaseInstruction::Fixup;
                    }
                };
                assert!(matches!(
                    p.with_plan(r),
                    Err(Error::Rebase(RebaseFailure::InvalidPlan))
                ));
            }
            f.write("検索/A.txt", "external\n");
            assert!(
                f.repo()
                    .apply_rebase(&p, &CancellationToken::new())
                    .is_err()
            );
            assert_eq!(oid(&f), head);
            assert_eq!(
                fs::read(f.repository.join("検索/A.txt")).unwrap(),
                b"external\n"
            );
        },
    );
}

fn conflict() -> Fixture {
    let f = Fixture::new();
    f.write("検索.txt", "base\n");
    f.commit("base");
    f.run(&["switch", "-c", "feature"]);
    f.write("検索.txt", "feature\n");
    f.commit("first");
    f.write("later", "later\n");
    f.commit("later");
    f.run(&["switch", "main"]);
    f.write("検索.txt", "main\n");
    f.commit("main");
    f.run(&["switch", "feature"]);
    f
}
#[test]
fn conflict_reword_continue_skip_abort_and_plan_tampering() {
    isolated(
        "ROCKET_IR_CASE",
        "conflict_reword_continue_skip_abort_and_plan_tampering",
        || {
            for action in [
                RebaseAction::Continue,
                RebaseAction::Skip,
                RebaseAction::Abort,
            ] {
                let f = conflict();
                let head = oid(&f);
                let p = preview(&f);
                let mut rows = p.plan().unwrap().to_vec();
                rows[0].instruction = RebaseInstruction::Reword;
                rows[0].message = "Resolved 中文\n\nbody".into();
                let p = p.with_plan(rows).unwrap();
                let r = apply(&f, &p);
                assert_eq!(r.failure, Some(RebaseFailure::ConflictsRemain), "{r:?}");
                let todo = f.repository.join(".git/rebase-merge/git-rebase-todo");
                let saved = fs::read(&todo).unwrap();
                fs::write(&todo, format!("drop {}\n", p.commits()[1].id)).unwrap();
                assert!(matches!(
                    f.repo()
                        .preview_rebase(RebaseAction::Abort, "", &CancellationToken::new()),
                    Err(Error::Rebase(RebaseFailure::UnsupportedSequence))
                ));
                fs::write(todo, saved).unwrap();
                if action == RebaseAction::Continue {
                    f.write("検索.txt", "resolved\n");
                    f.run(&["add", "--", "検索.txt"]);
                }
                let recovery = f
                    .repo()
                    .preview_rebase(action, "", &CancellationToken::new())
                    .unwrap();
                assert!(recovery.plan().is_some());
                success(apply(&f, &recovery));
                if action == RebaseAction::Abort {
                    assert_eq!(oid(&f), head);
                } else if action == RebaseAction::Continue {
                    assert_eq!(
                        text(&f, &["log", "--reverse", "--format=%s", "main..HEAD"]),
                        "Resolved 中文\nlater"
                    );
                } else {
                    assert_eq!(text(&f, &["rev-list", "--count", "main..HEAD"]), "1");
                }
            }
        },
    );
}

#[test]
fn all_drop_is_explicit_and_retains_recovery_tip() {
    isolated(
        "ROCKET_IR_CASE",
        "all_drop_is_explicit_and_retains_recovery_tip",
        || {
            let (f, _) = setup("sha1");
            let p = preview(&f);
            let mut rows = p.plan().unwrap().to_vec();
            for row in &mut rows {
                row.instruction = RebaseInstruction::Drop;
            }
            let p = p.with_plan(rows).unwrap();
            success(apply(&f, &p));
            assert_eq!(
                text(&f, &["rev-parse", "HEAD"]),
                text(&f, &["rev-parse", "main"])
            );
            assert_eq!(
                text(&f, &["rev-parse", &p.recovery_reference()]),
                p.original().as_str()
            );
        },
    );
}

#[test]
fn journal_changes_and_missing_review_are_refused() {
    isolated(
        "ROCKET_IR_CASE",
        "journal_changes_and_missing_review_are_refused",
        || {
            let f = conflict();
            let p = preview(&f);
            assert!(apply(&f, &p).state.unwrap().rebase);
            let journal = f.repository.join(".git/rocket-interactive-plan");
            let saved = fs::read(&journal).unwrap();
            fs::write(&journal, b"PRIVATE_CHANGED_PLAN").unwrap();
            for action in [RebaseAction::Abort, RebaseAction::Continue, RebaseAction::Skip] {
                assert!(matches!(
                    f.repo().preview_rebase(action, "", &CancellationToken::new()),
                    Err(Error::Rebase(RebaseFailure::PlanJournal))
                ));
            }
            assert_eq!(fs::read(&journal).unwrap(), b"PRIVATE_CHANGED_PLAN");
            fs::remove_file(&journal).unwrap();
            assert!(matches!(
                f.repo()
                    .preview_rebase(RebaseAction::Skip, "", &CancellationToken::new()),
                Err(Error::Rebase(RebaseFailure::PlanJournal))
            ));
            fs::write(&journal, saved).unwrap();
            let p = f
                .repo()
                .preview_rebase(RebaseAction::Abort, "", &CancellationToken::new())
                .unwrap();
            success(apply(&f, &p));
            assert!(!journal.exists());
        },
    );
}

#[test]
fn squash_conflict_keeps_reviewed_combined_message_and_guards_skip() {
    isolated(
        "ROCKET_IR_CASE",
        "squash_conflict_keeps_reviewed_combined_message_and_guards_skip",
        || {
            let f = conflict();
            let p = preview(&f);
            let mut rows = p.plan().unwrap().to_vec();
            rows[1].instruction = RebaseInstruction::Squash;
            rows[1].message = "PRIVATE_COMBINED resolution 中文".into();
            let p = p.with_plan(rows).unwrap();
            assert!(apply(&f, &p).state.unwrap().rebase);
            assert!(matches!(
                f.repo()
                    .preview_rebase(RebaseAction::Skip, "", &CancellationToken::new()),
                Err(Error::Rebase(RebaseFailure::SkipWouldFoldBase))
            ));
            f.write("検索.txt", "resolved\n");
            f.run(&["add", "--", "検索.txt"]);
            let p = f
                .repo()
                .preview_rebase(RebaseAction::Continue, "", &CancellationToken::new())
                .unwrap();
            success(apply(&f, &p));
            assert_eq!(text(&f, &["rev-list", "--count", "main..HEAD"]), "1");
            assert_eq!(
                text(&f, &["log", "-1", "--format=%s"]),
                "PRIVATE_COMBINED resolution 中文"
            );
            // Reorder to replay the independent commit first, then stop on a squash.
            let f = conflict();
            let p = preview(&f);
            let mut rows = p.plan().unwrap().to_vec();
            rows.swap(0, 1);
            rows[1].instruction = RebaseInstruction::Squash;
            rows[1].message = "PRIVATE_SQUASH resolution 中文".into();
            let p = p.with_plan(rows).unwrap();
            let r = apply(&f, &p);
            assert_eq!(r.failure, Some(RebaseFailure::ConflictsRemain), "{r:?}");
            f.write("検索.txt", "resolved\n");
            f.run(&["add", "--", "検索.txt"]);
            let p = f
                .repo()
                .preview_rebase(RebaseAction::Continue, "", &CancellationToken::new())
                .unwrap();
            success(apply(&f, &p));
            assert_eq!(
                text(&f, &["log", "-1", "--format=%s"]),
                "PRIVATE_SQUASH resolution 中文"
            );
            assert_eq!(text(&f, &["rev-list", "--count", "main..HEAD"]), "1");
        },
    );
}

#[test]
fn reword_hook_failure_can_resume_after_hook_is_fixed() {
    isolated(
        "ROCKET_IR_CASE",
        "reword_hook_failure_can_resume_after_hook_is_fixed",
        || {
            let (f, _) = setup("sha1");
            let p = preview(&f);
            let mut rows = p.plan().unwrap().to_vec();
            rows[0].instruction = RebaseInstruction::Reword;
            rows[0].message = "PRIVATE_REWORD after hook".into();
            let p = p.with_plan(rows).unwrap();
            let hook = f.root.join("no-hooks/commit-msg");
            fs::write(&hook, b"#!/bin/sh\necho PRIVATE_HOOK_FAILURE >&2\nexit 1\n").unwrap();
            let r = apply(&f, &p);
            assert!(r.write.result.is_err());
            assert!(r.state.unwrap().rebase);
            assert!(matches!(
                f.repo()
                    .preview_rebase(RebaseAction::Skip, "", &CancellationToken::new()),
                Err(Error::Rebase(RebaseFailure::MessageOnlyStop))
            ));
            fs::remove_file(hook).unwrap();
            let p = f
                .repo()
                .preview_rebase(RebaseAction::Continue, "", &CancellationToken::new())
                .unwrap_or_else(|e| {
                    panic!(
                        "{e:?}; metadata {:?}",
                        fs::read_dir(f.repository.join(".git/rebase-merge"))
                            .unwrap()
                            .map(|e| e.unwrap().file_name())
                            .collect::<Vec<_>>()
                    )
                });
            success(apply(&f, &p));
            assert_eq!(
                text(&f, &["log", "--reverse", "--format=%s", "main..HEAD"])
                    .lines()
                    .next(),
                Some("PRIVATE_REWORD after hook")
            );
        },
    );
}

#[test]
fn cancellation_after_message_amendment_resumes_without_amending_again() {
    isolated(
        "ROCKET_IR_CASE",
        "cancellation_after_message_amendment_resumes_without_amending_again",
        || {
            let (f, _) = setup("sha1");
            let p = preview(&f);
            let mut rows = p.plan().unwrap().to_vec();
            rows[0].instruction = RebaseInstruction::Reword;
            rows[0].message = "PRIVATE_RESUMED 中文".into();
            let p = p.with_plan(rows).unwrap();
            let hook = f.root.join("no-hooks/commit-msg");
            fs::write(&hook, b"#!/bin/sh\nexit 1\n").unwrap();
            let r = apply(&f, &p);
            assert_eq!(r.failure, Some(RebaseFailure::MessageNotSaved));
            fs::remove_file(hook).unwrap();
            let p = f
                .repo()
                .preview_rebase(RebaseAction::Continue, "", &CancellationToken::new())
                .unwrap();
            assert_eq!(p.message_recovery().map(|(retry, _)| retry), Some(true));
            let post = f.root.join("no-hooks/post-rewrite");
            fs::write(&post,b"#!/bin/sh\nif [ \"$1\" = amend ]; then echo ready > message-ready; sleep 30; fi\n").unwrap();
            let cancel = CancellationToken::new();
            let token = cancel.clone();
            let repo = f.repo();
            let thread = std::thread::spawn(move || repo.apply_rebase(&p, &token));
            let started = std::time::Instant::now();
            while !f.repository.join("message-ready").exists() {
                assert!(started.elapsed() < std::time::Duration::from_secs(20));
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            cancel.cancel();
            let report = thread.join().unwrap().unwrap();
            assert!(report.write.result.is_err());
            assert!(report.state.unwrap().rebase);
            fs::remove_file(post).unwrap();
            let amended = oid(&f);
            assert_eq!(
                text(&f, &["log", "-1", "--format=%s"]),
                "PRIVATE_RESUMED 中文"
            );
            let p = f
                .repo()
                .preview_rebase(RebaseAction::Continue, "", &CancellationToken::new())
                .unwrap();
            assert_eq!(p.message_recovery().map(|(retry, _)| retry), Some(false));
            success(apply(&f, &p));
            assert_eq!(text(&f, &["rev-parse", "HEAD~5"]), amended.as_str());
            assert_eq!(text(&f, &["rev-list", "--count", "main..HEAD"]), "6");
        },
    );
}

fn resume(f: &Fixture, action: RebaseAction) -> (RebasePreview, RebaseWriteReport) {
    let p = f
        .repo()
        .preview_rebase(action, "", &CancellationToken::new())
        .unwrap();
    let r = apply(f, &p);
    (p, r)
}
fn journal(f: &Fixture) -> Vec<u8> {
    fs::read(f.repository.join(".git/rocket-interactive-plan")).unwrap()
}

#[test]
fn edit_stops_amend_staged_changes_or_continue_unchanged_sha1_sha256() {
    isolated(
        "ROCKET_IR_CASE",
        "edit_stops_amend_staged_changes_or_continue_unchanged_sha1_sha256",
        || {
            for format in ["sha1", "sha256"] {
                let (f, ids) = setup(format);
                let p = preview(&f);
                let mut rows = p.plan().unwrap().to_vec();
                rows[1].instruction = RebaseInstruction::Edit;
                rows[3].instruction = RebaseInstruction::Edit;
                let p = p.with_plan(rows).unwrap();
                let r = apply(&f, &p);
                assert!(r.write.result.is_ok(), "{r:?}");
                assert_eq!(r.failure, None);
                assert_eq!(r.stop, Some(RebaseStop::Edit(ids[1].clone())));
                assert!(r.state.unwrap().rebase);
                assert!(journal(&f).starts_with(b"ROCKET-REBASE-2\0"));
                assert_eq!(text(&f, &["log", "-1", "--format=%s"]), "PRIVATE_MESSAGE B");
                // Stage an amendment for B; Continue amends it and stops at D.
                f.write("検索/B.txt", "B amended 中文\n");
                f.run(&["add", "--", "検索/B.txt"]);
                let (p, r) = resume(&f, RebaseAction::Continue);
                assert_eq!(p.planned_stop(), Some(&RebaseStop::Edit(ids[1].clone())));
                assert!(String::from_utf8_lossy(p.patch()).contains("B amended"));
                assert!(r.write.result.is_ok(), "{r:?}");
                assert_eq!(r.stop, Some(RebaseStop::Edit(ids[3].clone())));
                // A clean Continue keeps D unchanged and finishes.
                let (p, r) = resume(&f, RebaseAction::Continue);
                assert_eq!(p.planned_stop(), Some(&RebaseStop::Edit(ids[3].clone())));
                success(r);
                assert_eq!(
                    text(&f, &["log", "--reverse", "--format=%s", "main..HEAD"]),
                    "PRIVATE_MESSAGE A\nPRIVATE_MESSAGE B\nPRIVATE_MESSAGE C\nPRIVATE_MESSAGE D\nPRIVATE_MESSAGE E\nPRIVATE_MESSAGE F"
                );
                assert_eq!(text(&f, &["show", "HEAD~4:検索/B.txt"]), "B amended 中文");
                assert_eq!(
                    text(
                        &f,
                        &[
                            "-c",
                            "core.quotepath=false",
                            "diff",
                            "--name-only",
                            "HEAD~3",
                            "HEAD~2"
                        ]
                    ),
                    "検索/D.txt"
                );
                assert!(!f.repository.join(".git/rocket-interactive-plan").exists());
            }
        },
    );
}

#[test]
fn breaks_and_planned_commands_stop_resume_and_refuse_staged_continue() {
    isolated(
        "ROCKET_IR_CASE",
        "breaks_and_planned_commands_stop_resume_and_refuse_staged_continue",
        || {
            let (f, _) = setup("sha1");
            let p = preview(&f);
            let mut rows = p.plan().unwrap().to_vec();
            rows[0].stop_after = true;
            rows[1].exec = Some("echo ROCKET_EXEC_RAN >> exec-marker.txt".into());
            rows[2].exec = Some("exit 3".into());
            rows[5].exec = Some("echo ROCKET_LAST >> exec-marker.txt".into());
            let p = p.with_plan(rows).unwrap();
            assert!(p.runs_commands());
            let r = apply(&f, &p);
            assert!(r.write.result.is_ok(), "{r:?}");
            assert_eq!((r.failure, r.stop.clone()), (None, Some(RebaseStop::Break)));
            assert_eq!(text(&f, &["log", "-1", "--format=%s"]), "PRIVATE_MESSAGE A");
            // Git cannot continue between commits with staged changes.
            f.write("検索/A.txt", "staged at a stop\n");
            f.run(&["add", "--", "検索/A.txt"]);
            assert!(matches!(
                f.repo()
                    .preview_rebase(RebaseAction::Continue, "", &CancellationToken::new()),
                Err(Error::Rebase(RebaseFailure::StagedAfterStop))
            ));
            f.run(&["reset", "-q", "--hard", "HEAD"]);
            let (p, r) = resume(&f, RebaseAction::Continue);
            assert_eq!(p.planned_stop(), Some(&RebaseStop::Break));
            assert!(
                r.write.result.is_err(),
                "the failing command stops Git: {r:?}"
            );
            assert_eq!(r.failure, Some(RebaseFailure::CommandStopped));
            assert_eq!(r.stop, Some(RebaseStop::Command("exit 3".into())));
            assert_eq!(
                fs::read_to_string(f.repository.join("exec-marker.txt"))
                    .unwrap()
                    .trim(),
                "ROCKET_EXEC_RAN"
            );
            // Tampering with a planned command is refused.
            let todo = f.repository.join(".git/rebase-merge/git-rebase-todo");
            let saved = fs::read(&todo).unwrap();
            fs::write(
                &todo,
                String::from_utf8(saved.clone())
                    .unwrap()
                    .replace("ROCKET_LAST", "PRIVATE_TAMPERED"),
            )
            .unwrap();
            assert!(matches!(
                f.repo()
                    .preview_rebase(RebaseAction::Continue, "", &CancellationToken::new()),
                Err(Error::Rebase(RebaseFailure::UnsupportedSequence))
            ));
            fs::write(&todo, saved).unwrap();
            let (p, r) = resume(&f, RebaseAction::Continue);
            assert_eq!(
                p.planned_stop(),
                Some(&RebaseStop::Command("exit 3".into()))
            );
            success(r);
            assert_eq!(text(&f, &["rev-list", "--count", "main..HEAD"]), "6");
            assert_eq!(
                fs::read_to_string(f.repository.join("exec-marker.txt"))
                    .unwrap()
                    .lines()
                    .collect::<Vec<_>>(),
                ["ROCKET_EXEC_RAN", "ROCKET_LAST"]
            );
        },
    );
}

#[test]
fn skip_at_a_break_discards_tracked_changes_and_continues() {
    isolated(
        "ROCKET_IR_CASE",
        "skip_at_a_break_discards_tracked_changes_and_continues",
        || {
            let (f, _) = setup("sha1");
            let p = preview(&f);
            let mut rows = p.plan().unwrap().to_vec();
            rows[2].stop_after = true;
            let p = p.with_plan(rows).unwrap();
            assert_eq!(apply(&f, &p).stop, Some(RebaseStop::Break));
            f.write("検索/C.txt", "PRIVATE_DISCARDED\n");
            let (p, r) = resume(&f, RebaseAction::Skip);
            assert_eq!(p.planned_stop(), Some(&RebaseStop::Break));
            assert!(String::from_utf8_lossy(p.patch()).contains("PRIVATE_DISCARDED"));
            success(r);
            assert_eq!(text(&f, &["rev-list", "--count", "main..HEAD"]), "6");
            assert_eq!(
                fs::read_to_string(f.repository.join("検索/C.txt")).unwrap(),
                "C\n"
            );
        },
    );
}

#[test]
fn commands_and_stops_are_validated_and_old_plans_keep_version_one() {
    isolated(
        "ROCKET_IR_CASE",
        "commands_and_stops_are_validated_and_old_plans_keep_version_one",
        || {
            let (f, _) = setup("sha1");
            let p = preview(&f);
            let rows = p.plan().unwrap().to_vec();
            let invalid = |edit: &dyn Fn(&mut Vec<RebasePlanEntry>)| {
                let mut r = rows.clone();
                edit(&mut r);
                assert!(
                    matches!(
                        p.with_plan(r.clone()),
                        Err(Error::Rebase(RebaseFailure::InvalidPlan))
                    ),
                    "{r:?}"
                );
            };
            for command in [
                "",
                " leading",
                "trailing ",
                "two\nlines",
                "carriage\rreturn",
                "tab\tcontrol",
            ] {
                invalid(&|r| r[0].exec = Some(command.into()));
            }
            invalid(&|r| r[0].exec = Some("x".repeat(MAX_REBASE_COMMAND + 1)));
            invalid(&|r| {
                r[0].instruction = RebaseInstruction::Drop;
                r[0].stop_after = true;
            });
            invalid(&|r| {
                r[0].stop_after = true;
                r[1].instruction = RebaseInstruction::Fixup;
            });
            invalid(&|r| {
                r[0].exec = Some("true".into());
                r[1].instruction = RebaseInstruction::Drop;
                r[2].instruction = RebaseInstruction::Squash;
                r[2].message = "combined".into();
            });
            // Allowed: a command after the last commit and after a completed chain.
            let mut r = rows.clone();
            r[1].instruction = RebaseInstruction::Fixup;
            r[1].exec = Some("true".into());
            r[5].exec = Some("x".repeat(MAX_REBASE_COMMAND));
            r[5].stop_after = true;
            assert!(p.with_plan(r).is_ok());
            // A plan with only the earlier actions is saved in the earlier format.
            let f = conflict();
            let p = preview(&f);
            let mut rows = p.plan().unwrap().to_vec();
            rows[0].instruction = RebaseInstruction::Reword;
            rows[0].message = "Reworded".into();
            let p = p.with_plan(rows).unwrap();
            assert_eq!(apply(&f, &p).failure, Some(RebaseFailure::ConflictsRemain));
            assert!(journal(&f).starts_with(b"ROCKET-REBASE-1\0"));
            success(resume(&f, RebaseAction::Abort).1);
        },
    );
}

#[test]
fn a_stopped_rebase_counts_commits_not_plan_lines_and_names_its_branch() {
    isolated(
        "ROCKET_IR_CASE",
        "a_stopped_rebase_counts_commits_not_plan_lines_and_names_its_branch",
        || {
            let f = conflict();
            let p = preview(&f);
            assert_eq!(p.onto_commit().unwrap().subject_lossy(), "main", "main's tip isn't in this branch's history");
            assert_eq!(p.target(), "main");
            let mut rows = p.plan().unwrap().to_vec();
            rows[1].stop_after = true;
            rows[1].exec = Some("exit 0".into());
            let r = apply(&f, &p.with_plan(rows).unwrap());
            assert_eq!(r.failure, Some(RebaseFailure::ConflictsRemain), "{r:?}");
            let step = r.state.unwrap().step.unwrap();
            assert_eq!((step.at, step.total), (1, 2));
            assert_eq!(step.subject, "first");
            assert_eq!(step.kind, StepKind::Commit);
            assert_eq!(step.branch.as_deref(), Some(&b"feature"[..]));
            let clean = f.repo().preview_rebase(RebaseAction::Abort, "", &CancellationToken::new()).unwrap();
            assert!(clean.onto_commit().is_none(), "a paused rebase doesn't read it again");
            assert!(apply(&f, &clean).write.result.is_ok());
            assert_eq!(f.repo().operation_state(&CancellationToken::new()).unwrap().step, None);
        },
    );
}
