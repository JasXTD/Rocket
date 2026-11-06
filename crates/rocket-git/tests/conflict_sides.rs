#![cfg(any(windows, target_os = "linux"))]
mod common;
use common::{Fixture, isolated};
use rocket_git::*;
use std::{fs, io::Write, path::Path, process::Stdio};

const CASE: &str = "ROCKET_CONFLICT_SIDE_CASE";
const BIN: &str = "assets/图像 🚀.bin";
const DOC: &str = "docs/说明 é.txt";
const GONE: &str = "gone 中文.txt";

fn path(name: &str) -> RepoPath {
    RepoPath::new(name.as_bytes().to_vec()).unwrap()
}
fn token() -> CancellationToken {
    CancellationToken::new()
}
/// Binary content: Git's merge sees a NUL in the first 8,000 bytes.
fn binary(tag: &str) -> Vec<u8> {
    let mut bytes = vec![0, 1, 2, 0xff];
    bytes.extend_from_slice(tag.as_bytes());
    bytes
}
fn text(f: &Fixture, args: &[&str]) -> String {
    String::from_utf8(f.run(args)).unwrap().trim_end().into()
}
fn oid(f: &Fixture, revision: &str) -> ObjectId {
    f.repo().resolve_commit(revision, &token()).unwrap()
}
fn working(f: &Fixture, name: &str) -> Option<Vec<u8>> {
    fs::read(f.repository.join(name)).ok()
}
fn committed(f: &Fixture, revision: &str, name: &str) -> Option<Vec<u8>> {
    let output = f.output(&f.repository, &["show", &format!("{revision}:{name}")]);
    output.status.success().then_some(output.stdout)
}
fn remove(f: &Fixture, name: &str) {
    fs::remove_file(f.repository.join(name)).unwrap();
}
fn review(f: &Fixture, name: &str) -> WholeConflictPreview {
    f.repo()
        .review_whole_conflict(&path(name), &token())
        .unwrap()
}
fn keep(f: &Fixture, name: &str, side: ConflictKeep) -> WholeConflictReport {
    f.repo()
        .keep_conflict_side(&review(f, name), side, &token())
        .unwrap()
}
fn ok(r: &WholeConflictReport) {
    assert!(r.result.is_ok(), "{:?}", r.result);
    assert!(r.staged);
}
fn refused(r: &WholeConflictReport, failure: ConflictFailure) {
    assert!(
        matches!(r.result, Err(Error::Conflict(actual)) if actual == failure),
        "{:?}",
        r.result
    );
    assert!(r.file_change.is_none() && !r.staged && r.recovery_directory.is_none());
}
fn conflicted(r: &WholeConflictReport) -> bool {
    r.status
        .as_ref()
        .unwrap()
        .entries
        .iter()
        .any(|e| e.kind == EntryKind::Unmerged)
}
fn copy(dir: &Path, name: &str) -> Option<Vec<u8>> {
    fs::read(dir.join(name)).ok()
}
fn git_input(f: &Fixture, args: &[&str], input: &[u8]) -> Vec<u8> {
    let mut child = f
        .command(&f.repository, args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(input).unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success(), "{args:?}");
    output.stdout
}
fn commit_request(operation: CommitOperation, target: ObjectId) -> CommitActionRequest {
    CommitActionRequest {
        operation,
        targets: vec![target],
        mainline: None,
        recovery: None,
    }
}
fn continue_commit(f: &Fixture, operation: CommitOperation) {
    let repo = f.repo();
    let request = CommitActionRequest {
        operation,
        targets: Vec::new(),
        mainline: None,
        recovery: Some(CommitRecovery::Continue),
    };
    let p = repo.preview_commit_action(request, &token()).unwrap();
    let done = repo.apply_commit_action(&p, None, &token()).unwrap();
    done.write.result.unwrap();
    assert!(done.state.unwrap().is_idle());
}

