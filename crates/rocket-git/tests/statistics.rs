mod common;
use common::{Fixture, isolated};
use rocket_git::*;

fn token() -> CancellationToken {
    CancellationToken::new()
}
/// Commit everything as `author` (name, email) at `date` (author and committer).
fn commit_as(f: &Fixture, author: (&str, &str), date: &str, message: &str) {
    f.run(&["add", "--all", "--"]);
    let output = f
        .command(
            &f.repository,
            &["commit", "--no-verify", "--allow-empty", "-m", message],
        )
        .env("GIT_AUTHOR_NAME", author.0)
        .env("GIT_AUTHOR_EMAIL", author.1)
        .env("GIT_AUTHOR_DATE", date)
        .env("GIT_COMMITTER_DATE", date)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
fn seconds(date: &str) -> i64 {
    // Only the whole-day UTC dates below are needed.
    let days = match date {
        "2024-02-01" => 19_754,
        "2024-06-01" => 19_875,
        _ => unreachable!(),
    };
    days * 86_400
}
const ALICE: (&str, &str) = ("Alice", "alice@example.invalid");
const BOB: (&str, &str) = ("Bob 文", "bob@example.invalid");

#[test]
fn commit_activity_counts_authors_times_and_branches() {
    isolated(
        "ROCKET_STATISTICS_CASE",
        "commit_activity_counts_authors_times_and_branches",
        || {
            let f = Fixture::new();
            f.write("a.txt", "a\n");
            commit_as(&f, ALICE, "2024-01-15T10:30:00+02:00", "one");
            f.write("b.txt", "b\n");
            commit_as(&f, BOB, "2024-01-20T23:15:00-05:00", "two");
            f.run(&["switch", "-c", "side"]);
            f.write("c.txt", "c\n");
            commit_as(&f, BOB, "2024-02-10T09:00:00+00:00", "on side");
            f.run(&["switch", "main"]);
            f.write("d.txt", "d\n");
            commit_as(
                &f,
                ("Alice A.", "ALICE@example.invalid"),
                "2024-03-02T08:00:00+00:00",
                "three",
            );
            let repo = f.repo();
            let head = repo
                .commit_activity(&ActivityScope::default(), &token())
                .unwrap();
            assert_eq!((head.commits, head.merges, head.limited), (3, 0, false));
            let authors: Vec<_> = head
                .authors
                .iter()
                .map(|a| (String::from_utf8_lossy(&a.name).into_owned(), a.commits))
                .collect();
            assert_eq!(authors, [("Alice A.".into(), 2), ("Bob 文".into(), 1)]);
            assert_eq!(head.months, [(2024, 1, 2), (2024, 3, 1)]);
            // Monday 10:30, Saturday 23:15 and Saturday 08:00, each in its own zone.
            assert_eq!(head.weekdays, [1, 0, 0, 0, 0, 2, 0]);
            assert_eq!((head.hours[10], head.hours[23], head.hours[8]), (1, 1, 1));
            let all = repo
                .commit_activity(
                    &ActivityScope {
                        all_branches: true,
                        since: None,
                    },
                    &token(),
                )
                .unwrap();
            assert_eq!(all.commits, 4);
            assert_eq!(all.months, [(2024, 1, 2), (2024, 2, 1), (2024, 3, 1)]);
            let recent = repo
                .commit_activity(
                    &ActivityScope {
                        all_branches: true,
                        since: Some(seconds("2024-02-01")),
                    },
                    &token(),
                )
                .unwrap();
            assert_eq!(recent.commits, 2);
            let cancel = token();
            cancel.cancel();
            assert!(matches!(
                repo.commit_activity(&ActivityScope::default(), &cancel),
                Err(Error::Cancelled)
            ));
            // An unborn repository has no activity, and no treemap.
            let empty = Fixture::new();
            let repo = empty.repo();
            assert_eq!(
                repo.commit_activity(&ActivityScope::default(), &token())
                    .unwrap(),
                CommitActivity::default()
            );
            assert!(matches!(
                repo.activity_treemap(None, &token()),
                Err(Error::InvalidInput(_))
            ));
        },
    );
}

#[test]
fn the_treemap_sizes_files_and_counts_changes_in_the_period() {
    isolated(
        "ROCKET_STATISTICS_CASE",
        "the_treemap_sizes_files_and_counts_changes_in_the_period",
        || {
            let f = Fixture::new();
            f.write("src/a.rs", "a".repeat(100));
            f.write("src/b.rs", "b".repeat(50));
            f.write("検索/結果 🐦.txt", "x".repeat(10));
            f.write("README", "hello");
            f.write("old.txt", "old");
            commit_as(&f, ALICE, "2024-01-01T12:00:00+00:00", "add");
            f.write("src/a.rs", "A".repeat(100));
            commit_as(&f, ALICE, "2024-06-02T12:00:00+00:00", "change a");
            f.write("src/a.rs", "a".repeat(100));
            f.write("検索/結果 🐦.txt", "y".repeat(10));
            commit_as(&f, BOB, "2024-06-03T12:00:00+00:00", "change a and x");
            std::fs::remove_file(f.repository.join("old.txt")).unwrap();
            commit_as(&f, BOB, "2024-06-04T12:00:00+00:00", "remove old");
            let repo = f.repo();
            let map = repo
                .activity_treemap(Some(seconds("2024-06-01")), &token())
                .unwrap();
            assert_eq!(
                map.commit.as_str(),
                String::from_utf8(f.run(&["rev-parse", "HEAD"]))
                    .unwrap()
                    .trim()
            );
            assert_eq!((map.commits, map.limited), (3, false));
            let root = &map.root;
            assert_eq!((root.size, root.files, root.changes), (165, 4, 3));
            let names: Vec<_> = root
                .children
                .iter()
                .map(|c| String::from_utf8_lossy(&c.name).into_owned())
                .collect();
            assert_eq!(names, ["src", "検索", "README"]);
            let src = &root.children[0];
            assert_eq!(
                (src.folder, src.size, src.files, src.changes),
                (true, 150, 2, 2)
            );
            assert_eq!(
                (src.children[0].name.as_slice(), src.children[0].changes),
                (&b"a.rs"[..], 2)
            );
            assert_eq!(src.children[1].changes, 0);
            let search = &root.children[1];
            assert_eq!(search.changes, 1);
            assert_eq!(search.children[0].name, "結果 🐦.txt".as_bytes());
            assert_eq!(root.children[2].changes, 0);
            // All history: the first commit counts too.
            let all = repo.activity_treemap(None, &token()).unwrap();
            assert_eq!(
                (all.commits, all.root.changes, all.root.children[0].changes),
                (4, 4, 3)
            );
            assert_eq!(all.root.children[2].changes, 1);
            let cancel = token();
            cancel.cancel();
            assert!(matches!(
                repo.activity_treemap(None, &cancel),
                Err(Error::Cancelled)
            ));
            // A new commit (the fixture's default date, 2026-01-01) is read from the new HEAD.
            f.write("README", "hello again");
            f.commit("touch readme");
            let later = repo
                .activity_treemap(Some(seconds("2024-06-01")), &token())
                .unwrap();
            assert_eq!(
                (later.commits, later.root.size, later.root.changes),
                (4, 171, 4)
            );
            let readme = later.root.children.iter().find(|c| c.name == b"README");
            assert_eq!(readme.map(|c| (c.size, c.changes)), Some((11, 1)));
        },
    );
}
