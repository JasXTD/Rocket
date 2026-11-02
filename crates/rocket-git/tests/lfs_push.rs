#![cfg(windows)]
mod common;
use common::{Fixture, isolated};
use rocket_git::*;
use std::{
    fs,
    path::{Path, PathBuf},
    sync::mpsc::sync_channel,
};

fn case(name: &str, f: fn()) {
    isolated("ROCKET_LFS_PUSH_CASE", name, f)
}
fn object(admin: &Path, oid: &str) -> PathBuf {
    admin
        .join("lfs/objects")
        .join(&oid[..2])
        .join(&oid[2..4])
        .join(oid)
}
fn entry(f: &Fixture, name: &str, contents: &[u8]) -> (String, Vec<u8>) {
    let payload = f.root.join("payload");
    fs::write(&payload, contents).unwrap();
    let bytes = f.run(&["lfs", "pointer", &format!("--file={}", payload.display())]);
    let text = String::from_utf8(bytes.clone()).unwrap();
    let oid = text
        .lines()
        .find_map(|s| s.strip_prefix("oid sha256:"))
        .unwrap()
        .to_owned();
    let file = object(&f.repository.join(".git"), &oid);
    fs::create_dir_all(file.parent().unwrap()).unwrap();
    fs::write(file, contents).unwrap();
    f.write(name, &bytes);
    (oid, bytes)
}
fn remote(f: &Fixture, name: &str) -> PathBuf {
    let p = f.root.join(name);
    fs::create_dir(&p).unwrap();
    f.run_at(&p, &["init", "--bare"]);
    p
}
fn setup() -> (Fixture, PathBuf, String) {
    let f = Fixture::new();
    let (oid, _) = entry(&f, "素材/背景 🚀 é.dat", b"PRIVATE synthetic payload\n");
    f.commit("pointer");
    f.run(&["config", "lfs.repositoryformatversion", "0"]);
    let dst = remote(&f, "upstream 文.git");
    f.run(&["remote", "add", "origin", dst.to_str().unwrap()]);
    (f, dst, oid)
}
fn snapshot(f: &Fixture) -> (Vec<u8>, Vec<u8>, Vec<u8>) {
    (
        f.run(&["rev-parse", "HEAD"]),
        fs::read(f.repository.join(".git/index")).unwrap(),
        fs::read(f.repository.join(".git/config")).unwrap(),
    )
}
fn preview(f: &Fixture) -> LfsPreview {
    f.repo()
        .preview_lfs(
            LfsAction::Push("origin".into()),
            Some(&f.root),
            &CancellationToken::new(),
        )
        .unwrap()
}
fn passed(r: LfsWriteReport) {
    assert!(r.result.is_ok(), "{:?}", r.result);
    assert!(r.verification.is_ok(), "{:?}", r.verification);
    assert!(r.observed.is_ok());
    assert_eq!(r.upload_outcome, Some(LfsUploadOutcome::ConfirmedByHelper));
}