/// main and other both change the binary; other deletes DOC that main edits;
/// main deletes GONE that other edits.
fn merge_fixture(format: &str) -> Fixture {
    let f = Fixture::with_format(format);
    f.write(BIN, binary("base"));
    f.write(DOC, "base 文\n");
    f.write(GONE, "gone base\n");
    f.commit("base");
    f.run(&["checkout", "-b", "other"]);
    f.write(BIN, binary("theirs"));
    remove(&f, DOC);
    f.write(GONE, "gone theirs 界\n");
    f.commit("theirs");
    f.run(&["checkout", "main"]);
    f.write(BIN, binary("ours"));
    f.write(DOC, "ours 🚀\n");
    remove(&f, GONE);
    f.commit("ours");
    f
}
fn conflicted_merge(format: &str) -> Fixture {
    let f = merge_fixture(format);
    assert!(
        !f.output(&f.repository, &["merge", "other"])
            .status
            .success()
    );
    f
}

#[test]
fn merge_keeps_binary_and_deleted_versions_then_continues() {
    isolated(
        CASE,
        "merge_keeps_binary_and_deleted_versions_then_continues",
        || {
            for format in ["sha1", "sha256"] {
                let f = merge_fixture(format);
                let repo = f.repo();
                let merge = repo
                    .merge("other", MergeMode::NoFastForward, &token())
                    .unwrap();
                assert_eq!(merge.failure, Some(MergeFailure::Conflicts));
                let head = oid(&f, "HEAD");

                // Both sides changed a binary file; Git left ours in the tree.
                let p = review(&f, BIN);
                assert!(p.binary() && !p.deleted());
                for stage in 1..=3 {
                    assert!(p.version(stage).unwrap().binary);
                }
                assert_eq!(p.version(3).unwrap().size, binary("theirs").len());
                assert_eq!(p.working_size(), Some(binary("ours").len()));
                assert!(p.can_keep(ConflictKeep::Ours) && p.can_keep(ConflictKeep::Theirs));
                assert!(!p.can_keep(ConflictKeep::Deletion));
                assert!(p.working_matches(2) && !p.working_matches(1) && !p.working_matches(3));
                assert_eq!(p.planned_change(ConflictKeep::Ours), None);
                assert_eq!(
                    p.planned_change(ConflictKeep::Theirs),
                    Some(WholeFileChange::Replaced)
                );
                assert!(matches!(
                    repo.keep_conflict_side(&p, ConflictKeep::Deletion, &token()),
                    Err(Error::Conflict(ConflictFailure::VersionUnavailable))
                ));
                let r = repo
                    .keep_conflict_side(&p, ConflictKeep::Theirs, &token())
                    .unwrap();
                ok(&r);
                assert_eq!(r.file_change, Some(WholeFileChange::Replaced));
                assert_eq!(working(&f, BIN).unwrap(), binary("theirs"));
                assert_eq!(committed(&f, "", BIN).unwrap(), binary("theirs"));
                let dir = r.recovery_directory.clone().unwrap();
                assert_eq!(
                    dir.parent().unwrap().file_name().unwrap(),
                    "rocket-conflict-recovery"
                );
                assert_eq!(copy(&dir, "observed-original").unwrap(), binary("ours"));
                assert_eq!(copy(&dir, "replaced-original").unwrap(), binary("ours"));
                assert_eq!(copy(&dir, "result").unwrap(), binary("theirs"));
                assert!(copy(&dir, "replacement").is_none());
                assert!(conflicted(&r));

                // Modify/delete: theirs deleted the file and ours is in the tree.
                let p = review(&f, DOC);
                assert!(p.deleted() && !p.binary());
                assert!(p.version(2).is_some() && p.version(3).is_none());
                assert!(!p.can_keep(ConflictKeep::Theirs));
                let r = repo
                    .keep_conflict_side(&p, ConflictKeep::Deletion, &token())
                    .unwrap();
                ok(&r);
                assert_eq!(r.file_change, Some(WholeFileChange::Removed));
                assert!(working(&f, DOC).is_none());
                let dir = r.recovery_directory.clone().unwrap();
                assert_eq!(
                    copy(&dir, "replaced-original").unwrap(),
                    "ours 🚀\n".as_bytes()
                );
                assert_eq!(
                    copy(&dir, "observed-original").unwrap(),
                    "ours 🚀\n".as_bytes()
                );
                assert!(copy(&dir, "result").is_none());
                assert_eq!(text(&f, &["ls-files", "--stage", "--", DOC]), "");

                // Delete/modify: ours deleted it; the tree already holds theirs.
                let p = review(&f, GONE);
                assert!(p.version(2).is_none() && p.working_matches(3));
                assert_eq!(p.planned_change(ConflictKeep::Theirs), None);
                assert_eq!(
                    p.planned_change(ConflictKeep::Deletion),
                    Some(WholeFileChange::Removed)
                );
                let r = repo
                    .keep_conflict_side(&p, ConflictKeep::Theirs, &token())
                    .unwrap();
                ok(&r);
                assert_eq!(r.file_change, None);
                assert!(r.recovery_directory.is_none());
                assert!(!conflicted(&r));
                assert!(!r.state.as_ref().unwrap().is_idle());
                assert_eq!(oid(&f, "HEAD"), head);

                let done = repo
                    .continue_merge("Merge kept versions 中文", &token())
                    .unwrap();
                done.write.result.unwrap();
                assert!(done.state.unwrap().is_idle());
                assert_eq!(
                    text(&f, &["rev-list", "--parents", "-n", "1", "HEAD"])
                        .split(' ')
                        .count(),
                    3
                );
                assert_eq!(committed(&f, "HEAD", BIN).unwrap(), binary("theirs"));
                assert_eq!(
                    committed(&f, "HEAD", GONE).unwrap(),
                    "gone theirs 界\n".as_bytes()
                );
                assert!(committed(&f, "HEAD", DOC).is_none());
                assert_eq!(text(&f, &["status", "--porcelain"]), "");
            }
        },
    );
}

