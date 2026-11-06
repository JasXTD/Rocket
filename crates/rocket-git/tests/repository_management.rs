mod common;
use common::Fixture;
use rocket_git::*;
use std::{collections::HashSet, fs, io::Write, process::Stdio};
fn token() -> CancellationToken {
    CancellationToken::new()
}
fn isolated(name: &str, test: fn()) {
    common::isolated("ROCKET_MANAGEMENT_CASE", name, test);
}

#[test]
fn remote_add_edit_remove_preserve_worktree_index_and_other_settings() {
    isolated(
        "remote_add_edit_remove_preserve_worktree_index_and_other_settings",
        || {
            let f = Fixture::new();
            f.write("file", "base");
            f.commit("base");
            f.write("file", "staged");
            f.run(&["add", "file"]);
            f.write("file", "working");
            let repo = f.repo();
            let index = fs::read(repo.info().git_dir.join("index")).unwrap();
            let head = repo.head(&token()).unwrap();
            let url = f
                .root
                .join("PRIVATE upstream.git")
                .to_string_lossy()
                .into_owned();
            let report = repo
                .add_remote(b"up=stream.foo", url.as_bytes(), &token())
                .unwrap();
            assert!(report.result.is_ok());
            let original = report.remotes.unwrap().remove(0);
            assert_eq!(
                original.fetch_refspecs,
                [b"+refs/heads/*:refs/remotes/up=stream.foo/*".to_vec()]
            );
            assert!(repo.add_remote(&original.name, b"other", &token()).is_err());
            f.run(&["config", "remote.up=stream.foo.tagopt", "--no-tags"]);
            let report = repo
                .set_remote_url(
                    &original,
                    RemoteUrlKind::Fetch,
                    Some(b"PRIVATE-fetch-new"),
                    &token(),
                )
                .unwrap();
            assert!(report.result.is_ok());
            let changed = report.remotes.unwrap().remove(0);
            assert_eq!(changed.urls, [b"PRIVATE-fetch-new".to_vec()]);
            assert_eq!(changed.fetch_refspecs, original.fetch_refspecs);
            assert!(
                repo.set_remote_url(&original, RemoteUrlKind::Fetch, Some(b"stale"), &token())
                    .is_err()
            );
            assert_eq!(
                f.run(&["config", "remote.up=stream.foo.tagopt"]),
                b"--no-tags\n"
            );
            let report = repo
                .set_remote_url(
                    &changed,
                    RemoteUrlKind::Push,
                    Some(b"PRIVATE-push-new"),
                    &token(),
                )
                .unwrap();
            assert!(report.result.is_ok());
            let push = report.remotes.unwrap().remove(0);
            let report = repo
                .set_remote_url(&push, RemoteUrlKind::Push, None, &token())
                .unwrap();
            assert!(report.result.is_ok());
            let fallback = report.remotes.unwrap().remove(0);
            assert!(fallback.push_urls.is_empty());
            f.run(&["update-ref", "refs/remotes/up=stream.foo/main", "HEAD"]);
            f.run(&[
                "symbolic-ref",
                "refs/remotes/up=stream.foo/alias",
                "refs/heads/main",
            ]);
            let report = repo.remove_remote(&fallback, &token()).unwrap();
            assert!(report.result.is_ok());
            assert!(report.remotes.unwrap().is_empty());
            assert!(
                report
                    .branches
                    .unwrap()
                    .iter()
                    .all(|b| b.kind == BranchKind::Local)
            );
            assert_eq!(repo.head(&token()).unwrap(), head);
            assert_eq!(fs::read(repo.info().git_dir.join("index")).unwrap(), index);
            assert_eq!(fs::read(f.repository.join("file")).unwrap(), b"working");
        },
    );
}

