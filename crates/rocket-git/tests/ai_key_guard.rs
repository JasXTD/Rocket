mod common;
use common::{Fixture, isolated};
use rocket_git::*;

fn c() -> CancellationToken {
    CancellationToken::new()
}

fn base64(bytes: &[u8]) -> String {
    let alphabet = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut text = String::new();
    for group in bytes.chunks(3) {
        let n = group.iter().fold(0u32, |n, b| n << 8 | u32::from(*b)) << (8 * (3 - group.len()));
        for index in 0..4 {
            text.push(if index <= group.len() {
                alphabet[(n >> (18 - 6 * index) & 63) as usize] as char
            } else {
                '='
            });
        }
    }
    text
}

// Generated, deliberately recognizable fixtures; never contact an AI provider.
fn keys() -> Vec<String> {
    let hex = "0123456789abcdef";
    // An Amazon Bedrock long-term key encodes user-at-account:secret.
    let secret = base64(&"Ab9Cd7Ef5Gh3Ij1Kl0Mn".repeat(3).as_bytes()[..44]);
    let identity = format!("BedrockAPIKey-k3z9-at-{}:{secret}", "123456789012");
    vec![
        format!(
            "sk-{}T3BlbkFJ{}",
            "Ab9Cd7Ef5Gh3Ij1Kl0Mn", "Op8Qr6St4Uv2Wx0Yz9Ab"
        ),
        format!("sk-{}", hex.repeat(2)),
        format!("{}.{}", hex.repeat(2), "Ab9Cd7Ef5Gh3Ij1K"),
        format!("ABSK{}", base64(identity.as_bytes())),
    ]
}

fn assert_blocked(error: &Error, key: &str) {
    let Error::SecretProtection(failure) = error else {
        panic!("unexpected error variant")
    };
    assert_eq!(failure.code(), "ai_api_key_detected");
    assert!(error.to_string().contains("AI-provider API key"));
    for text in [error.to_string(), format!("{error:?}")] {
        assert!(!text.contains(key));
    }
}

#[test]
fn ai_guard_stays_on_without_general_guard_and_updates_existing_requests() {
    isolated(
        "ROCKET_AI_GUARD",
        "ai_guard_stays_on_without_general_guard_and_updates_existing_requests",
        || {
            let f = Fixture::new();
            f.write("readme", "clean\n");
            f.commit("base");
            let repo = f.repo();
            let log = DiagnosticLog::open(f.root.join("logs")).unwrap();
            let request = f
                .git
                .for_request(CommandLimits::default(), log.clone())
                .unwrap();
            let request_repo = request.open(&f.repository, &c()).unwrap();
            f.git.set_secret_protection(false);
            for key in keys() {
                f.write("資料/config.py", format!("api_key = \"{key}\"\n"));
                f.run(&["add", "--all"]);
                let head = f.run(&["rev-parse", "HEAD"]);
                let index = f.run(&["ls-files", "--stage", "-z"]);
                assert_blocked(
                    repo.commit("protected", &c())
                        .unwrap()
                        .result
                        .as_ref()
                        .unwrap_err(),
                    &key,
                );
                assert_blocked(
                    request_repo
                        .commit("protected request", &c())
                        .unwrap()
                        .result
                        .as_ref()
                        .unwrap_err(),
                    &key,
                );
                assert_eq!(head, f.run(&["rev-parse", "HEAD"]));
                assert_eq!(index, f.run(&["ls-files", "--stage", "-z"]));
                f.git.set_guard_ai_api_keys(false);
                request_repo
                    .commit("explicit isolated opt-out", &c())
                    .unwrap()
                    .result
                    .unwrap();
                // Re-enable through a clone; old repository handles see the change.
                request.set_guard_ai_api_keys(true);
                f.write("readme", format!("next {}\n", key.len()));
                f.run(&["add", "--all"]);
                assert_blocked(
                    repo.commit("protected again", &c())
                        .unwrap()
                        .result
                        .as_ref()
                        .unwrap_err(),
                    &key,
                );
            }
            drop(request_repo);
            drop(request);
            drop(log);
            for entry in std::fs::read_dir(f.root.join("logs")).unwrap() {
                let path = entry.unwrap().path();
                if path.is_file() {
                    let bytes = std::fs::read(path).unwrap();
                    for key in keys() {
                        assert!(!bytes.windows(key.len()).any(|part| part == key.as_bytes()));
                    }
                }
            }
        },
    );
}

