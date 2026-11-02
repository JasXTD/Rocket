//! Interactive rebase from the root commit (`git rebase --root`).
mod common;
use common::{Fixture, isolated};
use rocket_git::*;

const CASE: &str = "ROCKET_ROOT_REBASE_CASE";

fn text(f: &Fixture, args: &[&str]) -> String {
    String::from_utf8(f.run(args)).unwrap().trim().into()
}
fn head(f: &Fixture) -> ObjectId {
    ObjectId::parse(&text(f, &["rev-parse", "HEAD"])).unwrap()
}
fn token() -> CancellationToken {
    CancellationToken::new()
}
fn entry(id: &ObjectId, instruction: RebaseInstruction) -> RebasePlanEntry {
    RebasePlanEntry::new(id.clone(), instruction)
}
/// One commit per file, oldest first; the first is the root commit.
fn commits(f: &Fixture, files: &[(&str, &str, &str)]) -> Vec<ObjectId> {
    files
        .iter()
        .map(|(name, content, message)| {
            f.write(name, content);
            f.commit(message);
            head(f)
        })
        .collect()
}
/// The parentless commits reachable from HEAD.
fn roots(f: &Fixture) -> Vec<String> {
    text(f, &["rev-list", "--max-parents=0", "HEAD"])
        .lines()
        .map(Into::into)
        .collect()
}
fn succeeded(r: &RebaseWriteReport) {
    assert!(r.write.result.is_ok(), "{r:?}");
    assert_eq!(r.failure, None, "{r:?}");
    assert!(r.journal_cleanup.is_ok(), "{r:?}");
}

#[test]
fn the_whole_branch_is_rewritten_and_the_first_kept_commit_becomes_the_root() {
    isolated(
        CASE,
        "the_whole_branch_is_rewritten_and_the_first_kept_commit_becomes_the_root",
        || {
            for format in ["sha1", "sha256"] {
                let f = Fixture::with_format(format);
                let ids = commits(
                    &f,
                    &[
                        ("a.txt", "a\n", "add a"),
                        ("b.txt", "b\n", "add b"),
                        ("c.txt", "c\n", "add c"),
                    ],
                );
                let original = head(&f);
                let p = f.repo().preview_root_rebase(&token()).unwrap();
                assert_eq!(p.onto(), None);
                assert!(!p.up_to_date() && !p.fast_forward());
                let listed: Vec<ObjectId> = p.commits().iter().map(|c| c.id.clone()).collect();
                assert_eq!(listed, ids);
                // Reword the root commit and move c below b.
                let mut first = entry(&ids[0], RebaseInstruction::Reword);
                first.message = "PRIVATE_ROOT 中文".into();
                let p = p
                    .with_plan(vec![
                        first,
                        entry(&ids[2], RebaseInstruction::Pick),
                        entry(&ids[1], RebaseInstruction::Pick),
                    ])
                    .unwrap();
                let r = f.repo().apply_rebase(&p, &token()).unwrap();
                succeeded(&r);
                assert!(r.state.unwrap().is_idle());
                assert_eq!(
                    text(&f, &["log", "--reverse", "--format=%s"]),
                    "PRIVATE_ROOT 中文\nadd c\nadd b"
                );
                // The reworded commit is itself the root: no empty base was kept.
                assert_eq!(text(&f, &["rev-list", "--count", "HEAD"]), "3");
                assert_eq!(roots(&f), [text(&f, &["rev-parse", "HEAD~2"])]);
                assert_eq!(
                    text(&f, &["rev-parse", &p.recovery_reference()]),
                    original.as_str()
                );
            }
        },
    );
}

#[test]
fn a_planned_break_continues_the_root_rebase_after_a_dropped_root() {
    isolated(
        CASE,
        "a_planned_break_continues_the_root_rebase_after_a_dropped_root",
        || {
            let f = Fixture::new();
            let ids = commits(
                &f,
                &[
                    ("a.txt", "a\n", "add a"),
                    ("b.txt", "b\n", "add b"),
                    ("c.txt", "c\n", "add c"),
                ],
            );
            let mut second = entry(&ids[1], RebaseInstruction::Pick);
            second.stop_after = true;
            let p = f
                .repo()
                .preview_root_rebase(&token())
                .unwrap()
                .with_plan(vec![
                    entry(&ids[0], RebaseInstruction::Drop),
                    second,
                    entry(&ids[2], RebaseInstruction::Pick),
                ])
                .unwrap();
            let r = f.repo().apply_rebase(&p, &token()).unwrap();
            assert_eq!(r.stop, Some(RebaseStop::Break), "{r:?}");
            assert_eq!(r.failure, None);
            // Continuing reads Git's root-rebase state and keeps the reviewed plan.
            let c = f
                .repo()
                .preview_rebase(RebaseAction::Continue, "", &token())
                .unwrap();
            assert_eq!(c.onto(), None);
            assert_eq!(c.planned_stop(), Some(&RebaseStop::Break));
            assert_eq!(c.plan().map(<[_]>::len), Some(3));
            let r = f.repo().apply_rebase(&c, &token()).unwrap();
            succeeded(&r);
            assert!(r.state.unwrap().is_idle());
            assert_eq!(
                text(&f, &["log", "--reverse", "--format=%s"]),
                "add b\nadd c"
            );
            assert_eq!(roots(&f), [text(&f, &["rev-parse", "HEAD~1"])]);
            assert!(!f.repository.join("a.txt").exists());
        },
    );
}

