mod common;
use common::{Fixture, isolated};
use rocket_git::*;
use std::fs;

fn token() -> CancellationToken {
    CancellationToken::new()
}
fn setup(format: &str) -> Fixture {
    let f = Fixture::with_format(format);
    f.write("src/検索/結果 🚀 e\u{301}.rs", "base\n");
    f.commit("PRIVATE_FIRST 中文");
    f.write("other", "second\n");
    f.commit("PRIVATE_SECOND 🚀");
    f
}
fn text(f: &Fixture, args: &[&str]) -> String {
    String::from_utf8(f.run(args))
        .unwrap()
        .trim_end()
        .to_owned()
}
fn tags(f: &Fixture) -> Vec<TagEntry> {
    f.repo().tags(2000, &token()).unwrap().entries
}
fn entry(f: &Fixture, name: &str) -> TagEntry {
    tags(f)
        .into_iter()
        .find(|e| e.name == name.as_bytes())
        .unwrap()
}
fn create(f: &Fixture, name: &str, message: Option<&str>) -> TagPreview {
    f.repo()
        .preview_create_tag(name, "HEAD", message, &token())
        .unwrap()
}
fn ok(r: TagWriteReport) {
    assert!(r.result.is_ok(), "{r:?}");
    assert!(r.verification.is_ok(), "{r:?}");
    assert!(r.observed.is_ok());
    assert_eq!(r.failure, None);
}
fn refused<T: std::fmt::Debug>(r: Result<T>, want: TagFailure) {
    assert!(matches!(&r,Err(Error::Tag(f)) if *f==want), "{r:?}");
}

