mod common;
use common::{Fixture, isolated};
use rocket_git::*;
fn token() -> CancellationToken {
    CancellationToken::new()
}
fn path(s: &str) -> RepoPath {
    RepoPath::new(s.as_bytes().to_vec()).unwrap()
}
#[test]
fn renamed_unicode_file_history_and_blame_are_pinned_and_paged() {
    isolated(
        "ROCKET_FILE_CASE",
        "renamed_unicode_file_history_and_blame_are_pinned_and_paged",
        || {
            for format in ["sha1", "sha256"] {
                let f = Fixture::with_format(format);
                f.run(&["config", "user.name", "作者 林安"]);
                let a = "源/旧 [literal].rs";
                let b = "界面/问候 🚀.rs";
                let c = "最新/café.rs";
                f.write(a, "fn greeting() {\n    你好 🚀\n}\n");
                f.commit("PRIVATE root 中文");
                let repo = f.repo();
                let root = repo.resolve_commit("HEAD", &token()).unwrap();
                std::fs::create_dir_all(f.repository.join("界面")).unwrap();
                f.run(&["mv", a, b]);
                f.commit("rename one");
                let rename = repo.resolve_commit("HEAD", &token()).unwrap();
                f.write(b, "fn greeting() {\n    你好 🚀\n    café\n}\n");
                f.commit("edit");
                std::fs::create_dir_all(f.repository.join("最新")).unwrap();
                f.run(&["mv", b, c]);
                f.commit("rename two");
                let tip = repo.resolve_commit("HEAD", &token()).unwrap();
                let mut start = FileHistoryStart::File {
                    tip: tip.clone(),
                    path: path(c),
                };
                let mut entries = Vec::new();
                loop {
                    let page = repo.file_history(start, 1, &token()).unwrap();
                    entries.extend(page.entries);
                    if let Some(next) = page.next {
                        start = FileHistoryStart::Continue(next);
                    } else {
                        break;
                    }
                }
                assert_eq!(entries.len(), 4);
                assert_eq!(entries[0].previous_path, Some(path(b)));
                assert_eq!(entries[2].previous_path, Some(path(a)));
                assert_eq!(entries[3].path, path(a));
                assert_eq!(entries[3].commit.id, root);
                let index = std::fs::read(f.repository.join(".git/index")).unwrap();
                f.write(c, "UNSTAGED must not appear\n");
                let content = repo.historical_file(&tip, &path(c), &token()).unwrap();
                assert_eq!(content.kind, HistoricalFileKind::Text);
                assert!(content.bytes.ends_with(b"}\n"));
                let first = repo.file_blame(&tip, &path(c), 1, 2, &token()).unwrap();
                assert_eq!(first.total_lines, 4);
                assert_eq!(first.next_line, Some(3));
                assert_eq!(first.lines[0].commit, root);
                assert_eq!(first.lines[1].path, path(a));
                assert_eq!(first.lines[0].author, "作者 林安".as_bytes());
                let last = repo.file_blame(&tip, &path(c), 3, 2, &token()).unwrap();
                assert_eq!(last.lines[0].path, path(b));
                assert_eq!(last.lines[0].content, "    café".as_bytes());
                assert_eq!(last.next_line, None);
                assert_eq!(
                    repo.historical_file(&rename, &path(b), &token())
                        .unwrap()
                        .bytes,
                    "fn greeting() {\n    你好 🚀\n}\n".as_bytes()
                );
                assert_eq!(
                    std::fs::read(f.repository.join(".git/index")).unwrap(),
                    index
                );
                assert_eq!(
                    std::fs::read_to_string(f.repository.join(c)).unwrap(),
                    "UNSTAGED must not appear\n"
                );
                let page = repo
                    .file_history(
                        FileHistoryStart::File {
                            tip: tip.clone(),
                            path: path(c),
                        },
                        1,
                        &token(),
                    )
                    .unwrap();
                f.commit("later commit");
                let continued = repo
                    .file_history(FileHistoryStart::Continue(page.next.unwrap()), 2, &token())
                    .unwrap();
                assert_eq!(continued.tip, tip);
                assert_eq!(continued.entries[0].path, path(b));
            }
        },
    );
}
#[test]
fn file_reads_handle_empty_deleted_binary_limits_and_cancellation() {
    isolated(
        "ROCKET_FILE_CASE",
        "file_reads_handle_empty_deleted_binary_limits_and_cancellation",
        || {
            let f = Fixture::new();
            f.write("empty", b"");
            f.write("binary", b"a\0b");
            f.write("large", vec![b'a'; 2 * 1024 * 1024 + 1]);
            f.write("crlf", "a\r\nb\t\nlast");
            f.commit("root");
            let repo = f.repo();
            let tip = repo.resolve_commit("HEAD", &token()).unwrap();
            assert!(
                repo.file_blame(&tip, &path("empty"), 1, 10, &token())
                    .unwrap()
                    .lines
                    .is_empty()
            );
            assert_eq!(
                repo.historical_file(&tip, &path("binary"), &token())
                    .unwrap()
                    .kind,
                HistoricalFileKind::Binary
            );
            assert!(matches!(
                repo.file_blame(&tip, &path("binary"), 1, 10, &token()),
                Err(Error::Unsupported(_))
            ));
            assert!(matches!(
                repo.historical_file(&tip, &path("large"), &token()),
                Err(Error::Unsupported(_))
            ));
            let crlf = repo
                .file_blame(&tip, &path("crlf"), 1, 10, &token())
                .unwrap();
            assert_eq!(crlf.lines[0].content, b"a\r");
            assert_eq!(crlf.lines[2].content, b"last");
            f.run(&["rm", "crlf"]);
            f.commit("delete");
            let deleted = repo.resolve_commit("HEAD", &token()).unwrap();
            let history = repo
                .file_history(
                    FileHistoryStart::File {
                        tip: deleted.clone(),
                        path: path("crlf"),
                    },
                    10,
                    &token(),
                )
                .unwrap();
            assert!(history.entries[0].deleted);
            assert_eq!(history.entries.len(), 2);
            assert_eq!(
                repo.historical_file(&deleted, &path("crlf"), &token())
                    .unwrap()
                    .kind,
                HistoricalFileKind::Missing
            );
            assert!(
                repo.file_history(
                    FileHistoryStart::File {
                        tip: tip.clone(),
                        path: path("missing")
                    },
                    10,
                    &token()
                )
                .unwrap()
                .entries
                .is_empty()
            );
            let cancel = token();
            cancel.cancel();
            assert!(matches!(
                repo.file_blame(&tip, &path("crlf"), 1, 10, &cancel),
                Err(Error::Cancelled)
            ));
            assert!(matches!(
                repo.file_history(
                    FileHistoryStart::File {
                        tip: tip.clone(),
                        path: path("crlf")
                    },
                    10,
                    &cancel
                ),
                Err(Error::Cancelled)
            ));
            assert!(
                repo.file_blame(&tip, &path("crlf"), 0, 10, &token())
                    .is_err()
            );
            assert!(
                repo.file_blame(&tip, &path("crlf"), 4, 10, &token())
                    .is_err()
            );
        },
    );
}
#[test]
fn history_across_merge_matches_git_and_cursors_reject_other_repositories() {
    isolated(
        "ROCKET_FILE_CASE",
        "history_across_merge_matches_git_and_cursors_reject_other_repositories",
        || {
            let f = Fixture::new();
            f.write("old", "one\ntwo\nthree\n");
            f.commit("base");
            let base = f.run(&["rev-parse", "HEAD"]);
            f.run(&["switch", "-c", "side"]);
            f.run(&["mv", "old", "new"]);
            f.commit("rename");
            let renamed = f.run(&["rev-parse", "HEAD"]);
            f.write("new", "one\ntwo\nthree\nfour\n");
            f.commit("side edit");
            let edited = f.run(&["rev-parse", "HEAD"]);
            f.run(&["switch", "main"]);
            f.write("other", "other");
            f.commit("main edit");
            f.run(&["merge", "--no-ff", "--no-edit", "side"]);
            let repo = f.repo();
            let tip = repo.resolve_commit("HEAD", &token()).unwrap();
            let history = repo
                .file_history(
                    FileHistoryStart::File {
                        tip: tip.clone(),
                        path: path("new"),
                    },
                    200,
                    &token(),
                )
                .unwrap();
            let expected = f.run(&[
                "log",
                "--follow",
                "--topo-order",
                "--format=%H",
                "HEAD",
                "--",
                "new",
            ]);
            let expected: Vec<_> = String::from_utf8(expected)
                .unwrap()
                .lines()
                .map(str::to_owned)
                .collect();
            assert_eq!(
                history
                    .entries
                    .iter()
                    .map(|e| e.commit.id.to_string())
                    .collect::<Vec<_>>(),
                expected
            );
            let ancestry =
                [edited, renamed, base].map(|id| String::from_utf8(id).unwrap().trim().to_owned());
            assert_eq!(expected, ancestry);
            assert_eq!(
                history
                    .entries
                    .iter()
                    .map(|e| e.path.clone())
                    .collect::<Vec<_>>(),
                vec![path("new"), path("new"), path("old")]
            );
            assert_eq!(history.entries[1].previous_path, Some(path("old")));
            // Each one-entry page must retain the same rename identity/order.
            let mut next = FileHistoryStart::File {
                tip: tip.clone(),
                path: path("new"),
            };
            let mut paged = Vec::new();
            loop {
                let page = repo.file_history(next, 1, &token()).unwrap();
                for entry in page.entries {
                    paged.push((entry.commit.id.to_string(), entry.path));
                }
                match page.next {
                    Some(cursor) => next = FileHistoryStart::Continue(cursor),
                    None => break,
                }
            }
            assert_eq!(
                paged,
                history
                    .entries
                    .iter()
                    .map(|e| (e.commit.id.to_string(), e.path.clone()))
                    .collect::<Vec<_>>()
            );
            let page = repo
                .file_history(
                    FileHistoryStart::File {
                        tip,
                        path: path("new"),
                    },
                    1,
                    &token(),
                )
                .unwrap();
            let other = Fixture::new();
            assert!(matches!(
                other.repo().file_history(
                    FileHistoryStart::Continue(page.next.unwrap()),
                    1,
                    &token()
                ),
                Err(Error::InvalidInput(_))
            ));
        },
    );
}

