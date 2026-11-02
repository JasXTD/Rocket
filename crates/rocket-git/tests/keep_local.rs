mod common;
use common::{Fixture, isolated};
use rocket_git::*;
use std::fs;
fn c() -> CancellationToken {
    CancellationToken::new()
}
fn path(s: &str) -> RepoPath {
    RepoPath::new(s.as_bytes().to_vec()).unwrap()
}
fn base() -> Fixture {
    let f = Fixture::new();
    f.write("文档/本地.txt", "base\n");
    f.write("keep.txt", "base\n");
    f.write("delete.txt", "base\n");
    f.commit("base");
    f
}
fn index(f: &Fixture) -> Vec<u8> {
    fs::read(f.repository.join(".git/index")).unwrap()
}
fn state(f: &Fixture) -> Vec<u8> {
    f.run(&["--no-optional-locks", "status", "--porcelain=v2", "-z"])
}
#[test]
fn untrack_keeps_unicode_files_staging_and_recovery() {
    isolated(
        "ROCKET_KEEP_LOCAL",
        "untrack_keeps_unicode_files_staging_and_recovery",
        || {
            for format in ["sha1", "sha256"] {
                let f = Fixture::with_format(format);
                f.write("文档/本地.txt", "base\n");
                f.write("文档/[cache].dat", "binary\0data");
                f.write("keep.txt", "base\n");
                f.commit("base");
                f.write("keep.txt", "staged\n");
                f.run(&["add", "keep.txt"]);
                f.write("keep.txt", "working\n");
                f.write("文档/本地.txt", "local changes\n");
                let rules = b"\xef\xbb\xbf# preserve PRIVATE rules\r\n*.tmp\r\n!keep.tmp";
                fs::write(f.repository.join(".gitignore"), rules).unwrap();
                let before = index(&f);
                let kept = f.run(&["show", ":keep.txt"]);
                let repo = f.repo();
                let p = repo.preview_ignore_tracked(&[path("文档")], &c()).unwrap();
                assert_eq!(p.files().len(), 2);
                assert_eq!(index(&f), before);
                let r = repo.ignore_tracked(&p, &c()).unwrap();
                assert!(r.result.is_ok(), "{:?}", r.result);
                assert!(r.rules_saved && r.index_published);
                assert!(f.run(&["ls-files", "--", "文档"]).is_empty());
                assert_eq!(
                    fs::read_to_string(f.repository.join("文档/本地.txt")).unwrap(),
                    "local changes\n"
                );
                assert_eq!(f.run(&["show", ":keep.txt"]), kept);
                assert_eq!(
                    fs::read(f.repository.join("文档/[cache].dat")).unwrap(),
                    b"binary\0data"
                );
                assert!(
                    fs::read(f.repository.join(".gitignore"))
                        .unwrap()
                        .starts_with(rules)
                );
                assert_eq!(
                    fs::read(r.recovery.unwrap().join("index-before")).unwrap(),
                    before
                );
                f.run(&["check-ignore", "--", "文档/本地.txt"]);
            }
        },
    )
}
#[test]
fn untrack_blocks_staged_missing_hidden_and_root_rules() {
    isolated(
        "ROCKET_KEEP_LOCAL",
        "untrack_blocks_staged_missing_hidden_and_root_rules",
        || {
            let f = base();
            let repo = f.repo();
            f.write("文档/本地.txt", "staged\n");
            f.run(&["add", "文档/本地.txt"]);
            assert!(repo.preview_ignore_tracked(&[path("文档")], &c()).is_err());
            f.run(&["reset", "--", "文档/本地.txt"]);
            f.run(&["update-index", "--assume-unchanged", "文档/本地.txt"]);
            assert!(repo.preview_ignore_tracked(&[path("文档")], &c()).is_err());
            f.run(&["update-index", "--no-assume-unchanged", "文档/本地.txt"]);
            fs::remove_file(f.repository.join("文档/本地.txt")).unwrap();
            assert!(repo.preview_ignore_tracked(&[path("文档")], &c()).is_err());
            f.write(".gitignore", "# base\n");
            f.run(&["add", ".gitignore"]);
            assert!(
                repo.preview_ignore_tracked(&[path(".gitignore")], &c())
                    .is_err()
            );
        },
    )
}
#[test]
fn untrack_stale_cancel_and_foreign_lock_preserve_data() {
    isolated(
        "ROCKET_KEEP_LOCAL",
        "untrack_stale_cancel_and_foreign_lock_preserve_data",
        || {
            let f = base();
            let repo = f.repo();
            let p = repo.preview_ignore_tracked(&[path("文档")], &c()).unwrap();
            let before = index(&f);
            let cancel = c();
            cancel.cancel();
            assert!(matches!(
                repo.ignore_tracked(&p, &cancel),
                Err(Error::Cancelled)
            ));
            assert!(!f.repository.join(".gitignore").exists());
            f.write("文档/本地.txt", "external\n");
            assert!(repo.ignore_tracked(&p, &c()).is_err());
            assert_eq!(index(&f), before);
            let p = repo.preview_ignore_tracked(&[path("文档")], &c()).unwrap();
            f.write(".git/index.lock", "owned externally");
            let r = repo.ignore_tracked(&p, &c()).unwrap();
            assert!(r.result.is_err());
            assert!(!r.rules_saved && !r.index_published);
            assert_eq!(
                fs::read(f.repository.join(".git/index.lock")).unwrap(),
                b"owned externally"
            );
            fs::remove_file(f.repository.join(".git/index.lock")).unwrap();
            f.write(".gitignore", "# external\n");
            assert!(repo.ignore_tracked(&p, &c()).unwrap().result.is_err());
            assert_eq!(index(&f), before);
        },
    )
}
#[test]
fn nested_ignore_override_reports_rules_saved_without_untracking() {
    isolated(
        "ROCKET_KEEP_LOCAL",
        "nested_ignore_override_reports_rules_saved_without_untracking",
        || {
            let f = base();
            f.write("文档/.gitignore", "!本地.txt\n");
            f.commit("nested rules");
            let before = index(&f);
            let repo = f.repo();
            let p = repo
                .preview_ignore_tracked(&[path("文档/本地.txt")], &c())
                .unwrap();
            let r = repo.ignore_tracked(&p, &c()).unwrap();
            assert!(r.result.is_err());
            assert!(r.rules_saved);
            assert!(!r.index_published);
            assert!(r.recovery.is_some());
            assert_eq!(index(&f), before);
            assert!(f.repository.join("文档/本地.txt").exists());
        },
    )
}