#[test]
fn cherry_pick_and_revert_keep_whole_versions_then_continue() {
    isolated(
        CASE,
        "cherry_pick_and_revert_keep_whole_versions_then_continue",
        || {
            // Cherry-pick a commit that changes the binary, deletes a document main
            // edited, and adds a file.
            let f = Fixture::new();
            f.write(BIN, binary("base"));
            f.write(DOC, "base\n");
            f.commit("base");
            f.run(&["switch", "-c", "source"]);
            f.write(BIN, binary("picked"));
            remove(&f, DOC);
            f.write("added 新.txt", "added\n");
            f.commit("source");
            let source = oid(&f, "HEAD");
            f.run(&["switch", "main"]);
            f.write(BIN, binary("main"));
            f.write(DOC, "main 界\n");
            f.commit("main");
            let main = oid(&f, "HEAD");
            let repo = f.repo();
            let p = repo
                .preview_commit_action(
                    commit_request(CommitOperation::CherryPick, source),
                    &token(),
                )
                .unwrap();
            let r = repo.apply_commit_action(&p, None, &token()).unwrap();
            assert_eq!(r.failure, Some(CommitFailure::ConflictsRemain));
            // Keeping ours needs no file write: only the index changes.
            let r = keep(&f, BIN, ConflictKeep::Ours);
            ok(&r);
            assert_eq!(r.file_change, None);
            assert!(r.recovery_directory.is_none());
            assert_eq!(working(&f, BIN).unwrap(), binary("main"));
            let r = keep(&f, DOC, ConflictKeep::Deletion);
            ok(&r);
            assert_eq!(r.file_change, Some(WholeFileChange::Removed));
            continue_commit(&f, CommitOperation::CherryPick);
            assert_eq!(oid(&f, "HEAD^"), main);
            assert_eq!(committed(&f, "HEAD", BIN).unwrap(), binary("main"));
            assert!(committed(&f, "HEAD", DOC).is_none());
            assert_eq!(committed(&f, "HEAD", "added 新.txt").unwrap(), b"added\n");

            // Revert a commit whose binary and new document were changed again later.
            let f = Fixture::with_format("sha256");
            f.write(BIN, binary("v0"));
            f.write("keep.txt", "keep\n");
            f.commit("base");
            f.write(BIN, binary("v1"));
            f.write(DOC, "added by target\n");
            f.commit("target");
            let target = oid(&f, "HEAD");
            f.write(BIN, binary("v2"));
            f.write(DOC, "edited later 中文\n");
            f.commit("later");
            let later = oid(&f, "HEAD");
            let repo = f.repo();
            let p = repo
                .preview_commit_action(commit_request(CommitOperation::Revert, target), &token())
                .unwrap();
            let r = repo.apply_commit_action(&p, None, &token()).unwrap();
            assert_eq!(r.failure, Some(CommitFailure::ConflictsRemain));
            // Theirs is the reverted commit's parent: the original binary.
            let r = keep(&f, BIN, ConflictKeep::Theirs);
            ok(&r);
            assert_eq!(r.file_change, Some(WholeFileChange::Replaced));
            assert_eq!(working(&f, BIN).unwrap(), binary("v0"));
            // The later edit survives even after the working file was removed by hand.
            remove(&f, DOC);
            let p = review(&f, DOC);
            assert_eq!(p.working_size(), None);
            assert_eq!(
                p.planned_change(ConflictKeep::Ours),
                Some(WholeFileChange::Created)
            );
            let r = repo
                .keep_conflict_side(&p, ConflictKeep::Ours, &token())
                .unwrap();
            ok(&r);
            assert_eq!(r.file_change, Some(WholeFileChange::Created));
            assert_eq!(working(&f, DOC).unwrap(), "edited later 中文\n".as_bytes());
            let dir = r.recovery_directory.clone().unwrap();
            assert_eq!(
                copy(&dir, "result").unwrap(),
                "edited later 中文\n".as_bytes()
            );
            assert!(copy(&dir, "observed-original").is_none());
            assert!(copy(&dir, "replaced-original").is_none());
            assert!(copy(&dir, "replacement").is_none());
            continue_commit(&f, CommitOperation::Revert);
            assert_eq!(oid(&f, "HEAD^"), later);
            assert_eq!(committed(&f, "HEAD", BIN).unwrap(), binary("v0"));
            assert_eq!(
                committed(&f, "HEAD", DOC).unwrap(),
                "edited later 中文\n".as_bytes()
            );
            assert_eq!(text(&f, &["status", "--porcelain"]), "");
        },
    );
}

