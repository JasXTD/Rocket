#![cfg(windows)]
#[allow(dead_code)]
mod common;
use common::{Fixture, isolated};
use rocket_git::*;
use std::{
    fs,
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    thread,
    time::{Duration, Instant},
};
const TOKEN: &str = "disposable_PUBLISH_TOKEN_never_log";
type PostAction = Option<Box<dyn FnOnce() + Send>>;
struct Api {
    port: u16,
    mode: Arc<AtomicUsize>,
    posts: Arc<AtomicUsize>,
    bodies: Arc<Mutex<Vec<String>>>,
    action: Arc<Mutex<PostAction>>,
    stop: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}
impl Api {
    fn new() -> Self {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        let mode = Arc::new(AtomicUsize::new(0));
        let posts = Arc::new(AtomicUsize::new(0));
        let bodies = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let action: Arc<Mutex<PostAction>> = Arc::default();
        let (m, p, b, a, done) = (
            mode.clone(),
            posts.clone(),
            bodies.clone(),
            action.clone(),
            stop.clone(),
        );
        let worker = thread::spawn(move || {
            for stream in l.incoming() {
                if done.load(Ordering::Acquire) {
                    break;
                }
                let mut stream = stream.unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let mut head = vec![];
                let mut byte = [0];
                while !head.ends_with(b"\r\n\r\n") && head.len() < 65536 {
                    if stream.read(&mut byte).unwrap_or(0) != 1 {
                        break;
                    }
                    head.push(byte[0]);
                }
                let head = String::from_utf8(head).unwrap();
                let line = head.lines().next().unwrap_or("");
                let length = head
                    .lines()
                    .find_map(|s| {
                        s.to_lowercase()
                            .strip_prefix("content-length:")
                            .and_then(|n| n.trim().parse::<usize>().ok())
                    })
                    .unwrap_or(0);
                assert!(length < 65536);
                let mut body = vec![0; length];
                if stream.read_exact(&mut body).is_err() {
                    continue;
                }
                assert!(head.contains(TOKEN));
                let body = String::from_utf8(body).unwrap();
                let n = m.load(Ordering::Acquire);
                let (code, reply) = if line.starts_with("GET /github/user ") {
                    (200, "{\"login\":\"fixture\"}".into())
                } else if line.starts_with("GET /github/orgs/") {
                    (200, "{\"login\":\"team\"}".into())
                } else if line.starts_with("GET ") {
                    if n == 4 {
                        (200, "{}".into())
                    } else {
                        (404, "{}".into())
                    }
                } else {
                    p.fetch_add(1, Ordering::AcqRel);
                    b.lock().unwrap().push(format!("{line}\n{body}"));
                    if let Some(action) = a.lock().unwrap().take() {
                        action();
                    }
                    while m.load(Ordering::Acquire) == 5 && !done.load(Ordering::Acquire) {
                        thread::sleep(Duration::from_millis(5));
                    }
                    match n {
                        1 => (403, format!("{{\"message\":\"{TOKEN}\"}}")),
                        2 => (422, format!("{{\"message\":\"{TOKEN}\"}}")),
                        3 => (
                            201,
                            "{\"id\":99,\"full_name\":\"other/demo\",\"private\":false}".into(),
                        ),
                        _ => {
                            let owner = if line.contains("/orgs/team/") {
                                "team"
                            } else {
                                "fixture"
                            };
                            (
                                201,
                                format!(
                                    "{{\"id\":99,\"full_name\":\"{owner}/demo\",\"private\":{}}}",
                                    body.contains("\"private\":true")
                                ),
                            )
                        }
                    }
                };
                let reply = format!(
                    "HTTP/1.1 {code} Fixture\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{reply}",
                    reply.len()
                );
                let _ = stream.write_all(reply.as_bytes());
            }
        });
        Self {
            port,
            mode,
            posts,
            bodies,
            action,
            stop,
            thread: Some(worker),
        }
    }
    fn session(&self, f: &Fixture) -> ForgeSession {
        f.git
            .forge_session(
                ForgeConfig::new(ForgeProvider::GitHub, "https://github.com")
                    .unwrap()
                    .with_fixture(self.port)
                    .unwrap(),
                "",
                TOKEN,
                &CancellationToken::new(),
            )
            .unwrap()
            .with_publish_fixture_root(&f.root.join("remotes"))
            .unwrap()
    }
}
impl Drop for Api {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        let _ = TcpStream::connect(("127.0.0.1", self.port));
        self.thread.take().unwrap().join().unwrap();
    }
}
fn fixture() -> Fixture {
    let f = Fixture::new();
    f.write("資料.txt", "committed\n");
    f.commit("Publish fixture");
    fs::create_dir(f.root.join("remotes")).unwrap();
    f.run(&[
        "init",
        "--bare",
        f.root.join("remotes/demo.git").to_str().unwrap(),
    ]);
    f
}
fn draft() -> GitHubPublishDraft {
    GitHubPublishDraft {
        owner: "fixture".into(),
        name: "demo".into(),
        description: "Synthetic description 界".into(),
        private: true,
        branch: "main".into(),
        remote: "origin".into(),
        transport: GitHubPublishTransport::Https,
        set_upstream: true,
    }
}
fn untouched(f: &Fixture) -> Vec<Vec<u8>> {
    vec![
        f.run(&["rev-parse", "HEAD"]),
        fs::read(f.repository.join(".git/index")).unwrap(),
        fs::read(f.repository.join("資料.txt")).unwrap(),
    ]
}
fn local_config(f: &Fixture) -> Vec<u8> {
    fs::read(f.repository.join(".git/config")).unwrap()
}

