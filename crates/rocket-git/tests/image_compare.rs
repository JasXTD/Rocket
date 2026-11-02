mod common;
use common::{Fixture, isolated};
use rocket_git::*;
use std::fs;
const NAME: &str = "assets/検索 空白 🚀 é.png";
fn options(target: DiffTarget) -> DiffOptions {
    DiffOptions {
        target,
        ..Default::default()
    }
}

#[test]
fn image_versions_are_literal_pinned_read_only_and_rename_aware() {
    isolated(
        "ROCKET_IMAGE_TEST",
        "image_versions_are_literal_pinned_read_only_and_rename_aware",
        || {
            for format in ["sha1", "sha256"] {
                let f = Fixture::with_format(format);
                f.write(NAME, b"old\0image");
                f.commit("first image");
                let commit = ObjectId::parse(
                    std::str::from_utf8(&f.run(&["rev-parse", "HEAD"]))
                        .unwrap()
                        .trim(),
                )
                .unwrap();
                f.write(NAME, b"staged\0image");
                f.run(&["add", "--", NAME]);
                f.write(NAME, b"working\0image");
                let repo = f.repo();
                let cancel = CancellationToken::new();
                let index = fs::read(f.repository.join(".git/index")).unwrap();
                let staged = repo
                    .image_comparisons(&options(DiffTarget::Staged), &cancel)
                    .unwrap();
                assert_eq!(staged.len(), 1);
                assert_eq!(staged[0].path().as_bytes(), NAME.as_bytes());
                let v = repo.image_versions(&staged[0], &cancel).unwrap();
                assert_eq!(v.before.as_deref(), Some(b"old\0image".as_slice()));
                assert_eq!(v.after.as_deref(), Some(b"staged\0image".as_slice()));
                let work = repo
                    .image_comparisons(&options(DiffTarget::Unstaged), &cancel)
                    .unwrap();
                let v = repo.image_versions(&work[0], &cancel).unwrap();
                assert_eq!(v.before.as_deref(), Some(b"staged\0image".as_slice()));
                assert_eq!(v.after.as_deref(), Some(b"working\0image".as_slice()));
                let initial = repo
                    .image_comparisons(&options(DiffTarget::Commit { id: commit }), &cancel)
                    .unwrap();
                let v = repo.image_versions(&initial[0], &cancel).unwrap();
                assert!(v.before.is_none());
                assert_eq!(v.after.as_deref(), Some(b"old\0image".as_slice()));
                assert_eq!(index, fs::read(f.repository.join(".git/index")).unwrap());
                f.write(NAME, b"changed again\0image");
                f.run(&["add", "--", NAME]);
                assert_eq!(
                    repo.image_versions(&staged[0], &cancel)
                        .unwrap()
                        .after
                        .as_deref(),
                    Some(b"staged\0image".as_slice())
                );
                f.commit("second");
                f.run(&["mv", "--", NAME, "assets/renamed 日本.png"]);
                let renamed = repo
                    .image_comparisons(&options(DiffTarget::Staged), &cancel)
                    .unwrap();
                assert_eq!(renamed.len(), 1);
                assert_eq!(
                    renamed[0].path().as_bytes(),
                    "assets/renamed 日本.png".as_bytes()
                );
                let v = repo.image_versions(&renamed[0], &cancel).unwrap();
                assert_eq!(v.before, v.after);
                let other = Fixture::new();
                assert!(other.repo().image_versions(&renamed[0], &cancel).is_err());
            }
        },
    );
}
#[test]
fn images_handle_unborn_added_deleted_limits_and_cancellation() {
    isolated(
        "ROCKET_IMAGE_TEST",
        "images_handle_unborn_added_deleted_limits_and_cancellation",
        || {
            let f = Fixture::new();
            let cancel = CancellationToken::new();
            let path = RepoPath::new(NAME).unwrap();
            f.write(NAME, b"new\0image");
            let repo = f.repo();
            let untracked = repo.untracked_image(&path, &cancel).unwrap();
            let v = repo.image_versions(&untracked, &cancel).unwrap();
            assert!(v.before.is_none());
            assert!(v.after.is_some());
            f.run(&["add", "--", NAME]);
            let staged = repo
                .image_comparisons(&options(DiffTarget::Staged), &cancel)
                .unwrap();
            assert!(
                repo.image_versions(&staged[0], &cancel)
                    .unwrap()
                    .before
                    .is_none()
            );
            f.commit("first");
            fs::remove_file(f.repository.join(NAME)).unwrap();
            let unstaged = repo
                .image_comparisons(&options(DiffTarget::Unstaged), &cancel)
                .unwrap();
            assert!(
                repo.image_versions(&unstaged[0], &cancel)
                    .unwrap()
                    .after
                    .is_none()
            );
            f.run(&["add", "--", NAME]);
            let staged = repo
                .image_comparisons(&options(DiffTarget::Staged), &cancel)
                .unwrap();
            assert!(
                repo.image_versions(&staged[0], &cancel)
                    .unwrap()
                    .after
                    .is_none()
            );
            f.write("large.png", b"");
            fs::OpenOptions::new()
                .write(true)
                .open(f.repository.join("large.png"))
                .unwrap()
                .set_len((IMAGE_FILE_LIMIT + 1) as u64)
                .unwrap();
            let large = repo
                .untracked_image(&RepoPath::new("large.png").unwrap(), &cancel)
                .unwrap();
            assert!(matches!(
                repo.image_versions(&large, &cancel),
                Err(Error::Unsupported(_))
            ));
            cancel.cancel();
            assert!(matches!(
                repo.image_versions(&untracked, &cancel),
                Err(Error::Cancelled)
            ));
            assert!(matches!(
                repo.image_comparisons(&options(DiffTarget::Staged), &cancel),
                Err(Error::Cancelled)
            ));
        },
    );
}
#[test]
fn merge_and_between_images_use_the_requested_parents() {
    isolated(
        "ROCKET_IMAGE_TEST",
        "merge_and_between_images_use_the_requested_parents",
        || {
            let f = Fixture::new();
            let oid = || {
                ObjectId::parse(
                    std::str::from_utf8(&f.run(&["rev-parse", "HEAD"]))
                        .unwrap()
                        .trim(),
                )
                .unwrap()
            };
            f.write(NAME, b"base\0image");
            f.commit("base");
            let base = oid();
            f.run(&["switch", "-c", "feature"]);
            f.write(NAME, b"feature\0image");
            f.commit("feature");
            f.run(&["switch", "main"]);
            f.write("other.txt", "main branch");
            f.commit("main");
            f.run(&["merge", "--no-ff", "feature", "-m", "merge"]);
            let merged = oid();
            let repo = f.repo();
            let cancel = CancellationToken::new();
            for target in [
                DiffTarget::Commit { id: merged.clone() },
                DiffTarget::Between {
                    base: base.as_str().into(),
                    target: merged.as_str().into(),
                },
            ] {
                let entries = repo.image_comparisons(&options(target), &cancel).unwrap();
                assert_eq!(entries.len(), 1);
                let versions = repo.image_versions(&entries[0], &cancel).unwrap();
                assert_eq!(versions.before.as_deref(), Some(b"base\0image".as_slice()));
                assert_eq!(
                    versions.after.as_deref(),
                    Some(b"feature\0image".as_slice())
                );
            }
        },
    );
}
#[test]
fn image_reads_do_not_execute_filters_or_follow_git_symlinks() {
    isolated(
        "ROCKET_IMAGE_TEST",
        "image_reads_do_not_execute_filters_or_follow_git_symlinks",
        || {
            let f = Fixture::new();
            f.write(NAME, b"old\0image");
            f.commit("first");
            f.write(NAME, b"new\0image");
            f.write(".gitattributes", b"*.png diff=forbidden filter=forbidden\n");
            f.run(&[
                "config",
                "diff.forbidden.command",
                "this-command-must-never-run",
            ]);
            f.run(&[
                "config",
                "diff.forbidden.textconv",
                "this-command-must-never-run",
            ]);
            f.run(&[
                "config",
                "filter.forbidden.smudge",
                "this-command-must-never-run",
            ]);
            let repo = f.repo();
            let cancel = CancellationToken::new();
            let entries = repo
                .image_comparisons(&options(DiffTarget::Unstaged), &cancel)
                .unwrap();
            let v = repo.image_versions(&entries[0], &cancel).unwrap();
            assert_eq!(v.after.as_deref(), Some(b"new\0image".as_slice()));
            let oid = String::from_utf8(f.run(&["rev-parse", &format!("HEAD:{NAME}")])).unwrap();
            f.run(&[
                "update-index",
                "--add",
                "--cacheinfo",
                &format!("120000,{},link.png", oid.trim()),
            ]);
            let entries = repo
                .image_comparisons(&options(DiffTarget::Staged), &cancel)
                .unwrap();
            assert!(matches!(
                repo.image_versions(&entries[0], &cancel),
                Err(Error::Unsupported(_))
            ));
        },
    );
}

#[test]
fn tga_and_webp_files_are_image_candidates_by_name() {
    isolated(
        "ROCKET_IMAGE_TEST",
        "tga_and_webp_files_are_image_candidates_by_name",
        || {
            assert!(image_path(b"art/texture.tga") && image_path("面 🚀.TGA".as_bytes()));
            assert!(!image_path(b"notes.tga.txt") && !image_path(b"tga/readme.md"));
            assert!(image_path(b"art/photo.webp") && image_path(b"PHOTO.WEBP"));
            assert!(!image_path(b"photo.webp.bak"));
            let f = Fixture::new();
            f.write("art/sky.TGA", b"before");
            f.write("art/photo.webp", b"before");
            f.write("notes.txt", b"before");
            f.commit("add");
            f.write("art/sky.TGA", b"after");
            f.write("art/photo.webp", b"after");
            f.write("notes.txt", b"after");
            let found = f
                .repo()
                .image_comparisons(&options(DiffTarget::Unstaged), &CancellationToken::new())
                .unwrap();
            let paths: Vec<_> = found.iter().map(|c| c.path().as_bytes().to_vec()).collect();
            assert_eq!(paths, [b"art/photo.webp".to_vec(), b"art/sky.TGA".to_vec()]);
        },
    );
}
