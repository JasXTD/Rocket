mod common;
use common::Fixture;
use rocket_git::*;
use std::fs;

fn oid(bytes: &[u8]) -> ObjectId {
    ObjectId::parse(std::str::from_utf8(bytes).unwrap().trim()).unwrap()
}

fn counted_head(repo: &Repository, log: &DiagnosticLog) -> (Head, usize) {
    let before = fs::read_to_string(log.directory().join("rocket.jsonl")).unwrap();
    let head = repo
        .with_log(log.clone())
        .head(&CancellationToken::new())
        .unwrap();
    let after = fs::read_to_string(log.directory().join("rocket.jsonl")).unwrap();
    let commands = after[before.len()..]
        .lines()
        .filter(|line| line.contains("\"event\":\"git_start\""))
        .count();
    (head, commands)
}

#[test]
fn normal_head_is_fresh_in_one_command_across_hash_and_ref_formats() {
    common::isolated(
        "ROCKET_LOADING_CASE",
        "normal_head_is_fresh_in_one_command_across_hash_and_ref_formats",
        || {
            for format in ["sha1", "sha256"] {
                let f = Fixture::with_format(format);
                f.write("a", "first\n");
                f.commit("first");
                let first = oid(&f.run(&["rev-parse", "HEAD"]));
                let repo = f.repo();
                let log = DiagnosticLog::open(f.root.join("head-logs")).unwrap();
                let (head, commands) = counted_head(&repo, &log);
                assert_eq!(
                    head,
                    Head::Branch {
                        name: b"main".to_vec(),
                        target: first
                    }
                );
                assert_eq!(commands, 1, "normal branch HEAD should need one read");

                f.write("a", "second\n");
                f.commit("second");
                let second = oid(&f.run(&["rev-parse", "HEAD"]));
                f.run(&["pack-refs", "--all", "--prune"]);
                let (head, commands) = counted_head(&repo, &log);
                assert_eq!(
                    head,
                    Head::Branch {
                        name: b"main".to_vec(),
                        target: second.clone()
                    }
                );
                assert_eq!(commands, 1);
                f.run(&["symbolic-ref", "refs/heads/indirect", "refs/heads/main"]);
                f.run(&["symbolic-ref", "HEAD", "refs/heads/indirect"]);
                let (head, commands) = counted_head(&repo, &log);
                assert_eq!(
                    head,
                    Head::Branch {
                        name: b"main".to_vec(),
                        target: second.clone()
                    }
                );
                assert_eq!(commands, 1, "follow symbolic-ref chains as before");
                f.run(&["switch", "--detach", second.as_str()]);
                let (head, commands) = counted_head(&repo, &log);
                assert_eq!(
                    head,
                    Head::Detached {
                        target: second.clone()
                    }
                );
                assert_eq!(commands, 1);
                f.run(&["refs", "migrate", "--ref-format=reftable"]);
                let (head, commands) = counted_head(&f.repo(), &log);
                assert_eq!(head, Head::Detached { target: second });
                assert_eq!(commands, 1);
            }
        },
    );
}

#[test]
fn unborn_dangling_and_tag_heads_keep_their_existing_meanings() {
    common::isolated(
        "ROCKET_LOADING_CASE",
        "unborn_dangling_and_tag_heads_keep_their_existing_meanings",
        || {
            let f = Fixture::new();
            let repo = f.repo();
            let cancel = CancellationToken::new();
            assert_eq!(
                repo.head(&cancel).unwrap(),
                Head::Unborn {
                    branch: b"main".to_vec()
                }
            );
            f.run(&["symbolic-ref", "refs/heads/main", "refs/heads/missing"]);
            assert_eq!(
                repo.head(&cancel).unwrap(),
                Head::Unborn {
                    branch: b"missing".to_vec()
                }
            );
            f.run(&["symbolic-ref", "HEAD", "refs/heads/real"]);
            f.write("a", "base\n");
            f.commit("base");
            let target = oid(&f.run(&["rev-parse", "HEAD"]));
            f.run(&["tag", "-a", "release", "-m", "annotated"]);
            f.run(&["symbolic-ref", "HEAD", "refs/tags/release"]);
            assert_eq!(
                repo.head(&cancel).unwrap(),
                Head::Branch {
                    name: b"refs/tags/release".to_vec(),
                    target
                }
            );
            fs::write(
                f.repository.join(".git/HEAD"),
                format!("{}\n", "f".repeat(40)),
            )
            .unwrap();
            assert!(
                repo.head(&cancel).is_err(),
                "a missing object is not a healthy HEAD"
            );
            cancel.cancel();
            assert!(matches!(repo.head(&cancel), Err(Error::Cancelled)));
        },
    );
}