#[test]
fn rebase_keeps_replayed_and_deleted_versions_then_continues() {
    isolated(
        CASE,
        "rebase_keeps_replayed_and_deleted_versions_then_continues",
        || {
            let f = Fixture::new();
            f.write(BIN, binary("base"));
            f.write(DOC, "base\n");
            f.commit("base");
            f.run(&["switch", "-c", "feature/検索"]);
            f.write(BIN, binary("feature"));
            remove(&f, DOC);
            f.write("feature.txt", "feature\n");
            f.commit("feature");
            f.run(&["switch", "main"]);
            f.write(BIN, binary("main"));
            f.write(DOC, "main\n");
            f.commit("main");
            let main = oid(&f, "HEAD");
            f.run(&["switch", "feature/検索"]);
            let repo = f.repo();
            let start = repo
                .preview_rebase(RebaseAction::Start, "main", &token())
                .unwrap();
            let r = repo.apply_rebase(&start, &token()).unwrap();
            assert_eq!(r.failure, Some(RebaseFailure::ConflictsRemain));
            // During a rebase, ours (stage 2) is main and theirs (stage 3) is the
            // replayed commit, following Git.
            let p = review(&f, BIN);
            assert_eq!(p.working_size(), Some(binary("main").len()));
            let r = repo
                .keep_conflict_side(&p, ConflictKeep::Theirs, &token())
                .unwrap();
            ok(&r);
            assert_eq!(r.file_change, Some(WholeFileChange::Replaced));
            assert_eq!(working(&f, BIN).unwrap(), binary("feature"));
            let p = review(&f, DOC);
            assert!(p.version(3).is_none() && p.can_keep(ConflictKeep::Deletion));
            let r = repo
                .keep_conflict_side(&p, ConflictKeep::Deletion, &token())
                .unwrap();
            ok(&r);
            assert!(r.state.as_ref().unwrap().rebase);
            let resume = repo
                .preview_rebase(RebaseAction::Continue, "", &token())
                .unwrap();
            let done = repo.apply_rebase(&resume, &token()).unwrap();
            done.write.result.unwrap();
            assert!(done.state.unwrap().is_idle());
            assert_eq!(text(&f, &["branch", "--show-current"]), "feature/検索");
            assert_eq!(oid(&f, "HEAD^"), main);
            assert_eq!(committed(&f, "HEAD", BIN).unwrap(), binary("feature"));
            assert!(committed(&f, "HEAD", DOC).is_none());
            assert_eq!(committed(&f, "HEAD", "feature.txt").unwrap(), b"feature\n");
            assert_eq!(text(&f, &["status", "--porcelain"]), "");
        },
    );
}

