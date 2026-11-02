mod common;
use common::{Fixture, isolated};
use rocket_git::*;
use std::{
    fs,
    path::{Path, PathBuf},
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

struct Repos {
    f: Fixture,
    remote: PathBuf,
    old: ObjectId,
    source: ObjectId,
}
impl Repos {
    fn new(format: &str) -> Self {
        let f = Fixture::with_format(format);
        f.write("文件.txt", "base\n");
        f.commit("PRIVATE base");
        f.write("文件.txt", "old remote\n");
        f.commit("PRIVATE old message");
        let old = oid(&f.run(&["rev-parse", "HEAD"]));
        let remote = f.root.join("PRIVATE remote 文.git");
        fs::create_dir(&remote).unwrap();
        f.run_at(
            &remote,
            &[
                "init",
                "--bare",
                "--initial-branch=main",
                &format!("--object-format={format}"),
            ],
        );
        f.run(&["remote", "add", "origin", remote.to_str().unwrap()]);
        f.run(&["push", "origin", "main"]);
        f.run(&["reset", "--hard", "HEAD^"]);
        f.write("文件.txt", "new local\n");
        f.commit("PRIVATE replacement 文");
        let source = oid(&f.run(&["rev-parse", "HEAD"]));
        Self {
            f,
            remote,
            old,
            source,
        }
    }
    fn preview(&self) -> ForcePushPreview {
        self.f
            .repo()
            .preview_force_push(b"origin", b"main", b"main", &CancellationToken::new())
            .unwrap()
    }
    fn remote_id(&self) -> ObjectId {
        oid(&self
            .f
            .run_at(&self.remote, &["rev-parse", "refs/heads/main"]))
    }
    fn set_remote(&self, id: &ObjectId) {
        self.f.run_at(
            &self.remote,
            &["update-ref", "refs/heads/main", id.as_str()],
        );
    }
    fn dirty(&self) {
        self.f.write("文件.txt", "PRIVATE staged\n");
        self.f.run(&["add", "--", "文件.txt"]);
        self.f.write("文件.txt", "PRIVATE unstaged\n");
        self.f.write("未跟踪.txt", "PRIVATE untracked\n");
    }
    fn snapshot(&self) -> Vec<Vec<u8>> {
        let mut result = vec![
            self.f.run(&["rev-parse", "HEAD"]),
            self.f
                .run(&["for-each-ref", "--format=%(refname) %(objectname)"]),
            self.f.run(&["config", "--null", "--list"]),
        ];
        for name in [".git/index", "文件.txt", "未跟踪.txt"] {
            result.push(fs::read(self.f.repository.join(name)).unwrap_or_default());
        }
        result
    }
    fn hook(&self, server: bool, name: &str, body: &str) {
        let directory = if server {
            self.remote.join("hooks")
        } else {
            self.f.root.join("no-hooks")
        };
        fs::create_dir_all(&directory).unwrap();
        if server {
            self.f.run_at(
                &self.remote,
                &["config", "core.hooksPath", directory.to_str().unwrap()],
            );
        }
        let path = directory.join(name);
        fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
        }
    }
}
fn oid(bytes: &[u8]) -> ObjectId {
    ObjectId::parse(std::str::from_utf8(bytes).unwrap().trim()).unwrap()
}
fn posix(path: &Path) -> String {
    path.to_string_lossy()
        .replace('\\', "/")
        .replace('\'', "'\\''")
}
fn wait(path: &Path) {
    let start = Instant::now();
    while !path.exists() {
        assert!(
            start.elapsed() < Duration::from_secs(45),
            "hook marker timed out"
        );
        thread::sleep(Duration::from_millis(30));
    }
}