#[test]
fn rename_with_edits_case_only_rename_and_literal_names_keep_identity() {
    isolated(
        "ROCKET_FILE_CASE",
        "rename_with_edits_case_only_rename_and_literal_names_keep_identity",
        || {
            let f = Fixture::new();
            let old = "Case文件 [x].txt";
            let new = "case文件 [x].txt";
            f.write(old, "one\ntwo\nthree\nfour\nfive\nsix\n");
            f.commit("old");
            let repo = f.repo();
            let root = repo.resolve_commit("HEAD", &token()).unwrap();
            f.run(&["mv", "-f", old, new]);
            f.write(new, "one\ntwo EDITED\nthree\nfour\nfive\nsix\n");
            f.commit("rename and edit");
            let tip = repo.resolve_commit("HEAD", &token()).unwrap();
            let page = repo
                .file_history(
                    FileHistoryStart::File {
                        tip: tip.clone(),
                        path: path(new),
                    },
                    2,
                    &token(),
                )
                .unwrap();
            assert_eq!(page.entries.len(), 2);
            assert_eq!(page.entries[0].previous_path, Some(path(old)));
            assert_eq!(page.entries[1].path, path(old));
            let blame = repo.file_blame(&tip, &path(new), 1, 6, &token()).unwrap();
            assert_eq!(blame.lines[0].commit, root);
            assert_eq!(blame.lines[0].path, path(old));
            assert_eq!(blame.lines[1].commit, tip);
            assert_eq!(blame.lines[1].path, path(new));
            f.run(&["rm", new]);
            f.commit("delete");
            f.write(new, "new lifetime\n");
            f.commit("recreate");
            let tip = repo.resolve_commit("HEAD", &token()).unwrap();
            let history = repo
                .file_history(
                    FileHistoryStart::File {
                        tip,
                        path: path(new),
                    },
                    20,
                    &token(),
                )
                .unwrap();
            assert_eq!(history.entries.len(), 4);
            assert!(history.entries[1].deleted);
        },
    );
}
