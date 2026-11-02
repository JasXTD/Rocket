mod common;
use common::{Fixture, isolated};
use rocket_git::*;
use std::fs;

fn id(f: &Fixture, revision: &str) -> ObjectId {
    f.repo()
        .resolve_commit(revision, &CancellationToken::new())
        .unwrap()
}

#[test]
fn all_ref_logs_include_unvisited_refs_and_recover_deleted_branch_sha1_sha256() {
    isolated(
        "ROCKET_HISTORY_CASE",
        "all_ref_logs_include_unvisited_refs_and_recover_deleted_branch_sha1_sha256",
        || {
            for format in ["sha1", "sha256"] {
                let f = Fixture::with_format(format);
                f.write("file", "base\n");
                f.commit("PRIVATE_BASE");
                let original = id(&f, "HEAD");
                let branch =
                    String::from_utf8(f.run(&["symbolic-ref", "--short", "HEAD"])).unwrap();
                let branch = branch.trim();
                f.run(&["checkout", "-b", "lost/検索"]);
                f.write("file", "lost\n");
                f.commit("PRIVATE_LOST");
                let lost = id(&f, "HEAD");
                f.run(&["checkout", branch]);
                f.run(&["branch", "-D", "lost/検索"]);
                f.run(&[
                    "update-ref",
                    "--create-reflog",
                    "-m",
                    "PRIVATE_REMOTE_EVENT",
                    "refs/remotes/origin/検索",
                    lost.as_str(),
                ]);
                f.write("file", "local edit\n");
                let index = fs::read(f.repository.join(".git/index")).unwrap();
                let config = fs::read(f.repository.join(".git/config")).unwrap();
                let repo = f.repo();
                let c = CancellationToken::new();
                let head = repo.reflog(200, &c).unwrap();
                assert!(
                    !head
                        .entries
                        .iter()
                        .any(|e| e.subject == b"PRIVATE_REMOTE_EVENT")
                );
                let all = repo.reflog_scoped(ReflogScope::AllRefs, 200, &c).unwrap();
                assert!(all.entries.iter().any(|e| {
                    e.subject == b"PRIVATE_REMOTE_EVENT"
                        && String::from_utf8_lossy(&e.selector)
                            .starts_with("refs/remotes/origin/検索@{")
                }));
                assert!(all.entries.iter().any(|e| e.id == lost));
                let small = repo.reflog_scoped(ReflogScope::AllRefs, 1, &c).unwrap();
                assert_eq!(small.entries.len(), 1);
                assert!(small.more);
                let p = repo
                    .preview_recovery(&lost, RecoveryAction::Branch("recovered/検索".into()), &c)
                    .unwrap();
                let r = repo.apply_recovery(&p, &c).unwrap();
                assert!(r.write.result.is_ok(), "{r:?}");
                assert!(r.verification.is_ok(), "{r:?}");
                assert_eq!(id(&f, "recovered/検索"), lost);
                assert_eq!(id(&f, "HEAD"), original);
                assert_eq!(fs::read(f.repository.join(".git/index")).unwrap(), index);
                assert_eq!(fs::read(f.repository.join(".git/config")).unwrap(), config);
                assert_eq!(
                    fs::read_to_string(f.repository.join("file")).unwrap(),
                    "local edit\n"
                );
                // An unborn HEAD must not hide other retained logs.
                f.run(&["checkout", "--orphan", "unborn"]);
                assert!(repo.reflog(200, &c).unwrap().entries.is_empty());
                assert!(
                    !repo
                        .reflog_scoped(ReflogScope::AllRefs, 200, &c)
                        .unwrap()
                        .entries
                        .is_empty()
                );
                c.cancel();
                assert!(matches!(
                    repo.reflog_scoped(ReflogScope::AllRefs, 200, &c),
                    Err(Error::Cancelled)
                ));
            }
        },
    );
}

