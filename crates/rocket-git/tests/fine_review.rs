mod common;
use common::{Fixture, isolated};
use rocket_git::*;
use std::fs;
fn c() -> CancellationToken {
    CancellationToken::new()
}
fn p(name: &str) -> RepoPath {
    RepoPath::new(name.as_bytes().to_vec()).unwrap()
}
fn line(r: &ChangeReview, text: &[u8]) -> usize {
    r.patch()
        .split(|b| *b == b'\n')
        .position(|l| l == text)
        .unwrap()
}
fn apply(r: &Repository, p: &ChangeReview, lines: &[usize]) {
    let out = r.apply_review(p, lines, &c()).unwrap();
    out.result.unwrap();
    out.status.unwrap();
}

#[test]
fn new_file_line_staging_respects_safecrlf_and_rechecks_changes() {
    let f = Fixture::new();
    f.write("base", "base\n");
    f.commit("base");
    f.run(&["config", "core.autocrlf", "true"]);
    f.run(&["config", "core.safecrlf", "warn"]);
    f.write("mixed.txt", b"first\r\nsecond\nthird\r\n");
    let repo = f.repo();
    let name = p("mixed.txt");
    let old = repo.review_untracked(&name, &c()).unwrap();
    assert!(old.selection_unavailable().is_none());
    f.run(&["config", "core.safecrlf", "true"]);
    assert!(!f.output(&f.repository, &["add", "--", "mixed.txt"]).status.success());
    let blocked = repo.review_untracked(&name, &c()).unwrap();
    assert!(blocked.selection_unavailable().is_some_and(|s| s.contains("safecrlf")));
    assert!(repo.apply_review(&old, &old.changed_lines()[..2], &c()).is_err());
    assert!(f.run(&["ls-files", "--", "mixed.txt"]).is_empty());
    f.write("mixed.txt", b"first\r\nsecond\r\n");
    let allowed = repo.review_untracked(&name, &c()).unwrap();
    apply(&repo, &allowed, &[line(&allowed, b"+second")]);
    assert_eq!(f.run(&["show", ":mixed.txt"]), b"second\n");

    f.run(&["config", "core.autocrlf", "input"]);
    f.write("input.txt", b"line\r\n");
    assert!(!f.output(&f.repository, &["add", "--", "input.txt"]).status.success());
    assert!(repo.review_untracked(&p("input.txt"), &c()).unwrap().selection_unavailable().is_some());
}

#[test]
fn valueless_autocrlf_stages_the_same_blob_as_git() {
    use std::io::Write;
    let f = Fixture::new();
    f.write("base", "base\n");
    f.commit("base");
    let mut config = fs::OpenOptions::new().append(true).open(f.repository.join(".git/config")).unwrap();
    writeln!(config, "\n[core]\n\tautocrlf").unwrap();
    drop(config);
    assert_eq!(f.run(&["config", "--bool", "--get", "core.autocrlf"]), b"true\n");
    f.write("native.txt", b"first\r\nsecond\r\n");
    f.write("partial.txt", b"first\r\nsecond\r\n");
    f.run(&["add", "--", "native.txt"]);
    let repo = f.repo();
    let review = repo.review_untracked(&p("partial.txt"), &c()).unwrap();
    apply(&repo, &review, &review.changed_lines());
    assert_eq!(f.run(&["show", ":partial.txt"]), f.run(&["show", ":native.txt"]));
    // An explicitly empty value overrides the earlier, implicit true value.
    f.run(&["config", "--add", "core.autocrlf", ""]);
    f.write("literal.txt", b"first\r\n");
    let literal = repo.review_untracked(&p("literal.txt"), &c()).unwrap();
    apply(&repo, &literal, &literal.changed_lines());
    assert_eq!(f.run(&["show", ":literal.txt"]), b"first\r\n");
}