#[test]
fn head_parsing_keeps_commit_validation_and_replacement_behavior() {
    common::isolated(
        "ROCKET_LOADING_CASE",
        "head_parsing_keeps_commit_validation_and_replacement_behavior",
        || {
            let f = Fixture::new();
            f.write("a", "base\n");
            f.commit("base");
            let original = oid(&f.run(&["rev-parse", "HEAD"]));
            f.write("a", "replacement\n");
            f.commit("replacement");
            let replacement = oid(&f.run(&["rev-parse", "HEAD"]));
            f.run(&["switch", "--detach", original.as_str()]);
            f.run(&["replace", original.as_str(), replacement.as_str()]);
            let repo = f.repo();
            let cancel = CancellationToken::new();
            assert_eq!(
                repo.head(&cancel).unwrap(),
                Head::Detached {
                    target: oid(&f.run(&["rev-parse", "--verify", "HEAD^{commit}"])),
                }
            );
            let blob = oid(&f.run(&["rev-parse", "HEAD:a"]));
            f.run(&["replace", "-f", original.as_str(), blob.as_str()]);
            let oracle = f.output(&f.repository, &["rev-parse", "--verify", "HEAD^{commit}"]);
            assert!(!oracle.status.success());
            assert!(repo.head(&cancel).is_err());
            f.run(&["replace", "-d", original.as_str()]);

            let invalid = f.root.join("invalid-commit");
            fs::write(&invalid, b"not a commit\n").unwrap();
            let invalid_id = oid(&f.run(&[
                "hash-object",
                "--literally",
                "-w",
                "-t",
                "commit",
                invalid.to_str().unwrap(),
            ]));
            fs::write(f.repository.join(".git/HEAD"), format!("{invalid_id}\n")).unwrap();
            assert!(
                !f.output(&f.repository, &["rev-parse", "--verify", "HEAD^{commit}"])
                    .status
                    .success()
            );
            assert!(
                repo.head(&cancel).is_err(),
                "the optimized read must still parse the commit"
            );
        },
    );
}

#[test]
fn bounded_head_summary_falls_back_to_exact_reads() {
    common::isolated(
        "ROCKET_LOADING_CASE",
        "bounded_head_summary_falls_back_to_exact_reads",
        || {
            let f = Fixture::new();
            f.write("a", "base\n");
            f.commit("base");
            let name = format!("{}/{}", "a".repeat(220), "b".repeat(220));
            // Reftable avoids making this a Windows long-filesystem-path test.
            f.run(&["refs", "migrate", "--ref-format=reftable"]);
            f.run(&["switch", "-c", &name]);
            let expected = f.repo().head(&CancellationToken::new()).unwrap();
            let client = GitClient::new(
                f.git.executable(),
                CommandLimits {
                    stdout_bytes: 512,
                    ..Default::default()
                },
            )
            .unwrap();
            let repo = client
                .open(&f.repository, &CancellationToken::new())
                .unwrap();
            assert_eq!(repo.head(&CancellationToken::new()).unwrap(), expected);
        },
    );
}

#[test]
fn healthy_head_survives_unrelated_broken_refs() {
    common::isolated(
        "ROCKET_LOADING_CASE",
        "healthy_head_survives_unrelated_broken_refs",
        || {
            let f = Fixture::new();
            f.write("a", "base\n");
            f.commit("base");
            let client = GitClient::new(
                f.git.executable(),
                CommandLimits {
                    stderr_bytes: 1,
                    ..Default::default()
                },
            )
            .unwrap();
            let cancel = CancellationToken::new();
            let repo = client.open(&f.repository, &cancel).unwrap();
            let expected = repo.head(&cancel).unwrap();
            fs::write(f.repository.join(".git/refs/heads/broken"), b"invalid\n").unwrap();
            fs::write(
                f.repository.join(".git/refs/heads/missing-object"),
                format!("{}\n", "f".repeat(40)),
            )
            .unwrap();
            let log = DiagnosticLog::open(f.root.join("head-logs")).unwrap();
            let (head, commands) = counted_head(&repo, &log);
            assert_eq!(head, expected);
            assert_eq!(
                commands, 1,
                "healthy HEAD must not enumerate unrelated refs"
            );
        },
    );
}

#[test]
fn linked_and_bare_repositories_keep_their_own_head() {
    common::isolated(
        "ROCKET_LOADING_CASE",
        "linked_and_bare_repositories_keep_their_own_head",
        || {
            let f = Fixture::new();
            f.write("a", "base\n");
            f.commit("base");
            let target = oid(&f.run(&["rev-parse", "HEAD"]));
            let linked = f.root.join("linked");
            f.run(&["worktree", "add", "-b", "linked", linked.to_str().unwrap()]);
            let repo = f.git.open(&linked, &CancellationToken::new()).unwrap();
            assert_eq!(
                repo.head(&CancellationToken::new()).unwrap(),
                Head::Branch {
                    name: b"linked".to_vec(),
                    target: target.clone()
                }
            );
            let bare = f.root.join("bare.git");
            f.run_at(
                &f.root,
                &[
                    "clone",
                    "--bare",
                    "--no-hardlinks",
                    f.repository.to_str().unwrap(),
                    bare.to_str().unwrap(),
                ],
            );
            let repo = f.git.open(&bare, &CancellationToken::new()).unwrap();
            assert_eq!(
                repo.head(&CancellationToken::new()).unwrap(),
                Head::Branch {
                    name: b"main".to_vec(),
                    target
                }
            );
        },
    );
}