#[test]
fn lfs_push_uploads_only_reviewed_unique_snapshot_to_pushurl_with_progress() {
    case(
        "lfs_push_uploads_only_reviewed_unique_snapshot_to_pushurl_with_progress",
        || {
            let (f, fetch_dst, first) = setup();
            let (old, _) = entry(&f, "old.dat", b"historical only");
            f.commit("old");
            f.run(&["rm", "old.dat"]);
            let (second, bytes) = entry(&f, "second.dat", b"second content");
            f.write("alias.dat", &bytes);
            f.commit("snapshot");
            let (staged, _) = entry(&f, "staged.dat", b"staged only");
            f.run(&["add", "staged.dat"]);
            f.write("second.dat", b"PRIVATE working edits");
            f.write("untracked 文", b"keep me");
            let dst = remote(&f, "push target.git");
            f.run(&[
                "remote",
                "set-url",
                "--push",
                "origin",
                dst.to_str().unwrap(),
            ]);
            let before = snapshot(&f);
            let p = preview(&f);
            assert_eq!(p.entries().len(), 3);
            assert_eq!(p.object_count(), 2);
            assert_eq!(p.total_bytes(), 40);
            assert_eq!(
                p.local_source(),
                Some(dst.canonicalize().unwrap().as_path())
            );
            let (tx, rx) = sync_channel(128);
            passed(
                f.repo()
                    .apply_lfs_with_progress(&p, Some(&tx), &CancellationToken::new())
                    .unwrap(),
            );
            let progress: Vec<_> = rx.try_iter().collect();
            assert!(
                progress
                    .iter()
                    .any(|p| p.phase == FetchPhase::CheckingLfsObjects && p.current == 2)
            );
            assert!(
                progress
                    .iter()
                    .any(|p| p.phase == FetchPhase::UploadingLfsObjects && p.current > 0),
                "{progress:?}"
            );
            assert_eq!(
                fs::read(object(&dst, &first)).unwrap(),
                b"PRIVATE synthetic payload\n"
            );
            assert_eq!(fs::read(object(&dst, &second)).unwrap(), b"second content");
            for oid in [&old, &staged] {
                assert!(!object(&dst, oid).exists());
            }
            assert!(!fetch_dst.join("lfs/objects").exists());
            assert!(f.run_at(&dst, &["for-each-ref"]).is_empty());
            assert_eq!(snapshot(&f), before);
            assert_eq!(
                fs::read(f.repository.join("second.dat")).unwrap(),
                b"PRIVATE working edits"
            );
            assert_eq!(
                fs::read(f.repository.join("untracked 文")).unwrap(),
                b"keep me"
            );
            passed(
                f.repo()
                    .apply_lfs(&preview(&f), &CancellationToken::new())
                    .unwrap(),
            );
            assert_eq!(snapshot(&f), before);
        },
    )
}
#[test]
fn lfs_push_missing_and_corrupt_objects_never_upload() {
    case("lfs_push_missing_and_corrupt_objects_never_upload", || {
        let (f, dst, oid) = setup();
        let before = snapshot(&f);
        let cached = object(&f.repository.join(".git"), &oid);
        let original = fs::read(&cached).unwrap();
        fs::remove_file(&cached).unwrap();
        assert!(matches!(
            f.repo().preview_lfs(
                LfsAction::Push("origin".into()),
                Some(&f.root),
                &CancellationToken::new()
            ),
            Err(Error::Lfs(LfsFailure::MissingObject))
        ));
        fs::write(&cached, &original).unwrap();
        let p = preview(&f);
        fs::write(&cached, vec![b'x'; original.len()]).unwrap();
        let r = f.repo().apply_lfs(&p, &CancellationToken::new()).unwrap();
        assert!(matches!(
            r.result,
            Err(Error::Lfs(LfsFailure::InvalidObject))
        ));
        assert_eq!(r.upload_outcome, Some(LfsUploadOutcome::NotStarted));
        assert!(!object(&dst, &oid).exists());
        assert_eq!(snapshot(&f), before);
    })
}
#[test]
fn lfs_push_rechecks_index_head_and_destination_without_overwriting_edits() {
    case(
        "lfs_push_rechecks_index_head_and_destination_without_overwriting_edits",
        || {
            let (f, dst, oid) = setup();
            let repo = f.repo();
            let c = CancellationToken::new();
            for change in 0..3 {
                let p = preview(&f);
                match change {
                    0 => {
                        f.write("new file", "external");
                        f.run(&["add", "new file"]);
                    }
                    1 => {
                        f.run(&["commit", "--no-verify", "-m", "external commit"]);
                    }
                    _ => {
                        let other = remote(&f, "other.git");
                        f.run(&[
                            "remote",
                            "set-url",
                            "--push",
                            "origin",
                            other.to_str().unwrap(),
                        ]);
                    }
                }
                let changed = snapshot(&f);
                let r = repo.apply_lfs(&p, &c).unwrap();
                assert!(matches!(r.result, Err(Error::Lfs(LfsFailure::Stale))));
                assert_eq!(r.upload_outcome, Some(LfsUploadOutcome::NotStarted));
                assert_eq!(snapshot(&f), changed);
                assert!(!object(&dst, &oid).exists());
            }
        },
    )
}
#[test]
fn lfs_push_preserves_preexisting_corrupt_destination_and_rejects_unsafe_layout() {
    case(
        "lfs_push_preserves_preexisting_corrupt_destination_and_rejects_unsafe_layout",
        || {
            let (f, dst, oid) = setup();
            let path = object(&dst, &oid);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, b"PRIVATE destination sentinel").unwrap();
            let before = snapshot(&f);
            let p = preview(&f);
            let r = f.repo().apply_lfs(&p, &CancellationToken::new()).unwrap();
            assert!(matches!(
                r.result,
                Err(Error::Lfs(LfsFailure::InvalidObject))
            ));
            assert_eq!(r.upload_outcome, Some(LfsUploadOutcome::NotStarted));
            assert_eq!(fs::read(&path).unwrap(), b"PRIVATE destination sentinel");
            assert_eq!(snapshot(&f), before);
            fs::remove_file(&path).unwrap();
            fs::create_dir(&path).unwrap();
            assert!(
                f.repo()
                    .apply_lfs(&p, &CancellationToken::new())
                    .unwrap()
                    .result
                    .is_err()
            );
            assert!(path.is_dir());
        },
    )
}
#[test]
fn lfs_push_requires_one_destination_and_rejects_config_and_scope_redirects() {
    case(
        "lfs_push_requires_one_destination_and_rejects_config_and_scope_redirects",
        || {
            let (f, dst, _) = setup();
            let c = CancellationToken::new();
            let action = || LfsAction::Push("origin".into());
            f.run(&[
                "remote",
                "set-url",
                "--push",
                "origin",
                dst.to_str().unwrap(),
            ]);
            f.run(&[
                "remote",
                "set-url",
                "--add",
                "--push",
                "origin",
                dst.to_str().unwrap(),
            ]);
            assert!(matches!(
                f.repo().preview_lfs(action(), Some(&f.root), &c),
                Err(Error::Lfs(LfsFailure::Configuration))
            ));
            f.run(&["config", "--unset-all", "remote.origin.pushurl"]);
            for (key, value) in [
                ("lfs.pushurl", "https://example.invalid/private"),
                (
                    "url.https://example.invalid/.pushInsteadOf",
                    "https://example.invalid/",
                ),
            ] {
                f.run(&["config", key, value]);
                assert!(matches!(
                    f.repo().preview_lfs(action(), Some(&f.root), &c),
                    Err(Error::Lfs(LfsFailure::Configuration))
                ));
                f.run(&["config", "--unset", key]);
            }
            let outside = Fixture::new();
            f.run(&[
                "remote",
                "set-url",
                "--push",
                "origin",
                outside.repository.to_str().unwrap(),
            ]);
            assert!(f.repo().preview_lfs(action(), Some(&f.root), &c).is_err());
            f.run(&[
                "remote",
                "set-url",
                "--push",
                "origin",
                "https://example.invalid/unapproved.git",
            ]);
            assert!(f.repo().preview_lfs(action(), Some(&f.root), &c).is_err());
        },
    )
}
#[test]
fn lfs_push_empty_snapshot_and_precancellation_do_not_transfer() {
    case(
        "lfs_push_empty_snapshot_and_precancellation_do_not_transfer",
        || {
            let (f, dst, oid) = setup();
            let p = preview(&f);
            let before = snapshot(&f);
            let c = CancellationToken::new();
            c.cancel();
            assert!(matches!(f.repo().apply_lfs(&p, &c), Err(Error::Cancelled)));
            assert_eq!(snapshot(&f), before);
            assert!(!object(&dst, &oid).exists());
            f.run(&["rm", "素材/背景 🚀 é.dat"]);
            f.commit("no pointers");
            assert!(matches!(
                f.repo().preview_lfs(
                    LfsAction::Push("origin".into()),
                    Some(&f.root),
                    &CancellationToken::new()
                ),
                Err(Error::Lfs(LfsFailure::NoObjects))
            ));
        },
    )
}
