//! Search inside files with git grep, at the checkout and at a commit.
mod common;
use common::Fixture;
use rocket_git::*;
use std::sync::{Arc, Mutex};

const POINTER: &str = "version https://git-lfs.github.com/spec/v1\noid sha256:4d7a214614ab2935c943f9e0ff69d22eadbb8f32b1258daaa5e2ca24d17e2393\nsize 48211904\n";

fn fixture() -> Fixture {
    let f = Fixture::new();
    f.write("src/net/retry.rs", "pub const RETRY_LIMIT: u32 = 5;\n\nfn retry() {\n    for _ in 0..RETRY_LIMIT {}\n}\n");
    f.write("src/net/fetch.rs", "use super::retry::{retry, RETRY_LIMIT};\r\n");
    f.write("docs/guide.md", "Retry up to RETRY_LIMIT times.\n");
    f.write("legacy/latin1.txt", b"caf\xe9 RETRY_LIMIT = 3\n");
    f.write("assets/logo.png", b"\x89PNG\0\0\0RETRY_LIMIT\0");
    f.write("assets/hero.psd", POINTER);
    f.write("data/huge.log", "RETRY_LIMIT reached\n".repeat(133_120));
    f.write(".gitignore", "build/\n");
    f.commit("PRIVATE_FILES");
    f.write("notes/new.txt", "an untracked RETRY_LIMIT\n");
    f.write("build/out.txt", "an ignored RETRY_LIMIT\n");
    f
}

fn query(text: &str, source: FileSearchSource) -> FileSearchQuery {
    FileSearchQuery { text: text.into(), regex: false, match_case: false, path: None, source }
}

fn search(repo: &Repository, query: &FileSearchQuery) -> Result<(Vec<FileSearchFile>, FileSearchSummary)> {
    let mut files = Vec::new();
    let summary = repo.search_files(query, &CancellationToken::new(), &mut |file| files.push(file))?;
    Ok((files, summary))
}

fn paths(files: &[FileSearchFile]) -> Vec<String> {
    files.iter().map(|f| String::from_utf8_lossy(&f.path).into_owned()).collect()
}

fn head(f: &Fixture) -> FileSearchSource {
    let id = String::from_utf8(f.run(&["rev-parse", "HEAD"])).unwrap();
    FileSearchSource::Commit(ObjectId::parse(id.trim()).unwrap())
}

#[test]
fn checkout_and_commit_searches_group_matches_and_skip_binary_and_ignored_files() {
    let f = fixture();
    let repo = f.repo();
    let (files, summary) = search(&repo, &query("retry_limit", FileSearchSource::Checkout)).unwrap();
    // Untracked files count at the checkout; ignored and binary files don't.
    assert_eq!(
        paths(&files),
        ["data/huge.log", "docs/guide.md", "legacy/latin1.txt", "notes/new.txt", "src/net/fetch.rs", "src/net/retry.rs"]
    );
    assert_eq!(summary.end, FileSearchEnd::Complete);
    assert_eq!(summary.files, 6);
    assert_eq!(summary.matches, FILE_SEARCH_FILE_MATCHES + 6);
    let huge = &files[0];
    assert!(huge.capped && huge.lines.len() == FILE_SEARCH_FILE_MATCHES);
    assert_eq!(summary.details[0].size, Some(20 * 133_120));
    let retry = &files[5];
    assert_eq!(retry.lines.iter().map(|l| l.number).collect::<Vec<_>>(), [1, 4]);
    assert_eq!(retry.lines[1].text, "    for _ in 0..RETRY_LIMIT {}");
    assert_eq!(retry.lines[1].marks, [16..27]);
    assert_eq!(retry.lines[1].columns, [16..27]);
    // CRLF files lose the \r; Latin-1 shows a replacement mark and says so.
    assert_eq!(files[4].lines[0].text, "use super::retry::{retry, RETRY_LIMIT};");
    assert!(files[2].lines[0].not_utf8 && files[2].lines[0].text.starts_with("caf\u{fffd} "));
    // At the commit the untracked file isn't there, and paths keep no revision prefix.
    let (files, _) = search(&repo, &query("RETRY_LIMIT", head(&f))).unwrap();
    assert_eq!(
        paths(&files),
        ["data/huge.log", "docs/guide.md", "legacy/latin1.txt", "src/net/fetch.rs", "src/net/retry.rs"]
    );
}

