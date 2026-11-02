mod common;
use common::{Fixture, isolated};
use rocket_git::*;
use std::{fs, path::PathBuf};

const NAME: &str = "vendor/描画 🚀 é";
fn id(f: &Fixture) -> String {
    String::from_utf8(f.run(&["rev-parse", "HEAD"]))
        .unwrap()
        .trim()
        .to_owned()
}
fn setup() -> (Fixture, Fixture, String, String) {
    let child = Fixture::new();
    child.write("file.txt", "first\n");
    child.commit("first");
    let first = id(&child);
    child.write("file.txt", "second\n");
    child.commit("second");
    let second = id(&child);
    let parent = Fixture::new();
    parent.write("parent.txt", "parent\n");
    parent.commit("parent");
    parent.run(&[
        "config",
        "--file",
        ".gitmodules",
        "submodule.renderer.path",
        NAME,
    ]);
    parent.run(&[
        "config",
        "--file",
        ".gitmodules",
        "submodule.renderer.url",
        child.repository.to_str().unwrap(),
    ]);
    parent.run(&["add", ".gitmodules"]);
    parent.run(&[
        "update-index",
        "--add",
        "--cacheinfo",
        &format!("160000,{first},{NAME}"),
    ]);
    parent.run(&["commit", "-m", "record submodule"]);
    (parent, child, first, second)
}
fn list(f: &Fixture) -> Vec<SubmoduleEntry> {
    f.repo().submodules(&CancellationToken::new()).unwrap()
}
fn apply(f: &Fixture, e: &SubmoduleEntry) -> SubmoduleWriteReport {
    let c = CancellationToken::new();
    let r = f.repo();
    let p = r.preview_submodule(e, &c).unwrap();
    r.apply_submodule(&p, &c).unwrap()
}
fn snapshot(f: &Fixture) -> (Vec<u8>, Vec<u8>, Vec<u8>) {
    (
        f.run(&["rev-parse", "HEAD"]),
        fs::read(f.repository.join(".git/index")).unwrap(),
        fs::read(f.repository.join("parent.txt")).unwrap(),
    )
}
#[test]
fn initialize_update_unicode_and_parent_staging() {
    isolated(
        "ROCKET_SUBMODULE_CASE",
        "initialize_update_unicode_and_parent_staging",
        || {
            let (f, _source, first, second) = setup();
            f.write("parent.txt", "local parent edit\n");
            let before = snapshot(&f);
            let entries = list(&f);
            assert_eq!(entries.len(), 1);
            assert!(!entries[0].initialized);
            assert_eq!(entries[0].target.as_str(), first);
            let result = apply(&f, &entries[0]);
            assert!(result.result.is_ok(), "{:?}", result.result);
            assert!(result.verification.is_ok(), "{:?}", result.verification);
            assert_eq!(snapshot(&f), before);
            assert_eq!(
                fs::read_to_string(f.repository.join(NAME).join("file.txt")).unwrap(),
                "first\n"
            );
            assert!(!f.run(&["submodule", "status"]).starts_with(b"-"));
            f.run(&[
                "update-index",
                "--cacheinfo",
                &format!("160000,{second},{NAME}"),
            ]);
            let before = snapshot(&f);
            let result = apply(&f, &list(&f)[0]);
            assert!(result.result.is_ok(), "{:?}", result.result);
            assert!(result.verification.is_ok());
            assert_eq!(snapshot(&f), before);
            let e = &list(&f)[0];
            assert_eq!(e.head.as_ref().unwrap().as_str(), second);
            assert!(e.branch.is_none());
            assert!(result.recovery_reference.is_some());
            assert_eq!(
                f.repo()
                    .open_submodule(e, &CancellationToken::new())
                    .unwrap(),
                e.directory
            );
        },
    );
}
#[test]
fn dirty_hidden_and_occupied_folders_are_protected() {
    isolated(
        "ROCKET_SUBMODULE_CASE",
        "dirty_hidden_and_occupied_folders_are_protected",
        || {
            let (f, _source, _, _) = setup();
            let dest = f.repository.join(NAME);
            fs::create_dir_all(&dest).unwrap();
            fs::write(dest.join("keep.txt"), "keep").unwrap();
            assert!(matches!(
                f.repo()
                    .preview_submodule(&list(&f)[0], &CancellationToken::new()),
                Err(Error::Submodule(SubmoduleFailure::DestinationOccupied))
            ));
            assert_eq!(fs::read_to_string(dest.join("keep.txt")).unwrap(), "keep");
            fs::rename(&dest, f.root.join("saved occupied folder")).unwrap();
            assert!(apply(&f, &list(&f)[0]).result.is_ok());
            for (name, body) in [("file.txt", "edited"), ("untracked 文", "keep")] {
                fs::write(dest.join(name), body).unwrap();
                let before = snapshot(&f);
                assert!(matches!(
                    f.repo()
                        .preview_submodule(&list(&f)[0], &CancellationToken::new()),
                    Err(Error::Submodule(SubmoduleFailure::Dirty))
                ));
                assert_eq!(snapshot(&f), before);
            }
            f.run_at(&dest, &["update-index", "--assume-unchanged", "file.txt"]);
            assert_eq!(list(&f)[0].blocked, Some(SubmoduleFailure::HiddenIndex));
        },
    );
}
#[test]
fn stale_preview_and_cancel_before_apply_make_no_changes() {
    isolated(
        "ROCKET_SUBMODULE_CASE",
        "stale_preview_and_cancel_before_apply_make_no_changes",
        || {
            let (f, _source, _, second) = setup();
            let c = CancellationToken::new();
            let p = f.repo().preview_submodule(&list(&f)[0], &c).unwrap();
            let mut altered = p.clone();
            altered.entry.directory = altered.entry.parent.clone();
            let malformed_cancel = CancellationToken::new();
            let timeout = malformed_cancel.clone();
            let timer = std::thread::spawn(move || {
                std::thread::sleep(std::time::Duration::from_secs(1));
                timeout.cancel();
            });
            let invalid = f.repo().apply_submodule(&altered, &malformed_cancel);
            timer.join().unwrap();
            assert!(matches!(
                invalid,
                Err(Error::Submodule(SubmoduleFailure::StalePreview))
            ));
            let foreign = Fixture::new();
            assert!(matches!(
                foreign.repo().apply_submodule(&p, &c),
                Err(Error::Submodule(SubmoduleFailure::StalePreview))
            ));
            f.run(&[
                "update-index",
                "--cacheinfo",
                &format!("160000,{second},{NAME}"),
            ]);
            let before = snapshot(&f);
            assert!(matches!(
                f.repo().apply_submodule(&p, &c),
                Err(Error::Submodule(SubmoduleFailure::StalePreview))
            ));
            assert_eq!(snapshot(&f), before);
            assert!(!f.repository.join(NAME).exists());
            let p = f.repo().preview_submodule(&list(&f)[0], &c).unwrap();
            c.cancel();
            assert!(matches!(
                f.repo().apply_submodule(&p, &c),
                Err(Error::Cancelled)
            ));
            assert_eq!(snapshot(&f), before);
        },
    );
}
#[test]
fn nested_submodules_are_discovered_and_reviewed_individually() {
    isolated(
        "ROCKET_SUBMODULE_CASE",
        "nested_submodules_are_discovered_and_reviewed_individually",
        || {
            let (f, source, _, old_target) = setup();
            let nested = Fixture::new();
            nested.write("text.txt", "nested 文\n");
            nested.commit("nested");
            let n = id(&nested);
            source.run(&[
                "config",
                "--file",
                ".gitmodules",
                "submodule.text.path",
                "deps/文字 é",
            ]);
            source.run(&[
                "config",
                "--file",
                ".gitmodules",
                "submodule.text.url",
                nested.repository.to_str().unwrap(),
            ]);
            source.run(&["add", ".gitmodules"]);
            source.run(&[
                "update-index",
                "--add",
                "--cacheinfo",
                &format!("160000,{n},deps/文字 é"),
            ]);
            source.run(&["commit", "-m", "nested module"]);
            let target = id(&source);
            f.run(&[
                "update-index",
                "--cacheinfo",
                &format!("160000,{target},{NAME}"),
            ]);
            assert!(apply(&f, &list(&f)[0]).result.is_ok());
            let entries = list(&f);
            assert_eq!(entries.len(), 2);
            assert_eq!(entries[1].depth, 1);
            assert!(!entries[1].initialized);
            assert!(apply(&f, &entries[1]).result.is_ok());
            assert_eq!(list(&f)[1].head.as_ref().unwrap().as_str(), n);
            let path = f.repository.join(NAME).join("deps/文字 é");
            fs::write(path.join("text.txt"), "keep nested edit").unwrap();
            assert!(matches!(
                f.repo()
                    .preview_submodule(&list(&f)[0], &CancellationToken::new()),
                Err(Error::Submodule(SubmoduleFailure::Dirty))
            ));
            assert_eq!(
                fs::read_to_string(path.join("text.txt")).unwrap(),
                "keep nested edit"
            );
            f.run_at(&path, &["restore", "text.txt"]);
            f.run(&[
                "update-index",
                "--cacheinfo",
                &format!("160000,{old_target},{NAME}"),
            ]);
            let report = apply(&f, &list(&f)[0]);
            assert!(matches!(
                report.result,
                Err(Error::Submodule(SubmoduleFailure::NestedLayoutChange))
            ));
            assert_eq!(list(&f)[0].head.as_ref().unwrap().as_str(), target);
            assert_eq!(
                fs::read_to_string(path.join("text.txt")).unwrap(),
                "nested 文\n"
            );
        },
    );
}
#[test]
fn relative_url_and_transport_failure_preserve_destination() {
    isolated(
        "ROCKET_SUBMODULE_CASE",
        "relative_url_and_transport_failure_preserve_destination",
        || {
            let (f, source, _, _) = setup();
            let base = source.repository.parent().unwrap().join("super.git");
            f.run(&["remote", "add", "origin", base.to_str().unwrap()]);
            f.run(&[
                "config",
                "--file",
                ".gitmodules",
                "submodule.renderer.url",
                "../repo with spaces 文",
            ]);
            let c = CancellationToken::new();
            let p = f.repo().preview_submodule(&list(&f)[0], &c).unwrap();
            assert_eq!(
                PathBuf::from(&p.source).canonicalize().unwrap(),
                source.repository.canonicalize().unwrap()
            );
            let result = f.repo().apply_submodule(&p, &c).unwrap();
            assert!(result.result.is_ok(), "{:?}", result.result);
            assert!(result.verification.is_ok());
        },
    );
}
#[test]
fn failed_source_retains_preparation_but_preserves_existing_files() {
    isolated(
        "ROCKET_SUBMODULE_CASE",
        "failed_source_retains_preparation_but_preserves_existing_files",
        || {
            let (f, _source, _, _) = setup();
            f.run(&[
                "config",
                "--file",
                ".gitmodules",
                "submodule.renderer.url",
                f.root.join("missing.git").to_str().unwrap(),
            ]);
            let before = snapshot(&f);
            let r = apply(&f, &list(&f)[0]);
            assert!(r.result.is_err());
            assert!(r.retained_directory.unwrap().is_dir());
            assert!(!f.repository.join(NAME).exists());
            assert_eq!(snapshot(&f), before);
        },
    );
}