#[test]
fn remote_edit_refuses_includes_multiple_urls_and_unsafe_removal_mapping() {
    isolated(
        "remote_edit_refuses_includes_multiple_urls_and_unsafe_removal_mapping",
        || {
            let f = Fixture::new();
            let repo = f.repo();
            repo.add_remote(b"origin", b"one", &token())
                .unwrap()
                .result
                .unwrap();
            f.run(&["config", "--add", "remote.origin.url", "two"]);
            let r = repo.remotes(&token()).unwrap().remove(0);
            assert!(matches!(
                repo.set_remote_url(&r, RemoteUrlKind::Fetch, Some(b"new"), &token()),
                Err(Error::Unsupported(_))
            ));
            let include = f.root.join("included.config");
            fs::write(&include, b"[remote \"origin\"]\n  proxy = private-proxy\n").unwrap();
            f.run(&["config", "include.path", include.to_str().unwrap()]);
            assert!(matches!(
                repo.remove_remote(&r, &token()),
                Err(Error::Unsupported(_))
            ));
            assert!(matches!(
                repo.set_remote_url(&r, RemoteUrlKind::Push, Some(b"new"), &token()),
                Err(Error::Unsupported(_))
            ));
            assert_eq!(
                fs::read(&include).unwrap(),
                b"[remote \"origin\"]\n  proxy = private-proxy\n"
            );
            f.run(&["config", "--unset", "include.path"]);
            f.run(&[
                "config",
                "remote.origin.fetch",
                "+refs/heads/*:refs/heads/*",
            ]);
            let r = repo.remotes(&token()).unwrap().remove(0);
            assert!(repo.remove_remote(&r, &token()).is_err());
            assert_eq!(repo.remotes(&token()).unwrap(), [r]);
        },
    );
}

#[test]
fn remote_lock_cancellation_invalid_input_and_logs_are_explicit() {
    isolated(
        "remote_lock_cancellation_invalid_input_and_logs_are_explicit",
        || {
            let f = Fixture::new();
            let log_dir = f.root.join("logs");
            let log = DiagnosticLog::open(&log_dir).unwrap();
            let repo = f.repo().with_log(log.clone());
            for name in [b"--bad".as_slice(), b"bad\nname", b"..", b"a b"] {
                assert!(repo.add_remote(name, b"url", &token()).is_err());
            }
            assert!(repo.add_remote(b"origin", b"bad\nurl", &token()).is_err());
            let cancel = token();
            cancel.cancel();
            assert!(matches!(
                repo.add_remote(b"origin", b"url", &cancel),
                Err(Error::Cancelled)
            ));
            let report = log
                .request("remote-add", |_| {
                    repo.add_remote(b"PRIVATE-name", b"PRIVATE-url", &token())
                })
                .unwrap();
            report.result.unwrap();
            let r = report.remotes.unwrap().remove(0);
            let lock = repo.info().common_dir.join("config.lock");
            fs::write(&lock, b"owned sentinel").unwrap();
            let report = log
                .request("remote-edit", |_| {
                    repo.set_remote_url(&r, RemoteUrlKind::Fetch, Some(b"PRIVATE-edit"), &token())
                })
                .unwrap();
            assert!(report.result.is_err());
            assert_eq!(report.remotes.unwrap().as_slice(), std::slice::from_ref(&r));
            assert_eq!(fs::read(&lock).unwrap(), b"owned sentinel");
            fs::remove_file(lock).unwrap();
            log.request("remote-remove", |_| repo.remove_remote(&r, &token()))
                .unwrap()
                .result
                .unwrap();
            let bytes = fs::read_to_string(log_dir.join("rocket.jsonl")).unwrap();
            for private in ["PRIVATE-name", "PRIVATE-url", "PRIVATE-edit"] {
                assert!(!bytes.contains(private));
            }
            for op in ["remote-add", "remote-edit", "remote-remove"] {
                assert!(bytes.contains(op));
            }
            assert_eq!(log.health().dropped_events, 0);
        },
    );
}