#[test]
fn fixed_directory_history_is_literal_pinned_paginated_and_read_only() {
    isolated(
        "ROCKET_HISTORY_CASE",
        "fixed_directory_history_is_literal_pinned_paginated_and_read_only",
        || {
            for format in ["sha1", "sha256"] {
                let f = Fixture::with_format(format);
                let directory = "src/検索 [x] 🚀 e\u{301}";
                let file = format!("{directory}/old.rs");
                f.write(&file, "base\n");
                f.commit("PRIVATE_CREATE");
                let first = id(&f, "HEAD");
                f.write("src/other/outside", "outside\n");
                f.commit("PRIVATE_OUTSIDE");
                let outside = id(&f, "HEAD");
                f.run(&["mv", &file, &format!("{directory}/new.rs")]);
                f.commit("PRIVATE_RENAME_WITHIN");
                let renamed = id(&f, "HEAD");
                f.write(&format!("{directory}/new.rs"), "changed\n");
                f.commit("PRIVATE_EDIT");
                let changed = id(&f, "HEAD");
                f.run(&["mv", directory, "archived"]);
                f.commit("PRIVATE_DIRECTORY_REMOVED");
                let tip = id(&f, "HEAD");
                let repo = f.repo();
                let path = RepoPath::new(directory.as_bytes()).unwrap();
                let c = CancellationToken::new();
                f.run(&["config", "log.follow", "true"]);
                let mut page = repo
                    .file_history(
                        FileHistoryStart::Directory {
                            tip: tip.clone(),
                            path,
                        },
                        1,
                        &c,
                    )
                    .unwrap();
                assert!(page.directory);
                assert_eq!(page.entries[0].commit.id, tip);
                let foreign = Fixture::with_format(format);
                assert!(matches!(
                    foreign.repo().file_history(
                        FileHistoryStart::Continue(page.next.clone().unwrap()),
                        1,
                        &c
                    ),
                    Err(Error::InvalidInput(_))
                ));
                // A moved HEAD and staged/unstaged content do not alter pinned pages.
                f.write("archived/new.rs", "staged\n");
                f.run(&["add", "--", "archived/new.rs"]);
                f.write("archived/new.rs", "unstaged\n");
                f.write("untracked", "keep\n");
                f.run(&["update-ref", "refs/heads/later", changed.as_str()]);
                let index = fs::read(f.repository.join(".git/index")).unwrap();
                let mut ids = vec![tip.clone()];
                while let Some(cursor) = page.next {
                    page = repo
                        .file_history(FileHistoryStart::Continue(cursor), 1, &c)
                        .unwrap();
                    assert!(page.directory);
                    assert_eq!(page.tip, tip);
                    ids.extend(page.entries.iter().map(|e| e.commit.id.clone()));
                }
                assert_eq!(ids, [tip.clone(), changed, renamed, first]);
                assert!(!ids.contains(&outside));
                assert_eq!(id(&f, "HEAD"), tip);
                assert_eq!(fs::read(f.repository.join(".git/index")).unwrap(), index);
                assert_eq!(
                    fs::read_to_string(f.repository.join("archived/new.rs")).unwrap(),
                    "unstaged\n"
                );
                let path = RepoPath::new(directory.as_bytes()).unwrap();
                assert!(
                    repo.file_history(
                        FileHistoryStart::Directory {
                            tip: tip.clone(),
                            path: path.clone()
                        },
                        0,
                        &c
                    )
                    .is_err()
                );
                c.cancel();
                assert!(matches!(
                    repo.file_history(FileHistoryStart::Directory { tip, path }, 10, &c),
                    Err(Error::Cancelled)
                ));
            }
        },
    );
}

#[test]
fn empty_all_ref_scope_and_full_directory_merge_history() {
    isolated(
        "ROCKET_HISTORY_CASE",
        "empty_all_ref_scope_and_full_directory_merge_history",
        || {
            let f = Fixture::new();
            let c = CancellationToken::new();
            assert!(
                f.repo()
                    .reflog_scoped(ReflogScope::AllRefs, 20, &c)
                    .unwrap()
                    .entries
                    .is_empty()
            );
            assert!(f.repo().reflog_scoped(ReflogScope::AllRefs, 0, &c).is_err());
            f.write("folder/a", "base\n");
            f.commit("base");
            let branch = String::from_utf8(f.run(&["symbolic-ref", "--short", "HEAD"])).unwrap();
            f.run(&["checkout", "-b", "side"]);
            f.write("folder/b", "side\n");
            f.commit("side");
            let side = id(&f, "HEAD");
            f.run(&["checkout", branch.trim()]);
            f.write("folder/a", "main\n");
            f.commit("main");
            let main = id(&f, "HEAD");
            f.run(&["merge", "--no-edit", "side"]);
            let tip = id(&f, "HEAD");
            let page = f
                .repo()
                .file_history(
                    FileHistoryStart::Directory {
                        tip: tip.clone(),
                        path: RepoPath::new(b"folder".to_vec()).unwrap(),
                    },
                    20,
                    &c,
                )
                .unwrap();
            let ids: Vec<_> = page.entries.iter().map(|e| e.commit.id.clone()).collect();
            assert!(ids.contains(&tip));
            assert!(ids.contains(&main));
            assert!(ids.contains(&side));
            let missing = f
                .repo()
                .file_history(
                    FileHistoryStart::Directory {
                        tip,
                        path: RepoPath::new(b"not-present".to_vec()).unwrap(),
                    },
                    20,
                    &c,
                )
                .unwrap();
            assert!(missing.entries.is_empty());
        },
    );
}
