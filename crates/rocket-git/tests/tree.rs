mod common;
use common::{Fixture, isolated};
use rocket_git::*;
use std::{fs, io::Write, process::Stdio};

fn token() -> CancellationToken {
    CancellationToken::new()
}
fn input(f: &Fixture, args: &[&str], bytes: &[u8]) -> String {
    let mut child = f
        .command(&f.repository, args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(bytes).unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap().trim().into()
}
fn entry<'a>(list: &'a TreeListing, name: &str) -> &'a TreeEntry {
    list.entries
        .iter()
        .find(|e| e.name() == name.as_bytes())
        .expect(name)
}
fn blob(f: &Fixture, bytes: &[u8]) -> String {
    input(f, &["hash-object", "-w", "--stdin"], bytes)
}
fn snapshot(f: &Fixture) -> Vec<Vec<u8>> {
    vec![
        f.run(&["rev-parse", "HEAD"]),
        f.run(&["ls-files", "--stage", "-z"]),
        fs::read(f.repository.join(".git/index")).unwrap(),
        fs::read(f.repository.join(".git/config")).unwrap(),
        fs::read(f.repository.join("src/検索/結果 🚀 é.rs")).unwrap(),
    ]
}

#[test]
fn tree_browsing_is_lazy_pinned_unicode_and_read_only() {
    isolated(
        "ROCKET_TREE_CASE",
        "tree_browsing_is_lazy_pinned_unicode_and_read_only",
        || {
            for format in ["sha1", "sha256"] {
                let f = Fixture::with_format(format);
                f.write("src/検索/結果 🚀 é.rs", "PRIVATE_你好 🚀 é\n");
                f.write("README.md", "root\n");
                f.commit("PRIVATE_SOURCE");
                let repo = f.repo();
                let first = repo.resolve_commit("HEAD", &token()).unwrap();
                f.run(&["tag", "-a", "snapshot", "-m", "PRIVATE_TAG"]);
                let root = repo.repository_tree("snapshot", &token()).unwrap();
                assert_eq!(root.directory.commit(), &first);
                assert_eq!(root.entries.len(), 2);
                let src = entry(&root, "src");
                assert_eq!(src.kind(), TreeEntryKind::Directory);
                let src = repo
                    .tree_directory(&src.directory().unwrap(), &token())
                    .unwrap();
                let nested = entry(&src, "検索").directory().unwrap();
                f.write("src/検索/結果 🚀 é.rs", "second\n");
                f.commit("later");
                f.write("src/検索/結果 🚀 é.rs", "PRIVATE_STAGED\n");
                f.run(&["add", "."]);
                f.write("src/検索/結果 🚀 é.rs", "PRIVATE_WORKING\n");
                f.write("untracked", "PRIVATE_UNTRACKED");
                f.write(".git/index.lock", "PRIVATE_LOCK");
                let before = snapshot(&f);
                let nested = repo.tree_directory(&nested, &token()).unwrap();
                assert_eq!(nested.directory.commit(), &first);
                let file = entry(&nested, "結果 🚀 é.rs");
                assert_eq!(file.path(), "src/検索/結果 🚀 é.rs".as_bytes());
                assert_eq!(
                    repo.tree_file(file, &token()).unwrap().bytes,
                    "PRIVATE_你好 🚀 é\n".as_bytes()
                );
                let head = repo.repository_tree("HEAD", &token()).unwrap();
                assert_ne!(head.directory.commit(), &first);
                assert_eq!(snapshot(&f), before);
                assert_eq!(
                    fs::read(f.repository.join("untracked")).unwrap(),
                    b"PRIVATE_UNTRACKED"
                );
                assert_eq!(
                    fs::read(f.repository.join(".git/index.lock")).unwrap(),
                    b"PRIVATE_LOCK"
                );
            }
        },
    );
}

