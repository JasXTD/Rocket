mod common;
use common::{Fixture, isolated};
use rocket_git::*;
use std::{
    fs,
    path::PathBuf,
    thread,
    time::{Duration, Instant},
};
fn c() -> CancellationToken {
    CancellationToken::new()
}
struct Repos {
    f: Fixture,
    remote: PathBuf,
    base: String,
}
impl Repos {
    fn new(format: &str) -> Self {
        let f = Fixture::with_format(format);
        f.write("文.txt", "base\n");
        f.commit("PRIVATE base");
        let base = String::from_utf8(f.run(&["rev-parse", "HEAD"]))
            .unwrap()
            .trim()
            .to_owned();
        f.run(&["branch", "feature/文"]);
        let remote = f.root.join("PRIVATE upstream.git");
        f.run(&[
            "clone",
            "--bare",
            f.repository.to_str().unwrap(),
            remote.to_str().unwrap(),
        ]);
        f.run(&["remote", "add", "origin", remote.to_str().unwrap()]);
        f.run(&["fetch", "origin"]);
        f.write("文.txt", "main update\n");
        f.commit("PRIVATE main");
        f.run(&["switch", "feature/文"]);
        f.write("feature.txt", "feature update\n");
        f.commit("PRIVATE feature");
        f.run(&["switch", "main"]);
        Self { f, remote, base }
    }
    fn selection() -> Vec<AtomicPushSelection> {
        vec![
            AtomicPushSelection {
                local_branch: b"main".to_vec(),
                remote_branch: b"main".to_vec(),
            },
            AtomicPushSelection {
                local_branch: "feature/文".as_bytes().to_vec(),
                remote_branch: "feature/文".as_bytes().to_vec(),
            },
        ]
    }
    fn preview(&self) -> AtomicPushPreview {
        self.f
            .repo()
            .preview_atomic_push(b"origin", &Self::selection(), &c())
            .unwrap()
    }
    fn refs(&self) -> Vec<u8> {
        self.f.run_at(
            &self.remote,
            &["for-each-ref", "--format=%(refname) %(objectname)"],
        )
    }
    fn hook(&self, server: bool, body: &str) {
        let dir = if server {
            self.remote.join("hooks")
        } else {
            self.f.root.join("no-hooks")
        };
        fs::create_dir_all(&dir).unwrap();
        if server {
            self.f.run_at(
                &self.remote,
                &["config", "core.hooksPath", dir.to_str().unwrap()],
            );
        }
        let p = dir.join(if server { "update" } else { "pre-push" });
        fs::write(&p, format!("#!/bin/sh\n{body}\n")).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(p, fs::Permissions::from_mode(0o755)).unwrap();
        }
    }
}
fn posix(p: &std::path::Path) -> String {
    p.to_string_lossy()
        .replace('\\', "/")
        .replace('\'', "'\\''")
}
#[test]
fn atomic_success_new_up_to_date_unicode_and_dirty_files() {
    isolated(
        "ROCKET_ATOMIC",
        "atomic_success_new_up_to_date_unicode_and_dirty_files",
        || {
            for format in ["sha1", "sha256"] {
                let r = Repos::new(format);
                r.f.run_at(&r.remote, &["update-ref", "-d", "refs/heads/feature/文"]);
                r.f.write("文.txt", "PRIVATE staged\n");
                r.f.run(&["add", "文.txt"]);
                r.f.write("文.txt", "PRIVATE unstaged\n");
                r.f.write("untracked.txt", "PRIVATE untracked");
                let index = fs::read(r.f.repository.join(".git/index")).unwrap();
                let refs =
                    r.f.run(&["for-each-ref", "--format=%(refname) %(objectname)"]);
                let p = r.preview();
                assert_eq!(p.entries()[0].kind, AtomicPushKind::FastForward);
                assert_eq!(p.entries()[1].kind, AtomicPushKind::Create);
                let report = r.f.repo().apply_atomic_push(&p, &c()).unwrap();
                assert!(report.result.is_ok(), "{:?}", report.result);
                assert!(report.verification.is_ok());
                assert_eq!(report.remote_state, AtomicPushRemoteState::Published);
                assert_eq!(fs::read(r.f.repository.join(".git/index")).unwrap(), index);
                assert_eq!(
                    r.f.run(&["for-each-ref", "--format=%(refname) %(objectname)"]),
                    refs
                );
                assert_eq!(
                    fs::read_to_string(r.f.repository.join("文.txt")).unwrap(),
                    "PRIVATE unstaged\n"
                );
                assert!(
                    r.preview()
                        .entries()
                        .iter()
                        .all(|e| e.kind == AtomicPushKind::UpToDate)
                );
                assert!(
                    r.f.repo()
                        .apply_atomic_push(&r.preview(), &c())
                        .unwrap()
                        .result
                        .is_ok()
                );
            }
        },
    )
}
#[test]
fn one_server_rejection_and_unsupported_atomic_leave_every_ref_unchanged() {
    isolated(
        "ROCKET_ATOMIC",
        "one_server_rejection_and_unsupported_atomic_leave_every_ref_unchanged",
        || {
            let r = Repos::new("sha1");
            let before = r.refs();
            r.hook(
                true,
                "case \"$1\" in refs/heads/feature/*) echo PRIVATE-reject >&2; exit 1;; esac",
            );
            let report = r.f.repo().apply_atomic_push(&r.preview(), &c()).unwrap();
            assert!(report.result.is_err());
            assert_eq!(report.remote_state, AtomicPushRemoteState::Unchanged);
            assert_eq!(r.refs(), before);
            fs::remove_file(r.remote.join("hooks/update")).unwrap();
            r.f.run_at(&r.remote, &["config", "receive.advertiseAtomic", "false"]);
            let report = r.f.repo().apply_atomic_push(&r.preview(), &c()).unwrap();
            assert!(matches!(
                report.result,
                Err(Error::AtomicPush(AtomicPushFailure::Unsupported))
            ));
            assert_eq!(r.refs(), before);
        },
    )
}
#[test]
fn stale_sources_destinations_and_remote_tips_refuse_all() {
    isolated(
        "ROCKET_ATOMIC",
        "stale_sources_destinations_and_remote_tips_refuse_all",
        || {
            let r = Repos::new("sha1");
            let p = r.preview();
            let before = r.refs();
            r.f.write("external.txt", "external");
            r.f.commit("external");
            assert!(matches!(
                r.f.repo().apply_atomic_push(&p, &c()),
                Err(Error::AtomicPush(AtomicPushFailure::StalePreview))
            ));
            assert_eq!(r.refs(), before);
            let p = r.preview();
            r.f.run(&["config", "push.followTags", "true"]);
            assert!(matches!(
                r.f.repo().apply_atomic_push(&p, &c()),
                Err(Error::AtomicPush(AtomicPushFailure::StalePreview))
            ));
            let p = r.preview();
            r.f.run_at(&r.remote, &["update-ref", "-d", "refs/heads/feature/文"]);
            let after = r.refs();
            assert!(matches!(
                r.f.repo().apply_atomic_push(&p, &c()),
                Err(Error::AtomicPush(AtomicPushFailure::RemoteChanged))
            ));
            assert_eq!(r.refs(), after);
        },
    )
}
#[test]
fn non_fast_forward_duplicate_mapping_and_racing_remote_are_protected() {
    isolated(
        "ROCKET_ATOMIC",
        "non_fast_forward_duplicate_mapping_and_racing_remote_are_protected",
        || {
            let r = Repos::new("sha1");
            let mut selection = Repos::selection();
            selection[1].remote_branch = b"main".to_vec();
            assert!(matches!(
                r.f.repo().preview_atomic_push(b"origin", &selection, &c()),
                Err(Error::AtomicPush(AtomicPushFailure::Selection))
            ));
            r.f.run(&["push", "origin", "main"]);
            r.f.run(&["reset", "--hard", &r.base]);
            r.f.write("different.txt", "different");
            r.f.commit("diverged");
            let p = r.preview();
            assert_eq!(p.entries()[0].kind, AtomicPushKind::NonFastForward);
            assert!(!p.can_apply());
            let before = r.refs();
            assert!(matches!(
                r.f.repo().apply_atomic_push(&p, &c()),
                Err(Error::AtomicPush(AtomicPushFailure::Blocked))
            ));
            assert_eq!(r.refs(), before);
            let r = Repos::new("sha1");
            let p = r.preview();
            let topic_before =
                r.f.run_at(&r.remote, &["rev-parse", "refs/heads/feature/文"]);
            r.hook(
                false,
                &format!(
                    "git --git-dir='{}' update-ref -d refs/heads/main",
                    posix(&r.remote)
                ),
            );
            let report = r.f.repo().apply_atomic_push(&p, &c()).unwrap();
            assert!(report.result.is_err());
            assert_eq!(
                r.f.run_at(&r.remote, &["rev-parse", "refs/heads/feature/文"]),
                topic_before
            );
            assert_eq!(report.remote_state, AtomicPushRemoteState::Different);
        },
    )
}
#[test]
fn running_cancel_stops_hook_and_observes_every_branch() {
    isolated(
        "ROCKET_ATOMIC",
        "running_cancel_stops_hook_and_observes_every_branch",
        || {
            let r = Repos::new("sha1");
            let marker = r.f.root.join("hook-started");
            r.hook(
                false,
                &format!(
                    "printf started > '{}'\nwhile :; do sleep 1; done",
                    posix(&marker)
                ),
            );
            let p = r.preview();
            let before = r.refs();
            let token = c();
            let worker = token.clone();
            let repo = r.f.repo();
            let task = thread::spawn(move || repo.apply_atomic_push(&p, &worker));
            let start = Instant::now();
            while !marker.exists() {
                assert!(start.elapsed() < Duration::from_secs(30));
                thread::sleep(Duration::from_millis(10));
            }
            let stop = Instant::now();
            token.cancel();
            let report = task.join().unwrap().unwrap();
            assert!(matches!(report.result, Err(Error::Cancelled)));
            assert!(stop.elapsed() < Duration::from_secs(10));
            assert_eq!(report.remote_state, AtomicPushRemoteState::Unchanged);
            assert_eq!(r.refs(), before);
        },
    )
}
