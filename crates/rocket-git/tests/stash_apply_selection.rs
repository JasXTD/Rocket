mod common;
use common::{Fixture, isolated};
use rocket_git::*;
use std::fs;
fn c() -> CancellationToken {
    CancellationToken::new()
}
fn setup() -> Fixture {
    let f = Fixture::new();
    f.write(
        "解析 🚀.rs",
        (1..=30).map(|n| format!("line {n}\n")).collect::<String>(),
    );
    f.write("other.md", "base\n");
    f.write("keep.md", "base\n");
    f.commit("base");
    let text = fs::read_to_string(f.repository.join("解析 🚀.rs"))
        .unwrap()
        .replace("line 2\n", "first change\n")
        .replace("line 27\n", "second change\n");
    f.write("解析 🚀.rs", &text);
    f.write("other.md", "saved\n");
    f.run(&["add", "other.md"]);
    f.write("未追跡.txt", "saved untracked\n");
    f.run(&["stash", "push", "-u", "-m", "selection test"]);
    f
}
fn review(f: &Fixture, part: StashPart) -> StashApplyReview {
    let r = f.repo();
    r.review_stash_apply(&r.stashes(&c()).unwrap()[0], part, &c())
        .unwrap()
}
fn choose(r: &StashApplyReview, name: &str, hunks: Vec<usize>) -> StashApplySelection {
    StashApplySelection {
        file: r
            .files()
            .iter()
            .position(|f| f.path().as_bytes() == name.as_bytes())
            .unwrap(),
        hunks,
        lines: vec![],
    }
}
#[test]
fn hunks_and_whole_files_keep_stash_index_and_unrelated_edits() {
    isolated(
        "ROCKET_STASH_APPLY_CASE",
        "hunks_and_whole_files_keep_stash_index_and_unrelated_edits",
        || {
            let f = setup();
            let r = f.repo();
            f.write("keep.md", "staged\n");
            f.run(&["add", "keep.md"]);
            f.write("keep.md", "working\n");
            let index = fs::read(f.repository.join(".git/index")).unwrap();
            let stashes = r.stashes(&c()).unwrap();
            let rev = review(&f, StashPart::Tracked);
            let s = choose(&rev, "解析 🚀.rs", vec![1]);
            assert_eq!(rev.files()[s.file].hunk_count(), 2);
            let p = r
                .preview_stash_apply(&rev, &[s, choose(&rev, "other.md", vec![])], &c())
                .unwrap();
            let out = r.apply_stash_selection(&p, &c()).unwrap();
            assert!(out.result.is_ok(), "{:?}", out.result);
            assert!(out.verification.is_ok());
            assert_eq!(out.changed.len(), 2);
            assert_eq!(out.recovery.len(), 2);
            let text = fs::read_to_string(f.repository.join("解析 🚀.rs")).unwrap();
            assert!(text.contains("line 2\n"));
            assert!(text.contains("second change\n"));
            assert!(!text.contains("first change"));
            assert_eq!(fs::read(f.repository.join("other.md")).unwrap(), b"saved\n");
            assert_eq!(
                fs::read(f.repository.join("keep.md")).unwrap(),
                b"working\n"
            );
            assert_eq!(fs::read(f.repository.join(".git/index")).unwrap(), index);
            assert_eq!(r.stashes(&c()).unwrap(), stashes);
        },
    );
}
#[test]
fn saved_index_untracked_and_reordered_stash_refs_are_pinned() {
    isolated(
        "ROCKET_STASH_APPLY_CASE",
        "saved_index_untracked_and_reordered_stash_refs_are_pinned",
        || {
            let f = setup();
            let r = f.repo();
            let rev = review(&f, StashPart::Untracked);
            let p = r
                .preview_stash_apply(&rev, &[choose(&rev, "未追跡.txt", vec![])], &c())
                .unwrap();
            f.write("other.md", "another stash\n");
            f.run(&["stash", "push", "-m", "newer"]);
            let out = r.apply_stash_selection(&p, &c()).unwrap();
            assert!(out.result.is_ok(), "{:?}", out.result);
            assert_eq!(
                fs::read(f.repository.join("未追跡.txt")).unwrap(),
                b"saved untracked\n"
            );
            assert_eq!(r.stashes(&c()).unwrap().len(), 2);
            let original = r.stashes(&c()).unwrap()[1].clone();
            let rev = r
                .review_stash_apply(&original, StashPart::Index, &c())
                .unwrap();
            let p = r
                .preview_stash_apply(&rev, &[choose(&rev, "other.md", vec![])], &c())
                .unwrap();
            assert!(r.apply_stash_selection(&p, &c()).unwrap().result.is_ok());
            assert!(r.status(&c()).unwrap().entries.iter().all(|e| !e.staged()));
        },
    );
}
#[test]
fn whole_saved_rename_delete_and_binary_are_backed_up() {
    isolated(
        "ROCKET_STASH_APPLY_CASE",
        "whole_saved_rename_delete_and_binary_are_backed_up",
        || {
            let f = Fixture::new();
            f.write("old.txt", "rename content\n");
            f.write("gone.txt", "delete content\n");
            f.write("binary.bin", [0, 1, 2, 3, 4]);
            f.commit("base");
            f.run(&["mv", "old.txt", "新.txt"]);
            f.run(&["rm", "gone.txt"]);
            f.write("binary.bin", [0, 9, 8, 7, 6]);
            f.run(&["stash", "push", "-m", "whole files"]);
            let r = f.repo();
            let rev = review(&f, StashPart::Tracked);
            let selected: Vec<_> = (0..rev.files().len())
                .map(|file| StashApplySelection {
                    file,
                    hunks: vec![],
                    lines: vec![],
                })
                .collect();
            let index = fs::read(f.repository.join(".git/index")).unwrap();
            let p = r.preview_stash_apply(&rev, &selected, &c()).unwrap();
            let out = r.apply_stash_selection(&p, &c()).unwrap();
            assert!(out.result.is_ok(), "{:?}", out.result);
            assert!(out.verification.is_ok());
            assert!(!f.repository.join("old.txt").exists());
            assert!(!f.repository.join("gone.txt").exists());
            assert_eq!(
                fs::read(f.repository.join("新.txt")).unwrap(),
                b"rename content\n"
            );
            assert_eq!(
                fs::read(f.repository.join("binary.bin")).unwrap(),
                [0, 9, 8, 7, 6]
            );
            assert_eq!(fs::read(f.repository.join(".git/index")).unwrap(), index);
            assert_eq!(r.stashes(&c()).unwrap().len(), 1);
        },
    );
}
#[test]
fn conflicts_stale_files_head_cancel_and_invalid_selection_preserve_data() {
    isolated(
        "ROCKET_STASH_APPLY_CASE",
        "conflicts_stale_files_head_cancel_and_invalid_selection_preserve_data",
        || {
            let f = setup();
            let r = f.repo();
            let rev = review(&f, StashPart::Tracked);
            let sel = choose(&rev, "other.md", vec![]);
            assert!(r.preview_stash_apply(&rev, &[], &c()).is_err());
            assert!(
                r.preview_stash_apply(&rev, &[choose(&rev, "other.md", vec![99])], &c())
                    .is_err()
            );
            let p = r.preview_stash_apply(&rev, std::slice::from_ref(&sel), &c()).unwrap();
            let stop = c();
            stop.cancel();
            assert!(matches!(
                r.apply_stash_selection(&p, &stop),
                Err(Error::Cancelled)
            ));
            f.write("other.md", "external\n");
            assert!(r.apply_stash_selection(&p, &c()).is_err());
            assert!(r.preview_stash_apply(&rev, std::slice::from_ref(&sel), &c()).is_err());
            assert_eq!(
                fs::read(f.repository.join("other.md")).unwrap(),
                b"external\n"
            );
            f.write("other.md", "base\n");
            let p = r.preview_stash_apply(&rev, &[sel], &c()).unwrap();
            f.run(&["commit", "--allow-empty", "-m", "changed head"]);
            assert!(r.apply_stash_selection(&p, &c()).is_err());
            assert_eq!(fs::read(f.repository.join("other.md")).unwrap(), b"base\n");
            assert_eq!(r.stashes(&c()).unwrap().len(), 1);
        },
    );
}

