mod common;
use common::{Fixture, isolated};
use rocket_git::*;
use std::{
    fs,
    path::{Path, PathBuf},
};
fn token() -> CancellationToken {
    CancellationToken::new()
}
fn seed() -> Fixture {
    let f = Fixture::new();
    f.write("検索 é.txt", "base\n");
    f.commit("base");
    f
}
fn create(f: &Fixture, name: &str) -> WorktreeEntry {
    let p = f
        .repo()
        .preview_create_worktree(
            &f.root.canonicalize().unwrap().join(name),
            WorktreeCheckout::NewBranch(name.into()),
            "HEAD",
            &token(),
        )
        .unwrap();
    let r = f.repo().apply_worktree(&p, &token()).unwrap();
    assert!(r.command.is_ok() && r.verification.is_ok(), "{r:?}");
    f.repo()
        .worktrees(&token())
        .unwrap()
        .into_iter()
        .find(|e| e.path == p.path())
        .unwrap()
}
fn fresh(f: &Fixture, e: &WorktreeEntry) -> WorktreeEntry {
    f.repo()
        .worktrees(&token())
        .unwrap()
        .into_iter()
        .find(|v| v.branch == e.branch)
        .unwrap()
}
fn destination(f: &Fixture, name: &str) -> PathBuf {
    f.root.canonicalize().unwrap().join(name)
}
fn preview(
    f: &Fixture,
    k: WorktreeMaintenance,
    e: &WorktreeEntry,
    p: &Path,
) -> Result<WorktreePreview> {
    f.repo().preview_maintain_worktree(k, e, p, &token())
}
fn failure<T: std::fmt::Debug>(r: Result<T>, want: WorktreeFailure) {
    assert!(
        matches!(&r,Err(Error::Worktree(f))if *f==want),
        "want {want:?}, got {r:?}"
    )
}
fn ok(f: &Fixture, p: &WorktreePreview) -> WorktreeWriteReport {
    let r = f.repo().apply_worktree(p, &token()).unwrap();
    assert!(r.command.is_ok() && r.verification.is_ok(), "{r:?}");
    r
}
fn admin(f: &Fixture, e: &WorktreeEntry) -> PathBuf {
    PathBuf::from(
        String::from_utf8(f.run_at(&e.path, &["rev-parse", "--absolute-git-dir"]))
            .unwrap()
            .trim(),
    )
}

#[test]
fn move_unicode_is_non_replacing_and_preserves_source_changes() {
    isolated(
        "ROCKET_MAINTENANCE_CASE",
        "move_unicode_is_non_replacing_and_preserves_source_changes",
        || {
            let f = seed();
            let e = create(&f, "topic");
            let to = destination(&f, "移動 🚀 é");
            f.write("検索 é.txt", "main uncommitted");
            f.write("keep", "private");
            let index = fs::read(f.repository.join(".git/index")).unwrap();
            let p = preview(&f, WorktreeMaintenance::Move, &e, &to).unwrap();
            let cancel = token();
            cancel.cancel();
            assert!(matches!(
                f.repo().apply_worktree(&p, &cancel),
                Err(Error::Cancelled)
            ));
            assert!(e.path.exists());
            assert!(!to.exists());
            let r = ok(&f, &p);
            assert!(!e.path.exists());
            assert_eq!(fs::read(to.join("検索 é.txt")).unwrap(), b"base\n");
            assert_eq!(fs::read(f.repository.join(".git/index")).unwrap(), index);
            assert_eq!(fs::read(f.repository.join("keep")).unwrap(), b"private");
            assert!(r.metadata_backup.unwrap().join("gitdir.before").is_file());
            let moved = fresh(&f, &e);
            assert_eq!(f.repo().open_worktree(&moved, &token()).unwrap(), to);
            let exists = destination(&f, "existing");
            fs::create_dir(&exists).unwrap();
            fs::write(exists.join("keep"), "keep").unwrap();
            failure(
                preview(&f, WorktreeMaintenance::Move, &moved, &exists),
                WorktreeFailure::DestinationExists,
            );
            assert_eq!(fs::read(exists.join("keep")).unwrap(), b"keep");
        },
    );
}

#[test]
fn move_refuses_dirty_stale_metadata_locks_and_new_destination() {
    isolated(
        "ROCKET_MAINTENANCE_CASE",
        "move_refuses_dirty_stale_metadata_locks_and_new_destination",
        || {
            let f = seed();
            let e = create(&f, "topic");
            let to = destination(&f, "moved");
            let a = admin(&f, &e);
            let p = preview(&f, WorktreeMaintenance::Move, &e, &to).unwrap();
            fs::write(e.path.join("new"), "external").unwrap();
            failure(
                f.repo().apply_worktree(&p, &token()),
                WorktreeFailure::Dirty,
            );
            assert_eq!(fs::read(e.path.join("new")).unwrap(), b"external");
            fs::remove_file(e.path.join("new")).unwrap();
            fs::write(a.join("index.lock"), "other").unwrap();
            failure(
                preview(&f, WorktreeMaintenance::Move, &e, &to),
                WorktreeFailure::Locked,
            );
            assert_eq!(fs::read(a.join("index.lock")).unwrap(), b"other");
            fs::remove_file(a.join("index.lock")).unwrap();
            let p = preview(&f, WorktreeMaintenance::Move, &e, &to).unwrap();
            fs::write(a.join("config.worktree"), "[test]\n value=changed\n").unwrap();
            failure(
                f.repo().apply_worktree(&p, &token()),
                WorktreeFailure::StalePreview,
            );
            let p = preview(&f, WorktreeMaintenance::Move, &e, &to).unwrap();
            fs::create_dir(&to).unwrap();
            fs::write(to.join("keep"), "keep").unwrap();
            failure(
                f.repo().apply_worktree(&p, &token()),
                WorktreeFailure::DestinationExists,
            );
            assert!(e.path.exists());
            assert_eq!(fs::read(to.join("keep")).unwrap(), b"keep");
        },
    );
}