#[test]
fn a_conflicted_first_commit_is_skipped_or_aborted_but_never_continued() {
    isolated(
        CASE,
        "a_conflicted_first_commit_is_skipped_or_aborted_but_never_continued",
        || {
            let f = Fixture::new();
            let ids = commits(
                &f,
                &[
                    ("f.txt", "one\n", "create f"),
                    ("f.txt", "one\ntwo\n", "change f"),
                    ("g.txt", "g\n", "add g"),
                ],
            );
            let original = head(&f);
            // Without the commit that creates f.txt, the change to it conflicts first.
            let start = || {
                let p = f
                    .repo()
                    .preview_root_rebase(&token())
                    .unwrap()
                    .with_plan(vec![
                        entry(&ids[0], RebaseInstruction::Drop),
                        entry(&ids[1], RebaseInstruction::Pick),
                        entry(&ids[2], RebaseInstruction::Pick),
                    ])
                    .unwrap();
                let r = f.repo().apply_rebase(&p, &token()).unwrap();
                assert_eq!(r.failure, Some(RebaseFailure::ConflictsRemain), "{r:?}");
            };
            start();
            // Git would commit the resolution on top of its empty base commit.
            f.write("f.txt", "one\ntwo\n");
            f.run(&["add", "f.txt"]);
            let refused = f
                .repo()
                .preview_rebase(RebaseAction::Continue, "", &token())
                .unwrap_err();
            assert!(
                matches!(refused, Error::Rebase(RebaseFailure::RootConflict)),
                "{refused:?}"
            );
            let abort = f
                .repo()
                .preview_rebase(RebaseAction::Abort, "", &token())
                .unwrap();
            assert_eq!(abort.onto(), None);
            let r = f.repo().apply_rebase(&abort, &token()).unwrap();
            succeeded(&r);
            assert_eq!(head(&f), original);
            // Skip omits that commit, and the next one becomes the root.
            start();
            let skip = f
                .repo()
                .preview_rebase(RebaseAction::Skip, "", &token())
                .unwrap();
            let r = f.repo().apply_rebase(&skip, &token()).unwrap();
            succeeded(&r);
            assert!(r.state.unwrap().is_idle());
            assert_eq!(text(&f, &["log", "--format=%s"]), "add g");
            assert_eq!(roots(&f).len(), 1);
        },
    );
}

#[test]
fn a_root_rebase_started_outside_rocket_is_left_to_git() {
    isolated(
        CASE,
        "a_root_rebase_started_outside_rocket_is_left_to_git",
        || {
            let f = Fixture::new();
            let ids = commits(
                &f,
                &[
                    ("f.txt", "one\n", "create f"),
                    ("f.txt", "one\ntwo\n", "change f"),
                ],
            );
            // As if Rocket had rebased this tip before: its recovery reference exists.
            let original = head(&f);
            f.run(&[
                "update-ref",
                &format!("refs/rocket/recovery/rebase/{original}"),
                original.as_str(),
            ]);
            // Git's own root rebase, reordered so that it stops on a conflict with
            // nothing but picks in its todo.
            let editor = format!("printf 'pick {}\\npick {}\\n' >", ids[1], ids[0]);
            let output = f
                .command(&f.repository, &["rebase", "--interactive", "--root"])
                .env("GIT_SEQUENCE_EDITOR", editor)
                .output()
                .unwrap();
            assert!(!output.status.success());
            assert!(f.repository.join(".git/rebase-merge/squash-onto").exists());
            let refused = f
                .repo()
                .preview_rebase(RebaseAction::Abort, "", &token())
                .unwrap_err();
            assert!(
                matches!(refused, Error::Rebase(RebaseFailure::UnsupportedSequence)),
                "{refused:?}"
            );
            f.run(&["rebase", "--abort"]);
        },
    );
}
