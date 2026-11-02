mod common;
use common::{Fixture, isolated};
use rocket_git::*;
use std::{fs, io::Write, process::Stdio};
fn c() -> CancellationToken {
    CancellationToken::new()
}
fn input(f: &Fixture, args: &[&str], bytes: &[u8]) -> Vec<u8> {
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
    out.stdout
}
fn id(bytes: Vec<u8>) -> String {
    String::from_utf8(bytes).unwrap().trim().into()
}
fn stages(f: &Fixture, path: &str, mode: &str, ids: &[String; 3]) {
    let mut data = format!("0 {}\t{path}\0", ids[0]);
    for (i, id) in ids.iter().enumerate() {
        data.push_str(&format!("{mode} {id} {}\t{path}\0", i + 1));
    }
    input(f, &["update-index", "-z", "--index-info"], data.as_bytes());
}
#[test]
fn local_comparisons_bind_content_and_index_without_writing() {
    isolated(
        "ROCKET_LOCAL_COMPARE_CASE",
        "local_comparisons_bind_content_and_index_without_writing",
        || {
            let f = Fixture::new();
            let path = "src/検索/表示 🚀.rs";
            f.write(path, "base\n");
            f.write("old.md", "rename contents\n");
            f.commit("base");
            f.run(&["mv", "old.md", "新名.md"]);
            f.write(path, "staged\n");
            f.run(&["add", "--", path]);
            f.write(path, "working\n");
            f.write("untracked.md", "untouched\n");
            let index = fs::read(f.repository.join(".git/index")).unwrap();
            let repo = f.repo();
            for (staged, expected) in [(true, "+staged"), (false, "+working")] {
                let p = repo.compare_local("HEAD", staged, &c()).unwrap();
                assert_eq!(p.files.len(), 2);
                assert!(p.files.iter().any(|f| f.change == Change::Renamed));
                let n = p
                    .files
                    .iter()
                    .position(|f| f.path.as_bytes() == path.as_bytes())
                    .unwrap();
                assert!(
                    String::from_utf8(repo.diff(&p.file_options(n).unwrap(), &c()).unwrap().patch)
                        .unwrap()
                        .contains(expected)
                );
                repo.validate_comparison(&p, &c()).unwrap();
            }
            assert_eq!(fs::read(f.repository.join(".git/index")).unwrap(), index);
            let p = repo.compare_local("HEAD", false, &c()).unwrap();
            f.write(path, "changed\n");
            assert!(
                repo.validate_comparison(&p, &c())
                    .unwrap_err()
                    .to_string()
                    .contains("Local comparison changed")
            );
            let p = repo.compare_local("HEAD", true, &c()).unwrap();
            f.run(&["add", "--", path]);
            assert!(repo.validate_comparison(&p, &c()).is_err());
            let cancelled = c();
            cancelled.cancel();
            assert!(matches!(
                repo.compare_local("HEAD", false, &cancelled),
                Err(Error::Cancelled)
            ));
        },
    );
}
fn gitlink_fixture() -> (Fixture, String, [String; 3]) {
    let f = Fixture::new();
    let path = "vendor/引擎 🚀".to_owned();
    f.write("README.md", "parent\n");
    f.commit("parent");
    let child = f.repository.join(&path);
    fs::create_dir_all(&child).unwrap();
    f.run_at(&child, &["init", "--initial-branch=main"]);
    f.run_at(&child, &["config", "user.name", "Fixture"]);
    f.run_at(&child, &["config", "user.email", "fixture@example.invalid"]);
    f.run_at(&child, &["config", "commit.gpgsign", "false"]);
    let commit = |text: &str| {
        fs::write(child.join("file.md"), text).unwrap();
        f.run_at(&child, &["add", "."]);
        f.run_at(&child, &["commit", "-m", text]);
        id(f.run_at(&child, &["rev-parse", "HEAD"]))
    };
    let base = commit("base");
    let ours = commit("ours");
    f.run_at(&child, &["checkout", "--detach", &base]);
    let theirs = commit("theirs");
    f.write(
        ".gitmodules",
        format!("[submodule \"engine\"]\n\tpath = {path}\n\turl = ./synthetic\n"),
    );
    f.run(&["add", ".gitmodules"]);
    f.run(&[
        "update-index",
        "--add",
        "--cacheinfo",
        &format!("160000,{base},{path}"),
    ]);
    f.run(&["commit", "-m", "submodule base"]);
    (f, path, [base, ours, theirs])
}
#[test]
fn gitlink_resolution_preserves_checkout_nested_edits_and_recovers_index() {
    isolated(
        "ROCKET_LOCAL_COMPARE_CASE",
        "gitlink_resolution_preserves_checkout_nested_edits_and_recovers_index",
        || {
            let (f, path, ids) = gitlink_fixture();
            stages(&f, &path, "160000", &ids);
            f.write(&format!("{path}/file.md"), "dirty child\n");
            f.write(&format!("{path}/nested/keep.tmp"), "nested local\n");
            f.write("other.md", "staged unrelated\n");
            f.run(&["add", "other.md"]);
            let repo = f.repo();
            let path = RepoPath::new(path.as_bytes()).unwrap();
            let p = repo.review_whole_conflict(&path, &c()).unwrap();
            assert_eq!(p.kind(), ConflictKind::Submodule);
            assert_eq!(p.planned_change(ConflictKeep::Theirs), None);
            let before = fs::read(f.repository.join(".git/index")).unwrap();
            let cancel = c();
            cancel.cancel();
            assert!(
                repo.keep_conflict_side(&p, ConflictKeep::Theirs, &cancel)
                    .is_err()
            );
            assert_eq!(fs::read(f.repository.join(".git/index")).unwrap(), before);
            fs::write(f.repository.join(".git/index.lock"), b"external owner").unwrap();
            let out = repo
                .keep_conflict_side(&p, ConflictKeep::Theirs, &c())
                .unwrap();
            assert!(out.result.is_err() && !out.staged);
            assert_eq!(
                fs::read(f.repository.join(".git/index.lock")).unwrap(),
                b"external owner"
            );
            fs::remove_file(f.repository.join(".git/index.lock")).unwrap();
            let out = repo
                .keep_conflict_side(&p, ConflictKeep::Theirs, &c())
                .unwrap();
            out.result.unwrap();
            assert!(out.staged);
            assert_eq!(out.file_change, None);
            assert_eq!(
                fs::read(out.recovery_directory.unwrap().join("index-before")).unwrap(),
                before
            );
            assert_eq!(
                id(f.run(&["rev-parse", &format!(":{}", path.display_lossy())])),
                ids[2]
            );
            assert_eq!(
                fs::read(
                    f.repository
                        .join(path.display_lossy().as_ref())
                        .join("file.md")
                )
                .unwrap(),
                b"dirty child\n"
            );
            assert!(
                f.repository
                    .join(path.display_lossy().as_ref())
                    .join("nested/keep.tmp")
                    .is_file()
            );
            assert!(
                !f.run(&["diff", "--cached", "--name-only", "--", "other.md"])
                    .is_empty()
            );
            stages(&f, &path.display_lossy(), "160000", &ids);
            let p = repo.review_whole_conflict(&path, &c()).unwrap();
            f.run(&[
                "update-index",
                "--cacheinfo",
                &format!("160000,{},{}", ids[1], path.display_lossy()),
            ]);
            let index = fs::read(f.repository.join(".git/index")).unwrap();
            let out = repo
                .keep_conflict_side(&p, ConflictKeep::Theirs, &c())
                .unwrap();
            assert!(out.result.is_err());
            assert_eq!(fs::read(f.repository.join(".git/index")).unwrap(), index);
        },
    );
}
fn pointer(f: &Fixture, content: &[u8], name: &str) -> (String, String) {
    let payload = f.root.join(name);
    fs::write(&payload, content).unwrap();
    let bytes = f.run(&["lfs", "pointer", &format!("--file={}", payload.display())]);
    let text = String::from_utf8(bytes.clone()).unwrap();
    let sha = text
        .lines()
        .find_map(|l| l.strip_prefix("oid sha256:"))
        .unwrap()
        .to_owned();
    let cache = f
        .repository
        .join(".git/lfs/objects")
        .join(&sha[..2])
        .join(&sha[2..4])
        .join(&sha);
    fs::create_dir_all(cache.parent().unwrap()).unwrap();
    fs::write(cache, content).unwrap();
    (id(input(f, &["hash-object", "-w", "--stdin"], &bytes)), sha)
}
#[test]
fn lfs_resolution_verifies_objects_guards_edits_and_stages_exact_pointer() {
    isolated(
        "ROCKET_LOCAL_COMPARE_CASE",
        "lfs_resolution_verifies_objects_guards_edits_and_stages_exact_pointer",
        || {
            let f = Fixture::new();
            f.write(
                ".gitattributes",
                "*.dat filter=lfs diff=lfs merge=lfs -text\n",
            );
            f.commit("attributes");
            let path = "assets/画像 🚀.dat";
            let values = [b"base\0blob".as_slice(), b"ours\0blob", b"theirs\0blob"];
            let pairs = [
                pointer(&f, values[0], "base"),
                pointer(&f, values[1], "ours"),
                pointer(&f, values[2], "theirs"),
            ];
            let ids = pairs.clone().map(|p| p.0);
            stages(&f, path, "100644", &ids);
            f.write(path, values[1]);
            let repo = f.repo();
            let rp = RepoPath::new(path.as_bytes()).unwrap();
            let p = repo.review_whole_conflict(&rp, &c()).unwrap();
            assert_eq!(p.kind(), ConflictKind::Lfs);
            assert!(p.can_keep(ConflictKeep::Theirs));
            f.write(
                ".gitattributes",
                "*.dat filter=lfs diff=lfs merge=lfs -text\n# external attributes edit\n",
            );
            // A comment has no conversion effect and is preserved by the resolver.
            let attributes = fs::read(f.repository.join(".gitattributes")).unwrap();
            f.write(path, b"external edit");
            let index = fs::read(f.repository.join(".git/index")).unwrap();
            let out = repo
                .keep_conflict_side(&p, ConflictKeep::Theirs, &c())
                .unwrap();
            assert!(out.result.is_err());
            assert_eq!(fs::read(f.repository.join(path)).unwrap(), b"external edit");
            assert_eq!(fs::read(f.repository.join(".git/index")).unwrap(), index);
            let p = repo.review_whole_conflict(&rp, &c()).unwrap();
            let out = repo
                .keep_conflict_side(&p, ConflictKeep::Theirs, &c())
                .unwrap();
            out.result.unwrap();
            assert!(out.staged);
            assert_eq!(
                fs::read(f.repository.join(".gitattributes")).unwrap(),
                attributes
            );
            let recovery = out.recovery_directory.unwrap();
            assert_eq!(
                fs::read(recovery.join("replaced-original")).unwrap(),
                b"external edit"
            );
            assert!(recovery.join("index-before").is_file());
            assert_eq!(fs::read(f.repository.join(path)).unwrap(), values[2]);
            assert_eq!(id(f.run(&["rev-parse", &format!(":{path}")])), ids[2]);
            stages(&f, path, "100644", &ids);
            let sha = &pairs[2].1;
            let cache = f
                .repository
                .join(".git/lfs/objects")
                .join(&sha[..2])
                .join(&sha[2..4])
                .join(sha);
            fs::write(&cache, b"corrupt blob").unwrap();
            let p = repo.review_whole_conflict(&rp, &c()).unwrap();
            assert!(!p.can_keep(ConflictKeep::Theirs));
            assert_eq!(
                p.version(3).unwrap().unavailable,
                Some(LfsFailure::InvalidObject)
            );
            fs::remove_file(cache).unwrap();
            let p = repo.review_whole_conflict(&rp, &c()).unwrap();
            assert_eq!(
                p.version(3).unwrap().unavailable,
                Some(LfsFailure::MissingObject)
            );
        },
    );
}

