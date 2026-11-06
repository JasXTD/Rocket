mod common;
use common::*;
use rocket_git::*;
fn node(f: &Fixture, tree: &str, n: u8, author: u8, parents: &[String]) -> String {
    let mut args = vec!["commit-tree", tree, "-m"];
    let message = format!("node {n}");
    args.push(&message);
    for p in parents {
        args.extend(["-p", p]);
    }
    let out = f
        .command(&f.repository, &args)
        .env("GIT_COMMITTER_DATE", format!("2026-01-{n:02}T12:00:00Z"))
        .env("GIT_AUTHOR_DATE", format!("2026-01-{author:02}T12:00:00Z"))
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap().trim().into()
}
#[test]
fn orders_pin_pagination_search_and_ancestry_across_external_changes() {
    isolated(
        "ROCKET_ORDER_CASE",
        "orders_pin_pagination_search_and_ancestry_across_external_changes",
        || {
            let f = Fixture::new();
            f.write("資料.rs", "base\n");
            f.commit("seed");
            let tree = String::from_utf8(f.run(&["rev-parse", "HEAD^{tree}"]))
                .unwrap()
                .trim()
                .to_owned();
            let a = node(&f, &tree, 1, 1, &[]);
            let b = node(&f, &tree, 2, 7, std::slice::from_ref(&a));
            let c = node(&f, &tree, 3, 2, std::slice::from_ref(&a));
            let d = node(&f, &tree, 4, 3, std::slice::from_ref(&b));
            let e = node(&f, &tree, 5, 6, std::slice::from_ref(&c));
            let g = node(&f, &tree, 6, 4, &[e, d.clone()]);
            let h = node(&f, &tree, 7, 5, std::slice::from_ref(&d));
            let i = node(&f, &tree, 8, 8, &[g]);
            f.run(&["update-ref", "refs/heads/main", &h]);
            f.run(&["update-ref", "refs/heads/other", &i]);
            let repo = f.repo();
            let cancel = CancellationToken::new();
            let mut orders = Vec::new();
            for (order, arg) in [
                (HistoryOrder::Topological, "--topo-order"),
                (HistoryOrder::CommitDate, "--date-order"),
                (HistoryOrder::AuthorDate, "--author-date-order"),
            ] {
                let full = repo
                    .history_ordered(HistoryStart::AllBranches, 100, order, &cancel)
                    .unwrap();
                let expected = String::from_utf8(f.run(&["rev-list", arg, &h, &i, "--"]))
                    .unwrap()
                    .lines()
                    .map(str::to_owned)
                    .collect::<Vec<_>>();
                let ids = full
                    .commits
                    .iter()
                    .map(|c| c.id.to_string())
                    .collect::<Vec<_>>();
                assert_eq!(ids, expected);
                assert_eq!(ids.len(), 8);
                for (n, c) in full.commits.iter().enumerate() {
                    for p in &c.parents {
                        assert!(ids.iter().position(|v| v == p.as_str()).unwrap() > n);
                    }
                }
                let mut page = repo
                    .history_ordered(full.start(), 2, order, &cancel)
                    .unwrap();
                let mut paged = Vec::new();
                let next = node(&f, &tree, 9, 9, std::slice::from_ref(&i));
                f.run(&["update-ref", "refs/heads/other", &next]);
                loop {
                    paged.extend(page.commits.iter().map(|c| c.id.to_string()));
                    let Some(cursor) = page.next else {
                        break;
                    };
                    let wrong = if order == HistoryOrder::Topological {
                        HistoryOrder::AuthorDate
                    } else {
                        HistoryOrder::Topological
                    };
                    assert!(
                        repo.history_ordered(
                            HistoryStart::Continue(cursor.clone()),
                            2,
                            wrong,
                            &cancel
                        )
                        .is_err()
                    );
                    page = repo
                        .history(HistoryStart::Continue(cursor), 2, &cancel)
                        .unwrap();
                    assert_eq!(page.order, order);
                }
                assert_eq!(paged, ids);
                let search = repo
                    .search_history(
                        HistorySearchStart::Query {
                            start: full.start(),
                            query: "node".into(),
                            kind: HistorySearchKind::Text,
                            content: Default::default(),
                        },
                        &cancel,
                    )
                    .unwrap();
                assert_eq!(
                    search
                        .commits
                        .iter()
                        .map(|c| c.id.to_string())
                        .collect::<Vec<_>>(),
                    ids
                );
                f.run(&["update-ref", "refs/heads/other", &i]);
                orders.push(ids);
            }
            assert_ne!(orders[0], orders[1]);
            assert_ne!(orders[1], orders[2]);
            let stop = CancellationToken::new();
            stop.cancel();
            assert!(matches!(
                repo.history_ordered(HistoryStart::Head, 10, HistoryOrder::AuthorDate, &stop),
                Err(Error::Cancelled)
            ));
        },
    );
}
#[test]
fn commit_browser_links_cover_hosts_encoding_ambiguity_and_no_mutation() {
    isolated(
        "ROCKET_LINK_CASE",
        "commit_browser_links_cover_hosts_encoding_ambiguity_and_no_mutation",
        || {
            let f = Fixture::new();
            f.write("保持.txt", "unchanged");
            f.commit("seed");
            let repo = f.repo();
            let c = CancellationToken::new();
            let id = repo.resolve_commit("HEAD", &c).unwrap();
            for (provider, server, remote, suffix) in [
                (
                    ForgeProvider::GitHub,
                    "https://github.com",
                    "git@github.com:sample/demo.git",
                    "sample/demo/commit/",
                ),
                (
                    ForgeProvider::GitLab,
                    "https://code.example.test/gitlab",
                    "https://code.example.test/gitlab/team/sub/%E8%B3%87%E6%96%99.git",
                    "team/sub/%E8%B3%87%E6%96%99/-/commit/",
                ),
                (
                    ForgeProvider::Bitbucket,
                    "https://bitbucket.org",
                    "ssh://git@bitbucket.org/sample/demo.git",
                    "sample/demo/commits/",
                ),
                (
                    ForgeProvider::Azure,
                    "https://dev.azure.com/org",
                    "git@ssh.dev.azure.com:v3/org/project/repo",
                    "project/_git/repo/commit/",
                ),
                (
                    ForgeProvider::Gitea,
                    "https://code.example.test/gitea",
                    "https://code.example.test/gitea/sample/demo.git",
                    "sample/demo/commit/",
                ),
                (
                    ForgeProvider::BitbucketServer,
                    "https://code.example.test/bb",
                    "ssh://git@code.example.test:7999/~user/demo.git",
                    "users/user/repos/demo/commits/",
                ),
            ] {
                let config = ForgeConfig::new(provider, server).unwrap();
                let parsed =
                    ForgeRepository::from_remote(config.clone(), remote.as_bytes()).unwrap();
                assert_eq!(parsed.commit_url(&id), format!("{server}/{suffix}{id}"));
                assert!(
                    ForgeRepository::from_remote(
                        config,
                        b"https://user:secret@github.com/sample/demo.git"
                    )
                    .is_none()
                );
            }
            f.run(&[
                "remote",
                "add",
                "origin",
                "https://github.com/sample/demo.git",
            ]);
            f.run(&[
                "remote",
                "add",
                "upstream",
                "git@gitlab.com:sample/demo.git",
            ]);
            let config = std::fs::read(f.repository.join(".git/config")).unwrap();
            assert_eq!(
                repo.commit_browser_links(&id, None, None, &c)
                    .unwrap()
                    .len(),
                2
            );
            assert_eq!(
                repo.commit_browser_links(&id, Some(b"origin"), None, &c)
                    .unwrap()
                    .len(),
                1
            );
            assert!(
                repo.commit_browser_links(&id, Some(b"missing"), None, &c)
                    .is_err()
            );
            assert_eq!(
                config,
                std::fs::read(f.repository.join(".git/config")).unwrap()
            );
            assert_eq!(
                std::fs::read(f.repository.join("保持.txt")).unwrap(),
                b"unchanged"
            );
        },
    );
}
