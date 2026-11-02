mod common;
use common::{Fixture, isolated};
use rocket_git::*;
use std::{
    fs,
    path::PathBuf,
    thread,
    time::{Duration, Instant},
};

fn token() -> CancellationToken {
    CancellationToken::new()
}
fn text(f: &Fixture, args: &[&str]) -> String {
    String::from_utf8(f.run(args))
        .unwrap()
        .trim_end()
        .to_owned()
}
fn setup(format: &str) -> (Fixture, PathBuf) {
    let f = Fixture::with_format(format);
    f.write("検索/notes 🚀.txt", "base\n");
    f.commit("PRIVATE baseline");
    let remote = f.root.join("PRIVATE upstream 中文.git");
    f.run(&[
        "init",
        "--bare",
        &format!("--object-format={format}"),
        remote.to_str().unwrap(),
    ]);
    f.run(&["remote", "add", "origin", remote.to_str().unwrap()]);
    (f, remote)
}
fn snapshot(f: &Fixture) -> Vec<Vec<u8>> {
    vec![
        f.run(&["rev-parse", "HEAD"]),
        f.run(&["ls-files", "--stage", "-z"]),
        f.run(&["status", "--porcelain=v2", "-z"]),
        f.run(&["for-each-ref", "--format=%(refname) %(objectname)"]),
        fs::read(f.repository.join(".git/config")).unwrap(),
        fs::read(f.repository.join(".git/index")).unwrap(),
        fs::read(f.repository.join("検索/notes 🚀.txt")).unwrap(),
    ]
}
fn review(f: &Fixture, name: &str, action: RemoteTagAction) -> RemoteTagPreview {
    let repo = f.repo();
    let tags = repo.remote_tags(b"origin", &token()).unwrap();
    repo.preview_remote_tag(&tags, name.as_bytes(), action, &token())
        .unwrap()
}
fn refused<T: std::fmt::Debug>(result: Result<T>, want: TagFailure) {
    assert!(
        matches!(&result, Err(Error::Tag(f)) if *f==want),
        "{result:?}"
    );
}

#[test]
fn remote_tags_roundtrip_sha1_sha256_preserves_dirty_checkout_and_exact_objects() {
    isolated(
        "ROCKET_REMOTE_TAG_CASE",
        "remote_tags_roundtrip_sha1_sha256_preserves_dirty_checkout_and_exact_objects",
        || {
            for format in ["sha1", "sha256"] {
                let (f, remote) = setup(format);
                f.run(&[
                    "tag",
                    "--no-sign",
                    "-a",
                    "release/検索-🚀",
                    "-m",
                    "PRIVATE exact annotation",
                ]);
                f.run(&["tag", "local-only"]);
                f.run(&["config", "remote.origin.mirror", "true"]);
                f.run(&["config", "remote.origin.push", "+refs/*:refs/*"]);
                f.run(&["config", "push.followTags", "true"]);
                f.run(&["config", "push.default", "matching"]);
                f.write("検索/notes 🚀.txt", "staged\n");
                f.run(&["add", "--all"]);
                f.write("検索/notes 🚀.txt", "PRIVATE unstaged\n");
                f.write("PRIVATE untracked", "keep\n");
                let before = snapshot(&f);
                let p = review(&f, "release/検索-🚀", RemoteTagAction::Publish);
                let id = p.expected_result().unwrap().clone();
                let r = f.repo().apply_remote_tag(&p, &token()).unwrap();
                assert!(r.result.is_ok() && r.verification.is_ok(), "{r:?}");
                assert_eq!(r.observed.unwrap(), Some(id.clone()));
                assert_eq!(
                    f.run_at(&remote, &["for-each-ref", "--format=%(refname)"]),
                    "refs/tags/release/検索-🚀\n".as_bytes()
                );
                let tags = f.repo().remote_tags(b"origin", &token()).unwrap();
                let entry = tags
                    .entries
                    .iter()
                    .find(|e| e.name == "release/検索-🚀".as_bytes())
                    .unwrap();
                assert_eq!(entry.relation(), "published");
                assert!(entry.peeled.is_some());
                assert_eq!(entry.remote_id, Some(id));
                assert_eq!(snapshot(&f), before);
                let p = review(&f, "release/検索-🚀", RemoteTagAction::Delete);
                let r = f.repo().apply_remote_tag(&p, &token()).unwrap();
                assert!(r.result.is_ok() && r.verification.is_ok(), "{r:?}");
                assert_eq!(r.observed.unwrap(), None);
                assert_eq!(snapshot(&f), before);
                assert!(f.run_at(&remote, &["for-each-ref"]).is_empty());
            }
        },
    );
}