#[test]
fn native_git_metadata_and_source_overrides_are_supported() {
    isolated(
        "ROCKET_SUBMODULE_CASE",
        "native_git_metadata_and_source_overrides_are_supported",
        || {
            let (f, _source, _, second) = setup();
            f.run(&[
                "-c",
                "protocol.file.allow=always",
                "submodule",
                "update",
                "--init",
                "--",
                NAME,
            ]);
            assert!(f.repository.join(NAME).join(".git").is_file());
            f.run(&[
                "update-index",
                "--cacheinfo",
                &format!("160000,{second},{NAME}"),
            ]);
            let before = snapshot(&f);
            let r = apply(&f, &list(&f)[0]);
            assert!(r.result.is_ok(), "{:?}", r.result);
            assert!(r.verification.is_ok());
            assert_eq!(snapshot(&f), before);
            assert_eq!(list(&f)[0].head.as_ref().unwrap().as_str(), second);
        },
    );
}

#[test]
fn unsafe_paths_helpers_and_changed_urls_are_refused() {
    isolated(
        "ROCKET_SUBMODULE_CASE",
        "unsafe_paths_helpers_and_changed_urls_are_refused",
        || {
            let (f, source, _, _) = setup();
            let c = CancellationToken::new();
            let e = list(&f)[0].clone();
            let p = f.repo().preview_submodule(&e, &c).unwrap();
            f.run(&[
                "config",
                "--file",
                ".gitmodules",
                "submodule.renderer.url",
                "ext::echo unsafe",
            ]);
            assert!(matches!(
                f.repo().apply_submodule(&p, &c),
                Err(Error::Submodule(SubmoduleFailure::StalePreview))
            ));
            assert!(matches!(
                f.repo().preview_submodule(&list(&f)[0], &c),
                Err(Error::Submodule(SubmoduleFailure::UnsupportedSource))
            ));
            f.run(&[
                "config",
                "--file",
                ".gitmodules",
                "submodule.renderer.url",
                source.repository.to_str().unwrap(),
            ]);
            f.run(&[
                "config",
                "--file",
                ".gitmodules",
                "submodule.renderer.path",
                "../outside",
            ]);
            assert!(matches!(
                f.repo().submodules(&c),
                Err(Error::Submodule(SubmoduleFailure::InvalidLayout))
            ));
            assert!(!f.repository.join(NAME).exists());
        },
    );
}