#[test]
fn expressions_case_and_paths_narrow_the_search_and_bad_ones_say_why() {
    let f = fixture();
    let repo = f.repo();
    let mut q = query("RETRY_LIMIT *= *[0-9]", head(&f));
    q.regex = true;
    let (files, _) = search(&repo, &q).unwrap();
    assert_eq!(paths(&files), ["legacy/latin1.txt"]);
    q.text = "RETRY_(".into();
    let error = search(&repo, &q).unwrap_err();
    assert_eq!(pattern_error(&error), Some("Unmatched ( or \\("), "{error}");
    // Literal text never fails as an expression.
    q.regex = false;
    assert!(search(&repo, &q).unwrap().0.is_empty());
    let mut q = query("retry_limit", FileSearchSource::Checkout);
    q.match_case = true;
    let (files, summary) = search(&repo, &q).unwrap();
    assert!(files.is_empty() && summary.end == FileSearchEnd::Complete && summary.files == 0);
    let mut q = query("RETRY_LIMIT", FileSearchSource::Checkout);
    q.path = Some(RepoPath::new("src/net").unwrap());
    assert_eq!(paths(&search(&repo, &q).unwrap().0), ["src/net/fetch.rs", "src/net/retry.rs"]);
    for bad in ["", "two\nlines", "  "] {
        assert!(matches!(search(&repo, &query(bad, FileSearchSource::Checkout)), Err(Error::InvalidInput(_))));
    }
}

#[test]
fn lfs_pointers_are_labelled_and_large_files_open_around_the_match() {
    let f = fixture();
    let repo = f.repo();
    for source in [head(&f), FileSearchSource::Checkout] {
        let (files, summary) = search(&repo, &query("4d7a2146", source.clone())).unwrap();
        assert_eq!(paths(&files), ["assets/hero.psd"]);
        assert_eq!(summary.details[0], FileSearchDetail { size: Some(POINTER.len() as u64), lfs_pointer: true });
        // Ordinary text that merely mentions "size" is not a pointer.
        let (files, summary) = search(&repo, &query("Retry up", source.clone())).unwrap();
        assert_eq!(paths(&files), ["docs/guide.md"]);
        assert!(!summary.details[0].lfs_pointer);
        match repo.search_file(&source, b"src/net/retry.rs", 4, &CancellationToken::new()).unwrap() {
            SearchFileContent::Whole(file) => {
                assert_eq!(file.kind, HistoricalFileKind::Text);
                assert!(file.bytes.starts_with(b"pub const RETRY_LIMIT"));
            }
            other => panic!("{other:?}"),
        }
        match repo.search_file(&source, b"data/huge.log", 57, &CancellationToken::new()).unwrap() {
            SearchFileContent::Window(window) => {
                assert_eq!(window.first, 1);
                assert_eq!(window.lines.len() as u64, 57 + FILE_WINDOW_LINES);
                assert_eq!(window.lines[56], b"RETRY_LIMIT reached");
                assert_eq!(window.size, 20 * 133_120);
                assert!(!window.reaches_end && window.cut.is_empty());
            }
            other => panic!("{other:?}"),
        }
        match repo.search_file(&source, b"legacy/latin1.txt", 1, &CancellationToken::new()).unwrap() {
            SearchFileContent::Whole(file) => assert_eq!(file.kind, HistoricalFileKind::Binary),
            other => panic!("{other:?}"),
        }
        assert!(repo.search_file(&source, b"../outside", 1, &CancellationToken::new()).is_err());
        assert!(repo.search_file(&source, b"missing.txt", 1, &CancellationToken::new()).is_err());
    }
    match repo.search_file(&FileSearchSource::Checkout, b"data/huge.log", 133_000, &CancellationToken::new()).unwrap() {
        SearchFileContent::Window(window) => {
            assert_eq!(window.first, 133_000 - FILE_WINDOW_LINES);
            assert!(window.reaches_end);
            assert_eq!(window.first + window.lines.len() as u64 - 1, 133_120);
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn too_many_matches_stop_the_search_and_cancelling_keeps_what_was_found() {
    let f = Fixture::new();
    for file in 0..25 {
        f.write(&format!("many/f{file:02}.txt"), "needle\n".repeat(120));
    }
    f.commit("PRIVATE_MANY");
    let repo = f.repo();
    let (files, summary) = search(&repo, &query("needle", FileSearchSource::Checkout)).unwrap();
    assert_eq!(summary.end, FileSearchEnd::TooMany);
    assert_eq!(summary.matches, FILE_SEARCH_MATCHES);
    assert_eq!(files.len(), FILE_SEARCH_MATCHES / FILE_SEARCH_FILE_MATCHES);
    assert!(files.iter().all(|f| f.capped));
    // Cancelling after the first file stops Git and still hands that file over.
    let cancel = CancellationToken::new();
    let seen = Arc::new(Mutex::new(0usize));
    let (count, token) = (seen.clone(), cancel.clone());
    let result = repo.search_files(&query("needle", head(&f)), &cancel, &mut move |_| {
        *count.lock().unwrap() += 1;
        token.cancel();
    });
    assert!(matches!(result, Err(Error::Cancelled)), "{result:?}");
    assert!(*seen.lock().unwrap() >= 1);
    let cancel = CancellationToken::new();
    cancel.cancel();
    assert!(matches!(repo.search_files(&query("needle", FileSearchSource::Checkout), &cancel, &mut |_| {}), Err(Error::Cancelled)));
}
