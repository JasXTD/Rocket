mod common;
use common::{Fixture, isolated};
use rocket_git::*;
use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex, mpsc},
    time::Duration,
};

fn token() -> CancellationToken {
    CancellationToken::new()
}
fn setup() -> (Fixture, PathBuf) {
    let f = Fixture::new();
    f.write("readme.md", "initial\n");
    f.commit("initial");
    let remote = f.root.join("remote.git");
    f.run(&[
        "init",
        "--bare",
        "--initial-branch=main",
        remote.to_str().unwrap(),
    ]);
    f.run(&["remote", "add", "origin", remote.to_str().unwrap()]);
    (f, remote)
}
fn refs(f: &Fixture, remote: &Path) -> Vec<u8> {
    f.run_at(
        remote,
        &["for-each-ref", "--format=%(refname) %(objectname)"],
    )
}
fn oid(f: &Fixture, reference: &str) -> String {
    String::from_utf8(f.run(&["rev-parse", reference]))
        .unwrap()
        .trim()
        .into()
}
fn start(f: &Fixture) -> WriteTask<PushReport> {
    let repo = f.repo();
    WriteTask::spawn(move |cancel| {
        repo.push(
            "origin",
            "main",
            "main",
            PushOptions {
                set_upstream: false,
            },
            &cancel,
        )
    })
    .unwrap()
}
struct Gate {
    ready: mpsc::Sender<Vec<ObjectId>>,
    answer: Mutex<mpsc::Receiver<()>>,
}
impl PushCheck for Gate {
    fn check(
        &self,
        _: &Repository,
        sources: &[ObjectId],
        cancel: &CancellationToken,
    ) -> Result<()> {
        self.ready.send(sources.to_vec()).unwrap();
        loop {
            cancel.check()?;
            match self
                .answer
                .lock()
                .unwrap()
                .recv_timeout(Duration::from_millis(20))
            {
                Ok(()) => return Ok(()),
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => return Err(Error::Cancelled),
            }
        }
    }
}
fn gate(f: &Fixture) -> (mpsc::Receiver<Vec<ObjectId>>, mpsc::Sender<()>) {
    let (ready, received) = mpsc::channel();
    let (answer, decision) = mpsc::channel();
    f.git
        .set_push_check(Some(Arc::new(Gate {
            ready,
            answer: Mutex::new(decision),
        })))
        .unwrap();
    (received, answer)
}

#[test]
fn push_waits_cancellation_sends_nothing_and_each_push_needs_its_own_approval() {
    isolated(
        "ROCKET_PUSH_CHECK",
        "push_waits_cancellation_sends_nothing_and_each_push_needs_its_own_approval",
        || {
            let (f, remote) = setup();
            let (ready, approve) = gate(&f);
            let task = start(&f);
            assert_eq!(
                ready.recv_timeout(Duration::from_secs(10)).unwrap()[0].as_str(),
                oid(&f, "HEAD")
            );
            assert!(refs(&f, &remote).is_empty());
            task.cancel();
            let stopped = task.wait().unwrap();
            assert!(matches!(stopped.result, Err(Error::Cancelled)));
            assert_eq!(stopped.remote_outcome, PushRemoteOutcome::NotAttempted);
            assert!(refs(&f, &remote).is_empty());
            let task = start(&f);
            ready.recv_timeout(Duration::from_secs(10)).unwrap();
            assert!(refs(&f, &remote).is_empty());
            approve.send(()).unwrap();
            let pushed = task.wait().unwrap();
            assert_eq!(pushed.remote_outcome, PushRemoteOutcome::Confirmed);
            pushed.result.unwrap();
            let published = refs(&f, &remote);
            let task = start(&f);
            ready.recv_timeout(Duration::from_secs(10)).unwrap();
            task.cancel();
            assert_eq!(
                task.wait().unwrap().remote_outcome,
                PushRemoteOutcome::NotAttempted
            );
            assert_eq!(refs(&f, &remote), published);
            f.git.set_push_check(None).unwrap();
            assert_eq!(
                start(&f).wait().unwrap().remote_outcome,
                PushRemoteOutcome::Confirmed
            );
        },
    );
}

#[test]
fn approved_push_uses_pinned_source_even_if_branch_moves_during_review() {
    isolated(
        "ROCKET_PUSH_CHECK",
        "approved_push_uses_pinned_source_even_if_branch_moves_during_review",
        || {
            let (f, remote) = setup();
            let first = oid(&f, "HEAD");
            f.write("readme.md", "replacement\n");
            f.commit("replacement");
            let replacement = oid(&f, "HEAD");
            f.run(&["update-ref", "refs/heads/main", &first]);
            let (ready, approve) = gate(&f);
            let task = start(&f);
            assert_eq!(
                ready.recv_timeout(Duration::from_secs(10)).unwrap()[0].as_str(),
                first
            );
            f.run(&["update-ref", "refs/heads/main", &replacement]);
            approve.send(()).unwrap();
            task.wait().unwrap().result.unwrap();
            assert_eq!(
                String::from_utf8(f.run_at(&remote, &["rev-parse", "main"]))
                    .unwrap()
                    .trim(),
                first
            );
            assert_eq!(oid(&f, "HEAD"), replacement);
        },
    );
}

#[test]
fn changed_configuration_during_review_refuses_the_push() {
    isolated(
        "ROCKET_PUSH_CHECK",
        "changed_configuration_during_review_refuses_the_push",
        || {
            let (f, remote) = setup();
            let other = f.root.join("other.git");
            f.run(&["init", "--bare", other.to_str().unwrap()]);
            let (ready, approve) = gate(&f);
            let task = start(&f);
            ready.recv_timeout(Duration::from_secs(10)).unwrap();
            f.run(&["remote", "set-url", "origin", other.to_str().unwrap()]);
            approve.send(()).unwrap();
            let stopped = task.wait().unwrap();
            assert_eq!(stopped.remote_outcome, PushRemoteOutcome::NotAttempted);
            assert!(
                stopped
                    .result
                    .unwrap_err()
                    .to_string()
                    .contains("configuration changed")
            );
            assert!(refs(&f, &remote).is_empty());
            assert!(refs(&f, &other).is_empty());
        },
    );
}