fn import_history(f: &Fixture, count: usize) {
    let mut data = String::new();
    for i in 0..count {
        let subject = if i == 2 {
            "Old [literal].*\n\nNeedle in message body 文".to_string()
        } else {
            format!("Synthetic commit {i}")
        };
        let author = if i == 3 {
            "Rare Author <rare@example.invalid>"
        } else {
            "Fixture <fixture@example.invalid>"
        };
        data.push_str(&format!("commit refs/heads/main\nmark :{}\nauthor {author} {} +0000\ncommitter Fixture <fixture@example.invalid> {} +0000\ndata {}\n{subject}\n", i+1, 1700000000+i, 1700000000+i, subject.len()));
        if i > 0 {
            data.push_str(&format!("from :{i}\n"));
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

fn all_pages(repo: &Repository, mut start: HistoryStart, size: usize) -> Vec<ObjectId> {
    let mut ids = Vec::new();
    loop {
        let page = repo.history(start, size, &token()).unwrap();
        ids.extend(page.commits.into_iter().map(|c| c.id));
        match page.next {
            Some(c) => start = HistoryStart::Continue(c),
            None => return ids,
        }
    }
}

#[test]
fn all_branch_history_pins_every_tip_and_excludes_tags_and_stashes() {
    let f = Fixture::new();
    import_history(&f, 5);
    f.run(&["branch", "side", "HEAD~2"]);
    f.run(&["update-ref", "refs/remotes/origin/main", "HEAD~1"]);
    let repo = f.repo();
    let mut initial = repo
        .history(HistoryStart::AllBranches, 2, &token())
        .unwrap();
    let before = all_pages(&repo, HistoryStart::AllBranches, 100);
    assert_eq!(before.len(), 5);
    // Advance a different branch without changing the first displayed commit.
    let tree = repo
        .history(HistoryStart::Head, 1, &token())
        .unwrap()
        .commits[0]
        .tree
        .clone();
    let output = f
        .command(
            &f.repository,
            &["commit-tree", tree.as_str(), "-m", "side only"],
        )
        .env("GIT_AUTHOR_DATE", "2000-01-01T00:00:00Z")
        .env("GIT_COMMITTER_DATE", "2000-01-01T00:00:00Z")
        .output()
        .unwrap();
    assert!(output.status.success());
    let orphan = String::from_utf8(output.stdout).unwrap().trim().to_string();
    f.run(&["update-ref", "refs/heads/side", &orphan]);
    let new = repo
        .history(HistoryStart::AllBranches, 2, &token())
        .unwrap();
    assert_eq!(new.commits[0].id, initial.commits[0].id);
    assert!(!new.same_snapshot(&initial));
    let mut pinned: Vec<_> = initial.commits.drain(..).map(|c| c.id).collect();
    pinned.extend(all_pages(
        &repo,
        HistoryStart::Continue(initial.next.take().unwrap()),
        2,
    ));
    assert_eq!(pinned, before);
    let all = all_pages(&repo, HistoryStart::AllBranches, 2);
    assert_eq!(all.len(), 6);
    assert_eq!(all.iter().collect::<HashSet<_>>().len(), all.len());
    f.run(&["update-ref", "refs/tags/only-tag", &orphan]);
    f.run(&["update-ref", "refs/stash", &orphan]);
    f.run(&["branch", "-D", "side"]);
    assert_eq!(all_pages(&repo, HistoryStart::AllBranches, 100), before);
    f.run(&["checkout", "--detach", &orphan]);
    assert_eq!(all_pages(&repo, HistoryStart::AllBranches, 100).len(), 6);
}

#[test]
fn history_search_reads_beyond_loaded_rows_and_keeps_cursors_literal_and_pinned() {
    let f = Fixture::new();
    import_history(&f, 5210);
    let repo = f.repo();
    let first = repo.history(HistoryStart::Head, 200, &token()).unwrap();
    assert!(
        !first
            .commits
            .iter()
            .any(|c| c.subject_lossy().contains("Old"))
    );
    for query in [
        "[LITERAL].*",
        "needle in MESSAGE body 文",
        "rare AUTHOR",
        "RARE@EXAMPLE.INVALID",
    ] {
        let page = repo
            .search_history(
                HistorySearchStart::Query {
                    start: first.start(),
                    query: query.into(),
                    kind: HistorySearchKind::Text,
                    content: Default::default(),
                },
                &token(),
            )
            .unwrap();
        assert_eq!(page.scanned, 4096);
        assert!(page.commits.is_empty());
        let page = repo
            .search_history(HistorySearchStart::Continue(page.next.unwrap()), &token())
            .unwrap();
        assert_eq!(page.commits.len(), 1);
        assert!(page.next.is_none());
        assert_eq!(page.scanned, 1114);
    }
    let target = repo.resolve_commit("HEAD~5207", &token()).unwrap();
    let mut query = HistorySearchStart::Query {
        start: first.start(),
        query: target.as_str()[..10].into(),
        kind: HistorySearchKind::Text,
        content: Default::default(),
    };
    loop {
        let page = repo.search_history(query, &token()).unwrap();
        if let Some(c) = page.next {
            query = HistorySearchStart::Continue(c);
        } else {
            assert_eq!(page.commits[0].id, target);
            break;
        }
    }
    let page = repo
        .search_history(
            HistorySearchStart::Query {
                start: first.start(),
                query: "synthetic".into(),
                kind: HistorySearchKind::Text,
                content: Default::default(),
            },
            &token(),
        )
        .unwrap();
    assert_eq!(page.commits.len(), 200);
    assert_eq!(page.scanned, 200);
    let cursor = page.next.unwrap();
    let other = Fixture::new();
    assert!(
        other
            .repo()
            .search_history(HistorySearchStart::Continue(cursor.clone()), &token())
            .is_err()
    );
    f.run(&["update-ref", "refs/heads/main", "HEAD~300"]);
    let next = repo
        .search_history(HistorySearchStart::Continue(cursor), &token())
        .unwrap();
    assert_eq!(next.commits[0].id, first.commits.last().unwrap().parents[0]);
    let cancel = token();
    cancel.cancel();
    assert!(matches!(
        repo.search_history(
            HistorySearchStart::Query {
                start: first.start(),
                query: "x".into(),
                kind: HistorySearchKind::Text,
                content: Default::default(),
            },
            &cancel
        ),
        Err(Error::Cancelled)
    ));
}

#[test]
fn history_search_keeps_byte_limits_and_retries_a_smaller_batch() {
    let f = Fixture::new();
    import_history(&f, 5000);
    let limits = CommandLimits {
        stdout_bytes: 512 * 1024,
        ..CommandLimits::default()
    };
    let repo = GitClient::new(f.git.executable(), limits)
        .unwrap()
        .open(&f.repository, &token())
        .unwrap();
    let start = HistorySearchStart::Query {
        start: HistoryStart::Head,
        query: "[literal].*".into(),
        kind: HistorySearchKind::Text,
        content: Default::default(),
    };
    let first = repo.search_history(start, &token()).unwrap();
    // 4096 raw commits exceed the configured cap; the old 1000-commit read fits.
    assert_eq!(first.scanned, 1000);
    assert!(first.commits.is_empty());
    let mut cursor = first.next.unwrap();
    let mut scanned = first.scanned;
    loop {
        let page = repo
            .search_history(HistorySearchStart::Continue(cursor), &token())
            .unwrap();
        scanned += page.scanned;
        if let Some(next) = page.next {
            cursor = next;
        } else {
            assert_eq!(page.commits.len(), 1);
            assert_eq!(scanned, 5000);
            break;
        }
    }
    let repo = GitClient::new(
        f.git.executable(),
        CommandLimits {
            stdout_bytes: 4096,
            ..CommandLimits::default()
        },
    )
    .unwrap()
    .open(&f.repository, &token())
    .unwrap();
    assert!(matches!(
        repo.search_history(
            HistorySearchStart::Query {
                start: HistoryStart::Head,
                query: "absent".into(),
                kind: HistorySearchKind::Text,
                content: Default::default(),
            },
            &token()
        ),
        Err(Error::OutputLimit {
            stream: "stdout",
            ..
        })
    ));
}

#[test]
fn all_branch_history_works_before_first_commit_and_search_rejects_invalid_queries() {
    let f = Fixture::new();
    let repo = f.repo();
    assert!(
        repo.history(HistoryStart::AllBranches, 200, &token())
            .unwrap()
            .commits
            .is_empty()
    );
    for query in ["", "  ", "bad\0query"] {
        assert!(
            repo.search_history(
                HistorySearchStart::Query {
                    start: HistoryStart::AllBranches,
                    query: query.into(),
                    kind: HistorySearchKind::Text,
                    content: Default::default(),
                },
                &token()
            )
            .is_err()
        );
    }
}