#[test]
fn legacy_crlf_partial_staging_matches_git_add_and_modern_text_takes_precedence() {
    let f = Fixture::new();
    f.run(&["config", "core.autocrlf", "true"]);
    f.write(
        ".gitattributes",
        "*.raw -crlf\n*.input crlf=input\n*.modern -crlf text\n",
    );
    f.commit("attributes");
    let repo = f.repo();
    for name in ["new.raw", "new.input", "new.modern"] {
        f.write(name, b"first\r\nsecond\r\n");
        let review = repo.review_untracked(&p(name), &c()).unwrap();
        assert_eq!(review.selection_unavailable(), None);
        apply(&repo, &review, &review.changed_lines());
        let partial = f.run(&["show", &format!(":{name}")]);
        f.run(&["rm", "--cached", "-f", "--", name]);
        f.run(&["add", "--", name]);
        assert_eq!(partial, f.run(&["show", &format!(":{name}")]), "{name}");
    }
}

#[test]
fn new_file_lines_round_trip_and_never_touch_working_files() {
    isolated(
        "ROCKET_FINE_CASE",
        "new_file_lines_round_trip_and_never_touch_working_files",
        || {
            let f = Fixture::new();
            f.write("keep.txt", "base\n");
            f.commit("base");
            f.write("keep.txt", "staged unrelated\n");
            f.run(&["add", "keep.txt"]);
            let name = "new 解析 🚀.dart";
            let bytes = "first\nsecond 世界\nthird\n";
            f.write(name, bytes);
            let r = f.repo();
            let before = fs::read(f.repository.join(".git/index")).unwrap();
            let review = r.review_untracked(&p(name), &c()).unwrap();
            assert_eq!(review.selection_unavailable(), None);
            assert_eq!(fs::read(f.repository.join(".git/index")).unwrap(), before);
            apply(&r, &review, &[line(&review, "+second 世界".as_bytes())]);
            assert_eq!(
                f.run(&["show", &format!(":{name}")]),
                "second 世界\n".as_bytes()
            );
            let remaining = r.review_change(&p(name), false, &c()).unwrap();
            apply(&r, &remaining, &remaining.changed_lines());
            assert_eq!(f.run(&["show", &format!(":{name}")]), bytes.as_bytes());
            let staged = r.review_change(&p(name), true, &c()).unwrap();
            apply(&r, &staged, &[line(&staged, "+second 世界".as_bytes())]);
            assert_eq!(f.run(&["show", &format!(":{name}")]), b"first\nthird\n");
            let staged = r.review_change(&p(name), true, &c()).unwrap();
            apply(&r, &staged, &staged.changed_lines());
            assert!(f.run(&["ls-files", "--", name]).is_empty());
            assert_eq!(fs::read(f.repository.join(name)).unwrap(), bytes.as_bytes());
            assert_eq!(f.run(&["show", ":keep.txt"]), b"staged unrelated\n");
        },
    );
}

#[test]
fn deleted_file_partial_stage_and_unstage_keep_worktree_deleted() {
    isolated(
        "ROCKET_FINE_CASE",
        "deleted_file_partial_stage_and_unstage_keep_worktree_deleted",
        || {
            let f = Fixture::new();
            let name = "消えた.txt";
            f.write(name, "one\ntwo\nthree\n");
            f.commit("base");
            fs::remove_file(f.repository.join(name)).unwrap();
            let r = f.repo();
            let review = r.review_change(&p(name), false, &c()).unwrap();
            assert_eq!(review.selection_unavailable(), None);
            apply(&r, &review, &[line(&review, b"-two")]);
            assert_eq!(f.run(&["show", &format!(":{name}")]), b"one\nthree\n");
            let review = r.review_change(&p(name), false, &c()).unwrap();
            apply(&r, &review, &review.changed_lines());
            assert!(f.run(&["ls-files", "--", name]).is_empty());
            let review = r.review_change(&p(name), true, &c()).unwrap();
            apply(&r, &review, &[line(&review, b"-two")]);
            assert_eq!(f.run(&["show", &format!(":{name}")]), b"two\n");
            let review = r.review_change(&p(name), true, &c()).unwrap();
            apply(&r, &review, &review.changed_lines());
            assert_eq!(f.run(&["show", &format!(":{name}")]), b"one\ntwo\nthree\n");
            assert!(!f.repository.join(name).exists());
        },
    );
}