#[test]
fn remote_tags_refuse_stale_sources_destinations_and_remote_annotation_replacement() {
    isolated(
        "ROCKET_REMOTE_TAG_CASE",
        "remote_tags_refuse_stale_sources_destinations_and_remote_annotation_replacement",
        || {
            let (f, remote) = setup("sha1");
            f.run(&["tag", "--no-sign", "-a", "version", "-m", "first"]);
            let p = review(&f, "version", RemoteTagAction::Publish);
            f.run(&["tag", "--no-sign", "-f", "-a", "version", "-m", "second"]);
            refused(
                f.repo().apply_remote_tag(&p, &token()),
                TagFailure::StalePreview,
            );
            let p = review(&f, "version", RemoteTagAction::Publish);
            f.run(&[
                "config",
                "remote.origin.pushurl",
                "https://example.invalid/private",
            ]);
            refused(
                f.repo().apply_remote_tag(&p, &token()),
                TagFailure::RemoteDestinationChanged,
            );
            f.run(&["config", "--unset", "remote.origin.pushurl"]);
            let p = review(&f, "version", RemoteTagAction::Publish);
            assert!(
                f.repo()
                    .apply_remote_tag(&p, &token())
                    .unwrap()
                    .result
                    .is_ok()
            );
            let p = review(&f, "version", RemoteTagAction::Delete);
            f.run(&[
                "tag",
                "--no-sign",
                "-f",
                "-a",
                "version",
                "-m",
                "third same target",
            ]);
            f.run(&[
                "push",
                "--force",
                "origin",
                "refs/tags/version:refs/tags/version",
            ]);
            let changed = f.run_at(&remote, &["rev-parse", "refs/tags/version"]);
            refused(
                f.repo().apply_remote_tag(&p, &token()),
                TagFailure::RemoteChanged,
            );
            assert_eq!(
                changed,
                f.run_at(&remote, &["rev-parse", "refs/tags/version"])
            );
            let scan = f.repo().remote_tags(b"origin", &token()).unwrap();
            refused(
                f.repo()
                    .preview_remote_tag(&scan, b"version", RemoteTagAction::Publish, &token()),
                TagFailure::RemoteExists,
            );
            let other = Fixture::new();
            refused(
                other.repo().apply_remote_tag(&p, &token()),
                TagFailure::RemoteDestinationChanged,
            );
        },
    );
}

#[test]
fn remote_tags_push_destination_is_explicit_and_cannot_target_this_repository() {
    isolated(
        "ROCKET_REMOTE_TAG_CASE",
        "remote_tags_push_destination_is_explicit_and_cannot_target_this_repository",
        || {
            let (f, remote) = setup("sha1");
            f.run(&["tag", "v1"]);
            let other = f.root.join("separate push.git");
            f.run(&["init", "--bare", other.to_str().unwrap()]);
            f.run(&["config", "remote.origin.pushurl", other.to_str().unwrap()]);
            let p = review(&f, "v1", RemoteTagAction::Publish);
            assert!(
                f.repo()
                    .apply_remote_tag(&p, &token())
                    .unwrap()
                    .result
                    .is_ok()
            );
            assert!(f.run_at(&remote, &["for-each-ref"]).is_empty());
            assert!(!f.run_at(&other, &["for-each-ref"]).is_empty());
            f.run(&[
                "config",
                "--add",
                "remote.origin.pushurl",
                remote.to_str().unwrap(),
            ]);
            refused(
                f.repo().remote_tags(b"origin", &token()),
                TagFailure::RemoteUnsupported,
            );
            f.run(&["config", "--unset-all", "remote.origin.pushurl"]);
            for dest in [&f.repository, &f.repository.join(".git")] {
                f.run(&["config", "remote.origin.pushurl", dest.to_str().unwrap()]);
                refused(
                    f.repo().remote_tags(b"origin", &token()),
                    TagFailure::RemoteUnsupported,
                );
            }
            let worktree = f.root.join("linked");
            f.run(&[
                "worktree",
                "add",
                "-b",
                "linked",
                worktree.to_str().unwrap(),
            ]);
            f.run(&[
                "config",
                "remote.origin.pushurl",
                worktree.to_str().unwrap(),
            ]);
            refused(
                f.repo().remote_tags(b"origin", &token()),
                TagFailure::RemoteUnsupported,
            );
        },
    );
}