#[test]
fn local_image_reads_reject_stale_binary_contents() {
    isolated(
        "ROCKET_LOCAL_COMPARE_CASE",
        "local_image_reads_reject_stale_binary_contents",
        || {
            let f = Fixture::new();
            let name = "assets/画像.png";
            f.write(name, b"base\0image");
            f.commit("base");
            f.write(name, b"working\0image");
            let repo = f.repo();
            let p = std::sync::Arc::new(repo.compare_local("HEAD", false, &c()).unwrap());
            let mut image = repo
                .image_comparisons(&p.file_options(0).unwrap(), &c())
                .unwrap()
                .remove(0);
            image.bind_comparison(p);
            assert_eq!(
                repo.image_versions(&image, &c()).unwrap().after.as_deref(),
                Some(b"working\0image".as_slice())
            );
            f.write(name, b"changed\0image");
            assert!(matches!(
                repo.image_versions(&image, &c()),
                Err(Error::InvalidInput(_))
            ));
        },
    );
}

#[test]
fn specialized_deletion_and_empty_lfs_keep_recovery_and_child_files() {
    isolated(
        "ROCKET_LOCAL_COMPARE_CASE",
        "specialized_deletion_and_empty_lfs_keep_recovery_and_child_files",
        || {
            let (f, name, ids) = gitlink_fixture();
            let path = RepoPath::new(name.as_bytes()).unwrap();
            let deleted = format!(
                "0 {}\t{name}\0{mode} {} 1\t{name}\0{mode} {} 2\t{name}\0",
                ids[0],
                ids[0],
                ids[1],
                mode = "160000"
            );
            input(
                &f,
                &["update-index", "-z", "--index-info"],
                deleted.as_bytes(),
            );
            f.write(&format!("{name}/keep.tmp"), b"keep child");
            let repo = f.repo();
            let p = repo.review_whole_conflict(&path, &c()).unwrap();
            let out = repo
                .keep_conflict_side(&p, ConflictKeep::Deletion, &c())
                .unwrap();
            out.result.unwrap();
            assert!(out.staged && out.file_change.is_none());
            assert!(f.run(&["ls-files", "--stage", "--", &name]).is_empty());
            assert_eq!(
                fs::read(f.repository.join(&name).join("keep.tmp")).unwrap(),
                b"keep child"
            );
            assert!(
                out.recovery_directory
                    .unwrap()
                    .join("index-before")
                    .is_file()
            );

            let f = Fixture::new();
            f.write(
                ".gitattributes",
                "*.dat filter=lfs diff=lfs merge=lfs -text\n",
            );
            f.commit("attributes");
            let name = "empty.dat";
            let path = RepoPath::new(name.as_bytes()).unwrap();
            let full = pointer(&f, b"before\0", "payload").0;
            let empty = id(input(&f, &["hash-object", "-w", "--stdin"], b""));
            stages(
                &f,
                name,
                "100644",
                &[full.clone(), full.clone(), empty.clone()],
            );
            f.write(name, b"before\0");
            let repo = f.repo();
            let p = repo.review_whole_conflict(&path, &c()).unwrap();
            assert_eq!(p.version(3).unwrap().lfs.as_ref().unwrap().size, 0);
            let out = repo
                .keep_conflict_side(&p, ConflictKeep::Theirs, &c())
                .unwrap();
            out.result.unwrap();
            assert!(fs::read(f.repository.join(name)).unwrap().is_empty());
            assert_eq!(id(f.run(&["rev-parse", ":empty.dat"])), empty);
            input(
                &f,
                &["update-index", "-z", "--index-info"],
                format!(
                    "0 {full}\t{name}\0{mode} {full} 1\t{name}\0{mode} {full} 2\t{name}\0",
                    mode = "100644"
                )
                .as_bytes(),
            );
            f.write(name, b"working bytes");
            let p = repo.review_whole_conflict(&path, &c()).unwrap();
            let out = repo
                .keep_conflict_side(&p, ConflictKeep::Deletion, &c())
                .unwrap();
            out.result.unwrap();
            assert_eq!(out.file_change, Some(WholeFileChange::Removed));
            assert!(!f.repository.join(name).exists());
            assert_eq!(
                fs::read(out.recovery_directory.unwrap().join("replaced-original")).unwrap(),
                b"working bytes"
            );
            assert!(f.run(&["ls-files", "--stage", "--", name]).is_empty());
        },
    );
}