#[test]
fn unrelated_gitfile_and_existing_lock_are_never_overridden() {
    isolated(
        "ROCKET_SUBMODULE_CASE",
        "unrelated_gitfile_and_existing_lock_are_never_overridden",
        || {
            let (f, source, _, _) = setup();
            let dest = f.repository.join(NAME);
            fs::create_dir_all(&dest).unwrap();
            fs::write(
                dest.join(".git"),
                format!("gitdir: {}\n", source.repository.join(".git").display()),
            )
            .unwrap();
            assert!(matches!(
                f.repo().submodules(&CancellationToken::new()),
                Err(Error::Submodule(SubmoduleFailure::InvalidLayout))
            ));
            fs::rename(&dest, f.root.join("retained-unrelated-gitfile")).unwrap();
            assert!(apply(&f, &list(&f)[0]).result.is_ok());
            fs::write(dest.join(".git/index.lock"), "keep lock").unwrap();
            let e = &list(&f)[0];
            assert_eq!(e.blocked, Some(SubmoduleFailure::Busy));
            assert!(matches!(
                f.repo().preview_submodule(e, &CancellationToken::new()),
                Err(Error::Submodule(SubmoduleFailure::Busy))
            ));
            assert_eq!(
                fs::read_to_string(dest.join(".git/index.lock")).unwrap(),
                "keep lock"
            );
        },
    );
}

#[test]
fn child_url_rewrite_changes_invalidate_review_before_fetch() {
    isolated(
        "ROCKET_SUBMODULE_CASE",
        "child_url_rewrite_changes_invalidate_review_before_fetch",
        || {
            let (f, _source, _, second) = setup();
            assert!(apply(&f, &list(&f)[0]).result.is_ok());
            f.run(&[
                "update-index",
                "--cacheinfo",
                &format!("160000,{second},{NAME}"),
            ]);
            let c = CancellationToken::new();
            let p = f.repo().preview_submodule(&list(&f)[0], &c).unwrap();
            let destination = f.root.join("different-source");
            fs::create_dir(&destination).unwrap();
            f.run_at(
                &f.repository.join(NAME),
                &[
                    "config",
                    &format!("url.{}.insteadOf", destination.display()),
                    &p.source,
                ],
            );
            let before = snapshot(&f);
            let old = list(&f)[0].head.clone();
            assert!(matches!(
                f.repo().apply_submodule(&p, &c),
                Err(Error::Submodule(SubmoduleFailure::StalePreview))
            ));
            assert_eq!(snapshot(&f), before);
            assert_eq!(list(&f)[0].head, old);
        },
    );
}