#[test]
fn remote_tags_refuse_second_push_rewrite_and_literal_remote_alias() {
    isolated(
        "ROCKET_REMOTE_TAG_CASE",
        "remote_tags_refuse_second_push_rewrite_and_literal_remote_alias",
        || {
            let (f, remote) = setup("sha1");
            // Match the canonical literal URL used by the actual transport, rather
            // than the fixture constructor's equivalent ../ spelling.
            let path = remote
                .canonicalize()
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            let path = path.strip_prefix("//?/").unwrap_or(&path).to_owned();
            f.run(&["config", "remote.origin.pushurl", &path]);
            f.run(&[
                "config",
                "url.https://example.invalid/PRIVATE.pushInsteadOf",
                &path,
            ]);
            refused(
                f.repo().remote_tags(b"origin", &token()),
                TagFailure::RemoteUnsupported,
            );
            f.run(&[
                "config",
                "--unset",
                "url.https://example.invalid/PRIVATE.pushInsteadOf",
            ]);
            let mirror = format!("remote.{path}.mirror");
            f.run(&["config", &mirror, "true"]);
            refused(
                f.repo().remote_tags(b"origin", &token()),
                TagFailure::RemoteUnsupported,
            );
            assert!(f.run_at(&remote, &["for-each-ref"]).is_empty());
        },
    );
}

#[test]
fn remote_tags_delete_symbolic_remote_tag_preserves_its_branch_target() {
    isolated(
        "ROCKET_REMOTE_TAG_CASE",
        "remote_tags_delete_symbolic_remote_tag_preserves_its_branch_target",
        || {
            let (f, remote) = setup("sha1");
            f.run(&["push", "origin", "HEAD:refs/heads/main"]);
            f.run_at(
                &remote,
                &["symbolic-ref", "refs/tags/alias", "refs/heads/main"],
            );
            let before = f.run_at(&remote, &["rev-parse", "refs/heads/main"]);
            let repo = f.repo();
            let tags = repo.remote_tags(b"origin", &token()).unwrap();
            assert_eq!(tags.entries[0].relation(), "symbolic");
            refused(
                repo.preview_remote_tag(&tags, b"alias", RemoteTagAction::Delete, &token()),
                TagFailure::SymbolicTag,
            );
            assert_eq!(f.run_at(&remote, &["rev-parse", "refs/heads/main"]), before);
            assert_eq!(
                f.run_at(&remote, &["symbolic-ref", "refs/tags/alias"]),
                b"refs/heads/main\n"
            );
            // A direct tag can become indirect without changing its object ID.
            f.run_at(
                &remote,
                &[
                    "update-ref",
                    "--no-deref",
                    "refs/tags/direct",
                    text(&f, &["rev-parse", "HEAD"]).as_str(),
                ],
            );
            let p = review(&f, "direct", RemoteTagAction::Delete);
            f.run_at(
                &remote,
                &["symbolic-ref", "refs/tags/direct", "refs/heads/main"],
            );
            refused(repo.apply_remote_tag(&p, &token()), TagFailure::SymbolicTag);
            assert_eq!(f.run_at(&remote, &["rev-parse", "refs/heads/main"]), before);
            f.run(&["tag", "publication"]);
            let p = review(&f, "publication", RemoteTagAction::Publish);
            fs::create_dir_all(remote.join("hooks")).unwrap();
            fs::write(
                remote.join("hooks/post-receive"),
                "#!/bin/sh\ngit symbolic-ref refs/tags/publication refs/heads/main\n",
            )
            .unwrap();
            let report = repo.apply_remote_tag(&p, &token()).unwrap();
            assert!(report.result.is_ok());
            refused(report.observed, TagFailure::SymbolicTag);
            assert!(report.verification.is_err());
            assert_eq!(f.run_at(&remote, &["rev-parse", "refs/heads/main"]), before);
        },
    );
}

#[test]
fn remote_tags_lease_refuses_change_between_review_and_server_update() {
    isolated(
        "ROCKET_REMOTE_TAG_CASE",
        "remote_tags_lease_refuses_change_between_review_and_server_update",
        || {
            for action in [RemoteTagAction::Publish, RemoteTagAction::Delete] {
                let (f, remote) = setup("sha1");
                f.run(&["tag", "v1"]);
                f.write("second", "second");
                f.commit("next");
                let other = text(&f, &["rev-parse", "HEAD"]);
                f.run(&["push", "origin", "HEAD:refs/heads/main"]);
                if action == RemoteTagAction::Delete {
                    f.run(&["push", "origin", "refs/tags/v1:refs/tags/v1"]);
                }
                let p = review(&f, "v1", action);
                // pre-push runs after the server's advertisement. Move the ref before
                // receive-pack performs its compare-and-swap. The reviewed old ID
                // must still protect both creation and deletion in this last window.
                let hook = format!(
                    "#!/bin/sh\ngit --git-dir='{}' update-ref refs/tags/v1 {other}\n",
                    remote.to_string_lossy().replace('\\', "/")
                );
                fs::write(f.root.join("no-hooks/pre-push"), hook).unwrap();
                let r = f.repo().apply_remote_tag(&p, &token()).unwrap();
                assert!(r.result.is_err(), "{r:?}");
                assert!(r.verification.is_err());
                assert_eq!(r.observed.unwrap().unwrap().as_str(), other);
                assert_eq!(
                    text(&f, &["rev-parse", "refs/tags/v1"]),
                    p.entry().local.as_ref().unwrap().id.as_str()
                );
            }
        },
    );
}

