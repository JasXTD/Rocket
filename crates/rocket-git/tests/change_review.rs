mod common;
use common::Fixture;
use rocket_git::*;
use std::fs;

fn token() -> CancellationToken {
    CancellationToken::new()
}
fn path() -> RepoPath {
    RepoPath::new(b"review.txt".to_vec()).unwrap()
}
fn selected(review: &ChangeReview, text: &[u8]) -> usize {
    review
        .patch()
        .split(|b| *b == b'\n')
        .position(|line| line == text)
        .unwrap()
}
fn apply(repo: &Repository, review: &ChangeReview, lines: &[usize]) {
    let report = repo.apply_review(review, lines, &token()).unwrap();
    report.result.unwrap();
    report.status.unwrap();
}

#[test]
fn conflict_previews_are_exact_and_mark_resolved_rejects_leftover_markers() {
    let fixture = Fixture::new();
    fixture.write("review.txt", "base\n");
    fixture.commit("base");
    fixture.run(&["checkout", "-b", "other"]);
    fixture.write("review.txt", "theirs\n");
    fixture.commit("theirs");
    fixture.run(&["checkout", "main"]);
    fixture.write("review.txt", "ours\n");
    fixture.commit("ours");
    assert!(
        !fixture
            .output(&fixture.repository, &["merge", "other"])
            .status
            .success()
    );
    let repo = fixture.repo();
    for (stage, bytes) in [(1, b"base\n".as_slice()), (2, b"ours\n"), (3, b"theirs\n")] {
        assert_eq!(
            repo.conflict_preview(&path(), stage, &token())
                .unwrap()
                .bytes,
            bytes
        );
    }
    assert!(
        repo.mark_resolved(&path(), &token())
            .unwrap_err()
            .to_string()
            .contains("markers remain")
    );
    assert!(!fixture.run(&["ls-files", "--unmerged"]).is_empty());
    fixture.write("review.txt", "resolved together\n");
    let report = repo.mark_resolved(&path(), &token()).unwrap();
    report.result.unwrap();
    assert!(fixture.run(&["ls-files", "--unmerged"]).is_empty());
    assert_eq!(
        fixture.run(&["show", ":review.txt"]),
        b"resolved together\n"
    );
    assert_eq!(
        fs::read(fixture.repository.join("review.txt")).unwrap(),
        b"resolved together\n"
    );
    assert!(fixture.repository.join(".git/MERGE_HEAD").is_file());
}

#[test]
fn line_and_hunk_selection_round_trip_without_changing_worktree_or_other_index_entries() {
    let fixture = Fixture::new();
    fixture.write(
        "review.txt",
        "one\ntwo\nthree\nfour\nfive\nsix\nseven\neight\nnine\nten\neleven\ntwelve\n",
    );
    fixture.write("other.txt", "base\n");
    fixture.commit("base");
    let working =
        b"one\nTWO\nextra\nthree\nfour\nfive\nsix\nseven\neight\nnine\nten\nELEVEN\ntwelve\n";
    fs::write(fixture.repository.join("review.txt"), working).unwrap();
    fixture.write("other.txt", "preserve staged\n");
    fixture.run(&["add", "other.txt"]);
    let repo = fixture.repo();
    let review = repo.review_change(&path(), false, &token()).unwrap();
    assert_eq!(review.selection_unavailable(), None);
    apply(
        &repo,
        &review,
        &[
            selected(&review, b"-two"),
            selected(&review, b"+TWO"),
            selected(&review, b"+ELEVEN"),
            selected(&review, b"-eleven"),
        ],
    );
    assert_eq!(
        fixture.run(&["show", ":review.txt"]),
        b"one\nTWO\nthree\nfour\nfive\nsix\nseven\neight\nnine\nten\nELEVEN\ntwelve\n"
    );
    let staged = repo.review_change(&path(), true, &token()).unwrap();
    apply(
        &repo,
        &staged,
        &[selected(&staged, b"-two"), selected(&staged, b"+TWO")],
    );
    assert_eq!(
        fixture.run(&["show", ":review.txt"]),
        b"one\ntwo\nthree\nfour\nfive\nsix\nseven\neight\nnine\nten\nELEVEN\ntwelve\n"
    );
    let staged = repo.review_change(&path(), true, &token()).unwrap();
    apply(&repo, &staged, &staged.changed_lines());
    assert_eq!(
        fixture.run(&["show", ":review.txt"]),
        fixture.run(&["show", "HEAD:review.txt"])
    );
    assert_eq!(
        fs::read(fixture.repository.join("review.txt")).unwrap(),
        working
    );
    assert_eq!(fixture.run(&["show", ":other.txt"]), b"preserve staged\n");
}