#[test]
fn publish_private_preserves_dirty_unicode_and_rejects_replay() {
    isolated(
        "ROCKET_PUBLISH_CASE",
        "publish_private_preserves_dirty_unicode_and_rejects_replay",
        || {
            let f = fixture();
            let api = Api::new();
            let s = api.session(&f);
            let repo = f.repo();
            let c = CancellationToken::new();
            f.run(&["branch", "-m", "feature/資料"]);
            f.write("資料.txt", "staged\n");
            f.run(&["add", "--", "資料.txt"]);
            f.write("資料.txt", "unstaged\n");
            f.write("untracked.txt", "keep me");
            let before = untouched(&f);
            let mut d = draft();
            d.branch = "feature/資料".into();
            let p = repo.preview_github_publish(&s, d, &c).unwrap();
            let report = repo
                .publish_github(&s, &p, &GitHubPublishProgress::default(), &c)
                .unwrap();
            assert_eq!(report.creation, GitHubCreation::Created);
            assert!(report.remote_added);
            assert!(report.error.is_none(), "{:?}", report.error);
            assert_eq!(
                report.push.as_ref().unwrap().remote_outcome,
                PushRemoteOutcome::Confirmed
            );
            assert!(report.push.unwrap().upstream.unwrap().is_ok());
            assert_eq!(untouched(&f), before);
            assert_eq!(
                fs::read(f.repository.join("untracked.txt")).unwrap(),
                b"keep me"
            );
            assert_eq!(
                f.run_at(
                    &f.root.join("remotes/demo.git"),
                    &["rev-parse", "refs/heads/feature/資料"]
                ),
                before[0]
            );
            let err = repo
                .publish_github(&s, &p, &GitHubPublishProgress::default(), &c)
                .unwrap_err();
            assert!(err.to_string().contains("publish_stale"));
            assert_eq!(api.posts.load(Ordering::Acquire), 1);
            let body = &api.bodies.lock().unwrap()[0];
            assert!(body.contains("POST /github/user/repos"));
            assert!(body.contains("\"auto_init\":false"));
            assert!(body.contains("\"private\":true"));
        },
    );
}

#[test]
fn publish_refuses_existing_remote_repository_and_url_rewrite() {
    isolated(
        "ROCKET_PUBLISH_CASE",
        "publish_refuses_existing_remote_repository_and_url_rewrite",
        || {
            let f = fixture();
            let api = Api::new();
            let s = api.session(&f);
            let repo = f.repo();
            let c = CancellationToken::new();
            f.run(&[
                "remote",
                "add",
                "origin",
                f.root.join("remotes/demo.git").to_str().unwrap(),
            ]);
            let config = local_config(&f);
            assert!(
                repo.preview_github_publish(&s, draft(), &c)
                    .unwrap_err()
                    .to_string()
                    .contains("publish_remote_exists")
            );
            assert_eq!(local_config(&f), config);
            let mut d = draft();
            d.remote = "new-origin".into();
            api.mode.store(4, Ordering::Release);
            assert!(
                repo.preview_github_publish(&s, d.clone(), &c)
                    .unwrap_err()
                    .to_string()
                    .contains("publish_repository_exists")
            );
            api.mode.store(0, Ordering::Release);
            f.run(&[
                "config",
                "url.https://elsewhere.invalid/.insteadOf",
                "https://github.com/",
            ]);
            assert!(
                repo.preview_github_publish(&s, d, &c)
                    .unwrap_err()
                    .to_string()
                    .contains("publish_url_rewrite")
            );
            assert_eq!(api.posts.load(Ordering::Acquire), 0);
        },
    );
}