#[test]
fn individual_saved_lines_are_pinned_backed_up_and_preserve_index_and_stash() {
    isolated(
        "ROCKET_STASH_APPLY_CASE",
        "individual_saved_lines_are_pinned_backed_up_and_preserve_index_and_stash",
        || {
            let f = Fixture::new();
            let name = "解析 🚀.rs";
            f.write(name, "before\nold\nafter\n");
            f.write("other.txt", "base\n");
            f.commit("base");
            f.write(name, "before\nnew\nextra\nafter\n");
            f.run(&["stash", "push", "-m", "saved lines"]);
            f.write("other.txt", "unrelated staged\n");
            f.run(&["add", "other.txt"]);
            let r = f.repo();
            let rev = review(&f, StashPart::Tracked);
            let file = &rev.files()[0];
            let id = |text: &[u8]| {
                file.patch()
                    .split(|b| *b == b'\n')
                    .position(|l| l == text)
                    .unwrap()
            };
            let selection = StashApplySelection {
                file: 0,
                hunks: vec![],
                lines: vec![id(b"-old"), id(b"+new")],
            };
            let index = fs::read(f.repository.join(".git/index")).unwrap();
            let stash = f.run(&["rev-parse", "refs/stash"]);
            for invalid in [vec![0], vec![id(b"+new"), id(b"+new")]] {
                assert!(
                    r.preview_stash_apply(
                        &rev,
                        &[StashApplySelection {
                            file: 0,
                            hunks: vec![],
                            lines: invalid
                        }],
                        &c()
                    )
                    .is_err()
                );
            }
            assert!(
                r.preview_stash_apply(
                    &rev,
                    &[StashApplySelection {
                        file: 0,
                        hunks: vec![0],
                        lines: vec![id(b"+new")]
                    }],
                    &c()
                )
                .is_err()
            );
            let p = r
                .preview_stash_apply(&rev, std::slice::from_ref(&selection), &c())
                .unwrap();
            let stop = c();
            stop.cancel();
            assert!(matches!(
                r.apply_stash_selection(&p, &stop),
                Err(Error::Cancelled)
            ));
            f.write(name, "external\n");
            assert!(r.apply_stash_selection(&p, &c()).is_err());
            assert_eq!(fs::read(f.repository.join(name)).unwrap(), b"external\n");
            f.write(name, "before\nold\nafter\n");
            let p = r.preview_stash_apply(&rev, &[selection], &c()).unwrap();
            let out = r.apply_stash_selection(&p, &c()).unwrap();
            assert!(out.result.is_ok(), "{:?}", out.result);
            assert!(out.verification.is_ok());
            assert_eq!(
                fs::read(f.repository.join(name)).unwrap(),
                b"before\nnew\nafter\n"
            );
            assert_eq!(fs::read(f.repository.join(".git/index")).unwrap(), index);
            assert_eq!(f.run(&["rev-parse", "refs/stash"]), stash);
        },
    );
}