#[test]
fn tree_files_never_follow_links_submodules_or_filters() {
    isolated(
        "ROCKET_TREE_CASE",
        "tree_files_never_follow_links_submodules_or_filters",
        || {
            let f = Fixture::new();
            f.write("base", "base\n");
            f.commit("base");
            let commit = f.repo().resolve_commit("HEAD", &token()).unwrap();
            let outside = f.root.join("outside");
            fs::write(&outside, b"PRIVATE_OUTSIDE").unwrap();
            let target = outside.to_string_lossy();
            let link = blob(&f, target.as_bytes());
            let text = blob(&f, b"PRIVATE_BLOB\n");
            f.run(&[
                "update-index",
                "--add",
                "--cacheinfo",
                "120000",
                &link,
                "shortcut",
            ]);
            f.run(&[
                "update-index",
                "--add",
                "--cacheinfo",
                "160000",
                commit.as_str(),
                "module",
            ]);
            f.run(&[
                "update-index",
                "--add",
                "--cacheinfo",
                "100755",
                &text,
                "executable",
            ]);
            f.run(&["commit", "-m", "types"]);
            f.run(&["config", "filter.danger.smudge", "false"]);
            f.run(&["config", "filter.danger.required", "true"]);
            f.run(&["config", "diff.danger.textconv", "false"]);
            f.write(".gitattributes", "* filter=danger diff=danger\n");
            let repo = f.repo();
            let root = repo.repository_tree("HEAD", &token()).unwrap();
            let symlink = entry(&root, "shortcut");
            assert_eq!(symlink.kind(), TreeEntryKind::Symlink);
            assert_eq!(
                repo.tree_file(symlink, &token()).unwrap().bytes,
                target.as_bytes()
            );
            let submodule = entry(&root, "module");
            assert_eq!(submodule.kind(), TreeEntryKind::Submodule);
            assert!(submodule.directory().is_none());
            assert!(repo.tree_file(submodule, &token()).is_err());
            let executable = entry(&root, "executable");
            assert_eq!(executable.kind(), TreeEntryKind::Executable);
            assert_eq!(
                repo.tree_file(executable, &token()).unwrap().bytes,
                b"PRIVATE_BLOB\n"
            );
            assert_eq!(fs::read(&outside).unwrap(), b"PRIVATE_OUTSIDE");
            assert!(!f.repository.join("shortcut").exists());
            assert!(!f.repository.join("module").exists());
        },
    );
}

#[test]
fn tree_browsing_supports_names_not_representable_as_windows_paths() {
    isolated(
        "ROCKET_TREE_CASE",
        "tree_browsing_supports_names_not_representable_as_windows_paths",
        || {
            let f = Fixture::new();
            let oid = blob(&f, b"PRIVATE_BYTES\n");
            let names: &[&[u8]] = &[
                b"--option",
                b"[literal]",
                b"line\nname",
                b"colon:name",
                b"back\\slash",
                b"invalid-\xff",
            ];
            let mut bytes = Vec::new();
            for name in names {
                bytes.extend_from_slice(format!("100644 blob {oid}\t").as_bytes());
                bytes.extend_from_slice(name);
                bytes.push(0);
            }
            let tree = input(&f, &["mktree", "-z"], &bytes);
            let commit = input(&f, &["commit-tree", &tree], b"PRIVATE_NAMES");
            let repo = f.repo();
            let root = repo.repository_tree(&commit, &token()).unwrap();
            assert_eq!(root.entries.len(), names.len());
            for name in names {
                let entry = root.entries.iter().find(|e| e.name() == *name).unwrap();
                assert_eq!(
                    repo.tree_file(entry, &token()).unwrap().bytes,
                    b"PRIVATE_BYTES\n"
                );
            }
            assert_eq!(
                fs::read_dir(&f.repository).unwrap().count(),
                1,
                "Nothing was checked out"
            );
            assert!(repo.repository_tree("--all", &token()).is_err());
            assert!(repo.repository_tree("HEAD\0option", &token()).is_err());
        },
    );
}