#[test]
fn publish_stale_branch_config_and_cancel_before_submission() {
    isolated(
        "ROCKET_PUBLISH_CASE",
        "publish_stale_branch_config_and_cancel_before_submission",
        || {
            let f = fixture();
            let api = Api::new();
            let s = api.session(&f);
            let repo = f.repo();
            let c = CancellationToken::new();
            let p = repo.preview_github_publish(&s, draft(), &c).unwrap();
            f.write("another.txt", "new commit");
            f.commit("External commit");
            assert!(
                repo.publish_github(&s, &p, &GitHubPublishProgress::default(), &c)
                    .unwrap_err()
                    .to_string()
                    .contains("publish_stale")
            );
            let p = repo.preview_github_publish(&s, draft(), &c).unwrap();
            f.run(&["config", "example.changed", "true"]);
            assert!(
                repo.publish_github(&s, &p, &GitHubPublishProgress::default(), &c)
                    .unwrap_err()
                    .to_string()
                    .contains("publish_stale")
            );
            let p = repo.preview_github_publish(&s, draft(), &c).unwrap();
            let stopped = CancellationToken::new();
            stopped.cancel();
            assert!(matches!(
                repo.publish_github(&s, &p, &GitHubPublishProgress::default(), &stopped),
                Err(Error::Cancelled)
            ));
            assert_eq!(api.posts.load(Ordering::Acquire), 0);
        },
    );
}

#[test]
fn publish_creation_errors_and_mismatched_response_do_not_attach() {
    isolated(
        "ROCKET_PUBLISH_CASE",
        "publish_creation_errors_and_mismatched_response_do_not_attach",
        || {
            let f = fixture();
            let api = Api::new();
            let s = api.session(&f);
            let repo = f.repo();
            let c = CancellationToken::new();
            let config = local_config(&f);
            let before = untouched(&f);
            for mode in [1, 2, 3] {
                api.mode.store(mode, Ordering::Release);
                let p = repo.preview_github_publish(&s, draft(), &c).unwrap();
                let r = repo
                    .publish_github(&s, &p, &GitHubPublishProgress::default(), &c)
                    .unwrap();
                assert_eq!(
                    r.creation,
                    if mode == 3 {
                        GitHubCreation::Unknown
                    } else {
                        GitHubCreation::Rejected
                    }
                );
                assert!(!r.remote_added);
                assert!(r.push.is_none());
                assert!(!r.error.unwrap().to_string().contains(TOKEN));
                assert_eq!(local_config(&f), config);
                assert_eq!(untouched(&f), before);
            }
        },
    );
}

#[test]
fn publish_external_edit_after_creation_keeps_partial_result() {
    isolated(
        "ROCKET_PUBLISH_CASE",
        "publish_external_edit_after_creation_keeps_partial_result",
        || {
            let f = fixture();
            let api = Api::new();
            let s = api.session(&f);
            let repo = f.repo();
            let c = CancellationToken::new();
            let p = repo.preview_github_publish(&s, draft(), &c).unwrap();
            let config = f.repository.join(".git/config");
            let mut expected = fs::read(&config).unwrap();
            expected.extend_from_slice(b"\n[external]\n\tkeep = true\n");
            let copy = expected.clone();
            *api.action.lock().unwrap() = Some(Box::new(move || fs::write(config, copy).unwrap()));
            let r = repo
                .publish_github(&s, &p, &GitHubPublishProgress::default(), &c)
                .unwrap();
            assert_eq!(r.creation, GitHubCreation::Created);
            assert!(!r.remote_added);
            assert!(r.error.unwrap().to_string().contains("publish_stale"));
            assert_eq!(local_config(&f), expected);
            assert!(r.push.is_none());
        },
    );
}

#[test]
fn publish_cancelled_create_reports_unknown_without_retry() {
    isolated(
        "ROCKET_PUBLISH_CASE",
        "publish_cancelled_create_reports_unknown_without_retry",
        || {
            let f = fixture();
            let api = Api::new();
            let s = api.session(&f);
            let repo = f.repo();
            let c = CancellationToken::new();
            let p = repo.preview_github_publish(&s, draft(), &c).unwrap();
            let config = local_config(&f);
            api.mode.store(5, Ordering::Release);
            thread::scope(|scope| {
                let worker = scope.spawn(|| {
                    repo.publish_github(&s, &p, &GitHubPublishProgress::default(), &c)
                        .unwrap()
                });
                let start = Instant::now();
                while api.posts.load(Ordering::Acquire) == 0 {
                    assert!(start.elapsed() < Duration::from_secs(20));
                    thread::sleep(Duration::from_millis(5));
                }
                c.cancel();
                let r = worker.join().unwrap();
                api.mode.store(0, Ordering::Release);
                assert_eq!(r.creation, GitHubCreation::Unknown);
                assert!(!r.remote_added);
                assert!(r.push.is_none());
                assert!(
                    r.error
                        .unwrap()
                        .to_string()
                        .contains("publish_creation_unknown")
                );
            });
            assert_eq!(local_config(&f), config);
            assert_eq!(api.posts.load(Ordering::Acquire), 1);
        },
    );
}

