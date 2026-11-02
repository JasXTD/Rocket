#![cfg(windows)]
mod common;
use common::{Fixture, isolated};
use rocket_git::*;
use std::fs;
fn case(name: &str, f: fn()) {
    isolated("ROCKET_LFS_TRACKING_CASE", name, f)
}
fn track(pattern: &str, literal: bool) -> LfsAction {
    LfsAction::Track {
        pattern: pattern.into(),
        literal,
    }
}
fn snapshot(f: &Fixture) -> (Vec<u8>, Vec<u8>, Vec<u8>) {
    (
        f.run(&["rev-parse", "HEAD"]),
        fs::read(f.repository.join(".git/index")).unwrap(),
        fs::read(f.repository.join(".git/config")).unwrap(),
    )
}
fn setup() -> Fixture {
    let f = Fixture::new();
    f.write("base.txt", "base\n");
    f.commit("base");
    f
}
fn success(report: LfsWriteReport) -> std::path::PathBuf {
    assert!(report.result.is_ok(), "{:?}", report.result);
    assert!(report.verification.is_ok(), "{:?}", report.verification);
    assert!(report.observed.is_ok(), "{:?}", report.observed);
    assert_eq!(report.completed_steps, 2);
    report.recovery_directory.unwrap()
}
#[test]
fn track_preserves_dirty_attributes_index_and_other_files_with_recovery() {
    case(
        "track_preserves_dirty_attributes_index_and_other_files_with_recovery",
        || {
            let f = setup();
            f.write(
                ".gitattributes",
                "# staged comment\r\n*.txt text eol=crlf\r\n",
            );
            f.run(&["add", ".gitattributes"]);
            let original=b"\xef\xbb\xbf# staged comment\r\n*.txt text eol=crlf\r\n# PRIVATE working edit no final newline";
            f.write(".gitattributes", original);
            f.write("base.txt", "staged\n");
            f.run(&["add", "base.txt"]);
            f.write("base.txt", "unstaged\n");
            f.write("untracked 文", "keep");
            let before = snapshot(&f);
            let c = CancellationToken::new();
            let repo = f.repo();
            let p = repo.preview_lfs(track("*.dat", false), None, &c).unwrap();
            assert_eq!(p.attributes().unwrap().before(), Some(original.as_slice()));
            let recovery = success(repo.apply_lfs(&p, &c).unwrap());
            let expected = [
                original.as_slice(),
                b"\r\n\"*.dat\" filter=lfs diff=lfs merge=lfs -text\r\n",
            ]
            .concat();
            assert_eq!(
                fs::read(f.repository.join(".gitattributes")).unwrap(),
                expected
            );
            for file in ["before.gitattributes", "original.gitattributes"] {
                assert_eq!(fs::read(recovery.join(file)).unwrap(), original);
            }
            assert!(recovery.join("RECOVERY.md").exists());
            assert_eq!(snapshot(&f), before);
            assert_eq!(
                fs::read(f.repository.join("base.txt")).unwrap(),
                b"unstaged\n"
            );
            assert_eq!(
                fs::read(f.repository.join("untracked 文")).unwrap(),
                b"keep"
            );
            assert_eq!(
                f.run(&["check-attr", "filter", "--", "any.dat"]),
                b"any.dat: filter: lfs\n"
            );
            assert!(matches!(
                repo.preview_lfs(track("*.dat", false), None, &c),
                Err(Error::Lfs(LfsFailure::NoChange))
            ));
        },
    );
}
#[test]
fn literal_unicode_brackets_and_root_basename_have_exact_git_semantics() {
    case(
        "literal_unicode_brackets_and_root_basename_have_exact_git_semantics",
        || {
            let f = setup();
            let repo = f.repo();
            let c = CancellationToken::new();
            for name in ["素材/背景 [1] 🚀 é.dat", "[root].dat"] {
                let p = repo.preview_lfs(track(name, true), None, &c).unwrap();
                success(repo.apply_lfs(&p, &c).unwrap());
                let actual = f.run(&[
                    "-c",
                    "core.quotepath=false",
                    "check-attr",
                    "filter",
                    "--",
                    name,
                ]);
                assert_eq!(
                    String::from_utf8(actual).unwrap(),
                    format!("{name}: filter: lfs\n")
                );
            }
            assert_eq!(
                f.run(&["check-attr", "filter", "--", "sub/[root].dat", "root.dat"]),
                b"sub/[root].dat: filter: unspecified\nroot.dat: filter: unspecified\n"
            );
        },
    );
}
#[test]
fn untrack_preserves_other_fields_comments_line_endings_and_overlapping_rules() {
    case(
        "untrack_preserves_other_fields_comments_line_endings_and_overlapping_rules",
        || {
            let f = setup();
            let original = "# keep\r\n  \"素材/*.dat\"\tfilter=lfs  diff=lfs merge=lfs\t-text  export-ignore  custom=keep\r\n*.dat filter=lfs diff=lfs merge=lfs -text\n# tail";
            f.write(".gitattributes", original);
            let repo = f.repo();
            let c = CancellationToken::new();
            let before = snapshot(&f);
            let rules = repo.lfs_status(&c).unwrap().tracking.unwrap();
            assert_eq!(rules.len(), 2);
            let rule = &rules[0];
            let p = repo
                .preview_lfs(
                    LfsAction::Untrack {
                        line: rule.line,
                        expected: rule.text.clone(),
                    },
                    None,
                    &c,
                )
                .unwrap();
            assert_eq!(
                p.attributes().unwrap().added(),
                "  \"素材/*.dat\"  export-ignore  custom=keep"
            );
            success(repo.apply_lfs(&p, &c).unwrap());
            assert_eq!(
                fs::read_to_string(f.repository.join(".gitattributes")).unwrap(),
                "# keep\r\n  \"素材/*.dat\"  export-ignore  custom=keep\r\n*.dat filter=lfs diff=lfs merge=lfs -text\n# tail"
            );
            let rule = &rules[1];
            let p = repo
                .preview_lfs(
                    LfsAction::Untrack {
                        line: rule.line,
                        expected: rule.text.clone(),
                    },
                    None,
                    &c,
                )
                .unwrap();
            success(repo.apply_lfs(&p, &c).unwrap());
            assert_eq!(
                fs::read_to_string(f.repository.join(".gitattributes")).unwrap(),
                "# keep\r\n  \"素材/*.dat\"  export-ignore  custom=keep\r\n# tail"
            );
            assert_eq!(snapshot(&f), before);
        },
    );
}
#[test]
fn external_attribute_index_and_config_edits_reject_stale_reviews() {
    case(
        "external_attribute_index_and_config_edits_reject_stale_reviews",
        || {
            for mode in ["attributes", "index", "config", "arrival"] {
                let f = setup();
                let repo = f.repo();
                let c = CancellationToken::new();
                if mode != "arrival" {
                    f.write(".gitattributes", "# original\n");
                }
                let p = repo.preview_lfs(track("*.dat", false), None, &c).unwrap();
                match mode {
                    "attributes" | "arrival" => f.write(".gitattributes", "# external edit\n"),
                    "index" => {
                        f.write("base.txt", "external staged");
                        f.run(&["add", "base.txt"]);
                    }
                    _ => {
                        f.run(&["config", "rocket.external", "keep"]);
                    }
                }
                let before = snapshot(&f);
                let attrs = fs::read(f.repository.join(".gitattributes")).unwrap();
                let r = repo.apply_lfs(&p, &c).unwrap();
                assert!(
                    matches!(r.result, Err(Error::Lfs(LfsFailure::Stale))),
                    "{:?}",
                    r.result
                );
                assert_eq!(r.completed_steps, 0);
                assert!(r.recovery_directory.is_none());
                assert_eq!(snapshot(&f), before);
                assert_eq!(
                    fs::read(f.repository.join(".gitattributes")).unwrap(),
                    attrs
                );
            }
        },
    );
}
#[test]
fn cancellation_before_preview_or_apply_never_changes_files() {
    case(
        "cancellation_before_preview_or_apply_never_changes_files",
        || {
            let f = setup();
            f.write(".gitattributes", "# keep\n");
            let repo = f.repo();
            let c = CancellationToken::new();
            let p = repo.preview_lfs(track("*.dat", false), None, &c).unwrap();
            let before = snapshot(&f);
            c.cancel();
            assert!(matches!(
                repo.preview_lfs(track("*.png", false), None, &c),
                Err(Error::Cancelled)
            ));
            assert!(matches!(repo.apply_lfs(&p, &c), Err(Error::Cancelled)));
            assert_eq!(snapshot(&f), before);
            assert_eq!(
                fs::read(f.repository.join(".gitattributes")).unwrap(),
                b"# keep\n"
            );
        },
    );
}
#[test]
fn unsafe_attributes_and_patterns_are_refused_without_mutation() {
    case(
        "unsafe_attributes_and_patterns_are_refused_without_mutation",
        || {
            let f = setup();
            let repo = f.repo();
            let c = CancellationToken::new();
            for pattern in [
                "",
                "../escape",
                "/absolute",
                "C:/absolute",
                "*.dat\n* text",
                "!*.dat",
                "[attr]macro",
                "dir/",
                "a\\b",
                "#comment",
                " trailing ",
            ] {
                assert!(
                    matches!(
                        repo.preview_lfs(track(pattern, false), None, &c),
                        Err(Error::Lfs(LfsFailure::Pattern))
                    ),
                    "{pattern}"
                );
            }
            for data in [
                vec![255],
                vec![0],
                b"\"unclosed filter=lfs\n".to_vec(),
                vec![b'x'; 1024 * 1024 + 1],
            ] {
                f.write(".gitattributes", &data);
                assert!(matches!(
                    repo.preview_lfs(track("*.dat", false), None, &c),
                    Err(Error::Lfs(LfsFailure::Attributes))
                ));
                assert_eq!(fs::read(f.repository.join(".gitattributes")).unwrap(), data);
            }
            f.write(".gitattributes", "# keep\n");
            let target = f.repository.join(".gitattributes");
            let perms = fs::metadata(&target).unwrap().permissions();
            let mut readonly = perms.clone();
            readonly.set_readonly(true);
            fs::set_permissions(&target, readonly).unwrap();
            assert!(matches!(
                repo.preview_lfs(track("*.dat", false), None, &c),
                Err(Error::Lfs(LfsFailure::ReadOnly))
            ));
            fs::set_permissions(&target, perms).unwrap();
            f.run(&["add", ".gitattributes"]);
            f.run(&["update-index", "--skip-worktree", ".gitattributes"]);
            assert!(matches!(
                repo.preview_lfs(track("*.dat", false), None, &c),
                Err(Error::Lfs(LfsFailure::Attributes))
            ));
            f.run(&["update-index", "--no-skip-worktree", ".gitattributes"]);
            fs::remove_file(&target).unwrap();
            assert!(matches!(
                repo.preview_lfs(track("*.dat", false), None, &c),
                Err(Error::Lfs(LfsFailure::Attributes))
            ));
        },
    );
}
#[test]
fn hardlink_target_is_replaced_without_editing_other_link() {
    case(
        "hardlink_target_is_replaced_without_editing_other_link",
        || {
            let f = setup();
            f.write(".gitattributes", "# keep\n");
            let other = f.root.join("outside-attributes");
            fs::hard_link(f.repository.join(".gitattributes"), &other).unwrap();
            let repo = f.repo();
            let c = CancellationToken::new();
            let p = repo.preview_lfs(track("*.dat", false), None, &c).unwrap();
            success(repo.apply_lfs(&p, &c).unwrap());
            assert_eq!(fs::read(&other).unwrap(), b"# keep\n");
        },
    );
}
#[test]
fn initial_repository_can_create_attributes_and_stale_rule_selection_is_refused() {
    case(
        "initial_repository_can_create_attributes_and_stale_rule_selection_is_refused",
        || {
            let f = Fixture::new();
            let repo = f.repo();
            let c = CancellationToken::new();
            let p = repo.preview_lfs(track("*.dat", false), None, &c).unwrap();
            let recovery = success(repo.apply_lfs(&p, &c).unwrap());
            assert!(!recovery.join("before.gitattributes").exists());
            assert!(!f.repository.join(".git/index").exists());
            let rule = repo.lfs_status(&c).unwrap().tracking.unwrap().remove(0);
            f.write(
                ".gitattributes",
                "# inserted\n\"*.dat\" filter=lfs diff=lfs merge=lfs -text\n",
            );
            assert!(matches!(
                repo.preview_lfs(
                    LfsAction::Untrack {
                        line: rule.line,
                        expected: rule.text
                    },
                    None,
                    &c
                ),
                Err(Error::Lfs(LfsFailure::Stale))
            ));
        },
    );
}
