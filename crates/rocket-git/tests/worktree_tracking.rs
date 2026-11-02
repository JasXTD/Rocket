//! A new worktree on a new local branch that tracks a remote branch.
mod common;
use common::{Fixture, isolated};
use rocket_git::*;
use std::path::{Path, PathBuf};

const CASE: &str = "ROCKET_WORKTREE_TRACKING_CASE";

fn token() -> CancellationToken {
    CancellationToken::new()
}
fn text(f: &Fixture, at: &Path, args: &[&str]) -> String {
    String::from_utf8(f.run_at(at, args)).unwrap().trim().into()
}
fn tracking(branch: &str, upstream: &str) -> WorktreeCheckout {
    WorktreeCheckout::TrackingBranch {
        branch: branch.into(),
        upstream: upstream.into(),
    }
}
fn preview(f: &Fixture, path: &Path, checkout: WorktreeCheckout) -> Result<WorktreePreview> {
    f.repo()
        .preview_create_worktree(path, checkout, "", &token())
}
fn failure<T: std::fmt::Debug>(r: Result<T>, want: WorktreeFailure) {
    assert!(
        matches!(&r, Err(Error::Worktree(f)) if *f == want),
        "expected {want:?}, got {r:?}"
    )
}
/// main at `base`; origin/feature/x one commit ahead, known only as a remote branch.
fn setup() -> (Fixture, String, String, PathBuf) {
    let f = Fixture::new();
    f.write("a.txt", "a\n");
    f.commit("base");
    let base = text(&f, &f.repository, &["rev-parse", "HEAD"]);
    f.write("b.txt", "b\n");
    f.commit("feature work");
    let feature = text(&f, &f.repository, &["rev-parse", "HEAD"]);
    f.run(&["reset", "--hard", &base]);
    // A remote without any network: its refspec and remote-tracking refs only.
    f.run(&[
        "config",
        "remote.origin.url",
        "https://example.invalid/r.git",
    ]);
    f.run(&[
        "config",
        "remote.origin.fetch",
        "+refs/heads/*:refs/remotes/origin/*",
    ]);
    f.run(&["update-ref", "refs/remotes/origin/feature/x", &feature]);
    f.run(&[
        "symbolic-ref",
        "refs/remotes/origin/HEAD",
        "refs/remotes/origin/feature/x",
    ]);
    let root = f.root.canonicalize().unwrap();
    (f, base, feature, root)
}

#[test]
fn a_remote_branch_becomes_a_new_tracking_branch_in_the_new_worktree() {
    isolated(
        CASE,
        "a_remote_branch_becomes_a_new_tracking_branch_in_the_new_worktree",
        || {
            let (f, _, feature, root) = setup();
            let destination = root.join("tracking 文");
            let p = preview(&f, &destination, tracking("feature/x", "origin/feature/x")).unwrap();
            assert_eq!(p.target().as_str(), feature);
            assert_eq!(
                p.checkout().and_then(WorktreeCheckout::upstream),
                Some("origin/feature/x")
            );
            let r = f.repo().apply_worktree(&p, &token()).unwrap();
            assert!(r.command.is_ok() && r.verification.is_ok(), "{r:?}");
            assert!(r.retained_workspace.is_none(), "{r:?}");
            assert_eq!(text(&f, &destination, &["rev-parse", "HEAD"]), feature);
            assert_eq!(
                text(&f, &destination, &["symbolic-ref", "--short", "HEAD"]),
                "feature/x"
            );
            assert_eq!(
                text(
                    &f,
                    &f.repository,
                    &["rev-parse", "--abbrev-ref", "feature/x@{upstream}"]
                ),
                "origin/feature/x"
            );
            assert!(destination.join("b.txt").exists());
            // The current worktree keeps its branch and files.
            assert_eq!(
                text(&f, &f.repository, &["symbolic-ref", "--short", "HEAD"]),
                "main"
            );
            assert!(!f.repository.join("b.txt").exists());
        },
    );
}

#[test]
fn only_real_remote_branches_are_tracked_and_a_moved_one_is_stale() {
    isolated(
        CASE,
        "only_real_remote_branches_are_tracked_and_a_moved_one_is_stale",
        || {
            let (f, base, _, root) = setup();
            let destination = root.join("refused");
            for upstream in ["origin/HEAD", "origin/missing", "main", "-x"] {
                failure(
                    preview(&f, &destination, tracking("topic", upstream)),
                    WorktreeFailure::UpstreamUnavailable,
                );
            }
            failure(
                preview(&f, &destination, tracking("main", "origin/feature/x")),
                WorktreeFailure::BranchExists,
            );
            failure(
                preview(&f, &destination, tracking("bad name", "origin/feature/x")),
                WorktreeFailure::InvalidBranch,
            );
            // The remote branch moves (a fetch, say) after review: nothing is created.
            let p = preview(&f, &destination, tracking("topic", "origin/feature/x")).unwrap();
            f.run(&["update-ref", "refs/remotes/origin/feature/x", &base]);
            failure(
                f.repo().apply_worktree(&p, &token()),
                WorktreeFailure::StalePreview,
            );
            assert!(!destination.exists());
            assert!(f.run(&["branch", "--list", "topic"]).is_empty());
        },
    );
}