#[test]
fn remote_tags_cancellation_reconciles_before_and_after_remote_commit() {
    isolated(
        "ROCKET_REMOTE_TAG_CASE",
        "remote_tags_cancellation_reconciles_before_and_after_remote_commit",
        || {
            for (action, after) in [
                (RemoteTagAction::Publish, false),
                (RemoteTagAction::Publish, true),
                (RemoteTagAction::Delete, false),
                (RemoteTagAction::Delete, true),
            ] {
                let (f, remote) = setup("sha1");
                f.run(&["tag", "v1"]);
                if action == RemoteTagAction::Delete {
                    f.run(&["push", "origin", "refs/tags/v1:refs/tags/v1"]);
                }
                let p = review(&f, "v1", action);
                let before = snapshot(&f);
                let cancel = token();
                cancel.cancel();
                assert!(matches!(
                    f.repo().apply_remote_tag(&p, &cancel),
                    Err(Error::Cancelled)
                ));
                let marker = f.root.join("hook-ready");
                let hook = format!(
                    "#!/bin/sh\necho ready > '{}'\nsleep 30\n",
                    marker.to_string_lossy().replace('\\', "/")
                );
                let hooks = if after {
                    remote.join("hooks")
                } else {
                    f.root.join("no-hooks")
                };
                fs::create_dir_all(&hooks).unwrap();
                fs::write(
                    hooks.join(if after { "post-receive" } else { "pre-push" }),
                    hook,
                )
                .unwrap();
                let repo = f.repo();
                let cancel = token();
                let worker_cancel = cancel.clone();
                let worker = thread::spawn(move || repo.apply_remote_tag(&p, &worker_cancel));
                let deadline = Instant::now() + Duration::from_secs(20);
                while !marker.exists() && Instant::now() < deadline {
                    thread::sleep(Duration::from_millis(25));
                }
                assert!(marker.exists(), "hook did not start");
                let start = Instant::now();
                cancel.cancel();
                let r = worker.join().unwrap().unwrap();
                assert!(start.elapsed() < Duration::from_secs(15));
                assert!(matches!(r.result, Err(Error::Cancelled)));
                assert_eq!(r.verification.is_ok(), after);
                assert_eq!(
                    r.observed.unwrap().is_some(),
                    if action == RemoteTagAction::Publish {
                        after
                    } else {
                        !after
                    }
                );
                assert_eq!(snapshot(&f), before);
            }
        },
    );
}

#[test]
fn a_read_names_both_sides_without_fetching() {
    isolated(
        "ROCKET_REMOTE_TAG_CASE",
        "a_read_names_both_sides_without_fetching",
        || {
            let (f, remote) = setup("sha1");
            let first = text(&f, &["rev-parse", "HEAD"]);
            f.write("second.txt", "second\n");
            f.commit("Second subject");
            let second = text(&f, &["rev-parse", "HEAD"]);
            f.run(&["tag", "here", &second]);
            f.run(&["push", "origin", &format!("{first}:refs/tags/here")]);
            f.run(&["push", "origin", &format!("{first}:refs/tags/there")]);
            // A tag on a commit this repository has never seen.
            let other = f.root.join("other");
            f.run(&["init", other.to_str().unwrap()]);
            let git = |args: &[&str]| {
                let mut all = vec!["-C", other.to_str().unwrap(), "-c", "user.name=x", "-c", "user.email=x@example.invalid"];
                all.extend(args);
                f.run(&all);
            };
            fs::write(other.join("file"), "x").unwrap();
            git(&["add", "."]);
            git(&["commit", "-m", "Unseen"]);
            git(&["push", remote.to_str().unwrap(), "HEAD:refs/tags/unseen"]);
            let tags = f.repo().remote_tags(b"origin", &token()).unwrap();
            let entry = |name: &[u8]| tags.entries.iter().find(|e| e.name == name).unwrap();
            let here = entry(b"here");
            assert_eq!(here.relation(), "different");
            let summary = here.summary.as_ref().unwrap();
            assert_eq!(summary.subject, b"Second subject");
            assert_eq!(summary.target.as_ref().unwrap().as_str(), second);
            assert_eq!(here.remote_commit.as_ref().unwrap().subject, b"PRIVATE baseline");
            let there = entry(b"there");
            assert_eq!(there.relation(), "remote_only");
            assert_eq!(there.remote_commit.as_ref().unwrap().subject, b"PRIVATE baseline");
            assert!(entry(b"unseen").remote_commit.is_none(), "no commit, no name");
            assert!(text(&f, &["cat-file", "-t", "HEAD"]) == "commit");
        },
    );
}