#[test]
fn specialized_split_index_refuses_before_writing() {
    isolated(
        "ROCKET_LOCAL_COMPARE_CASE",
        "specialized_split_index_refuses_before_writing",
        || {
            let (f, name, ids) = gitlink_fixture();
            stages(&f, &name, "160000", &ids);
            let repo = f.repo();
            let path = RepoPath::new(name.as_bytes()).unwrap();
            let p = repo.review_whole_conflict(&path, &c()).unwrap();
            f.run(&["update-index", "--split-index"]);
            let before = fs::read(f.repository.join(".git/index")).unwrap();
            assert!(matches!(
                repo.review_whole_conflict(&path, &c()),
                Err(Error::Unsupported(_))
            ));
            let out = repo
                .keep_conflict_side(&p, ConflictKeep::Theirs, &c())
                .unwrap();
            assert!(matches!(out.result, Err(Error::Unsupported(_))));
            assert_eq!(fs::read(f.repository.join(".git/index")).unwrap(), before);
            assert!(!f.repository.join(".git/rocket-conflict-recovery").exists());
            assert!(!f.repository.join(".git/index.lock").exists());
        },
    );
}
#[test]
fn submodule_diff_lists_added_removed_commits_and_falls_back_when_missing() {
    isolated(
        "ROCKET_LOCAL_COMPARE_CASE",
        "submodule_diff_lists_added_removed_commits_and_falls_back_when_missing",
        || {
            let (f, path, ids) = gitlink_fixture();
            let base = id(f.run(&["rev-parse", "HEAD"]));
            f.run(&[
                "update-index",
                "--cacheinfo",
                &format!("160000,{},{}", ids[1], path),
            ]);
            f.run(&["commit", "-m", "ours"]);
            let ours = id(f.run(&["rev-parse", "HEAD"]));
            f.run(&[
                "update-index",
                "--cacheinfo",
                &format!("160000,{},{}", ids[2], path),
            ]);
            f.run(&["commit", "-m", "theirs"]);
            let theirs = id(f.run(&["rev-parse", "HEAD"]));
            let options = DiffOptions {
                target: DiffTarget::Between {
                    base: ours,
                    target: theirs,
                },
                ..Default::default()
            };
            let repo = f.repo();
            let diff = repo.submodule_diffs(&options, &c()).unwrap().remove(0);
            assert_eq!(diff.path.as_bytes(), path.as_bytes());
            assert!(diff.notice.is_none(), "{:?}", diff.notice);
            assert!(
                diff.commits
                    .iter()
                    .any(|c| c.id.as_str() == ids[1] && !c.added)
            );
            assert!(
                diff.commits
                    .iter()
                    .any(|c| c.id.as_str() == ids[2] && c.added)
            );
            fs::rename(f.repository.join(&path), f.root.join("retained-submodule")).unwrap();
            let diff = repo.submodule_diffs(&options, &c()).unwrap().remove(0);
            assert!(diff.notice.is_some());
            assert_eq!(diff.before.unwrap().as_str(), ids[1]);
            let cancelled = c();
            cancelled.cancel();
            assert!(matches!(
                repo.submodule_diffs(&options, &cancelled),
                Err(Error::Cancelled)
            ));
            assert!(!base.is_empty());
        },
    );
}
