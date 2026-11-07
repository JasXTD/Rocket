//! LFS cache pruning: the review verifies each recovery copy on the remote, and cleanup
//! removes only reviewed cache objects that are still unchanged. Every mutation is in a
//! disposable repository; transports and user/system configuration are isolated by the
//! child runner. Needs Git LFS 3.8.1 or later beside the Git that Rocket uses.
#![cfg(any(windows, target_os = "linux"))]
#[allow(dead_code)]
mod common;
use common::{Fixture, isolated};
use rocket_git::*;
use std::{fs, path::PathBuf};
fn case(name: &str, f: fn()) {
    isolated("ROCKET_LFS_PRUNE_CASE", name, f)
}
fn c() -> CancellationToken {
    CancellationToken::new()
}
fn base() -> Fixture {
    let f = Fixture::new();
    f.write("文件.txt", "base\n");
    f.commit("synthetic base");
    f
}
fn remote(f: &Fixture) -> PathBuf {
    let path = f.root.join("remote.git");
    fs::create_dir(&path).unwrap();
    f.run_at(&path, &["init", "--bare", "--initial-branch=main"]);
    f.run(&["remote", "add", "origin", path.to_str().unwrap()]);
    f.run(&["push", "--set-upstream", "origin", "main"]);
    path
}
fn index(f: &Fixture) -> Vec<u8> {
    f.run(&["ls-files", "--stage", "-z"])
}
fn cached(f: &Fixture, remote: &std::path::Path, data: &[u8]) -> (PathBuf, String) {
    let input = f.root.join("payload");
    fs::write(&input, data).unwrap();
    let pointer =
        String::from_utf8(f.run(&["lfs", "pointer", &format!("--file={}", input.display())]))
            .unwrap();
    let id = pointer
        .lines()
        .find_map(|s| s.strip_prefix("oid sha256:"))
        .unwrap()
        .to_owned();
    let suffix = PathBuf::from(&id[..2]).join(&id[2..4]).join(&id);
    let file = f.repository.join(".git/lfs/objects").join(&suffix);
    fs::create_dir_all(file.parent().unwrap()).unwrap();
    fs::write(&file, data).unwrap();
    let dst = remote.join("lfs/objects").join(suffix);
    fs::create_dir_all(dst.parent().unwrap()).unwrap();
    fs::write(dst, data).unwrap();
    (file, id)
}
#[test]
fn lfs_prune_verifies_remote_and_deletes_only_reviewed_owned_objects() {
    case(
        "lfs_prune_verifies_remote_and_deletes_only_reviewed_owned_objects",
        || {
            let f = base();
            let server = remote(&f);
            let (file, id) = cached(&f, &server, b"synthetic unused object");
            let (kept, kept_id) = cached(&f, &server, b"current object");
            f.write(
                ".gitattributes",
                "*.dat filter=lfs diff=lfs merge=lfs -text\n",
            );
            f.write(
                "current.dat",
                format!(
                    "version https://git-lfs.github.com/spec/v1\noid sha256:{kept_id}\nsize 14\n"
                ),
            );
            f.commit("current LFS pointer");
            let repo = f.repo();
            let scope = LfsSourceScope {
                root: f.root.clone(),
                network_sources: vec![],
            };
            let p = repo
                .preview_lfs_prune("origin", Some(&scope), &c())
                .unwrap();
            assert_eq!(p.objects().len(), 1);
            assert_eq!(p.objects()[0].oid, id);
            assert_eq!(p.bytes(), 23);
            let before = index(&f);
            let r = repo.apply_lfs_prune(&p, &c()).unwrap();
            r.result.unwrap();
            assert_eq!(r.removed, 1);
            assert_eq!(r.removed_bytes, 23);
            assert!(!file.exists());
            assert_eq!(fs::read(&kept).unwrap(), b"current object");
            assert!(
                repo.preview_lfs_prune("origin", Some(&scope), &c())
                    .unwrap()
                    .objects()
                    .is_empty()
            );
            assert_eq!(index(&f), before);
            assert_eq!(fs::read(f.repository.join("文件.txt")).unwrap(), b"base\n");
        },
    );
}

#[test]
fn lfs_prune_retains_corrupt_dirty_unverified_and_cancelled_objects() {
    case(
        "lfs_prune_retains_corrupt_dirty_unverified_and_cancelled_objects",
        || {
            let f = base();
            let server = remote(&f);
            let (file, id) = cached(&f, &server, b"synthetic unused object");
            let repo = f.repo();
            let scope = LfsSourceScope {
                root: f.root.clone(),
                network_sources: vec![],
            };
            let p = repo
                .preview_lfs_prune("origin", Some(&scope), &c())
                .unwrap();
            let stop = c();
            stop.cancel();
            assert!(matches!(
                repo.apply_lfs_prune(&p, &stop),
                Err(Error::Cancelled)
            ));
            assert!(file.exists());
            fs::write(&file, b"CORRUPTED unused object").unwrap();
            let r = repo.apply_lfs_prune(&p, &c()).unwrap();
            assert!(r.result.is_err());
            assert_eq!(r.removed, 0);
            assert!(file.exists());
            fs::write(&file, b"synthetic unused object").unwrap();
            f.write("文件.txt", "dirty\n");
            assert!(
                repo.preview_lfs_prune("origin", Some(&scope), &c())
                    .is_err()
            );
            f.write("文件.txt", "base\n");
            fs::remove_file(
                server
                    .join("lfs/objects")
                    .join(&id[..2])
                    .join(&id[2..4])
                    .join(id),
            )
            .unwrap();
            let missing = repo.preview_lfs_prune("origin", Some(&scope), &c());
            assert!(missing.is_err(), "missing remote result: {missing:?}");
            assert!(file.exists());
        },
    );
}