#[test]
fn renamed_file_unstages_text_at_new_path_and_detects_index_changes() {
    isolated(
        "ROCKET_FINE_CASE",
        "renamed_file_unstages_text_at_new_path_and_detects_index_changes",
        || {
            let f = Fixture::new();
            let old = "旧名.rs";
            let new = "新名.rs";
            let base = (0..30).map(|i| format!("line {i}\n")).collect::<String>();
            f.write(old, &base);
            f.commit("base");
            f.run(&["mv", old, new]);
            let edited = base.replace("line 5\n", "changed five\n");
            f.write(new, &edited);
            f.run(&["add", new]);
            let r = f.repo();
            let review = r.review_change(&p(new), true, &c()).unwrap();
            assert_eq!(review.selection_unavailable(), None);
            assert!(
                review
                    .selection_note()
                    .unwrap()
                    .contains("rename stays staged")
            );
            assert!(!review.supports_worktree_selection());
            apply(&r, &review, &review.changed_lines());
            assert_eq!(f.run(&["show", &format!(":{new}")]), base.as_bytes());
            assert!(f.run(&["ls-files", "--", old]).is_empty());
            assert_eq!(fs::read(f.repository.join(new)).unwrap(), edited.as_bytes());
            assert!(
                r.apply_review(&review, &review.changed_lines(), &c())
                    .is_err()
            );
            assert!(
                r.preview_discard_selection(&review, &review.changed_lines(), &c())
                    .is_err()
            );
        },
    );
}

#[test]
fn new_file_cancel_stale_ignore_and_filters_cannot_stage() {
    isolated(
        "ROCKET_FINE_CASE",
        "new_file_cancel_stale_ignore_and_filters_cannot_stage",
        || {
            let f = Fixture::new();
            f.write("keep.txt", "base\n");
            f.commit("base");
            f.write("new.txt", "one\ntwo\n");
            let r = f.repo();
            let review = r.review_untracked(&p("new.txt"), &c()).unwrap();
            let before = fs::read(f.repository.join(".git/index")).unwrap();
            let stop = c();
            stop.cancel();
            assert!(matches!(
                r.apply_review(&review, &review.changed_lines(), &stop),
                Err(Error::Cancelled)
            ));
            f.write("new.txt", "external\n");
            assert!(
                r.apply_review(&review, &review.changed_lines(), &c())
                    .is_err()
            );
            assert_eq!(fs::read(f.repository.join(".git/index")).unwrap(), before);
            f.write(".gitignore", "new.txt\n");
            assert!(r.review_untracked(&p("new.txt"), &c()).is_err());
            f.write(".gitignore", "");
            f.write(".gitattributes", "new.txt filter=unsafe\n");
            f.run(&["config", "filter.unsafe.clean", "touch FILTER_EXECUTED"]);
            let blocked = r.review_untracked(&p("new.txt"), &c()).unwrap();
            assert!(blocked.selection_unavailable().unwrap().contains("filter"));
            assert!(r.apply_review(&blocked, &[5], &c()).is_err());
            assert!(!f.repository.join("FILTER_EXECUTED").exists());
            assert_eq!(fs::read(f.repository.join(".git/index")).unwrap(), before);
        },
    );
}

#[test]
fn new_file_crlf_matches_git_and_conversion_edits_invalidate_review() {
    isolated(
        "ROCKET_FINE_CASE",
        "new_file_crlf_matches_git_and_conversion_edits_invalidate_review",
        || {
            let f = Fixture::new();
            f.write("keep.txt", "base\n");
            f.commit("base");
            f.run(&["config", "core.autocrlf", "true"]);
            f.write("new.txt", "one\r\ntwo\r\n");
            let r = f.repo();
            let review = r.review_untracked(&p("new.txt"), &c()).unwrap();
            assert_eq!(review.selection_unavailable(), None);
            apply(&r, &review, &[line(&review, b"+two")]);
            assert_eq!(f.run(&["show", ":new.txt"]), b"two\n");
            assert_eq!(
                fs::read(f.repository.join("new.txt")).unwrap(),
                b"one\r\ntwo\r\n"
            );
            f.write("another.txt", "x\n");
            let review = r.review_untracked(&p("another.txt"), &c()).unwrap();
            f.write(".gitattributes", "another.txt -text\n");
            assert!(
                r.apply_review(&review, &review.changed_lines(), &c())
                    .is_err()
            );
        },
    );
}
