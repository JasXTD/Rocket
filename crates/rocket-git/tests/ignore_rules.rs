mod common;
use common::{Fixture, isolated};
use rocket_git::*;
use std::fs;
fn c() -> CancellationToken {
    CancellationToken::new()
}
fn f() -> Fixture {
    let f = Fixture::new();
    f.write("tracked.txt", "base\n");
    f.commit("base");
    f
}
#[test]
fn exact_unicode_rules_preserve_bom_crlf_comments_and_index() {
    isolated(
        "ROCKET_IGNORE",
        "exact_unicode_rules_preserve_bom_crlf_comments_and_index",
        || {
            let f = f();
            let before = b"\xef\xbb\xbf# PRIVATE comment\r\n*.tmp\r\n!keep.tmp";
            fs::write(f.repository.join(".gitignore"), before).unwrap();
            f.write("[cache]/日本 語.txt", "private");
            f.write("文档/缓存/file.dat", "private");
            f.write("keep.tmp", "keep");
            f.write("tracked.txt", "staged");
            f.run(&["add", "tracked.txt"]);
            let index = fs::read(f.repository.join(".git/index")).unwrap();
            let repo = f.repo();
            let s = repo.ignore_snapshot(&c()).unwrap();
            let rules = vec![
                literal_ignore_rule(
                    &RepoPath::new("[cache]/日本 語.txt".as_bytes().to_vec()).unwrap(),
                    false,
                )
                .unwrap(),
                literal_ignore_rule(
                    &RepoPath::new("文档/缓存".as_bytes().to_vec()).unwrap(),
                    true,
                )
                .unwrap(),
            ];
            let text = append_ignore_rules(s.text(), &rules).unwrap();
            let p = repo.preview_ignore(&s, &text, &c()).unwrap();
            assert!(p.after().starts_with(before));
            let report = repo.apply_ignore(&p, &c()).unwrap();
            assert!(report.result.is_ok(), "{:?}", report.result);
            assert!(report.published);
            assert_eq!(
                fs::read(report.recovery.unwrap().join("before.gitignore")).unwrap(),
                before
            );
            assert_eq!(fs::read(f.repository.join(".git/index")).unwrap(), index);
            let output = f.run(&[
                "check-ignore",
                "--",
                "[cache]/日本 語.txt",
                "文档/缓存/file.dat",
            ]);
            let output = String::from_utf8(output).unwrap();
            assert!(output.contains("[cache]"));
            assert!(f.repository.join("keep.tmp").exists());
            assert_eq!(
                fs::read_to_string(f.repository.join("tracked.txt")).unwrap(),
                "staged"
            );
        },
    )
}
#[test]
fn templates_append_and_editor_changes_only_the_reviewed_file() {
    isolated(
        "ROCKET_IGNORE",
        "templates_append_and_editor_changes_only_the_reviewed_file",
        || {
            let f = f();
            let repo = f.repo();
            let s = repo.ignore_snapshot(&c()).unwrap();
            assert!(!s.exists());
            let rules: Vec<_> = IgnoreTemplate::Rust
                .rules()
                .iter()
                .map(|s| s.to_string())
                .collect();
            let text = append_ignore_rules(s.text(), &rules).unwrap();
            let p = repo.preview_ignore(&s, &text, &c()).unwrap();
            assert!(repo.apply_ignore(&p, &c()).unwrap().result.is_ok());
            let s = repo.ignore_snapshot(&c()).unwrap();
            let text =
                append_ignore_rules(s.text(), &["tracked.txt".into(), "!target/keep".into()])
                    .unwrap();
            let p = repo.preview_ignore(&s, &text, &c()).unwrap();
            assert!(repo.apply_ignore(&p, &c()).unwrap().result.is_ok());
            assert_eq!(f.run(&["ls-files", "tracked.txt"]), b"tracked.txt\n");
            assert!(matches!(
                repo.preview_ignore(&repo.ignore_snapshot(&c()).unwrap(), &text, &c()),
                Err(Error::Ignore(IgnoreFailure::Unchanged))
            ));
        },
    )
}
#[test]
fn external_file_and_index_changes_refuse_without_overwrite() {
    isolated(
        "ROCKET_IGNORE",
        "external_file_and_index_changes_refuse_without_overwrite",
        || {
            let f = f();
            f.write(".gitignore", "# existing\n");
            let repo = f.repo();
            let s = repo.ignore_snapshot(&c()).unwrap();
            let p = repo.preview_ignore(&s, "# desired\n", &c()).unwrap();
            f.write(".gitignore", "# external\n");
            assert!(matches!(
                repo.apply_ignore(&p, &c()),
                Err(Error::Ignore(IgnoreFailure::Stale))
            ));
            assert_eq!(
                fs::read_to_string(f.repository.join(".gitignore")).unwrap(),
                "# external\n"
            );
            assert!(repo.preview_ignore(&s, "# stale editor\n", &c()).is_err());
            let p = repo
                .preview_ignore(&repo.ignore_snapshot(&c()).unwrap(), "# desired\n", &c())
                .unwrap();
            f.write("tracked.txt", "new staged");
            f.run(&["add", "tracked.txt"]);
            let index = fs::read(f.repository.join(".git/index")).unwrap();
            assert!(matches!(
                repo.apply_ignore(&p, &c()),
                Err(Error::Ignore(IgnoreFailure::Stale))
            ));
            assert_eq!(fs::read(f.repository.join(".git/index")).unwrap(), index);
        },
    )
}
#[test]
fn cancelled_save_preserves_original_and_hardlink_alias() {
    isolated(
        "ROCKET_IGNORE",
        "cancelled_save_preserves_original_and_hardlink_alias",
        || {
            let f = f();
            f.write(".gitignore", "# original\n");
            let alias = f.root.join("alias");
            fs::hard_link(f.repository.join(".gitignore"), &alias).unwrap();
            let repo = f.repo();
            let p = repo
                .preview_ignore(&repo.ignore_snapshot(&c()).unwrap(), "/cache/\n", &c())
                .unwrap();
            let cancel = c();
            cancel.cancel();
            assert!(matches!(
                repo.apply_ignore(&p, &cancel),
                Err(Error::Cancelled)
            ));
            assert_eq!(fs::read_to_string(&alias).unwrap(), "# original\n");
            assert!(repo.apply_ignore(&p, &c()).unwrap().result.is_ok());
            assert_eq!(fs::read_to_string(&alias).unwrap(), "# original\n");
            assert_eq!(
                fs::read_to_string(f.repository.join(".gitignore")).unwrap(),
                "/cache/\n"
            );
        },
    )
}
#[test]
fn special_targets_and_injected_paths_are_refused() {
    isolated(
        "ROCKET_IGNORE",
        "special_targets_and_injected_paths_are_refused",
        || {
            let f = f();
            fs::create_dir(f.repository.join(".gitignore")).unwrap();
            assert!(f.repo().ignore_snapshot(&c()).is_err());
            fs::remove_dir(f.repository.join(".gitignore")).unwrap();
            fs::write(f.repository.join(".gitignore"), b"\xff\xfe\0").unwrap();
            assert!(f.repo().ignore_snapshot(&c()).is_err());
            for name in [".git/config", "x\n/evil", "a\\b"] {
                let p = RepoPath::new(name.as_bytes().to_vec());
                if let Ok(p) = p {
                    assert!(literal_ignore_rule(&p, false).is_err())
                }
            }
            assert!(append_ignore_rules("", &["one\ntwo".into()]).is_err());
        },
    )
}