#[test]
fn tree_handles_empty_binary_large_and_missing_objects() {
    isolated(
        "ROCKET_TREE_CASE",
        "tree_handles_empty_binary_large_and_missing_objects",
        || {
            let f = Fixture::new();
            let repo = f.repo();
            assert!(repo.repository_tree("HEAD", &token()).is_err());
            f.run(&["commit", "--allow-empty", "-m", "empty"]);
            assert!(
                repo.repository_tree("HEAD", &token())
                    .unwrap()
                    .entries
                    .is_empty()
            );
            f.write("empty", b"");
            f.write("binary", [0u8, 255, 10]);
            f.write("large", vec![b'x'; TREE_FILE_LIMIT + 1]);
            f.commit("files");
            let root = repo.repository_tree("HEAD", &token()).unwrap();
            let empty = repo.tree_file(entry(&root, "empty"), &token()).unwrap();
            assert_eq!(empty.kind, HistoricalFileKind::Text);
            assert!(empty.bytes.is_empty());
            assert_eq!(
                repo.tree_file(entry(&root, "binary"), &token())
                    .unwrap()
                    .kind,
                HistoricalFileKind::Binary
            );
            assert!(matches!(
                repo.tree_file(entry(&root, "large"), &token()),
                Err(Error::Unsupported(_))
            ));
            let raw = format!("100644 blob {}\tPRIVATE_MISSING\0", "a".repeat(40));
            let tree = input(&f, &["mktree", "-z", "--missing"], raw.as_bytes());
            let commit = input(&f, &["commit-tree", &tree], b"missing file");
            f.run(&["config", "remote.origin.promisor", "true"]);
            f.run(&[
                "config",
                "remote.origin.url",
                "https://invalid.invalid/never-contact",
            ]);
            let root = repo.repository_tree(&commit, &token()).unwrap();
            assert_eq!(root.entries.len(), 1);
            assert!(matches!(
                repo.tree_file(&root.entries[0], &token()),
                Err(Error::Git { .. })
            ));
            let cancel = token();
            cancel.cancel();
            assert!(matches!(
                repo.repository_tree("HEAD", &cancel),
                Err(Error::Cancelled)
            ));
            assert!(matches!(
                repo.tree_directory(&root.directory, &cancel),
                Err(Error::Cancelled)
            ));
            assert!(matches!(
                repo.tree_file(&root.entries[0], &cancel),
                Err(Error::Cancelled)
            ));
        },
    );
}

#[test]
fn tree_locations_remain_bound_and_bare_repositories_work() {
    isolated(
        "ROCKET_TREE_CASE",
        "tree_locations_remain_bound_and_bare_repositories_work",
        || {
            let f = Fixture::new();
            f.write("base", "content\n");
            f.commit("base");
            let repo = f.repo();
            let root = repo.repository_tree("HEAD", &token()).unwrap();
            let other = Fixture::new();
            assert!(matches!(
                other.repo().tree_directory(&root.directory, &token()),
                Err(Error::InvalidInput(_))
            ));
            assert!(matches!(
                other.repo().tree_file(&root.entries[0], &token()),
                Err(Error::InvalidInput(_))
            ));
            let bare = f.root.join("bare.git");
            f.run(&["clone", "--bare", ".", bare.to_str().unwrap()]);
            let bare = f.git.open(&bare, &token()).unwrap();
            let listing = bare.repository_tree("HEAD", &token()).unwrap();
            assert_eq!(
                bare.tree_file(&listing.entries[0], &token()).unwrap().bytes,
                b"content\n"
            );
            let linked = f.root.join("linked");
            f.run(&["worktree", "add", "-b", "linked", linked.to_str().unwrap()]);
            let linked = f.git.open(&linked, &token()).unwrap();
            let listing = linked.repository_tree("HEAD", &token()).unwrap();
            assert_eq!(
                linked
                    .tree_file(&listing.entries[0], &token())
                    .unwrap()
                    .bytes,
                b"content\n"
            );
            assert!(linked.tree_directory(&root.directory, &token()).is_err());
        },
    );
}