#[test]
fn stale_reviews_and_unavailable_versions_are_refused_without_writing() {
    isolated(
        CASE,
        "stale_reviews_and_unavailable_versions_are_refused_without_writing",
        || {
            let f = conflicted_merge("sha1");
            let repo = f.repo();
            let index = || fs::read(f.repository.join(".git/index")).unwrap();
            let before = index();
            assert!(matches!(
                repo.review_whole_conflict(&path("missing.txt"), &token()),
                Err(Error::Conflict(ConflictFailure::NotConflicted))
            ));

            let p = review(&f, BIN);
            let cancelled = token();
            cancelled.cancel();
            assert!(matches!(
                repo.keep_conflict_side(&p, ConflictKeep::Theirs, &cancelled),
                Err(Error::Cancelled)
            ));
            let other = conflicted_merge("sha1");
            assert!(matches!(
                other
                    .repo()
                    .keep_conflict_side(&p, ConflictKeep::Theirs, &token()),
                Err(Error::Conflict(ConflictFailure::WrongRepository))
            ));
            assert!(matches!(
                repo.keep_conflict_side(&review(&f, DOC), ConflictKeep::Theirs, &token()),
                Err(Error::Conflict(ConflictFailure::VersionUnavailable))
            ));

            // An external edit after review is never overwritten.
            f.write(BIN, binary("external"));
            let r = repo
                .keep_conflict_side(&p, ConflictKeep::Theirs, &token())
                .unwrap();
            refused(&r, ConflictFailure::ReviewStale);
            assert_eq!(working(&f, BIN).unwrap(), binary("external"));

            // A file removed after review is not recreated.
            let p = review(&f, DOC);
            remove(&f, DOC);
            let r = repo
                .keep_conflict_side(&p, ConflictKeep::Ours, &token())
                .unwrap();
            refused(&r, ConflictFailure::ReviewStale);
            assert!(working(&f, DOC).is_none());
            f.write(DOC, "ours 🚀\n");

            // The same commit under another branch name, or new conversion
            // settings, invalidate the review.
            let p = review(&f, DOC);
            f.run(&["branch", "same-tip"]);
            f.run(&["symbolic-ref", "HEAD", "refs/heads/same-tip"]);
            let r = repo
                .keep_conflict_side(&p, ConflictKeep::Deletion, &token())
                .unwrap();
            refused(&r, ConflictFailure::ReviewStale);
            f.run(&["symbolic-ref", "HEAD", "refs/heads/main"]);
            let p = review(&f, DOC);
            f.run(&["config", "core.autocrlf", "true"]);
            let r = repo
                .keep_conflict_side(&p, ConflictKeep::Deletion, &token())
                .unwrap();
            refused(&r, ConflictFailure::ReviewStale);
            f.run(&["config", "core.autocrlf", "false"]);
            assert_eq!(working(&f, DOC).unwrap(), "ours 🚀\n".as_bytes());
            assert_eq!(index(), before);

            // Resolved elsewhere after review: Rocket's choice never overwrites it.
            let p = review(&f, BIN);
            f.run(&["add", "--", BIN]);
            let staged = index();
            let r = repo
                .keep_conflict_side(&p, ConflictKeep::Theirs, &token())
                .unwrap();
            refused(&r, ConflictFailure::NotConflicted);
            assert_eq!(working(&f, BIN).unwrap(), binary("external"));
            assert_eq!(index(), staged);
            assert!(!f.repository.join(".git/rocket-conflict-recovery").exists());
        },
    );
}