#[test]
fn untrack_rejects_new_or_edited_nested_rules_since_review() {
    isolated(
        "ROCKET_KEEP_LOCAL",
        "untrack_rejects_new_or_edited_nested_rules_since_review",
        || {
            let f = base();
            let repo = f.repo();
            let before = index(&f);
            let p = repo
                .preview_ignore_tracked(&[path("文档/本地.txt")], &c())
                .unwrap();
            f.write("文档/.gitignore", "# added externally\n");
            assert!(repo.ignore_tracked(&p, &c()).is_err());
            assert!(!f.repository.join(".gitignore").exists());
            assert_eq!(index(&f), before);
            let p = repo
                .preview_ignore_tracked(&[path("文档/本地.txt")], &c())
                .unwrap();
            f.write("文档/.gitignore", "!本地.txt\n");
            assert!(repo.ignore_tracked(&p, &c()).is_err());
            assert_eq!(index(&f), before);
        },
    )
}

#[test]
fn snapshot_sha256_and_ignored_files_keep_normal_git_semantics() {
    isolated(
        "ROCKET_KEEP_LOCAL",
        "snapshot_sha256_and_ignored_files_keep_normal_git_semantics",
        || {
            let f = Fixture::with_format("sha256");
            f.write("tracked.txt", "base\n");
            f.write(".gitignore", "*.tmp\n");
            f.commit("base");
            f.write("new.txt", "new\n");
            f.write("secret.tmp", "PRIVATE ignored bytes");
            let before = index(&f);
            let repo = f.repo();
            let p = repo.preview_stash_snapshot("sha256", true, &c()).unwrap();
            assert_eq!(p.files().len(), 1);
            let r = repo.stash_snapshot(&p, &c()).unwrap();
            assert!(r.result.is_ok(), "{:?}", r.result);
            let id = r.object.unwrap();
            assert_eq!(id.as_str().len(), 64);
            assert_eq!(
                f.run(&["ls-tree", "--name-only", &format!("{id}^3")]),
                b"new.txt\n"
            );
            assert_eq!(index(&f), before);
            assert_eq!(
                fs::read(f.repository.join("secret.tmp")).unwrap(),
                b"PRIVATE ignored bytes"
            );
        },
    )
}
#[test]
fn snapshot_preserves_index_worktree_and_restores_both_versions() {
    isolated(
        "ROCKET_KEEP_LOCAL",
        "snapshot_preserves_index_worktree_and_restores_both_versions",
        || {
            let f = base();
            f.write("文档/本地.txt", "staged\n");
            f.run(&["add", "文档/本地.txt"]);
            f.write("文档/本地.txt", "working\n");
            f.write("新 文件.bin", "new\0binary");
            fs::remove_file(f.repository.join("delete.txt")).unwrap();
            let before = index(&f);
            let status = state(&f);
            let repo = f.repo();
            let p = repo
                .preview_stash_snapshot("snapshot PRIVATE 合成", true, &c())
                .unwrap();
            let r = repo.stash_snapshot(&p, &c()).unwrap();
            assert!(r.result.is_ok(), "{:?}", r.result);
            assert_eq!(r.published, Some(true));
            assert_eq!(index(&f), before);
            assert_eq!(state(&f), status);
            let id = r.object.unwrap();
            assert_eq!(
                f.run(&["show", &format!("{id}:文档/本地.txt")]),
                b"working\n"
            );
            assert_eq!(
                f.run(&["show", &format!("{id}^2:文档/本地.txt")]),
                b"staged\n"
            );
            assert_eq!(
                f.run(&["show", &format!("{id}^3:新 文件.bin")]),
                b"new\0binary"
            );
            assert_eq!(r.stashes.unwrap()[0].id, id);
            // Only the disposable fixture is cleaned to prove ordinary Git can restore it.
            f.run(&["reset", "--hard", "HEAD"]);
            fs::remove_file(f.repository.join("新 文件.bin")).unwrap();
            f.run(&["stash", "apply", "--index", id.as_str()]);
            assert_eq!(state(&f), status);
            assert_eq!(
                fs::read_to_string(f.repository.join("文档/本地.txt")).unwrap(),
                "working\n"
            );
            assert!(!f.repository.join("delete.txt").exists());
        },
    )
}
#[test]
fn snapshot_staged_rename_delete_and_untracked_only_are_recoverable() {
    isolated(
        "ROCKET_KEEP_LOCAL",
        "snapshot_staged_rename_delete_and_untracked_only_are_recoverable",
        || {
            let f = base();
            f.run(&["mv", "keep.txt", "renamed.txt"]);
            f.run(&["rm", "delete.txt"]);
            let before = index(&f);
            let repo = f.repo();
            let p = repo
                .preview_stash_snapshot("rename and deletion", false, &c())
                .unwrap();
            let r = repo.stash_snapshot(&p, &c()).unwrap();
            assert!(r.result.is_ok(), "{:?}", r.result);
            assert_eq!(index(&f), before);
            let id = r.object.unwrap();
            assert_eq!(f.run(&["show", &format!("{id}:renamed.txt")]), b"base\n");
            f.run(&["reset", "--hard", "HEAD"]);
            f.write("new.txt", "new\n");
            assert!(
                repo.preview_stash_snapshot("excluded", false, &c())
                    .is_err()
            );
            let p = repo
                .preview_stash_snapshot("untracked only", true, &c())
                .unwrap();
            assert!(repo.stash_snapshot(&p, &c()).unwrap().result.is_ok());
        },
    )
}
#[test]
fn snapshot_stale_files_index_head_and_stash_ref_are_refused() {
    isolated(
        "ROCKET_KEEP_LOCAL",
        "snapshot_stale_files_index_head_and_stash_ref_are_refused",
        || {
            let f = base();
            let repo = f.repo();
            f.write("keep.txt", "before\n");
            let p = repo.preview_stash_snapshot("before", false, &c()).unwrap();
            f.write("keep.txt", "external\n");
            assert!(repo.stash_snapshot(&p, &c()).is_err());
            let p = repo.preview_stash_snapshot("index", false, &c()).unwrap();
            f.run(&["add", "keep.txt"]);
            assert!(repo.stash_snapshot(&p, &c()).is_err());
            let p = repo.preview_stash_snapshot("tip", false, &c()).unwrap();
            let other = repo.stash_snapshot(&p, &c()).unwrap();
            assert!(other.result.is_ok());
            assert!(repo.stash_snapshot(&p, &c()).is_err());
            let p = repo
                .preview_stash_snapshot("checkout", false, &c())
                .unwrap();
            f.run(&["switch", "-c", "other"]);
            assert!(repo.stash_snapshot(&p, &c()).is_err());
            assert_eq!(
                fs::read(f.repository.join("keep.txt")).unwrap(),
                b"external\n"
            );
        },
    )
}
#[test]
fn snapshot_cancel_lock_and_filters_never_clean_files() {
    isolated(
        "ROCKET_KEEP_LOCAL",
        "snapshot_cancel_lock_and_filters_never_clean_files",
        || {
            let f = base();
            f.write("keep.txt", "private\n");
            let repo = f.repo();
            let p = repo.preview_stash_snapshot("cancel", false, &c()).unwrap();
            let before = index(&f);
            let cancel = c();
            cancel.cancel();
            assert!(matches!(
                repo.stash_snapshot(&p, &cancel),
                Err(Error::Cancelled)
            ));
            f.write(".git/index.lock", "external lock");
            let r = repo.stash_snapshot(&p, &c()).unwrap();
            assert!(r.result.is_err());
            assert_eq!(r.published, Some(false));
            assert!(repo.stashes(&c()).unwrap().is_empty());
            assert_eq!(index(&f), before);
            assert_eq!(
                fs::read(f.repository.join(".git/index.lock")).unwrap(),
                b"external lock"
            );
            fs::remove_file(f.repository.join(".git/index.lock")).unwrap();
            f.write(".gitattributes", "keep.txt filter=unknown\n");
            assert!(repo.preview_stash_snapshot("filter", false, &c()).is_err());
            assert_eq!(
                fs::read(f.repository.join("keep.txt")).unwrap(),
                b"private\n"
            );
        },
    )
}