#[test]
fn tree_directory_limit_is_explicit_and_logs_are_private() {
    isolated(
        "ROCKET_TREE_CASE",
        "tree_directory_limit_is_explicit_and_logs_are_private",
        || {
            let f = Fixture::new();
            f.write("PRIVATE_文", "PRIVATE_CONTENT");
            f.commit("PRIVATE_MESSAGE");
            let log = DiagnosticLog::open(f.root.join("diagnostics")).unwrap();
            let repo = f.repo();
            let root = log
                .request("tree-list", |log| {
                    repo.with_log(log.clone()).repository_tree("HEAD", &token())
                })
                .unwrap();
            log.request("tree-content", |log| {
                repo.with_log(log.clone())
                    .tree_file(&root.entries[0], &token())
            })
            .unwrap();
            assert!(
                log.request("tree-list", |log| repo
                    .with_log(log.clone())
                    .repository_tree("PRIVATE_UNKNOWN", &token()))
                    .is_err()
            );
            let cancel = token();
            cancel.cancel();
            assert!(
                log.request("tree-list", |log| repo
                    .with_log(log.clone())
                    .tree_directory(&root.directory, &cancel))
                    .is_err()
            );
            let data = fs::read_to_string(log.directory().join("rocket.jsonl")).unwrap();
            for step in [
                "resolve-revision",
                "read-directory",
                "parse-directory",
                "check-blob-size",
                "read-blob",
            ] {
                assert!(data.contains(step), "{step}");
            }
            assert!(data.contains("cancelled"));
            assert!(data.contains("\"outcome\":\"git_error\""));
            for private in [
                "PRIVATE_",
                "文",
                root.entries[0].object().as_str(),
                root.directory.commit().as_str(),
            ] {
                assert!(!data.contains(private), "leaked {private}");
            }
            let mut bytes = Vec::new();
            for i in 0..=TREE_ENTRY_LIMIT {
                bytes.extend_from_slice(
                    format!("100644 blob {}\tf{i:06}\0", root.entries[0].object()).as_bytes(),
                );
            }
            let tree = input(&f, &["mktree", "-z"], &bytes);
            let commit = input(&f, &["commit-tree", &tree], b"large tree");
            assert!(matches!(
                repo.repository_tree(&commit, &token()),
                Err(Error::Unsupported(_))
            ));
        },
    );
}

#[test]
fn replacement_refs_cannot_change_a_pinned_tree_or_blob() {
    isolated(
        "ROCKET_TREE_CASE",
        "replacement_refs_cannot_change_a_pinned_tree_or_blob",
        || {
            let f = Fixture::new();
            f.write("original", "ORIGINAL\n");
            f.commit("original");
            let repo = f.repo();
            let first = repo.repository_tree("HEAD", &token()).unwrap();
            let other_blob = blob(&f, b"REPLACEMENT\n");
            f.run(&["replace", first.entries[0].object().as_str(), &other_blob]);
            assert_eq!(
                repo.tree_file(&first.entries[0], &token()).unwrap().bytes,
                b"ORIGINAL\n"
            );
            f.write("later", "LATER\n");
            f.commit("later");
            let later = repo.resolve_commit("HEAD", &token()).unwrap();
            f.run(&["replace", first.directory.commit().as_str(), later.as_str()]);
            let pinned = repo.tree_directory(&first.directory, &token()).unwrap();
            assert_eq!(pinned.entries.len(), 1);
            assert_eq!(pinned.entries[0].name(), b"original");
            let reopened = repo
                .repository_tree(first.directory.commit().as_str(), &token())
                .unwrap();
            assert_eq!(reopened.entries.len(), 1);
        },
    );
}