struct Refuse(Mutex<Vec<Vec<ObjectId>>>);
impl PushCheck for Refuse {
    fn check(&self, _: &Repository, sources: &[ObjectId], _: &CancellationToken) -> Result<()> {
        self.0.lock().unwrap().push(sources.to_vec());
        Err(Error::InvalidInput("Synthetic dependency refusal".into()))
    }
}

#[test]
fn deleting_a_remote_tag_needs_no_dependency_tree() {
    isolated(
        "ROCKET_PUSH_CHECK",
        "deleting_a_remote_tag_needs_no_dependency_tree",
        || {
            let (f, remote) = setup();
            f.run(&["tag", "v1"]);
            f.run(&["push", "origin", "refs/tags/v1"]);
            let guard = Arc::new(Refuse(Mutex::new(vec![])));
            f.git.set_push_check(Some(guard.clone())).unwrap();
            let repo = f.repo();
            let tags = repo.remote_tags(b"origin", &token()).unwrap();
            let p = repo
                .preview_remote_tag(&tags, b"v1", RemoteTagAction::Delete, &token())
                .unwrap();
            repo.apply_remote_tag(&p, &token()).unwrap();
            assert!(guard.0.lock().unwrap().is_empty());
            assert!(refs(&f, &remote).is_empty());
        },
    );
}

#[test]
fn force_atomic_tag_and_commit_push_all_reach_the_shared_guard() {
    isolated(
        "ROCKET_PUSH_CHECK",
        "force_atomic_tag_and_commit_push_all_reach_the_shared_guard",
        || {
            for kind in ["force", "atomic", "tag", "commit"] {
                let (f, remote) = setup();
                let guard = Arc::new(Refuse(Mutex::new(vec![])));
                if kind == "force" {
                    f.run(&["push", "origin", "main"]);
                    f.write("readme.md", "replacement\n");
                    f.run(&["add", "--all"]);
                    f.run(&["commit", "--amend", "--no-edit"]);
                }
                if kind == "atomic" {
                    f.run(&["branch", "feature"]);
                    f.write("readme.md", "main update\n");
                    f.commit("main update");
                }
                if kind == "tag" {
                    f.run(&["tag", "-a", "v1", "-m", "fixture tag"]);
                }
                if kind == "commit" {
                    f.write("new.md", "staged\n");
                    f.run(&["add", "--all"]);
                }
                let before = refs(&f, &remote);
                f.git.set_secret_protection(false); // This preference is independent.
                f.git.set_push_check(Some(guard.clone())).unwrap();
                let repo = f.repo();
                match kind {
                    "force" => {
                        let p = repo
                            .preview_force_push(b"origin", b"main", b"main", &token())
                            .unwrap();
                        let _ = repo.apply_force_push(&p, &token());
                    }
                    "atomic" => {
                        let selection: Vec<_> = [b"main".as_slice(), b"feature".as_slice()]
                            .into_iter()
                            .map(|name| AtomicPushSelection {
                                local_branch: name.to_vec(),
                                remote_branch: name.to_vec(),
                            })
                            .collect();
                        let p = repo
                            .preview_atomic_push(b"origin", &selection, &token())
                            .unwrap();
                        let _ = repo.apply_atomic_push(&p, &token());
                    }
                    "tag" => {
                        let tags = repo.remote_tags(b"origin", &token()).unwrap();
                        let p = repo
                            .preview_remote_tag(&tags, b"v1", RemoteTagAction::Publish, &token())
                            .unwrap();
                        let _ = repo.apply_remote_tag(&p, &token());
                    }
                    "commit" => {
                        let old = oid(&f, "HEAD");
                        let p = repo
                            .preview_commit_push(
                                "local commit retained",
                                b"origin",
                                b"main",
                                &token(),
                            )
                            .unwrap();
                        let result = repo.apply_commit_push(&p, &token()).unwrap();
                        assert!(result.reviewed_commit_confirmed);
                        assert_ne!(oid(&f, "HEAD"), old);
                        assert_eq!(
                            result.push.unwrap().remote_outcome,
                            PushRemoteOutcome::NotAttempted
                        );
                    }
                    _ => unreachable!(),
                }
                let calls = guard.0.lock().unwrap();
                assert_eq!(calls.len(), 1, "{kind}");
                assert_eq!(calls[0].len(), if kind == "atomic" { 2 } else { 1 });
                assert_eq!(refs(&f, &remote), before, "{kind}");
            }
        },
    );
}

#[test]
fn secret_protection_still_refuses_before_the_optional_guard() {
    isolated(
        "ROCKET_PUSH_CHECK",
        "secret_protection_still_refuses_before_the_optional_guard",
        || {
            let (f, remote) = setup();
            f.write(
                "settings.toml",
            format!("{} = {:?}\n", "password", "A9mK7pR4sT2vX8n6Q5bC3z"),
            );
            f.commit("synthetic credential fixture");
            let guard = Arc::new(Refuse(Mutex::new(vec![])));
            f.git.set_push_check(Some(guard.clone())).unwrap();
            let report = start(&f).wait().unwrap();
            assert!(matches!(report.result, Err(Error::SecretProtection(_))));
            assert!(guard.0.lock().unwrap().is_empty());
            assert!(refs(&f, &remote).is_empty());
        },
    );
}