#[test]
fn selected_addition_or_removal_in_a_replacement_is_independent_and_preserves_raw_bytes() {
    let fixture = Fixture::new();
    fixture.run(&["config", "core.autocrlf", "false"]);
    fs::write(
        fixture.repository.join("review.txt"),
        b"before\r\nold\xff\r\nafter\r\n",
    )
    .unwrap();
    fixture.commit("base");
    let working = b"before\r\nnew\xfe\r\nafter\r\n";
    fs::write(fixture.repository.join("review.txt"), working).unwrap();
    let repo = fixture.repo();
    let review = repo.review_change(&path(), false, &token()).unwrap();
    apply(&repo, &review, &[selected(&review, b"+new\xfe\r")]);
    assert_eq!(
        fixture.run(&["show", ":review.txt"]),
        b"before\r\nold\xff\r\nnew\xfe\r\nafter\r\n"
    );
    let staged = repo.review_change(&path(), true, &token()).unwrap();
    apply(&repo, &staged, &staged.changed_lines());
    assert_eq!(
        fixture.run(&["show", ":review.txt"]),
        b"before\r\nold\xff\r\nafter\r\n"
    );
    let review = repo.review_change(&path(), false, &token()).unwrap();
    apply(&repo, &review, &[selected(&review, b"-old\xff\r")]);
    assert_eq!(
        fixture.run(&["show", ":review.txt"]),
        b"before\r\nafter\r\n"
    );
    assert_eq!(
        fs::read(fixture.repository.join("review.txt")).unwrap(),
        working
    );
}

#[test]
fn stale_reviews_invalid_selections_cancellation_and_other_repositories_cannot_write() {
    let fixture = Fixture::new();
    fixture.write("review.txt", "base\n");
    fixture.commit("base");
    fixture.write("review.txt", "changed\n");
    let repo = fixture.repo();
    let review = repo.review_change(&path(), false, &token()).unwrap();
    let index = fs::read(fixture.repository.join(".git/index")).unwrap();
    assert!(repo.apply_review(&review, &[0], &token()).is_err());
    assert!(repo.apply_review(&review, &[], &token()).is_err());
    let cancelled = token();
    cancelled.cancel();
    assert!(matches!(
        repo.apply_review(&review, &review.changed_lines(), &cancelled),
        Err(Error::Cancelled)
    ));
    let other = Fixture::new();
    assert!(
        other
            .repo()
            .apply_review(&review, &review.changed_lines(), &token())
            .is_err()
    );
    fixture.write("review.txt", "edited after review\n");
    let error = repo
        .apply_review(&review, &review.changed_lines(), &token())
        .unwrap_err();
    assert!(error.to_string().contains("changed since"));
    assert_eq!(
        fs::read(fixture.repository.join(".git/index")).unwrap(),
        index
    );
    fixture.run(&["add", "review.txt"]);
    let staged = repo.review_change(&path(), true, &token()).unwrap();
    fixture.write("review.txt", "another index version\n");
    fixture.run(&["add", "review.txt"]);
    assert!(
        repo.apply_review(&staged, &staged.changed_lines(), &token())
            .is_err()
    );
    assert_eq!(
        fixture.run(&["show", ":review.txt"]),
        b"another index version\n"
    );
}

#[test]
fn unsupported_patches_are_viewable_but_not_partially_applied_and_previews_are_bounded() {
    let fixture = Fixture::new();
    fixture.write("review.txt", "base\n");
    fixture.commit("base");
    fixture.write("review.txt", "no final newline");
    let repo = fixture.repo();
    let review = repo.review_change(&path(), false, &token()).unwrap();
    assert!(review.selection_unavailable().is_some());
    assert!(!review.patch().is_empty());
    assert!(repo.apply_review(&review, &[5], &token()).is_err());
    fs::write(fixture.repository.join("new.bin"), b"hello\0world").unwrap();
    let preview = repo
        .preview_untracked(&RepoPath::new(b"new.bin".to_vec()).unwrap(), &token())
        .unwrap();
    assert!(preview.binary);
    assert!(!preview.truncated);
    fs::write(
        fixture.repository.join("large.txt"),
        vec![b'x'; 3 * 1024 * 1024],
    )
    .unwrap();
    let preview = repo
        .preview_untracked(&RepoPath::new(b"large.txt".to_vec()).unwrap(), &token())
        .unwrap();
    assert!(preview.truncated);
    assert_eq!(preview.bytes.len(), 2 * 1024 * 1024);
    let preview = repo
        .preview_untracked(&RepoPath::new(b"review.txt".to_vec()).unwrap(), &token())
        .unwrap();
    assert_eq!(preview.bytes, b"no final newline");
}

#[test]
fn full_hunk_round_trips_cover_insertions_deletions_empty_file_and_multiple_offsets() {
    for (before, after) in [
        ("a\nb\nc\n", "before\na\nb\nc\nafter\n"),
        ("a\nb\nc\n", "b\n"),
        ("a\n", ""),
        ("", "added\n"),
        (
            "1\n2\n3\n4\n5\n6\n7\n8\n9\n10\n11\n12\n13\n14\n15\n16\n",
            "1\nextra\n2\n3\n4\n5\n6\n7\n8\n9\n10\n11\n12\n13\n15\n16\nlast\n",
        ),
    ] {
        let fixture = Fixture::new();
        fixture.write("review.txt", before);
        fixture.commit("base");
        fixture.write("review.txt", after);
        let repo = fixture.repo();
        let review = repo.review_change(&path(), false, &token()).unwrap();
        assert_eq!(
            review.selection_unavailable(),
            None,
            "{before:?} -> {after:?}"
        );
        apply(&repo, &review, &review.changed_lines());
        assert_eq!(fixture.run(&["show", ":review.txt"]), after.as_bytes());
        let review = repo.review_change(&path(), true, &token()).unwrap();
        apply(&repo, &review, &review.changed_lines());
        assert_eq!(fixture.run(&["show", ":review.txt"]), before.as_bytes());
        assert_eq!(
            fs::read(fixture.repository.join("review.txt")).unwrap(),
            after.as_bytes()
        );
    }
}