#[test]
fn ai_guard_checks_deleted_history_and_cancellation_without_touching_remote() {
    isolated(
        "ROCKET_AI_GUARD",
        "ai_guard_checks_deleted_history_and_cancellation_without_touching_remote",
        || {
            for format in ["sha1", "sha256"] {
                for key in keys() {
                    let f = Fixture::with_format(format);
                    f.write("readme", "clean\n");
                    f.commit("base");
                    let remote = f.root.join("remote.git");
                    f.run(&[
                        "init",
                        "--bare",
                        "--initial-branch=main",
                        &format!("--object-format={format}"),
                        remote.to_str().unwrap(),
                    ]);
                    f.run(&["remote", "add", "origin", remote.to_str().unwrap()]);
                    f.write("old.py", format!("api_key = '{key}'\n"));
                    f.commit("synthetic old credential");
                    f.run(&["rm", "old.py"]);
                    f.commit("delete later");
                    f.git.set_secret_protection(false);
                    let repo = f.repo();
                    let objects = f.run_at(&remote, &["count-objects", "-v"]);
                    let head = f.run(&["rev-parse", "HEAD"]);
                    let report = repo
                        .push(
                            "origin",
                            "main",
                            "main",
                            PushOptions {
                                set_upstream: false,
                            },
                            &c(),
                        )
                        .unwrap();
                    assert_blocked(report.result.as_ref().unwrap_err(), &key);
                    assert_eq!(report.remote_outcome, PushRemoteOutcome::NotAttempted);
                    let cancel = c();
                    cancel.cancel();
                    assert!(matches!(
                        repo.push(
                            "origin",
                            "main",
                            "main",
                            PushOptions {
                                set_upstream: false
                            },
                            &cancel
                        ),
                        Err(Error::Cancelled)
                    ));
                    assert_eq!(objects, f.run_at(&remote, &["count-objects", "-v"]));
                    assert!(f.run_at(&remote, &["for-each-ref"]).is_empty());
                    assert_eq!(head, f.run(&["rev-parse", "HEAD"]));
                }
            }
        },
    );
}

#[test]
fn ai_guard_blocks_mail_import_before_checkout_changes() {
    isolated(
        "ROCKET_AI_GUARD",
        "ai_guard_blocks_mail_import_before_checkout_changes",
        || {
            let f = Fixture::new();
            f.write("readme", "base\n");
            f.commit("base");
            let base = String::from_utf8(f.run(&["rev-parse", "HEAD"]))
                .unwrap()
                .trim()
                .to_owned();
            let key = keys().remove(2);
            f.write("資料/credentials.py", format!("ZAI_API_KEY = '{key}'\n"));
            f.commit("synthetic patch");
            let folder = f.root.canonicalize().unwrap().join("mail");
            let repo = f.repo();
            let export = repo
                .preview_patch_series(&base, "HEAD", &folder, &c())
                .unwrap();
            repo.export_patch_series(&export, &c()).unwrap();
            f.run(&["reset", "--hard", &base]);
            f.git.set_secret_protection(false);
            let index = f.run(&["ls-files", "--stage", "-z"]);
            assert_blocked(&repo.preview_mail_import(&folder, &c()).unwrap_err(), &key);
            f.git.set_guard_ai_api_keys(false);
            let preview = repo.preview_mail_import(&folder, &c()).unwrap();
            f.git.set_guard_ai_api_keys(true);
            let error = repo.apply_mail_import(&preview, &c()).unwrap_err();
            assert_blocked(&error, &key);
            assert_eq!(index, f.run(&["ls-files", "--stage", "-z"]));
            assert_eq!(base.as_bytes(), f.run(&["rev-parse", "HEAD"]).trim_ascii());
            assert!(!repo.operation_state(&c()).unwrap().mail_import);
            assert!(!f.repository.join("資料/credentials.py").exists());
        },
    );
}