#[test]
fn locked_index_readonly_and_unsupported_versions_report_truthfully() {
    isolated(
        CASE,
        "locked_index_readonly_and_unsupported_versions_report_truthfully",
        || {
            let f = conflicted_merge("sha1");
            let repo = f.repo();
            // The file is published, but Git cannot take the index lock.
            let p = review(&f, BIN);
            f.write(".git/index.lock", "test-owned lock");
            let r = repo
                .keep_conflict_side(&p, ConflictKeep::Theirs, &token())
                .unwrap();
            assert!(r.result.is_err() && !r.staged);
            assert_eq!(r.file_change, Some(WholeFileChange::Replaced));
            assert_eq!(working(&f, BIN).unwrap(), binary("theirs"));
            assert_eq!(
                fs::read(f.repository.join(".git/index.lock")).unwrap(),
                b"test-owned lock"
            );
            assert!(conflicted(&r));
            let dir = r.recovery_directory.clone().unwrap();
            assert_eq!(copy(&dir, "replaced-original").unwrap(), binary("ours"));
            fs::remove_file(f.repository.join(".git/index.lock")).unwrap();
            // A fresh review finds theirs already in place and only stages it.
            let r = keep(&f, BIN, ConflictKeep::Theirs);
            ok(&r);
            assert_eq!(r.file_change, None);

            // A read-only working file is refused before any recovery copy.
            let file = f.repository.join(DOC);
            let original = fs::metadata(&file).unwrap().permissions();
            let mut readonly = original.clone();
            readonly.set_readonly(true);
            fs::set_permissions(&file, readonly).unwrap();
            let r = repo
                .keep_conflict_side(&review(&f, DOC), ConflictKeep::Deletion, &token())
                .unwrap();
            fs::set_permissions(&file, original).unwrap();
            assert!(r.result.unwrap_err().to_string().contains("read-only"));
            assert!(r.file_change.is_none() && r.recovery_directory.is_none());
            assert_eq!(working(&f, DOC).unwrap(), "ours 🚀\n".as_bytes());

            // Content filters such as Git LFS are refused, never run.
            f.write(".git/info/attributes", "*.txt filter=lfs\n");
            assert!(matches!(
                repo.review_whole_conflict(&path(DOC), &token()),
                Err(Error::Conflict(ConflictFailure::UnsupportedVersion))
            ));
            remove(&f, ".git/info/attributes");

            // Links and other special modes need Git.
            let blob =
                String::from_utf8(git_input(&f, &["hash-object", "-w", "--stdin"], b"target"))
                    .unwrap();
            let blob = blob.trim();
            git_input(
                &f,
                &["update-index", "-z", "--index-info"],
                format!("120000 {blob} 2\tlink\0120000 {blob} 3\tlink\0").as_bytes(),
            );
            assert!(matches!(
                repo.review_whole_conflict(&path("link"), &token()),
                Err(Error::Conflict(ConflictFailure::UnsupportedVersion))
            ));

            // Versions larger than the 16 MiB limit are refused before reading.
            let large = f.root.join("large-version");
            fs::write(&large, vec![b'x'; CONFLICT_FILE_LIMIT + 1]).unwrap();
            let blob = text(&f, &["hash-object", "-w", large.to_str().unwrap()]);
            git_input(
                &f,
                &["update-index", "-z", "--index-info"],
                format!("100644 {blob} 2\tlarge.bin\0100644 {blob} 3\tlarge.bin\0").as_bytes(),
            );
            assert!(matches!(
                repo.review_whole_conflict(&path("large.bin"), &token()),
                Err(Error::Conflict(ConflictFailure::UnsupportedVersion))
            ));
            assert!(working(&f, "large.bin").is_none() && working(&f, "link").is_none());
        },
    );
}