#[test]
fn reviewed_lease_replaces_exact_branch_and_preserves_local_state() {
    isolated(
        "ROCKET_FORCE_CASE",
        "reviewed_lease_replaces_exact_branch_and_preserves_local_state",
        || {
            for format in ["sha1", "sha256"] {
                let t = Repos::new(format);
                // Push URL is authoritative even when fetch points elsewhere.
                t.f.run(&[
                    "config",
                    "remote.origin.pushurl",
                    t.remote.to_str().unwrap(),
                ]);
                t.f.run(&[
                    "config",
                    "remote.origin.url",
                    "https://PRIVATE.invalid/not-contacted",
                ]);
                t.f.run(&["config", "remote.origin.mirror", "true"]);
                t.f.run(&["config", "push.followTags", "true"]);
                t.f.run(&["config", "push.autoSetupRemote", "true"]);
                t.f.run(&[
                    "config",
                    "--add",
                    "remote.origin.push",
                    "+refs/heads/*:refs/heads/*",
                ]);
                t.f.run(&["branch", "unrelated"]);
                t.f.run(&["tag", "PRIVATE-tag"]);
                t.dirty();
                let before = t.snapshot();
                let p = t.preview();
                assert_eq!(p.expected(), &t.old);
                assert_eq!(p.source(), &t.source);
                assert_eq!((p.removed_count(), p.added_count()), (1, 1));
                assert!(p.removed()[0].subject_lossy().contains("old message"));
                assert!(!format!("{p:?}").contains("PRIVATE"));
                let report =
                    t.f.repo()
                        .apply_force_push(&p, &CancellationToken::new())
                        .unwrap();
                assert!(report.result.is_ok(), "{:?}", report.result);
                assert!(report.verification.is_ok());
                assert_eq!(report.observed.unwrap(), Some(t.source.clone()));
                assert_eq!(t.remote_id(), t.source);
                assert_eq!(before, t.snapshot());
                assert!(
                    !t.f.output(&t.remote, &["show-ref", "--verify", "refs/heads/unrelated"])
                        .status
                        .success()
                );
                assert!(
                    !t.f.output(
                        &t.remote,
                        &["show-ref", "--verify", "refs/tags/PRIVATE-tag"]
                    )
                    .status
                    .success()
                );
            }
        },
    );
}
#[test]
fn stale_source_configuration_remote_and_unsupported_reviews_are_refused() {
    isolated(
        "ROCKET_FORCE_CASE",
        "stale_source_configuration_remote_and_unsupported_reviews_are_refused",
        || {
            let t = Repos::new("sha1");
            let cancel = CancellationToken::new();
            let repo = t.f.repo();
            let p = t.preview();
            t.f.run(&["update-ref", "refs/heads/main", t.old.as_str()]);
            assert!(matches!(
                repo.apply_force_push(&p, &cancel),
                Err(Error::ForcePush(ForcePushFailure::StalePreview))
            ));
            t.f.run(&["update-ref", "refs/heads/main", t.source.as_str()]);
            t.f.run(&["config", "push.default", "matching"]);
            assert!(matches!(
                repo.apply_force_push(&p, &cancel),
                Err(Error::ForcePush(ForcePushFailure::StalePreview))
            ));
            t.f.run(&["config", "--unset", "push.default"]);
            let p = t.preview();
            t.set_remote(&oid(&t.f.run(&["rev-parse", "HEAD^"])));
            assert!(matches!(
                repo.apply_force_push(&p, &cancel),
                Err(Error::ForcePush(ForcePushFailure::RemoteChanged))
            ));
            let up = t.preview();
            assert!(!up.can_apply());
            assert!(matches!(
                repo.apply_force_push(&up, &cancel),
                Err(Error::ForcePush(ForcePushFailure::NoReplacement))
            ));
            t.set_remote(&t.old);
            t.f.run_at(
                &t.remote,
                &["symbolic-ref", "refs/heads/alias", "refs/heads/main"],
            );
            assert!(matches!(
                repo.preview_force_push(b"origin", b"main", b"alias", &cancel),
                Err(Error::ForcePush(ForcePushFailure::SymbolicBranch))
            ));
            assert!(matches!(
                repo.preview_force_push(b"origin", b"main", b"absent", &cancel),
                Err(Error::ForcePush(ForcePushFailure::RemoteMissing))
            ));
            t.f.run(&["symbolic-ref", "refs/heads/local-alias", "refs/heads/main"]);
            assert!(matches!(
                repo.preview_force_push(b"origin", b"local-alias", b"main", &cancel),
                Err(Error::ForcePush(ForcePushFailure::SymbolicBranch))
            ));
            t.f.run(&[
                "config",
                "--add",
                "remote.origin.pushurl",
                t.remote.to_str().unwrap(),
            ]);
            t.f.run(&[
                "config",
                "--add",
                "remote.origin.pushurl",
                t.remote.to_str().unwrap(),
            ]);
            assert!(matches!(
                repo.preview_force_push(b"origin", b"main", b"main", &cancel),
                Err(Error::ForcePush(ForcePushFailure::DestinationUnsupported))
            ));
            t.f.run(&["config", "--unset-all", "remote.origin.pushurl"]);
            t.f.write(".git/shallow", format!("{}\n", t.source));
            assert!(matches!(
                repo.preview_force_push(b"origin", b"main", b"main", &cancel),
                Err(Error::ForcePush(ForcePushFailure::HistoryUnsupported))
            ));
            assert_eq!(t.remote_id(), t.old);
        },
    );
}
#[test]
fn concurrent_remote_change_after_advertisement_cannot_be_overwritten() {
    isolated(
        "ROCKET_FORCE_CASE",
        "concurrent_remote_change_after_advertisement_cannot_be_overwritten",
        || {
            let t = Repos::new("sha1");
            let peer = t.f.root.join("peer");
            fs::create_dir(&peer).unwrap();
            t.f.run_at(&peer, &["clone", t.remote.to_str().unwrap(), "."]);
            t.f.run_at(&peer, &["config", "user.name", "PRIVATE peer"]);
            t.f.run_at(&peer, &["config", "user.email", "peer@example.invalid"]);
            t.f.run_at(
                &peer,
                &["commit", "--allow-empty", "-m", "PRIVATE concurrent"],
            );
            let concurrent = oid(&t.f.run_at(&peer, &["rev-parse", "HEAD"]));
            t.f.run_at(&peer, &["push", "origin", "HEAD:refs/heads/held"]);
            let ready = t.f.root.join("ready");
            let release = t.f.root.join("release");
            t.hook(
                false,
                "pre-push",
                &format!(
                    "printf ready > '{}'\nwhile ! test -f '{}'; do sleep 0.1; done",
                    posix(&ready),
                    posix(&release)
                ),
            );
            t.dirty();
            let before = t.snapshot();
            let p = t.preview();
            let repo = t.f.repo();
            let task = thread::spawn(move || {
                repo.apply_force_push(&p, &CancellationToken::new())
                    .unwrap()
            });
            wait(&ready);
            t.set_remote(&concurrent);
            fs::write(&release, b"go").unwrap();
            let report = task.join().unwrap();
            assert!(report.result.is_err());
            assert!(report.verification.is_err());
            assert_eq!(report.observed.unwrap(), Some(concurrent.clone()));
            assert_eq!(t.remote_id(), concurrent);
            assert_eq!(t.snapshot(), before);
            assert!(matches!(
                t.f.repo().preview_force_push(
                    b"origin",
                    b"main",
                    b"main",
                    &CancellationToken::new()
                ),
                Err(Error::ForcePush(ForcePushFailure::HistoryUnavailable))
            ));
        },
    );
}
#[test]
fn server_rejection_has_safe_guidance_and_actual_observation() {
    isolated(
        "ROCKET_FORCE_CASE",
        "server_rejection_has_safe_guidance_and_actual_observation",
        || {
            let t = Repos::new("sha1");
            t.hook(
                true,
                "pre-receive",
                "echo 'PRIVATE protected branch policy' >&2\nexit 1",
            );
            t.dirty();
            let before = t.snapshot();
            let p = t.preview();
            let report =
                t.f.repo()
                    .apply_force_push(&p, &CancellationToken::new())
                    .unwrap();
            let error = report.result.unwrap_err();
            assert_eq!(error.push_failure(), Some(PushFailure::RemoteRejected));
            assert!(!error.to_string().contains("PRIVATE"));
            assert!(error.to_string().contains("push_remote_rejected"));
            assert_eq!(report.observed.unwrap(), Some(t.old.clone()));
            assert!(report.verification.is_err());
            assert_eq!(before, t.snapshot());
            assert_eq!(t.remote_id(), t.old);
        },
    );
}
#[test]
fn cancellation_before_and_after_remote_update_is_reconciled() {
    isolated(
        "ROCKET_FORCE_CASE",
        "cancellation_before_and_after_remote_update_is_reconciled",
        || {
            for after in [false, true] {
                let t = Repos::new("sha1");
                let ready = t.f.root.join("ready");
                t.hook(
                    after,
                    if after { "post-receive" } else { "pre-push" },
                    &format!("printf ready > '{}'\nsleep 30", posix(&ready)),
                );
                t.dirty();
                let before = t.snapshot();
                let p = t.preview();
                let repo = t.f.repo();
                let cancel = CancellationToken::new();
                let token = cancel.clone();
                let (tx, rx) = mpsc::sync_channel(4);
                let task = thread::spawn(move || {
                    repo.apply_force_push_with_progress(&p, Some(&tx), &token)
                        .unwrap()
                });
                wait(&ready);
                assert_eq!(
                    rx.try_iter().collect::<Vec<_>>(),
                    vec![ForcePushProgress::CheckingLease, ForcePushProgress::Pushing]
                );
                let start = Instant::now();
                cancel.cancel();
                let report = task.join().unwrap();
                assert!(start.elapsed() < Duration::from_secs(20));
                assert!(matches!(report.result, Err(Error::Cancelled)));
                assert_eq!(
                    rx.try_iter().collect::<Vec<_>>(),
                    vec![ForcePushProgress::CheckingRemote {
                        push_cancelled: true
                    }]
                );
                let expected = if after { &t.source } else { &t.old };
                assert_eq!(report.observed.unwrap().as_ref(), Some(expected));
                assert_eq!(report.verification.is_ok(), after);
                assert_eq!(t.remote_id(), *expected);
                assert_eq!(before, t.snapshot());
            }
        },
    );
}
