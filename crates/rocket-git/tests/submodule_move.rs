mod common;
use common::{Fixture, isolated};
use rocket_git::*;
use std::fs;
fn c() -> CancellationToken {
    CancellationToken::new()
}
fn path(s: &str) -> RepoPath {
    RepoPath::new(s.as_bytes().to_vec()).unwrap()
}
fn base() -> Fixture {
    let f = Fixture::new();
    f.write("base.md", "base\n");
    f.commit("base");
    f
}
fn add(parent: &Fixture, source: &Fixture, name: &str) {
    parent.run(&[
        "-c",
        "protocol.file.allow=always",
        "submodule",
        "add",
        "--",
        source.repository.to_str().unwrap(),
        name,
    ]);
    parent.commit("add module");
}
fn request(from: &str, to: &str) -> SubmoduleAction {
    SubmoduleAction::Move {
        path: path(from),
        destination: path(to),
    }
}
#[test]
fn unicode_move_keeps_nested_checkouts_and_unrelated_staging() {
    isolated(
        "ROCKET_MOVE_CASE",
        "unicode_move_keeps_nested_checkouts_and_unrelated_staging",
        || {
            let leaf = base();
            let child = base();
            add(&child, &leaf, "nested/葉");
            let f = base();
            add(&f, &child, "vendor/引擎");
            f.run(&[
                "-c",
                "protocol.file.allow=always",
                "submodule",
                "update",
                "--init",
                "--recursive",
            ]);
            fs::create_dir(f.repository.join("components")).unwrap();
            f.write("base.md", "staged\n");
            f.run(&["add", "base.md"]);
            f.write("base.md", "working\n");
            let staged = f.run(&["show", ":base.md"]);
            let head = f.run(&["rev-parse", "HEAD"]);
            let config = fs::read(f.repository.join(".git/config")).unwrap();
            let r = f.repo();
            let p = r
                .preview_submodule_action(&request("vendor/引擎", "components/新 🚀"), &c())
                .unwrap();
            assert_eq!(p.descendants(), 1);
            let out = r.apply_submodule_action(&p, &c()).unwrap();
            assert!(out.result.is_ok(), "{:?}", out.result);
            assert!(out.verification.is_ok(), "{:?}", out.verification);
            assert!(!f.repository.join("vendor/引擎").exists());
            assert!(
                f.repository
                    .join("components/新 🚀/nested/葉/base.md")
                    .is_file()
            );
            assert_eq!(f.run(&["show", ":base.md"]), staged);
            assert_eq!(f.run(&["rev-parse", "HEAD"]), head);
            assert_eq!(fs::read(f.repository.join(".git/config")).unwrap(), config);
            assert_eq!(
                fs::read(f.repository.join("base.md")).unwrap(),
                b"working\n"
            );
            assert!(
                out.recovery_directory
                    .unwrap()
                    .join("MOVE-RECOVERY.md")
                    .is_file()
            );
            assert!(
                r.submodules(&c())
                    .unwrap()
                    .iter()
                    .all(|e| e.initialized && !e.dirty && e.blocked.is_none())
            );
            f.run_at(
                &f.repository.join("components/新 🚀/nested/葉"),
                &["status", "--porcelain"],
            );
        },
    );
}
#[test]
fn move_refuses_dirty_occupied_metadata_edits_and_cancel() {
    isolated(
        "ROCKET_MOVE_CASE",
        "move_refuses_dirty_occupied_metadata_edits_and_cancel",
        || {
            let s = base();
            let f = base();
            add(&f, &s, "module");
            let r = f.repo();
            let a = request("module", "renamed");
            f.write("module/local.txt", "keep\n");
            assert!(r.preview_submodule_action(&a, &c()).is_err());
            fs::remove_file(f.repository.join("module/local.txt")).unwrap();
            fs::create_dir(f.repository.join("renamed")).unwrap();
            assert!(r.preview_submodule_action(&a, &c()).is_err());
            fs::remove_dir(f.repository.join("renamed")).unwrap();
            let modules = fs::read(f.repository.join(".gitmodules")).unwrap();
            f.write(
                ".gitmodules",
                &(String::from_utf8(modules.clone()).unwrap() + "# user draft\n"),
            );
            assert!(r.preview_submodule_action(&a, &c()).is_err());
            fs::write(f.repository.join(".gitmodules"), &modules).unwrap();
            assert_eq!(
                fs::read(f.repository.join(".gitmodules")).unwrap(),
                f.run(&["show", "HEAD:.gitmodules"])
            );
            let p = r.preview_submodule_action(&a, &c()).unwrap_or_else(|e| {
                panic!(
                    "{e:?}; working diff: {:?}; cached diff: {:?}",
                    String::from_utf8_lossy(&f.run(&["diff", "--raw"])),
                    String::from_utf8_lossy(&f.run(&["diff", "--cached", "--raw"]))
                )
            });
            let index = fs::read(f.repository.join(".git/index")).unwrap();
            let stop = c();
            stop.cancel();
            assert!(matches!(
                r.apply_submodule_action(&p, &stop),
                Err(Error::Cancelled)
            ));
            f.write("module/base.md", "external edit\n");
            assert!(r.apply_submodule_action(&p, &c()).is_err());
            assert!(f.repository.join("module").is_dir());
            assert!(!f.repository.join("renamed").exists());
            assert_eq!(fs::read(f.repository.join(".git/index")).unwrap(), index);
            assert_eq!(fs::read(f.repository.join(".gitmodules")).unwrap(), modules);
        },
    );
}
#[test]
fn move_refuses_new_destination_and_stale_index_without_overwrite() {
    isolated(
        "ROCKET_MOVE_CASE",
        "move_refuses_new_destination_and_stale_index_without_overwrite",
        || {
            let s = base();
            let f = base();
            add(&f, &s, "module");
            let r = f.repo();
            let a = request("module", "renamed");
            let p = r.preview_submodule_action(&a, &c()).unwrap();
            f.write("renamed/user.txt", "precious\n");
            assert!(r.apply_submodule_action(&p, &c()).is_err());
            assert_eq!(
                fs::read(f.repository.join("renamed/user.txt")).unwrap(),
                b"precious\n"
            );
            let a = request("module", "other");
            let p = r.preview_submodule_action(&a, &c()).unwrap();
            f.write("base.md", "new staging\n");
            f.run(&["add", "base.md"]);
            let index = fs::read(f.repository.join(".git/index")).unwrap();
            assert!(r.apply_submodule_action(&p, &c()).is_err());
            assert_eq!(fs::read(f.repository.join(".git/index")).unwrap(), index);
        },
    );
}
