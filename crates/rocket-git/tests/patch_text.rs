mod common;
use common::{Fixture, isolated};
use rocket_git::*;
use std::fs;
fn token() -> CancellationToken {
    CancellationToken::new()
}

/// A patch from text (the clipboard) is reviewed and applied like a patch file.
#[test]
fn text_patches_are_reviewed_and_applied_like_files() {
    isolated(
        "ROCKET_PATCH_TEXT_CASE",
        "text_patches_are_reviewed_and_applied_like_files",
        || {
            let f = Fixture::new();
            f.write("src/検索 🚀.rs", "one\ntwo\nthree\n");
            f.commit("base");
            f.write("src/検索 🚀.rs", "one\n你好 🚀\nthree\n");
            let patch = f.run(&["diff", "--no-color"]);
            f.run(&["checkout", "--", "."]);
            let index = fs::read(f.repository.join(".git/index")).unwrap();
            let repo = f.repo();
            let preview = repo.preview_patch_text(&patch, &token()).unwrap();
            assert_eq!(preview.label(), "patch-import");
            assert_eq!(preview.files().len(), 1);
            assert_eq!(preview.patch(), patch.as_slice());
            let report = repo.apply_patch_preview(&preview, &token()).unwrap();
            assert!(report.result.is_ok() && report.verification.is_ok());
            assert_eq!(
                fs::read_to_string(f.repository.join("src/検索 🚀.rs")).unwrap(),
                "one\n你好 🚀\nthree\n"
            );
            assert_eq!(fs::read(f.repository.join(".git/index")).unwrap(), index);
            // Applying the same text again no longer fits the working file.
            assert!(matches!(
                repo.preview_patch_text(&patch, &token()),
                Err(Error::PatchTarget {
                    failure: PatchFailure::Conflict,
                    ..
                })
            ));
        },
    );
}

#[test]
fn empty_oversized_and_non_patch_text_is_refused() {
    isolated(
        "ROCKET_PATCH_TEXT_CASE",
        "empty_oversized_and_non_patch_text_is_refused",
        || {
            let f = Fixture::new();
            f.write("a.txt", "a\n");
            f.commit("base");
            let repo = f.repo();
            assert!(matches!(
                repo.preview_patch_text(b"", &token()),
                Err(Error::Patch(PatchFailure::Empty))
            ));
            assert!(matches!(
                repo.preview_patch_text(&vec![b'x'; PATCH_LIMIT + 1], &token()),
                Err(Error::Patch(PatchFailure::Unsupported))
            ));
            assert!(
                repo.preview_patch_text(b"Just some copied text\nwith two lines\n", &token())
                    .is_err()
            );
            let cancel = token();
            cancel.cancel();
            let patch =
                b"diff --git a/a.txt b/a.txt\n--- a/a.txt\n+++ b/a.txt\n@@ -1 +1 @@\n-a\n+b\n";
            assert!(matches!(
                repo.preview_patch_text(patch, &cancel),
                Err(Error::Cancelled)
            ));
            assert_eq!(fs::read(f.repository.join("a.txt")).unwrap(), b"a\n");
        },
    );
}
