mod common;
use common::{Fixture, isolated};
use rocket_git::*;
fn oid(f: &Fixture, r: &str) -> String {
    String::from_utf8(f.run(&["rev-parse", r]))
        .unwrap()
        .trim()
        .into()
}
fn setup(conflict: bool) -> Fixture {
    let f = Fixture::new();
    f.write("資料.txt", "base\n");
    f.commit("base");
    f.run(&["switch", "-c", "feature"]);
    f.write("feature", "feature\n");
    f.commit("feature");
    f.run(&["switch", "-c", "side"]);
    f.write("資料.txt", "side\n");
    f.commit("side 文");
    f.run(&["switch", "feature"]);
    f.write("other", "other\n");
    f.commit("other");
    f.run(&["merge", "--no-ff", "side", "-m", "merge 文"]);
    f.run(&["switch", "main"]);
    if conflict {
        f.write("資料.txt", "upstream\n")
    } else {
        f.write("main", "main\n")
    };
    f.commit("main");
    f.run(&["switch", "feature"]);
    f
}
#[test]
fn preserve_merge_graph_and_recover_conflicts_with_stale_checks() {
    isolated(
        "ROCKET_MERGE_REBASE_CASE",
        "preserve_merge_graph_and_recover_conflicts_with_stale_checks",
        || {
            let c = CancellationToken::new();
            let f = setup(false);
            let original = oid(&f, "HEAD");
            let side = oid(&f, "side");
            let p = f
                .repo()
                .preview_rebase_preserving_merges("main", &c)
                .unwrap();
            assert!(p.preserves_merges());
            assert!(p.prediction().is_none());
            let r = f.repo().apply_rebase(&p, &c).unwrap();
            assert!(r.write.result.is_ok(), "{r:?}");
            assert!(r.state.unwrap().is_idle());
            assert_eq!(
                String::from_utf8(f.run(&["rev-list", "--parents", "-n", "1", "HEAD"]))
                    .unwrap()
                    .split_whitespace()
                    .count(),
                3
            );
            assert_ne!(oid(&f, "HEAD"), original);
            assert_eq!(oid(&f, "side"), side);
            assert_eq!(
                oid(&f, &format!("refs/rocket/recovery/rebase/{original}")),
                original
            );
            let f = setup(true);
            let original = oid(&f, "HEAD");
            let p = f
                .repo()
                .preview_rebase_preserving_merges("main", &c)
                .unwrap();
            let r = f.repo().apply_rebase(&p, &c).unwrap();
            assert!(r.write.result.is_err());
            assert!(r.state.unwrap().rebase);
            f.write("outside", "external edit\n");
            let abort = f
                .repo()
                .preview_rebase(RebaseAction::Abort, "", &c)
                .unwrap();
            f.write("資料.txt", "changed resolution\n");
            assert!(matches!(
                f.repo().apply_rebase(&abort, &c),
                Err(Error::Rebase(RebaseFailure::StalePreview))
            ));
            let abort = f
                .repo()
                .preview_rebase(RebaseAction::Abort, "", &c)
                .unwrap();
            let r = f.repo().apply_rebase(&abort, &c).unwrap();
            assert!(r.write.result.is_ok(), "{r:?}");
            assert_eq!(oid(&f, "HEAD"), original);
            assert_eq!(
                std::fs::read(f.repository.join("outside")).unwrap(),
                b"external edit\n"
            );
            for action in [RebaseAction::Continue, RebaseAction::Skip] {
                let f = setup(true);
                let p = f
                    .repo()
                    .preview_rebase_preserving_merges("main", &c)
                    .unwrap();
                let stopped = CancellationToken::new();
                stopped.cancel();
                assert!(f.repo().apply_rebase(&p, &stopped).is_err());
                assert!(!f.repo().operation_state(&c).unwrap().rebase);
                let r = f.repo().apply_rebase(&p, &c).unwrap();
                assert!(r.state.unwrap().rebase);
                if action == RebaseAction::Continue {
                    f.write("資料.txt", "resolved 文\n");
                    f.run(&["add", "--", "資料.txt"]);
                }
                let p = f.repo().preview_rebase(action, "", &c).unwrap();
                let r = f.repo().apply_rebase(&p, &c).unwrap();
                assert!(r.write.result.is_ok(), "{r:?}");
                assert!(r.state.unwrap().is_idle());
            }
        },
    );
}

#[test]
fn merge_commit_conflict_rejects_todo_edits_and_continues_with_two_parents() {
    isolated(
        "ROCKET_MERGE_REBASE_CASE",
        "merge_commit_conflict_rejects_todo_edits_and_continues_with_two_parents",
        || {
            let f = Fixture::new();
            let c = CancellationToken::new();
            f.write("資料.txt", "base\n");
            f.commit("base");
            f.run(&["switch", "-c", "feature"]);
            f.write("資料.txt", "first\n");
            f.commit("first");
            f.run(&["switch", "-c", "side", "main"]);
            f.write("資料.txt", "second\n");
            f.commit("second");
            f.run(&["switch", "feature"]);
            assert!(
                !f.output(&f.repository, &["merge", "--no-ff", "side"])
                    .status
                    .success()
            );
            f.write("資料.txt", "original resolution\n");
            f.commit("Original merge 文");
            f.run(&["switch", "main"]);
            f.write("upstream", "unrelated\n");
            f.commit("upstream");
            f.run(&["switch", "feature"]);
            let p = f
                .repo()
                .preview_rebase_preserving_merges("main", &c)
                .unwrap();
            let r = f.repo().apply_rebase(&p, &c).unwrap();
            let state = r.state.unwrap();
            assert!(state.rebase);
            assert!(!state.merge_heads.is_empty());
            let todo = f.repository.join(".git/rebase-merge/git-rebase-todo");
            let saved = std::fs::read(&todo).unwrap();
            std::fs::write(&todo, [saved.as_slice(), b"exec echo unwanted\n"].concat()).unwrap();
            assert!(
                f.repo()
                    .preview_rebase(RebaseAction::Abort, "", &c)
                    .is_err()
            );
            std::fs::write(&todo, saved).unwrap();
            f.write("資料.txt", "new resolution 文\n");
            f.run(&["add", "資料.txt"]);
            let p = f
                .repo()
                .preview_rebase(RebaseAction::Continue, "", &c)
                .unwrap();
            let r = f.repo().apply_rebase(&p, &c).unwrap();
            assert!(r.write.result.is_ok(), "{r:?}");
            assert!(r.state.unwrap().is_idle());
            assert_eq!(
                String::from_utf8(f.run(&["rev-list", "--parents", "-n", "1", "HEAD"]))
                    .unwrap()
                    .split_whitespace()
                    .count(),
                3
            );
            assert_eq!(
                std::fs::read(f.repository.join("資料.txt")).unwrap(),
                "new resolution 文\n".as_bytes()
            );
        },
    );
}
