//! The history of selected lines of a file (`git log -L`).
mod common;
use common::{Fixture, isolated};
use rocket_git::*;

fn token() -> CancellationToken {
    CancellationToken::new()
}
fn head(f: &Fixture) -> ObjectId {
    ObjectId::parse(
        String::from_utf8(f.run(&["rev-parse", "HEAD"]))
            .unwrap()
            .trim(),
    )
    .unwrap()
}
fn path(text: &str) -> RepoPath {
    RepoPath::new(text.as_bytes().to_vec()).unwrap()
}
fn subjects(page: &LineHistoryPage) -> Vec<String> {
    page.entries
        .iter()
        .map(|e| e.commit.subject_lossy().into_owned())
        .collect()
}

#[test]
fn line_history_follows_the_lines_through_edits_and_a_rename() {
    isolated(
        "ROCKET_LINE_HISTORY_CASE",
        "line_history_follows_the_lines_through_edits_and_a_rename",
        || {
            let f = Fixture::new();
            f.write("検索 old.txt", "a\nb\nc\nd\n");
            f.commit("add the file");
            f.write("検索 old.txt", "a\nB2\nc\nd\n");
            f.commit("change line 2");
            f.run(&["mv", "検索 old.txt", "文 new.txt"]);
            f.commit("rename");
            f.write("文 new.txt", "a\nB3\nc\nD\n");
            f.commit("change lines 2 and 4");
            f.write("other.txt", "x\n");
            f.commit("unrelated");
            let tip = head(&f);
            let file = path("文 new.txt");
            let second = f.repo().line_history(&tip, &file, 2..=2, &token()).unwrap();
            // The rename and the unrelated commit left line 2 alone.
            assert_eq!(
                subjects(&second),
                ["change lines 2 and 4", "change line 2", "add the file"]
            );
            assert!(!second.limited);
            let patch = |i: usize| String::from_utf8(second.entries[i].patch.clone()).unwrap();
            assert!(patch(0).contains("-B2\n+B3\n"), "{}", patch(0));
            // Before the rename the patch names the old path.
            assert!(
                patch(1).contains("-b\n+B2\n") && patch(1).contains("old.txt"),
                "{}",
                patch(1)
            );
            let fourth = f.repo().line_history(&tip, &file, 4..=4, &token()).unwrap();
            assert_eq!(subjects(&fourth), ["change lines 2 and 4", "add the file"]);
            let both = f.repo().line_history(&tip, &file, 2..=4, &token()).unwrap();
            assert_eq!(both.entries.len(), 3);
        },
    );
}

#[test]
fn line_ranges_outside_the_file_or_backwards_are_refused() {
    isolated(
        "ROCKET_LINE_HISTORY_CASE",
        "line_ranges_outside_the_file_or_backwards_are_refused",
        || {
            let f = Fixture::new();
            f.write("f.txt", "a\nb\n");
            f.commit("add");
            let (tip, file) = (head(&f), path("f.txt"));
            // Line 0, and a range whose last line comes before its first.
            for lines in [0..=1, std::ops::RangeInclusive::new(2, 1)] {
                assert!(
                    matches!(
                        f.repo().line_history(&tip, &file, lines, &token()),
                        Err(Error::InvalidInput(_))
                    ),
                    "range accepted"
                );
            }
            // Git itself refuses lines past the end of the file.
            let past = f.repo().line_history(&tip, &file, 9..=9, &token());
            assert!(matches!(past, Err(Error::Git { .. })), "{past:?}");
        },
    );
}
