#![cfg(windows)]
mod common;
use common::{Fixture, isolated};
use rocket_git::*;
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    thread,
    time::{Duration, Instant},
};
fn token() -> CancellationToken {
    CancellationToken::new()
}
fn oid(f: &Fixture, r: &str) -> String {
    String::from_utf8(f.run(&["rev-parse", r]))
        .unwrap()
        .trim()
        .into()
}
fn stub(f: &Fixture) -> PathBuf {
    let folder = f.root.join("AI fixture 文");
    fs::create_dir_all(&folder).unwrap();
    let exe = folder.join("claude.exe");
    let out = Command::new("rustc")
        .args(["--edition=2024", "--crate-name=claude_cli_fixture"])
        .arg(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples/claude_cli_fixture.rs"))
        .arg("-o")
        .arg(&exe)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    exe
}
fn request(exe: &Path, provider: DraftProvider) -> DraftRequest {
    DraftRequest {
        provider,
        executable: Some(exe.into()),
        style: DraftStyle::Plain,
        language: "English".into(),
    }
}
fn count(exe: &Path) -> usize {
    (1..)
        .take_while(|n| {
            exe.parent()
                .unwrap()
                .join(format!("request-{n}.txt"))
                .exists()
        })
        .count()
}
fn state(f: &Fixture) -> Vec<Vec<u8>> {
    ["HEAD", "index", "config"]
        .into_iter()
        .map(|p| fs::read(f.repository.join(".git").join(p)).unwrap())
        .chain(
            ["local.txt", "private-untracked.txt"]
                .into_iter()
                .map(|p| fs::read(f.repository.join(p)).unwrap()),
        )
        .collect()
}
fn prepared() -> Fixture {
    let f = Fixture::new();
    f.write("old 文.rs", "fn hello() { println!(\"界 🚀\"); }\n");
    f.write("edited.rs", "fn work() { old(); }\n");
    f.write("local.txt", "base\n");
    f.commit("base");
    f.run(&["branch", "review-base"]);
    f.run(&["mv", "old 文.rs", "new 文.rs"]);
    f.write("edited.rs", "fn work() { changed(); }\n");
    f.write("binary.bin", [0, 1, 2]);
    f.commit("feature");
    f.run(&["branch", "review-target"]);
    f.run(&["switch", "review-base"]);
    f.write("main-only.txt", "PRIVATE_BASE_ONLY\n");
    f.commit("base diverges");
    f.run(&["update-ref", "refs/heads/review-base", &oid(&f, "HEAD")]);
    f.write("local.txt", "PRIVATE_STAGED\n");
    f.run(&["add", "local.txt"]);
    f.write("local.txt", "PRIVATE_UNSTAGED\n");
    f.write("private-untracked.txt", "PRIVATE_UNTRACKED\n");
    f
}
#[test]
fn preview_is_pinned_uses_merge_base_and_protects_local_state() {
    isolated(
        "ROCKET_BRANCH_REVIEW_CASE",
        "preview_is_pinned_uses_merge_base_and_protects_local_state",
        || {
            let f = prepared();
            let repo = f.repo();
            let before = state(&f);
            let p = repo
                .prepare_branch_review("review-base", "review-target", &token())
                .unwrap();
            assert_eq!(p.commits, 1);
            assert_eq!(p.scope.files, 3);
            assert_eq!(p.scope.binary_files, 1);
            assert!(p.files.iter().any(|f| f.original_path.is_some()));
            assert!(p.payload().contains("new 文.rs") && p.payload().contains("changed()"));
            for secret in [
                "PRIVATE_BASE_ONLY",
                "PRIVATE_STAGED",
                "PRIVATE_UNSTAGED",
                "PRIVATE_UNTRACKED",
            ] {
                assert!(!p.payload().contains(secret));
            }
            assert_ne!(p.base, p.merge_base);
            assert!(p.diff().starts_with("diff --git ") && p.payload().contains(p.diff()));
            assert!(p.diff().contains("changed()") && !p.diff().contains("<repository-diff>"));
            assert_eq!((p.base_input(), p.target_input()), ("review-base", "review-target"));
            // Which field names nothing: each is said on its own.
            assert!(matches!(
                repo.prepare_branch_review("review-bsae", "review-target", &token()),
                Err(Error::BranchReview(BranchReviewFailure::BaseNotFound))
            ));
            assert!(matches!(
                repo.prepare_branch_review("review-base", "review-trget", &token()),
                Err(Error::BranchReview(BranchReviewFailure::TargetNotFound))
            ));
            assert!(matches!(
                repo.prepare_branch_review("review-target", "review-target", &token()),
                Err(Error::BranchReview(BranchReviewFailure::NoChanges))
            ));
            let tree = oid(&f, "review-target^{tree}");
            let orphan = String::from_utf8(f.run(&["commit-tree", &tree, "-m", "orphan"])).unwrap();
            assert!(matches!(
                repo.prepare_branch_review(orphan.trim(), "review-target", &token()),
                Err(Error::BranchReview(BranchReviewFailure::NoUniqueBase))
            ));
            let cancel = token();
            cancel.cancel();
            assert!(matches!(
                repo.prepare_branch_review("review-base", "review-target", &cancel),
                Err(Error::Cancelled)
            ));
            assert_eq!(state(&f), before);
        },
    )
}
#[test]
fn both_cli_reviews_use_preview_reject_moved_refs_and_cancel_owned_processes() {
    isolated(
        "ROCKET_BRANCH_REVIEW_CASE",
        "both_cli_reviews_use_preview_reject_moved_refs_and_cancel_owned_processes",
        || {
            let f = prepared();
            let repo = f.repo();
            let before = state(&f);
            let p = repo
                .prepare_branch_review("review-base", "review-target", &token())
                .unwrap();
            let claude = stub(&f);
            let codex = claude.with_file_name("codex.exe");
            fs::copy(&claude, &codex).unwrap();
            let folder = claude.parent().unwrap();
            fs::write(
                folder.join("reply"),
                "Possible bug at edited.rs:1: check the missing error path.",
            )
            .unwrap();
            for (provider, exe) in [
                (DraftProvider::Claude, &claude),
                (DraftProvider::Codex, &codex),
            ] {
                let req = request(exe, provider);
                let n = count(exe);
                let review = repo.review_branch(&p, &req, &token()).unwrap();
                assert!(review.contains("edited.rs:1"));
                let prompt =
                    fs::read_to_string(folder.join(format!("request-{}.txt", n + 1))).unwrap();
                assert!(prompt.contains(p.payload()));
                assert!(prompt.contains("introduced bugs"));
                assert!(prompt.contains(r#""findings":[{"kind":"bug""#), "the reply format is asked for");
                assert!(prompt.find("Reply with one JSON object") < prompt.find(p.payload()), "before the repository data");
                for private in ["PRIVATE_STAGED", "PRIVATE_UNSTAGED", "PRIVATE_UNTRACKED"] {
                    assert!(!prompt.contains(private));
                }
                let arguments =
                    fs::read_to_string(folder.join(format!("arguments-{}.txt", n + 1))).unwrap();
                assert!(arguments.contains("You review the committed Git diff"));
                f.run(&[
                    "update-ref",
                    "refs/heads/review-target",
                    p.base.as_str(),
                    p.target.as_str(),
                ]);
                assert!(matches!(
                    repo.review_branch(&p, &req, &token()),
                    Err(Error::BranchReview(BranchReviewFailure::Changed))
                ));
                assert_eq!(count(exe), n + 1);
                f.run(&[
                    "update-ref",
                    "refs/heads/review-target",
                    p.target.as_str(),
                    p.base.as_str(),
                ]);
                // A deleted name moved too.
                f.run(&["update-ref", "-d", "refs/heads/review-target"]);
                assert!(matches!(
                    repo.review_branch(&p, &req, &token()),
                    Err(Error::BranchReview(BranchReviewFailure::Changed))
                ));
                assert_eq!(count(exe), n + 1);
                f.run(&["update-ref", "refs/heads/review-target", p.target.as_str()]);
                fs::write(folder.join("mode"), "garbage").unwrap();
                assert!(matches!(
                    repo.review_branch(&p, &req, &token()),
                    Err(Error::CommitDraft {
                        failure: DraftFailure::InvalidReply,
                        ..
                    })
                ));
                fs::write(folder.join("mode"), "slow").unwrap();
                if folder.join("heartbeat").exists() {
                    fs::remove_file(folder.join("heartbeat")).unwrap();
                }
                let cancel = token();
                let child = cancel.clone();
                let repository = repo.clone();
                let preview = p.clone();
                let worker =
                    thread::spawn(move || repository.review_branch(&preview, &req, &child));
                let deadline = Instant::now() + Duration::from_secs(20);
                while !folder.join("heartbeat").exists() && Instant::now() < deadline {
                    thread::sleep(Duration::from_millis(20));
                }
                assert!(folder.join("heartbeat").exists(), "CLI child did not start");
                // The CLI fixture creates the marker before starting its child.
                cancel.cancel();
                assert!(matches!(worker.join().unwrap(), Err(Error::Cancelled)));
                let stopped = fs::metadata(folder.join("heartbeat")).unwrap().len();
                thread::sleep(Duration::from_millis(180));
                assert_eq!(
                    fs::metadata(folder.join("heartbeat")).unwrap().len(),
                    stopped,
                    "CLI child outlived cancellation"
                );
                fs::write(folder.join("mode"), "ok").unwrap();
            }
            assert_eq!(state(&f), before);
        },
    )
}
