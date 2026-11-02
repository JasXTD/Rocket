//! History search by changed path and by changed content (Git's literal `-S`).
mod common;
use common::Fixture;
use rocket_git::*;
use std::{io::Write, process::Stdio};

fn token() -> CancellationToken {
    CancellationToken::new()
}

/// Subjects of every match in a full scan, following continuation cursors, sorted.
fn all_matches(repo: &Repository, query: &str, kind: HistorySearchKind) -> Vec<String> {
    let mut start = HistorySearchStart::Query {
        start: HistoryStart::AllBranches,
        query: query.into(),
        kind,
    };
    let mut subjects = Vec::new();
    loop {
        let page = repo.search_history(start, &token()).unwrap();
        subjects.extend(page.commits.iter().map(|c| c.subject_lossy().into_owned()));
        match page.next {
            Some(next) => start = HistorySearchStart::Continue(next),
            None => break,
        }
    }
    subjects.sort();
    subjects
}

#[test]
fn path_search_matches_changed_paths_and_renames_but_not_merges() {
    let f = Fixture::new();
    f.write("src/Main.rs", "fn main() {}\n");
    f.write("docs/说明.md", "notes\n");
    f.commit("PRIVATE_ROOT both files");
    f.write("src/Main.rs", "fn main() { println!(); }\n");
    f.commit("PRIVATE_EDIT main");
    f.run(&["mv", "docs/说明.md", "docs/guide.md"]);
    f.commit("PRIVATE_RENAME guide");
    // The merge brings other.txt into main without changing anything itself.
    f.run(&["switch", "-c", "side", "HEAD~1"]);
    f.write("other.txt", "x\n");
    f.commit("PRIVATE_SIDE other");
    f.run(&["switch", "main"]);
    f.run(&["merge", "--no-edit", "--no-ff", "side"]);
    let repo = f.repo();
    let path = HistorySearchKind::Path;
    assert_eq!(
        all_matches(&repo, "MAIN.RS", path),
        ["PRIVATE_EDIT main", "PRIVATE_ROOT both files"]
    );
    // A Windows-style separator matches Git's `/`; Unicode paths are compared as text.
    assert_eq!(
        all_matches(&repo, "docs\\说明", path),
        ["PRIVATE_RENAME guide", "PRIVATE_ROOT both files"]
    );
    assert_eq!(
        all_matches(&repo, "guide.md", path),
        ["PRIVATE_RENAME guide"]
    );
    assert_eq!(all_matches(&repo, "other", path), ["PRIVATE_SIDE other"]);
    assert!(all_matches(&repo, "missing/path", path).is_empty());
    // Message text is not part of a path search, and path text is not part of a
    // message search.
    assert!(all_matches(&repo, "PRIVATE_EDIT", path).is_empty());
    assert!(all_matches(&repo, "Main.rs", HistorySearchKind::Text).is_empty());
}

/// 230 commits; commit 5 adds the needle, 100 only moves it within the file and
/// 220 removes it. Other commits change unrelated files.
fn import_content_history(f: &Fixture) {
    let mut data = String::new();
    for i in 0..230 {
        let subject = format!("PRIVATE_STEP {i}");
        data.push_str(&format!(
            "commit refs/heads/main\nmark :{}\nauthor Fixture <fixture@example.invalid> {} +0000\ncommitter Fixture <fixture@example.invalid> {} +0000\ndata {}\n{subject}\n",
            i + 1,
            1_700_000_000 + i,
            1_700_000_000 + i,
            subject.len()
        ));
        if i > 0 {
            data.push_str(&format!("from :{i}\n"));
        }
        let mut file = |path: &str, body: &str| {
            data.push_str(&format!(
                "M 100644 inline {path}\ndata {}\n{body}\n",
                body.len()
            ));
        };
        file(&format!("other{}.txt", i % 7), &format!("line {i}\n"));
        match i {
            0 => file("a.txt", "x\n"),
            5 => file("a.txt", "x\nNeedle [Text].* here\n"),
            100 => file("a.txt", "Needle [Text].* here\nx\n"),
            220 => file("a.txt", "x\n"),
            _ => {}
        }
        data.push('\n');
    }
    data.push_str("done\n");
    let mut child = f
        .command(&f.repository, &["fast-import", "--quiet"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(data.as_bytes())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn content_search_is_literal_ignores_case_and_continues_in_bounded_steps() {
    let f = Fixture::new();
    import_content_history(&f);
    let repo = f.repo();
    let start = repo
        .history(HistoryStart::Head, 200, &token())
        .unwrap()
        .start();
    // Regex characters are literal and case is ignored. Moving the text without
    // changing how often it occurs (commit 100) is not a match.
    let page = repo
        .search_history(
            HistorySearchStart::Query {
                start: start.clone(),
                query: "NEEDLE [text].*".into(),
                kind: HistorySearchKind::Content,
            },
            &token(),
        )
        .unwrap();
    assert_eq!(page.scanned, 200);
    let subjects: Vec<_> = page.commits.iter().map(|c| c.subject_lossy()).collect();
    assert_eq!(subjects, ["PRIVATE_STEP 220"]);
    let page = repo
        .search_history(HistorySearchStart::Continue(page.next.unwrap()), &token())
        .unwrap();
    assert_eq!(page.scanned, 30);
    assert!(page.next.is_none());
    let subjects: Vec<_> = page.commits.iter().map(|c| c.subject_lossy()).collect();
    assert_eq!(subjects, ["PRIVATE_STEP 5"]);
    // As a regular expression this would match "Needle Text"; literally it is absent.
    assert!(all_matches(&repo, "Needle .* here", HistorySearchKind::Content).is_empty());
    // A path search over the same history finds every change to a.txt.
    assert_eq!(
        all_matches(&repo, "a.txt", HistorySearchKind::Path),
        [
            "PRIVATE_STEP 0",
            "PRIVATE_STEP 100",
            "PRIVATE_STEP 220",
            "PRIVATE_STEP 5"
        ]
    );
    for (query, kind) in [
        ("two\nlines", HistorySearchKind::Content),
        ("tab\there", HistorySearchKind::Path),
        ("", HistorySearchKind::Content),
    ] {
        assert!(matches!(
            repo.search_history(
                HistorySearchStart::Query {
                    start: start.clone(),
                    query: query.into(),
                    kind,
                },
                &token(),
            ),
            Err(Error::InvalidInput(_))
        ));
    }
    let cancel = token();
    cancel.cancel();
    assert!(matches!(
        repo.search_history(
            HistorySearchStart::Query {
                start,
                query: "needle".into(),
                kind: HistorySearchKind::Content,
            },
            &cancel,
        ),
        Err(Error::Cancelled)
    ));
}