#[test]
fn publish_first_push_cannot_replace_concurrent_remote_branch() {
    isolated(
        "ROCKET_PUBLISH_CASE",
        "publish_first_push_cannot_replace_concurrent_remote_branch",
        || {
            let f = fixture();
            let api = Api::new();
            let s = api.session(&f);
            let repo = f.repo();
            let c = CancellationToken::new();
            let original = String::from_utf8(f.run(&["rev-parse", "HEAD"])).unwrap();
            f.write("concurrent.txt", "remote history");
            f.commit("Concurrent remote commit");
            let remote = f.root.join("remotes/demo.git");
            f.run(&["push", remote.to_str().unwrap(), "HEAD:refs/heads/main"]);
            let remote_before = f.run_at(&remote, &["rev-parse", "main"]);
            f.run(&["update-ref", "refs/heads/main", original.trim()]);
            let p = repo.preview_github_publish(&s, draft(), &c).unwrap();
            let r = repo
                .publish_github(&s, &p, &GitHubPublishProgress::default(), &c)
                .unwrap();
            assert_eq!(r.creation, GitHubCreation::Created);
            assert!(r.remote_added);
            assert_eq!(f.run_at(&remote, &["rev-parse", "main"]), remote_before);
            assert_eq!(r.push.unwrap().remote_outcome, PushRemoteOutcome::Rejected);
        },
    );
}

#[test]
fn publish_config_lock_and_existing_upstream_are_preserved() {
    isolated(
        "ROCKET_PUBLISH_CASE",
        "publish_config_lock_and_existing_upstream_are_preserved",
        || {
            let f = fixture();
            let api = Api::new();
            let s = api.session(&f);
            let repo = f.repo();
            let c = CancellationToken::new();
            let lock = f.repository.join(".git/config.lock");
            fs::write(&lock, b"owned by another process").unwrap();
            assert!(
                repo.preview_github_publish(&s, draft(), &c)
                    .unwrap_err()
                    .to_string()
                    .contains("publish_config_locked")
            );
            assert_eq!(fs::read(&lock).unwrap(), b"owned by another process");
            fs::remove_file(&lock).unwrap();
            f.run(&["config", "branch.main.remote", "existing"]);
            f.run(&["config", "branch.main.merge", "refs/heads/main"]);
            assert!(
                repo.preview_github_publish(&s, draft(), &c)
                    .unwrap_err()
                    .to_string()
                    .contains("publish_upstream_exists")
            );
            let mut d = draft();
            d.set_upstream = false;
            let p = repo.preview_github_publish(&s, d, &c).unwrap();
            let r = repo
                .publish_github(&s, &p, &GitHubPublishProgress::default(), &c)
                .unwrap();
            assert_eq!(r.push.unwrap().remote_outcome, PushRemoteOutcome::Confirmed);
            assert_eq!(f.run(&["config", "branch.main.remote"]), b"existing\n");
        },
    );
}

#[test]
fn publish_org_public_and_secret_history_preflight() {
    isolated(
        "ROCKET_PUBLISH_CASE",
        "publish_org_public_and_secret_history_preflight",
        || {
            let f = fixture();
            let api = Api::new();
            let s = api.session(&f);
            let repo = f.repo();
            let c = CancellationToken::new();
            let mut d = draft();
            d.owner = "team".into();
            d.private = false;
            d.transport = GitHubPublishTransport::Ssh;
            let p = repo.preview_github_publish(&s, d, &c).unwrap();
            let r = repo
                .publish_github(&s, &p, &GitHubPublishProgress::default(), &c)
                .unwrap();
            assert_eq!(r.push.unwrap().remote_outcome, PushRemoteOutcome::Confirmed);
            let body = &api.bodies.lock().unwrap()[0];
            assert!(body.contains("POST /github/orgs/team/repos"));
            assert!(body.contains("\"private\":false"));
            f.write(
                "secret.rs",
                format!("const ACCESS: &str = \"ghp_{}\";", "a".repeat(36)),
            );
            f.commit("Synthetic leaked fixture");
            let mut d = draft();
            d.remote = "new-origin".into();
            d.set_upstream = false;
            assert!(matches!(
                repo.preview_github_publish(&s, d, &c),
                Err(Error::SecretProtection(_))
            ));
            assert_eq!(api.posts.load(Ordering::Acquire), 1);
        },
    );
}