#[test]
fn keeping_a_gitattributes_version_may_change_its_own_attributes() {
    isolated(
        CASE,
        "keeping_a_gitattributes_version_may_change_its_own_attributes",
        || {
            // Ours deleted .gitattributes and theirs edited it; its `* text=auto`
            // also applies to itself. With no ours stage to fall back to, removing
            // the working file changes that path's attributes after publication.
            // The reviewed stages are unchanged, so the deletion is still staged.
            let f = Fixture::new();
            f.write(".gitattributes", "* text=auto\n");
            f.write("keep.txt", "keep\n");
            f.commit("base");
            f.run(&["checkout", "-b", "other"]);
            f.write(".gitattributes", "* text=auto\n*.bin binary\n");
            f.commit("theirs");
            f.run(&["checkout", "main"]);
            remove(&f, ".gitattributes");
            f.commit("ours");
            assert!(
                !f.output(&f.repository, &["merge", "other"])
                    .status
                    .success()
            );
            let before = f.run(&["check-attr", "text", "--", ".gitattributes"]);
            let r = keep(&f, ".gitattributes", ConflictKeep::Deletion);
            ok(&r);
            assert_eq!(r.file_change, Some(WholeFileChange::Removed));
            assert_ne!(
                f.run(&["check-attr", "text", "--", ".gitattributes"]),
                before
            );
            assert_eq!(text(&f, &["ls-files", "-u"]), "");
            let done = f
                .repo()
                .continue_merge("Keep the deletion", &token())
                .unwrap();
            done.write.result.unwrap();
            assert!(committed(&f, "HEAD", ".gitattributes").is_none());
        },
    );
}

/// Linux: keeping a version gives the file that version's executable bit, so the
/// staged entry and the working file agree.
#[cfg(target_os = "linux")]
#[test]
fn keeping_a_version_applies_its_executable_bit() {
    isolated(CASE, "keeping_a_version_applies_its_executable_bit", || {
        use std::os::unix::fs::PermissionsExt;
        for (side, executable) in [(ConflictKeep::Theirs, true), (ConflictKeep::Ours, false)] {
            let f = Fixture::new();
            let tool = f.repository.join("tool");
            f.write("tool", binary("base"));
            f.commit("base");
            f.run(&["checkout", "-b", "other"]);
            f.write("tool", binary("theirs"));
            fs::set_permissions(&tool, fs::Permissions::from_mode(0o755)).unwrap();
            f.commit("theirs");
            f.run(&["checkout", "main"]);
            f.write("tool", binary("ours"));
            f.commit("ours");
            assert!(!f.output(&f.repository, &["merge", "other"]).status.success());
            ok(&keep(&f, "tool", side));
            let mode = fs::metadata(&tool).unwrap().permissions().mode();
            assert_eq!(mode & 0o111 != 0, executable, "{side:?}: {mode:o}");
            let entry = text(&f, &["ls-files", "--stage", "--", "tool"]);
            assert!(entry.starts_with(if executable { "100755 " } else { "100644 " }), "{entry}");
            let status = text(&f, &["status", "--porcelain", "--", "tool"]);
            assert!(status.is_empty() || status.as_bytes()[1] == b' ', "{side:?}: {status}");
        }
    });
}