#[test]
fn repair_preserves_dirty_checkout_and_rejects_wrong_folder_or_external_metadata() {
    isolated(
        "ROCKET_MAINTENANCE_CASE",
        "repair_preserves_dirty_checkout_and_rejects_wrong_folder_or_external_metadata",
        || {
            let f = seed();
            let e = create(&f, "topic");
            let a = admin(&f, &e);
            let to = destination(&f, "移動 🚀");
            fs::write(e.path.join("検索 é.txt"), "staged").unwrap();
            f.run_at(&e.path, &["add", "."]);
            fs::write(e.path.join("検索 é.txt"), "working").unwrap();
            fs::write(e.path.join("untracked"), "private").unwrap();
            let index = fs::read(a.join("index")).unwrap();
            fs::rename(&e.path, &to).unwrap();
            let e = fresh(&f, &e);
            assert!(e.prunable.is_some());
            failure(
                preview(&f, WorktreeMaintenance::Repair, &e, &f.repository),
                WorktreeFailure::InvalidDestination,
            );
            let p = preview(&f, WorktreeMaintenance::Repair, &e, &to).unwrap();
            fs::write(a.join("external"), "outside edit").unwrap();
            failure(
                f.repo().apply_worktree(&p, &token()),
                WorktreeFailure::StalePreview,
            );
            let p = preview(&f, WorktreeMaintenance::Repair, &e, &to).unwrap();
            let cancel = token();
            cancel.cancel();
            assert!(matches!(
                f.repo().apply_worktree(&p, &cancel),
                Err(Error::Cancelled)
            ));
            let _ = ok(&f, &p);
            assert_eq!(fs::read(a.join("index")).unwrap(), index);
            assert_eq!(fs::read(to.join("検索 é.txt")).unwrap(), b"working");
            assert_eq!(fs::read(to.join("untracked")).unwrap(), b"private");
            assert!(fresh(&f, &e).prunable.is_none());
        },
    );
}

#[test]
fn prune_archives_only_selected_missing_registration_with_recovery_ref() {
    isolated(
        "ROCKET_MAINTENANCE_CASE",
        "prune_archives_only_selected_missing_registration_with_recovery_ref",
        || {
            let f = seed();
            let e = create(&f, "topic");
            let other = create(&f, "other");
            let a = admin(&f, &e);
            let other_admin = admin(&f, &other);
            let index = fs::read(a.join("index")).unwrap();
            let head = fs::read(a.join("HEAD")).unwrap();
            // Simulate removed folders by moving them inside this disposable fixture, retaining files.
            fs::rename(&e.path, destination(&f, "archived-folder")).unwrap();
            fs::rename(&other.path, destination(&f, "other-folder")).unwrap();
            let e = fresh(&f, &e);
            let p = preview(&f, WorktreeMaintenance::Prune, &e, Path::new("")).unwrap();
            let reference = p.recovery_reference().unwrap();
            let cancel = token();
            cancel.cancel();
            assert!(matches!(
                f.repo().apply_worktree(&p, &cancel),
                Err(Error::Cancelled)
            ));
            assert!(a.exists());
            let r = ok(&f, &p);
            let backup = r.metadata_backup.unwrap().join("registration");
            assert_eq!(fs::read(backup.join("index")).unwrap(), index);
            assert_eq!(fs::read(backup.join("HEAD")).unwrap(), head);
            assert!(!a.exists());
            assert!(other_admin.exists());
            assert!(!backup.join("locked").exists());
            assert_eq!(
                f.run(&["rev-parse", &reference]).trim_ascii(),
                p.target().as_str().as_bytes()
            );
            assert!(
                f.repo()
                    .worktrees(&token())
                    .unwrap()
                    .iter()
                    .any(|x| x.branch == other.branch)
            );
        },
    );
}

#[test]
fn prune_protects_resurrected_folder_locked_and_active_registration() {
    isolated(
        "ROCKET_MAINTENANCE_CASE",
        "prune_protects_resurrected_folder_locked_and_active_registration",
        || {
            let f = seed();
            let e = create(&f, "topic");
            let a = admin(&f, &e);
            fs::rename(&e.path, destination(&f, "moved")).unwrap();
            let e = fresh(&f, &e);
            let p = preview(&f, WorktreeMaintenance::Prune, &e, Path::new("")).unwrap();
            fs::create_dir(&e.path).unwrap();
            fs::write(e.path.join("keep"), "external").unwrap();
            assert!(f.repo().apply_worktree(&p, &token()).is_err());
            assert!(a.exists());
            assert_eq!(fs::read(e.path.join("keep")).unwrap(), b"external");
            fs::remove_file(e.path.join("keep")).unwrap();
            fs::remove_dir(&e.path).unwrap();
            fs::write(a.join("locked"), "offline").unwrap();
            let locked = fresh(&f, &e);
            failure(
                preview(&f, WorktreeMaintenance::Prune, &locked, Path::new("")),
                WorktreeFailure::Locked,
            );
            fs::remove_file(a.join("locked")).unwrap();
            fs::write(a.join("MERGE_HEAD"), e.head.as_ref().unwrap().as_str()).unwrap();
            failure(
                preview(&f, WorktreeMaintenance::Prune, &e, Path::new("")),
                WorktreeFailure::ActiveOperation,
            );
        },
    );
}