#[test]
fn tags_roundtrip_unicode_sha1_sha256_keeps_local_changes() {
    isolated(
        "ROCKET_TAG_CASE",
        "tags_roundtrip_unicode_sha1_sha256_keeps_local_changes",
        || {
            for format in ["sha1", "sha256"] {
                let f = setup(format);
                f.write("other", "staged\n");
                f.run(&["add", "other"]);
                f.write("other", "working\n");
                f.write("local", "untracked\n");
                let before = (
                    f.run(&["status", "--porcelain=v2", "-z"]),
                    f.run(&["ls-files", "--stage", "-z"]),
                    f.run(&["show-ref"]),
                    fs::read(f.repository.join(".git/config")).unwrap(),
                );
                for (message, name) in [
                    (None, "local/検索-🚀"),
                    (
                        Some("PRIVATE_ANNOTATION\n# literal\n検索 🚀\n"),
                        "release/中文-e\u{301}",
                    ),
                ] {
                    let p = create(&f, name, message);
                    let reviewed = p.details().entry.id.clone();
                    assert!(tags(&f).is_empty());
                    ok(f.repo().apply_tag(&p, &token()).unwrap());
                    let e = entry(&f, name);
                    assert_eq!(e.id, reviewed);
                    assert_eq!(e.annotated(), message.is_some());
                    let detail = f.repo().inspect_tag(&e, &token()).unwrap();
                    assert_eq!(detail.target.as_str(), text(&f, &["rev-parse", "HEAD"]));
                    if let Some(message) = message {
                        assert!(
                            detail
                                .annotation
                                .as_ref()
                                .unwrap()
                                .ends_with(message.as_bytes())
                        );
                        assert_ne!(detail.target, e.id);
                    }
                    let delete = f.repo().preview_delete_tag(&e, &token()).unwrap();
                    let reference = delete.recovery_reference().unwrap();
                    let r = f.repo().apply_tag(&delete, &token()).unwrap();
                    assert_eq!(r.recovery_reference.as_deref(), Some(reference.as_str()));
                    ok(r);
                    assert!(tags(&f).is_empty());
                    assert_eq!(text(&f, &["rev-parse", &reference]), reviewed.as_str());
                    // Recover the exact object, then delete again with the already-existing backup.
                    f.run(&[
                        "update-ref",
                        &format!("refs/tags/{name}"),
                        reviewed.as_str(),
                    ]);
                    ok(f.repo()
                        .apply_tag(
                            &f.repo()
                                .preview_delete_tag(&entry(&f, name), &token())
                                .unwrap(),
                            &token(),
                        )
                        .unwrap());
                }
                assert_eq!(f.run(&["status", "--porcelain=v2", "-z"]), before.0);
                assert_eq!(f.run(&["ls-files", "--stage", "-z"]), before.1);
                assert_eq!(f.run(&["show-ref", "--heads"]), before.2);
                assert_eq!(
                    fs::read(f.repository.join(".git/config")).unwrap(),
                    before.3
                );
                assert_eq!(fs::read(f.repository.join("other")).unwrap(), b"working\n");
                assert_eq!(
                    fs::read(f.repository.join("local")).unwrap(),
                    b"untracked\n"
                );
            }
        },
    );
}
#[test]
fn tag_names_messages_targets_and_identity_are_validated() {
    isolated(
        "ROCKET_TAG_CASE",
        "tag_names_messages_targets_and_identity_are_validated",
        || {
            let f = setup("sha1");
            for name in [
                "",
                "-force",
                "refs/tags/name",
                "HEAD",
                "@",
                "a b",
                "a..b",
                "a@{1}",
                "a\nb",
                "a\\b",
                "a\0b",
                "a.lock",
            ] {
                refused(
                    f.repo().preview_create_tag(name, "HEAD", None, &token()),
                    TagFailure::InvalidName,
                );
            }
            for message in [
                String::new(),
                " \n".into(),
                "a\0b".into(),
                "x".repeat(65537),
            ] {
                refused(
                    f.repo()
                        .preview_create_tag("valid", "HEAD", Some(&message), &token()),
                    TagFailure::InvalidMessage,
                );
            }
            refused(
                f.repo()
                    .preview_create_tag("valid", "missing-commit", None, &token()),
                TagFailure::TargetUnavailable,
            );
            refused(
                f.repo()
                    .preview_create_tag("valid", "--all", None, &token()),
                TagFailure::TargetUnavailable,
            );
            f.run(&["config", "--unset", "user.name"]);
            f.run(&["config", "--unset", "user.email"]);
            f.run(&["config", "user.useConfigOnly", "true"]);
            refused(
                f.repo()
                    .preview_create_tag("valid", "HEAD", Some("annotation"), &token()),
                TagFailure::IdentityUnavailable,
            );
            ok(f.repo()
                .apply_tag(&create(&f, "no-identity-needed", None), &token())
                .unwrap());
        },
    );
}
#[test]
fn tag_preview_pins_target_identity_and_unsigned_annotation() {
    isolated(
        "ROCKET_TAG_CASE",
        "tag_preview_pins_target_identity_and_unsigned_annotation",
        || {
            let f = setup("sha1");
            f.run(&["config", "tag.gpgSign", "true"]);
            f.run(&["config", "gpg.program", "must-never-run-rocket-fixture"]);
            let p = create(&f, "v1", Some("release"));
            let reviewed = p.details().annotation.clone().unwrap();
            let target = p.details().target.clone();
            f.run(&["config", "user.name", "Changed after preview"]);
            f.write("third", "third");
            f.commit("PRIVATE_THIRD");
            ok(f.repo().apply_tag(&p, &token()).unwrap());
            let d = f.repo().inspect_tag(&entry(&f, "v1"), &token()).unwrap();
            assert_eq!(d.target, target);
            assert_eq!(d.annotation, Some(reviewed));
            assert_eq!(text(&f, &["config", "tag.gpgSign"]), "true");
        },
    );
}
#[test]
fn duplicate_cross_repository_and_replaced_annotations_refuse_stale_writes() {
    isolated(
        "ROCKET_TAG_CASE",
        "duplicate_cross_repository_and_replaced_annotations_refuse_stale_writes",
        || {
            let f = setup("sha1");
            let other = setup("sha1");
            let p = create(&f, "v1", Some("original"));
            refused(
                other.repo().apply_tag(&p, &token()),
                TagFailure::StalePreview,
            );
            ok(f.repo().apply_tag(&p, &token()).unwrap());
            refused(f.repo().apply_tag(&p, &token()), TagFailure::AlreadyExists);
            let old = entry(&f, "v1");
            let delete = f.repo().preview_delete_tag(&old, &token()).unwrap();
            f.run(&[
                "tag",
                "--no-sign",
                "-f",
                "-a",
                "-m",
                "external annotation",
                "v1",
                "HEAD",
            ]);
            let replacement = entry(&f, "v1");
            assert_ne!(old.id, replacement.id);
            refused(
                f.repo().inspect_tag(&old, &token()),
                TagFailure::StalePreview,
            );
            refused(
                f.repo().apply_tag(&delete, &token()),
                TagFailure::StalePreview,
            );
            assert_eq!(entry(&f, "v1"), replacement);
            f.run(&["tag", "-d", "v1"]);
            refused(
                f.repo().apply_tag(&delete, &token()),
                TagFailure::StalePreview,
            );
            let child = create(&f, "prefix/child", None);
            ok(f.repo().apply_tag(&child, &token()).unwrap());
            // Prefix lookup must not mistake a descendant for the exact ref.
            let parent = create(&f, "prefix", None);
            let r = f.repo().apply_tag(&parent, &token()).unwrap();
            assert!(r.result.is_err());
            assert!(r.observed.unwrap().is_none());
            assert_eq!(tags(&f).len(), 1);
        },
    );
}
#[test]
fn tag_and_backup_locks_are_preserved_and_transaction_aborts() {
    isolated(
        "ROCKET_TAG_CASE",
        "tag_and_backup_locks_are_preserved_and_transaction_aborts",
        || {
            let f = setup("sha1");
            let p = create(&f, "v1", Some("annotation"));
            let lock = f.repository.join(".git/refs/tags/v1.lock");
            fs::create_dir_all(lock.parent().unwrap()).unwrap();
            fs::write(&lock, b"OTHER_LOCK").unwrap();
            let r = f.repo().apply_tag(&p, &token()).unwrap();
            assert_eq!(r.failure, Some(TagFailure::ReferenceLocked));
            assert!(tags(&f).is_empty());
            assert_eq!(fs::read(&lock).unwrap(), b"OTHER_LOCK");
            fs::remove_file(lock).unwrap();
            ok(f.repo().apply_tag(&p, &token()).unwrap());
            let delete = f
                .repo()
                .preview_delete_tag(&entry(&f, "v1"), &token())
                .unwrap();
            let backup = delete.recovery_reference().unwrap();
            let lock = f.repository.join(".git").join(format!("{backup}.lock"));
            fs::create_dir_all(lock.parent().unwrap()).unwrap();
            fs::write(&lock, b"BACKUP_LOCK").unwrap();
            let r = f.repo().apply_tag(&delete, &token()).unwrap();
            assert_eq!(r.failure, Some(TagFailure::ReferenceLocked));
            assert!(r.recovery_reference.is_none());
            assert_eq!(entry(&f, "v1").id, p.details().entry.id);
            assert_eq!(fs::read(&lock).unwrap(), b"BACKUP_LOCK");
        },
    );
}
#[test]
fn symbolic_refs_and_conflicting_recovery_refs_are_never_followed() {
    isolated(
        "ROCKET_TAG_CASE",
        "symbolic_refs_and_conflicting_recovery_refs_are_never_followed",
        || {
            let f = setup("sha1");
            let p = create(&f, "v1", None);
            ok(f.repo().apply_tag(&p, &token()).unwrap());
            let delete = f
                .repo()
                .preview_delete_tag(&entry(&f, "v1"), &token())
                .unwrap();
            let head = text(&f, &["rev-parse", "HEAD"]);
            f.run(&["symbolic-ref", "refs/tags/v1", "refs/heads/main"]);
            assert!(entry(&f, "v1").symbolic);
            refused(
                f.repo().apply_tag(&delete, &token()),
                TagFailure::StalePreview,
            );
            refused(
                f.repo().preview_delete_tag(&entry(&f, "v1"), &token()),
                TagFailure::SymbolicTag,
            );
            assert_eq!(text(&f, &["rev-parse", "HEAD"]), head);
            f.run(&["symbolic-ref", "--delete", "refs/tags/v1"]);
            ok(f.repo().apply_tag(&p, &token()).unwrap());
            let backup = delete.recovery_reference().unwrap();
            f.run(&["update-ref", &backup, "HEAD~"]);
            refused(
                f.repo().apply_tag(&delete, &token()),
                TagFailure::RecoveryReferenceConflict,
            );
            assert_eq!(entry(&f, "v1").id, p.details().entry.id);
            f.run(&["update-ref", "-d", &backup]);
            f.run(&["symbolic-ref", &backup, "refs/heads/main"]);
            refused(
                f.repo().apply_tag(&delete, &token()),
                TagFailure::RecoveryReferenceConflict,
            );
            assert_eq!(text(&f, &["rev-parse", "HEAD"]), head);
        },
    );
}
#[test]
fn inspect_nested_noncommit_packed_tags_and_ignore_replace_objects() {
    isolated(
        "ROCKET_TAG_CASE",
        "inspect_nested_noncommit_packed_tags_and_ignore_replace_objects",
        || {
            let f = setup("sha1");
            f.run(&["tag", "tree", "HEAD^{tree}"]);
            f.run(&["tag", "blob", "HEAD:other"]);
            f.run(&["tag", "--no-sign", "-a", "-m", "inner", "inner", "HEAD"]);
            f.run(&["tag", "--no-sign", "-a", "-m", "outer", "outer", "inner"]);
            f.run(&["pack-refs", "--all", "--prune"]);
            for (name, kind) in [("tree", "tree"), ("blob", "blob"), ("outer", "commit")] {
                let d = f.repo().inspect_tag(&entry(&f, name), &token()).unwrap();
                assert_eq!(d.target_type, kind);
                assert_eq!(d.commit.is_some(), kind == "commit");
                ok(f.repo()
                    .apply_tag(
                        &f.repo().preview_delete_tag(&d.entry, &token()).unwrap(),
                        &token(),
                    )
                    .unwrap());
            }
            f.run(&["replace", "HEAD", "HEAD~"]);
            let d = f.repo().inspect_tag(&entry(&f, "inner"), &token()).unwrap();
            assert_eq!(d.commit.unwrap().subject_lossy(), "PRIVATE_SECOND 🚀");
        },
    );
}
#[test]
fn tag_pages_unborn_cancellation_and_linked_worktree_ownership() {
    isolated(
        "ROCKET_TAG_CASE",
        "tag_pages_unborn_cancellation_and_linked_worktree_ownership",
        || {
            let unborn = Fixture::new();
            assert!(tags(&unborn).is_empty());
            refused(
                unborn
                    .repo()
                    .preview_create_tag("v1", "HEAD", None, &token()),
                TagFailure::TargetUnavailable,
            );
            let f = setup("sha1");
            let p = create(&f, "v1", None);
            let cancelled = token();
            cancelled.cancel();
            assert!(matches!(
                f.repo().apply_tag(&p, &cancelled),
                Err(Error::Cancelled)
            ));
            assert!(matches!(
                f.repo().tags(200, &cancelled),
                Err(Error::Cancelled)
            ));
            assert!(tags(&f).is_empty());
            ok(f.repo().apply_tag(&p, &token()).unwrap());
            ok(f.repo()
                .apply_tag(&create(&f, "v2", None), &token())
                .unwrap());
            let page = f.repo().tags(1, &token()).unwrap();
            assert_eq!(page.entries[0].name, b"v1");
            assert!(page.more);
            assert!(!f.repo().tags(2, &token()).unwrap().more);
            assert!(f.repo().tags(0, &token()).is_err());
            assert!(f.repo().tags(2001, &token()).is_err());
            let linked = f.root.join("linked 文");
            f.run(&[
                "worktree",
                "add",
                "-b",
                "linked",
                linked.to_str().unwrap(),
                "HEAD",
            ]);
            let repo = f.git.open(&linked, &token()).unwrap();
            assert_eq!(repo.tags(200, &token()).unwrap().entries, tags(&f));
            let p = f
                .repo()
                .preview_delete_tag(&entry(&f, "v1"), &token())
                .unwrap();
            ok(repo.apply_tag(&p, &token()).unwrap());
            assert_eq!(tags(&f).len(), 1);
        },
    );
}

#[test]
fn bare_reftable_and_literal_metacharacter_tags() {
    isolated(
        "ROCKET_TAG_CASE",
        "bare_reftable_and_literal_metacharacter_tags",
        || {
            let f = setup("sha1");
            let bare = f.root.join("bare-reftable.git");
            f.run(&[
                "clone",
                "--bare",
                "--no-local",
                "--ref-format=reftable",
                f.repository.to_str().unwrap(),
                bare.to_str().unwrap(),
            ]);
            f.run_at(&bare, &["config", "user.name", "Rocket Bare Fixture"]);
            f.run_at(&bare, &["config", "user.email", "bare@example.invalid"]);
            let repo = f.git.open(&bare, &token()).unwrap();
            assert!(repo.info().worktree.is_none());
            let name = "release/中文;$(literal)";
            let p = repo
                .preview_create_tag(name, "HEAD", Some("literal annotation"), &token())
                .unwrap();
            ok(repo.apply_tag(&p, &token()).unwrap());
            let page = repo.tags(200, &token()).unwrap();
            assert_eq!(page.entries.len(), 1);
            assert_eq!(page.entries[0].name, name.as_bytes());
            let d = repo.inspect_tag(&page.entries[0], &token()).unwrap();
            assert_eq!(d.target, p.details().target);
            ok(repo
                .apply_tag(
                    &repo.preview_delete_tag(&page.entries[0], &token()).unwrap(),
                    &token(),
                )
                .unwrap());
            assert!(repo.tags(200, &token()).unwrap().entries.is_empty());
            assert!(tags(&f).is_empty());
        },
    );
}
