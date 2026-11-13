mod common;
use common::{Fixture, isolated};
use rocket_git::*;
use std::{fs, path::PathBuf};

fn c() -> CancellationToken {
    CancellationToken::new()
}
fn token() -> String {
    format!("ghp_{}", "A7".repeat(18))
}
fn blocked(error: &Error, code: &str) {
    let Error::SecretProtection(failure) = error else {
        panic!("Expected a safe secret refusal: {error:?}")
    };
    assert_eq!(failure.code(), code);
    assert!(!error.to_string().contains(&token()));
    assert!(!format!("{error:?}").contains(&token()));
    assert!(error.to_string().contains("No "));
    assert!(error.to_string().contains("Do not bypass"));
}
fn base(format: &str) -> (Fixture, PathBuf) {
    let f = Fixture::with_format(format);
    f.write("readme.md", "clean\n");
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
    (f, remote)
}
fn push(f: &Fixture) -> PushReport {
    f.repo()
        .push(
            "origin",
            "main",
            "main",
            PushOptions {
                set_upstream: false,
            },
            &c(),
        )
        .unwrap()
}
fn refs(f: &Fixture, remote: &std::path::Path) -> Vec<u8> {
    f.run_at(
        remote,
        &["for-each-ref", "--format=%(refname) %(objectname)"],
    )
}

fn utf16_fixture(text: &str, little: bool) -> Vec<u8> {
    let mut bytes = if little {
        vec![0xff, 0xfe]
    } else {
        vec![0xfe, 0xff]
    };
    bytes.extend(text.encode_utf16().flat_map(|word| {
        if little {
            word.to_le_bytes()
        } else {
            word.to_be_bytes()
        }
    }));
    bytes
}

#[test]
fn short_marker_constants_can_commit_and_push_unchanged_history() {
    isolated(
        "ROCKET_SECRET",
        "short_marker_constants_can_commit_and_push_unchanged_history",
        || {
            for format in ["sha1", "sha256"] {
                let (f, remote) = base(format);
                let repo = f.repo();
                let source = concat!(
                    "const SECRET",
                    ": &str = \"XD-PRIVATE-7f3a\";\r\n",
                    "pub const token",
                    ": &'static str = r#\"CACHE-PROBE-5ac9d1\"#;\r\n",
                    "export const secret",
                    ": string = 'AUDIT_TRACE_9e20c7a3';\r\n",
                    "const TOKEN",
                    ": &str = \"fixture-token-DO-NOT-LOG\";\r\n",
                    "const secret",
                    " = \"BUILD-SIGNAL-worker-ready\";\r\n",
                    "const TOKEN",
                    ": &str = \"disposable_PUBLISH_TOKEN_never_log\";\r\n",
                    "const TOKEN",
                    ": &str = \"gho_rocketFakeToken_0123456789\";\r\n",
                    "pub const secret",
                    ": &str = \"build_compilerSessionLabel_9876543210\";\r\n",
                );
                f.write("資料/markers.txt", source);
                f.write("資料/markers-utf16.txt", utf16_fixture(source, true));
                repo.stage_all(&c()).unwrap().result.unwrap();
                repo.commit("const secret: &str = \"QUEUE-ITEM-c4f8\";", &c())
                    .unwrap()
                    .result
                    .unwrap();
                let historical = String::from_utf8(f.run(&["rev-parse", "HEAD"])).unwrap();
                f.write("資料/markers.txt", "updated ordinary source\n");
                repo.stage_all(&c()).unwrap().result.unwrap();
                repo.commit("Update current snapshot", &c())
                    .unwrap()
                    .result
                    .unwrap();
                let pushed = push(&f);
                pushed.result.unwrap();
                assert_eq!(pushed.remote_outcome, PushRemoteOutcome::Confirmed);
                let spec = format!("{}:資料/markers.txt", historical.trim());
                assert_eq!(f.run_at(&remote, &["show", &spec]), source.as_bytes());
                assert_eq!(
                    f.run(&["rev-parse", "HEAD"]),
                    f.run_at(&remote, &["rev-parse", "main"])
                );
                // Also inspect the same immutable marker from already-published
                // history during a later push, without rewriting that commit.
                f.write("readme.md", "clean follow-up\n");
                repo.stage_all(&c()).unwrap().result.unwrap();
                repo.commit("Clean follow-up", &c())
                    .unwrap()
                    .result
                    .unwrap();
                push(&f).result.unwrap();
                assert_eq!(f.run_at(&remote, &["show", &spec]), source.as_bytes());
                assert_eq!(
                    f.run(&["rev-parse", "HEAD"]),
                    f.run_at(&remote, &["rev-parse", "main"])
                );
            }
        },
    );
}

#[test]
fn short_marker_context_cannot_hide_staged_or_historical_credentials() {
    isolated(
        "ROCKET_SECRET",
        "short_marker_context_cannot_hide_staged_or_historical_credentials",
        || {
            let (f, remote) = base("sha1");
            let log = DiagnosticLog::open(f.root.join("logs")).unwrap();
            let repo = f
                .git
                .with_log(log.clone())
                .open(&f.repository, &c())
                .unwrap();
            let safe_head = String::from_utf8(f.run(&["rev-parse", "HEAD"])).unwrap();
            let remote_objects = f.run_at(&remote, &["count-objects", "-v"]);
            let opaque = ["N8a4", "fT3pQ7xL9mV2sK5zR0cB6dW1yH"].concat();
            let marker = "EXPORT-CANARY-4b92";
            let word_label = "example-context-NEVER-TRANSMIT";
            let counter = "svc_transferBatchMarker_0123456789";
            for source in [
                format!("const SECRET: &str = \"{opaque}\";"),
                format!("const password: &str = \"{marker}\";"),
                format!("const api_key: &str = \"{marker}\";"),
                format!("const secret: &str = \"{marker}\";\nconst token = \"{opaque}\";"),
                format!("const password = \"{word_label}\";"),
                format!("const api_key = \"{word_label}\";"),
                format!("const token = \"{word_label}-{opaque}\";"),
                format!("const TOKEN: &str = \"{word_label}\";\nconst secret = \"{opaque}\";"),
                format!("const token = \"offline_TRACE_EVENT_ignore_output-{opaque}\";"),
                format!("const password = \"{counter}\";"),
                format!("const api_key = \"{counter}\";"),
                format!("const token = \"{counter}-{opaque}\";"),
                format!(
                    "const TOKEN = \"{}{}\";",
                    "svc_transferBatchMarker_", "0123546789"
                ),
            ] {
                f.write("資料/source.txt", &source);
                repo.stage_all(&c()).unwrap().result.unwrap();
                let index = fs::read(f.repository.join(".git/index")).unwrap();
                let report = repo.commit("Review constant", &c()).unwrap();
                let error = report.result.as_ref().unwrap_err();
                blocked(error, "credential_detected");
                assert!(!format!("{error:?}").contains(&opaque));
                assert!(!format!("{error:?}").contains(marker));
                assert_eq!(f.run(&["rev-parse", "HEAD"]), safe_head.as_bytes());
                assert_eq!(fs::read(f.repository.join(".git/index")).unwrap(), index);

                // Raw Git seeds only this disposable local fixture. A clean tip
                // must not conceal earlier values from Rocket's push scan.
                f.commit("Synthetic historical credential");
                f.run(&["rm", "--", "資料/source.txt"]);
                f.commit("Clean tip");
                let head = f.run(&["rev-parse", "HEAD"]);
                let index = fs::read(f.repository.join(".git/index")).unwrap();
                let report = push(&f);
                blocked(report.result.as_ref().unwrap_err(), "credential_detected");
                assert_eq!(report.remote_outcome, PushRemoteOutcome::NotAttempted);
                assert!(refs(&f, &remote).is_empty());
                assert_eq!(f.run_at(&remote, &["count-objects", "-v"]), remote_objects);
                assert_eq!(f.run(&["rev-parse", "HEAD"]), head);
                assert_eq!(fs::read(f.repository.join(".git/index")).unwrap(), index);
                f.run(&["reset", "--hard", safe_head.trim()]);
            }
            drop(repo);
            drop(log);
            let mut recorded_refusal = false;
            for entry in fs::read_dir(f.root.join("logs")).unwrap() {
                let path = entry.unwrap().path();
                if path.is_file() {
                    let text = fs::read_to_string(path).unwrap();
                    recorded_refusal |= text.contains("credential_detected");
                    assert!(!text.contains(&opaque));
                    assert!(!text.contains(marker));
                    assert!(!text.contains(word_label));
                    assert!(!text.contains(counter));
                    assert!(!text.contains("source.txt"));
                }
            }
            assert!(recorded_refusal);
        },
    );
}

#[test]
fn protocol_identifiers_can_commit_and_push_local_and_remote_history() {
    isolated(
        "ROCKET_SECRET",
        "protocol_identifiers_can_commit_and_push_local_and_remote_history",
        || {
            for format in ["sha1", "sha256"] {
                let (f, remote) = base(format);
                let repo = f.repo();
                let old = concat!(
                    "let token = \"IDM_EX_AV_REQUESTED_V1:malwarebytes\";\n",
                    "let token = \"UPLOAD_REQUEST_ACCEPTED_V27:file.sync\";\n",
                    "let token = \"CACHE_STATUS_READY:local_worker\";\n",
                    "let token = \"IDM_EX_AV_REQUESTED_V1:malwarebytes\\n\";\n",
                    "let token = \"UPLOAD_REQUEST_ACCEPTED_V27:file.sync\\r\\n\";\n",
                    "let token: &str = \"JOB_RUN_FINISHED:worker.pool\";\n",
                    "let token = r#\"NETWORK_REQUEST_ACCEPTED_V12:local_client\"#;\n",
                    "token",
                    " := `TELEMETRY_EVENT_ACKNOWLEDGED_V23:telemetry`\n",
                );
                f.write("acknowledgment.rs", old);
                repo.stage_all(&c()).unwrap().result.unwrap();
                repo.commit(
                    "Record token = \"PREVIEW_SESSION_STARTED_V2026:another-engine\"",
                    &c(),
                )
                .unwrap()
                .result
                .unwrap();
                let old_commit = f.run(&["rev-parse", "HEAD"]);
                f.write("acknowledgment.rs", old.replace("token", "acknowledgment"));
                repo.stage_all(&c()).unwrap().result.unwrap();
                repo.commit("Rename acknowledgment", &c())
                    .unwrap()
                    .result
                    .unwrap();
                let pushed = push(&f);
                pushed.result.unwrap();
                assert_eq!(pushed.remote_outcome, PushRemoteOutcome::Confirmed);
                assert_eq!(
                    f.run(&["rev-parse", "HEAD"]),
                    f.run_at(&remote, &["rev-parse", "refs/heads/main"])
                );
                let old_path = format!(
                    "{}:acknowledgment.rs",
                    String::from_utf8(old_commit).unwrap().trim()
                );
                assert_eq!(
                    f.run_at(&remote, &["show", &old_path]),
                    old.as_bytes(),
                    "The original commit is retained"
                );

                // A later push must also pass with protocol identifiers already
                // reachable on the destination. Nothing rewrites that history.
                f.write("readme.md", "another clean update\n");
                repo.stage_all(&c()).unwrap().result.unwrap();
                repo.commit("Clean follow-up", &c())
                    .unwrap()
                    .result
                    .unwrap();
                let pushed = push(&f);
                pushed.result.unwrap();
                assert_eq!(pushed.remote_outcome, PushRemoteOutcome::Confirmed);
                assert_eq!(
                    f.run(&["rev-parse", "HEAD"]),
                    f.run_at(&remote, &["rev-parse", "refs/heads/main"])
                );
            }
        },
    );
}

#[test]
fn protocol_identifiers_cannot_hide_staged_or_historical_credentials() {
    isolated(
        "ROCKET_SECRET",
        "protocol_identifiers_cannot_hide_staged_or_historical_credentials",
        || {
            let (f, remote) = base("sha1");
            let repo = f.repo();
            let acknowledgment = "let token = \"UPLOAD_REQUEST_ACCEPTED_V27:local_worker\\n\";\n";
            let credential = "A9mK7pR4sT2vX8n6Q5bC3z";
            let mixed = format!("{acknowledgment}let password = \"{credential}\";\n");
            f.write("acknowledgment.rs", &mixed);
            repo.stage_all(&c()).unwrap().result.unwrap();
            let head = f.run(&["rev-parse", "HEAD"]);
            let index = fs::read(f.repository.join(".git/index")).unwrap();
            let report = repo.commit("Store acknowledgment", &c()).unwrap();
            let error = report.result.as_ref().unwrap_err();
            blocked(error, "credential_detected");
            assert!(!error.to_string().contains(credential));
            assert_eq!(f.run(&["rev-parse", "HEAD"]), head);
            assert_eq!(fs::read(f.repository.join(".git/index")).unwrap(), index);

            // Seed only the local fixture. Fixing its latest snapshot must not
            // hide the unrelated credential in the earlier immutable object.
            f.commit("Synthetic historical credential beside acknowledgment");
            let old_commit = f.run(&["rev-parse", "HEAD"]);
            f.write("acknowledgment.rs", acknowledgment);
            repo.stage_all(&c()).unwrap().result.unwrap();
            repo.commit("Keep only acknowledgment", &c())
                .unwrap()
                .result
                .unwrap();
            let remote_objects = f.run_at(&remote, &["count-objects", "-v"]);
            let pushed = push(&f);
            blocked(pushed.result.as_ref().unwrap_err(), "credential_detected");
            assert_eq!(pushed.remote_outcome, PushRemoteOutcome::NotAttempted);
            assert!(refs(&f, &remote).is_empty());
            assert_eq!(f.run_at(&remote, &["count-objects", "-v"]), remote_objects);

            let spec = format!(
                "{}:refs/heads/main",
                String::from_utf8(old_commit).unwrap().trim()
            );
            f.run(&["push", "origin", &spec]);
            let before = refs(&f, &remote);
            let remote_objects = f.run_at(&remote, &["count-objects", "-v"]);
            let pushed = push(&f);
            blocked(pushed.result.as_ref().unwrap_err(), "credential_detected");
            assert_eq!(pushed.remote_outcome, PushRemoteOutcome::NotAttempted);
            assert_eq!(refs(&f, &remote), before);
            assert_eq!(f.run_at(&remote, &["count-objects", "-v"]), remote_objects);
        },
    );
}

#[test]
fn typed_raw_and_shorthand_credentials_refuse_commit_and_historical_push() {
    isolated(
        "ROCKET_SECRET",
        "typed_raw_and_shorthand_credentials_refuse_commit_and_historical_push",
        || {
            let (f, remote) = base("sha1");
            let log = DiagnosticLog::open(f.root.join("logs")).unwrap();
            let repo = f
                .git
                .with_log(log.clone())
                .open(&f.repository, &c())
                .unwrap();
            let safe_head = String::from_utf8(f.run(&["rev-parse", "HEAD"])).unwrap();
            let remote_objects = f.run_at(&remote, &["count-objects", "-v"]);
            let candidate = ["J7p9", "aD8cR2nH5sW1yB6mL0vF3kT9uP4xQ7zE"].concat();
            let cases = [
                format!("let token: &str = \"{candidate}\";"),
                format!("let token = r#\"{candidate}\"#;"),
                format!("const token: string = \"{candidate}\";"),
                format!("token: str = '{candidate}'"),
                format!("token = r'{candidate}'"),
                format!("token = b'{candidate}'"),
                format!("token := \"{candidate}\""),
                format!("var token string = \"{candidate}\""),
                format!("token := `{candidate}`"),
                format!("string token = @\"{candidate}\";"),
                format!("val token: String = \"{candidate}\""),
                format!("let token: String = \"{candidate}\""),
            ];
            for (index, source) in cases.iter().enumerate() {
                let source = format!("{source}\r\n");
                let bytes = match index % 3 {
                    0 => source.into_bytes(),
                    1 => utf16_fixture(&source, true),
                    _ => utf16_fixture(&source, false),
                };
                f.write("資料/credential-source.txt", bytes);
                repo.stage_all(&c()).unwrap().result.unwrap();
                let index_before = fs::read(f.repository.join(".git/index")).unwrap();
                let report = repo.commit("Check source literal", &c()).unwrap();
                let error = report.result.as_ref().unwrap_err();
                blocked(error, "credential_detected");
                assert!(!error.to_string().contains(&candidate));
                assert!(!format!("{error:?}").contains(&candidate));
                assert_eq!(f.run(&["rev-parse", "HEAD"]), safe_head.as_bytes());
                assert_eq!(
                    fs::read(f.repository.join(".git/index")).unwrap(),
                    index_before
                );

                // Seed only this disposable fixture using raw Git. A clean tip
                // must not conceal a credential in a typed/raw ancestor.
                f.commit("Synthetic historical source literal");
                f.run(&["rm", "--", "資料/credential-source.txt"]);
                f.commit("Clean current snapshot");
                let head_before = f.run(&["rev-parse", "HEAD"]);
                let index_before = fs::read(f.repository.join(".git/index")).unwrap();
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
                let error = report.result.as_ref().unwrap_err();
                blocked(error, "credential_detected");
                assert!(!format!("{error:?}").contains(&candidate));
                assert_eq!(report.remote_outcome, PushRemoteOutcome::NotAttempted);
                assert!(refs(&f, &remote).is_empty());
                assert_eq!(f.run_at(&remote, &["count-objects", "-v"]), remote_objects);
                assert_eq!(f.run(&["rev-parse", "HEAD"]), head_before);
                assert_eq!(
                    fs::read(f.repository.join(".git/index")).unwrap(),
                    index_before
                );
                // Each case starts from a clean fixture so another finding
                // cannot mask the specific syntax under test.
                f.run(&["reset", "--hard", safe_head.trim()]);
            }
            drop(repo);
            drop(log);
            let mut recorded_refusal = false;
            for entry in fs::read_dir(f.root.join("logs")).unwrap() {
                let path = entry.unwrap().path();
                if path.is_file() {
                    let text = fs::read_to_string(path).unwrap();
                    recorded_refusal |= text.contains("credential_detected");
                    assert!(!text.contains(&candidate));
                    assert!(!text.contains("credential-source.txt"));
                }
            }
            assert!(recorded_refusal);
        },
    );
}

#[test]
fn password_literals_are_blocked_in_messages_staging_and_push_history() {
    isolated(
        "ROCKET_SECRET",
        "password_literals_are_blocked_in_messages_staging_and_push_history",
        || {
            let value = "A9mK7pR4sT2vX8n6Q5bC3z";
            let mut candidates = vec![
                format!("${value}"),
                format!("%{value}"),
                format!("{value}:R7"),
                format!("{value};[R7]"),
                format!("%DATABASE_PASSWORD%-{value}"),
            ];
            candidates.extend(
                [
                    "your_",
                    "example",
                    "placeholder",
                    "changeme",
                    "replace_me",
                    "redacted",
                    "fixture",
                    "dummy",
                    "test-only",
                    "process.env",
                    "std::env",
                    "os.environ",
                    "getenv(",
                    "env(",
                    "#/",
                ]
                .map(|prefix| format!("{prefix}-{value}")),
            );
            for candidate in candidates {
                let (f, remote) = base("sha1");
                let repo = f.repo();
                f.write("settings.toml", "# clean staged change\n");
                repo.stage_all(&c()).unwrap().result.unwrap();
                let head = f.run(&["rev-parse", "HEAD"]);
                let index = fs::read(f.repository.join(".git/index")).unwrap();
                let text = format!("password = \"{candidate}\"\n");
                let report = repo.commit(&text, &c()).unwrap();
                let error = report.result.as_ref().unwrap_err();
                blocked(error, "credential_detected");
                assert!(!format!("{error:?}").contains(&candidate));
                assert_eq!(f.run(&["rev-parse", "HEAD"]), head);
                assert_eq!(fs::read(f.repository.join(".git/index")).unwrap(), index);

                f.write("settings.toml", &text);
                repo.stage_all(&c()).unwrap().result.unwrap();
                let index = fs::read(f.repository.join(".git/index")).unwrap();
                // The clean working copy must not hide the staged password.
                f.write("settings.toml", "# clean working copy\n");
                let report = repo.commit("safe message", &c()).unwrap();
                let error = report.result.as_ref().unwrap_err();
                blocked(error, "credential_detected");
                assert!(!format!("{error:?}").contains(&candidate));
                assert_eq!(f.run(&["rev-parse", "HEAD"]), head);
                assert_eq!(fs::read(f.repository.join(".git/index")).unwrap(), index);

                // Seed old history with raw Git, then remove the newest copy.
                // Rocket must still refuse to send the historical password.
                f.write("settings.toml", &text);
                f.commit("synthetic historical password");
                f.run(&["rm", "--", "settings.toml"]);
                f.commit("removed latest copy");
                let remote_objects = f.run_at(&remote, &["count-objects", "-v"]);
                let pushed = push(&f);
                blocked(pushed.result.as_ref().unwrap_err(), "credential_detected");
                assert_eq!(pushed.remote_outcome, PushRemoteOutcome::NotAttempted);
                assert!(refs(&f, &remote).is_empty());
                assert_eq!(f.run_at(&remote, &["count-objects", "-v"]), remote_objects);
            }
        },
    );
}

#[test]
fn password_placeholders_can_commit_and_push() {
    isolated(
        "ROCKET_SECRET",
        "password_placeholders_can_commit_and_push",
        || {
            let (f, remote) = base("sha1");
            let text = concat!(
                "PASSWORD=${ProductionPasswordVersion123}\n",
                "PASSWORD=$ProductionPasswordVersion123\n",
                "PASSWORD=${ProductionPasswordVersion123:-}\n",
                "PASSWORD=%ProductionPasswordVersion123%\n",
                "password = \"${process.env.DATABASE_PASSWORD}\"\n",
                "password = \"$env:ProductionPasswordVersion123\"\n",
                "password = \"%ProductionPasswordVersion123%\"\n",
                "password = \"<production-password-v12>\"\n",
                "password = \"{{ .Values.database.password }}\"\n",
                "password = \"${{ secrets.ProductionPasswordVersion123 }}\"\n",
                "password = \"#/definitions/ProductionPasswordVersion123\"\n",
                "password = \"example-password\"\n",
            );
            f.write(".env.example", text);
            let repo = f.repo();
            repo.stage_all(&c()).unwrap().result.unwrap();
            repo.commit(
                "Document password = \"${ProductionPasswordVersion123}\"",
                &c(),
            )
            .unwrap()
            .result
            .unwrap();
            let pushed = push(&f);
            pushed.result.unwrap();
            assert_eq!(pushed.remote_outcome, PushRemoteOutcome::Confirmed);
            assert_eq!(
                f.run(&["rev-parse", "HEAD"]),
                f.run_at(&remote, &["rev-parse", "refs/heads/main"])
            );
            assert_eq!(
                f.run_at(&remote, &["show", "refs/heads/main:.env.example"]),
                text.as_bytes()
            );
        },
    );
}

#[test]
fn commit_message_prose_references_can_commit_amend_and_push() {
    isolated(
        "ROCKET_SECRET",
        "commit_message_prose_references_can_commit_amend_and_push",
        || {
            for format in ["sha1", "sha256"] {
                let (f, remote) = base(format);
                let repo = f.repo();
                for (index, message) in [
                    concat!("Fix token: ", "OAuth2AuthenticationTokenV3 ownership"),
                    "Rename token: \"AuthenticationTokenVersion2026\"",
                    "token: 0123456789abcdef0123456789abcdef01234567",
                    "Fix GitHub token refresh",
                    "Explain token: CancellationToken initialization",
                ]
                .into_iter()
                .enumerate()
                {
                    f.write("ordinary.cfg", format!("revision {index}\n"));
                    repo.stage_all(&c()).unwrap().result.unwrap();
                    repo.commit(message, &c()).unwrap().result.unwrap();
                }
                let preview = repo
                    .preview_commit_action(
                        CommitActionRequest {
                            operation: CommitOperation::Amend,
                            targets: vec![],
                            mainline: None,
                            recovery: None,
                        },
                        &c(),
                    )
                    .unwrap();
                let message = "Rename token: \"AuthenticationTokenVersion2026\"";
                repo.apply_commit_action(&preview, Some(message), &c())
                    .unwrap()
                    .write
                    .result
                    .unwrap();
                assert_eq!(
                    String::from_utf8(f.run(&["log", "-1", "--format=%B"]))
                        .unwrap()
                        .trim(),
                    message
                );
                let report = push(&f);
                report.result.unwrap();
                assert_eq!(report.remote_outcome, PushRemoteOutcome::Confirmed);
                assert_eq!(
                    f.run(&["rev-parse", "HEAD"]),
                    f.run_at(&remote, &["rev-parse", "main"])
                );
            }
        },
    );
}

#[test]
fn jwk_json_whitespace_refuses_staged_messages_and_history_without_leaks() {
    isolated(
        "ROCKET_SECRET",
        "jwk_json_whitespace_refuses_staged_messages_and_history_without_leaks",
        || {
            // Synthetic material assembled at runtime; no real key or network.
            let material = ["Q3x7A9b2N6f0L8p4R5v1", "W2s7T9a6D0c8Kq4Zr8Xw2Pm"].concat();
            for format in ["sha1", "sha256"] {
                let (f, remote) = base(format);
                let log = DiagnosticLog::open(f.root.join("logs")).unwrap();
                let repo = f.git.with_log(log.clone()).open(&f.repository, &c()).unwrap();
                // Public material with the same formatting must remain usable.
                let public = format!("{{\"kty\"\r\n:\t\n\"EC\",\"x\"\n:\r\"{material}\"}}\n");
                f.write("public.json", &public);
                repo.stage_all(&c()).unwrap().result.unwrap();
                repo.commit("Public key metadata", &c()).unwrap().result.unwrap();
                repo.push("origin", "main", "main", PushOptions { set_upstream: false }, &c()).unwrap().result.unwrap();
                let safe_head = String::from_utf8(f.run(&["rev-parse", "HEAD"])).unwrap();
                let before_refs = refs(&f, &remote);
                let before_objects = f.run_at(&remote, &["count-objects", "-v"]);
                let source = format!("{{\n\"kty\"\r\n:\n\"EC\",\n\"d\"\n:\r\n\"{material}\"\n}}");
                let file = "資料/signing-key.json";
                let bytes = if format == "sha256" { utf16_fixture(&source, false) } else { source.as_bytes().to_vec() };
                f.write(file, &bytes);
                f.write("local-note.md", "untracked local changes\n");
                repo.stage(&[RepoPath::new(file.as_bytes().to_vec()).unwrap()], &c()).unwrap().result.unwrap();
                let index = fs::read(f.repository.join(".git/index")).unwrap();
                let report = repo.commit("Review new configuration", &c()).unwrap();
                blocked(report.result.as_ref().unwrap_err(), "private_key_detected");
                assert!(!format!("{:?}", report.result).contains(&material));
                assert_eq!(f.run(&["rev-parse", "HEAD"]), safe_head.as_bytes());
                assert_eq!(fs::read(f.repository.join(".git/index")).unwrap(), index);
                assert_eq!(fs::read(f.repository.join(file)).unwrap(), bytes);

                // A clean tree must also refuse a multiline key in its message.
                f.run(&["reset", "--hard", safe_head.trim()]);
                let index = fs::read(f.repository.join(".git/index")).unwrap();
                let report = repo.commit(&source, &c()).unwrap();
                blocked(report.result.as_ref().unwrap_err(), "private_key_detected");
                assert!(!format!("{:?}", report.result).contains(&material));
                assert_eq!(f.run(&["rev-parse", "HEAD"]), safe_head.as_bytes());
                assert_eq!(fs::read(f.repository.join(".git/index")).unwrap(), index);

                // Seed only this disposable repo's history using raw Git. A clean
                // descendant must not conceal earlier private key material.
                f.write(file, &bytes);
                f.run(&["add", "--", file]);
                f.run(&["commit", "-m", "Synthetic private key fixture"]);
                f.run(&["rm", "--", file]);
                f.run(&["commit", "-m", "Clean descendant"]);
                let head = f.run(&["rev-parse", "HEAD"]);
                let index = fs::read(f.repository.join(".git/index")).unwrap();
                let report = repo.push("origin", "main", "main", PushOptions { set_upstream: false }, &c()).unwrap();
                blocked(report.result.as_ref().unwrap_err(), "private_key_detected");
                assert_eq!(report.remote_outcome, PushRemoteOutcome::NotAttempted);
                assert_eq!(refs(&f, &remote), before_refs);
                assert_eq!(f.run_at(&remote, &["count-objects", "-v"]), before_objects);
                assert_eq!(f.run(&["rev-parse", "HEAD"]), head);
                assert_eq!(fs::read(f.repository.join(".git/index")).unwrap(), index);
                assert_eq!(fs::read(f.repository.join("local-note.md")).unwrap(), b"untracked local changes\n");
                drop(repo);
                drop(log);
                let mut refusal_logged = false;
                for entry in fs::read_dir(f.root.join("logs")).unwrap() {
                    let path = entry.unwrap().path();
                    if path.is_file() {
                        let text = fs::read_to_string(path).unwrap();
                        refusal_logged |= text.contains("private_key_detected");
                        assert!(!text.contains(&material));
                        assert!(!text.contains("signing-key.json"));
                    }
                }
                assert!(refusal_logged);
            }
        },
    );
}

#[test]
fn real_secret_shapes_in_messages_refuse_commits_and_historical_pushes() {
    isolated(
        "ROCKET_SECRET",
        "real_secret_shapes_in_messages_refuse_commits_and_historical_pushes",
        || {
            for format in ["sha1", "sha256"] {
                let (f, remote) = base(format);
                let repo = f.repo();
                let safe_head = String::from_utf8(f.run(&["rev-parse", "HEAD"])).unwrap();
                let objects_before = f.run_at(&remote, &["count-objects", "-v"]);
                let random = "n6F0dL8pR2vW4sT9aQ3x";
                let hash = "0123456789abcdef0123456789abcdef01234567";
                let key_body = "Q3x7A9b2N6f0L8p4R5v1W2s7T9a6D0c8";
                let aws_body = "Q7ZK2M4R6T3V5X7N";
                let jwk_private = "Q3x7A9b2N6f0L8p4R5v1W2s7T9a6D0c8Kq4Zr8Xw2Pm";
                let google_body = "Q3x7A9b2N6f0L8p4R5v1W2s7T9a6";
                let cases = [
                    (
                        format!("Fix token: {} ownership", token()),
                        "github_token_detected",
                        token(),
                    ),
                    (
                        format!(
                            "Update token: github_pat_{}_{}",
                            "A7".repeat(11),
                            "z5Q".repeat(20)
                        ),
                        "github_token_detected",
                        "github_pat_".to_owned(),
                    ),
                    (
                        format!(
                            "token: ghs_1_eyJhbGciOiJIUzI1NiJ9.e30.{}",
                            "Q3-_".repeat(30)
                        ),
                        "github_token_detected",
                        "ghs_1_".to_owned(),
                    ),
                    (
                        format!("Rotate token: \"{random}\""),
                        "credential_detected",
                        random.to_owned(),
                    ),
                    (
                        format!("token={hash}"),
                        "credential_detected",
                        hash.to_owned(),
                    ),
                    (
                        format!("github_token: {hash}"),
                        "credential_detected",
                        hash.to_owned(),
                    ),
                    (
                        format!("token: \"{hash}\""),
                        "credential_detected",
                        hash.to_owned(),
                    ),
                    (
                        "password: \"CorrectHorseBatteryStaple\"".to_owned(),
                        "credential_detected",
                        "CorrectHorseBatteryStaple".to_owned(),
                    ),
                    (
                        format!("Configuration example:\n```yaml\ntoken: {hash}\n```"),
                        "credential_detected",
                        hash.to_owned(),
                    ),
                    (
                        format!(
                            "Key example:\n-----BEGIN PRIVATE KEY-----\n{key_body}\n-----END PRIVATE KEY-----"
                        ),
                        "private_key_detected",
                        key_body.to_owned(),
                    ),
                    (
                        format!("Deploy with AKIA{aws_body}"),
                        "aws_access_key_detected",
                        aws_body.to_owned(),
                    ),
                    (
                        format!(
                            "Signing key:\n{{\"kty\":\"EC\",\"crv\":\"P-256\",\"d\":\"{jwk_private}\"}}"
                        ),
                        "private_key_detected",
                        jwk_private.to_owned(),
                    ),
                    (
                        format!("OAuth client: GOCSPX-{google_body}"),
                        "credential_detected",
                        google_body.to_owned(),
                    ),
                    (
                        format!("OIDC_CLIENT_SECRET={random}"),
                        "credential_detected",
                        random.to_owned(),
                    ),
                ];
                for (message, code, sentinel) in cases {
                    f.write("clean.cfg", "ordinary staged contents\n");
                    repo.stage_all(&c()).unwrap().result.unwrap();
                    let index = fs::read(f.repository.join(".git/index")).unwrap();
                    let result = repo.commit(&message, &c()).unwrap();
                    let error = result.result.as_ref().unwrap_err();
                    blocked(error, code);
                    assert!(!error.to_string().contains(&sentinel));
                    assert!(!format!("{error:?}").contains(&sentinel));
                    assert_eq!(f.run(&["rev-parse", "HEAD"]), safe_head.as_bytes());
                    assert_eq!(fs::read(f.repository.join(".git/index")).unwrap(), index);
                    // Seed synthetic pre-existing history outside the API under
                    // test, then ensure an innocent descendant cannot conceal it.
                    f.commit(&message);
                    f.run(&["commit", "--allow-empty", "-m", "clean descendant"]);
                    let result = push(&f);
                    blocked(result.result.as_ref().unwrap_err(), code);
                    assert_eq!(result.remote_outcome, PushRemoteOutcome::NotAttempted);
                    assert!(refs(&f, &remote).is_empty());
                    assert_eq!(f.run_at(&remote, &["count-objects", "-v"]), objects_before);
                    f.run(&["reset", "--hard", safe_head.trim()]);
                }
            }
        },
    );
}

#[test]
fn message_reference_exemptions_do_not_apply_to_staged_files_or_commit_headers() {
    isolated(
        "ROCKET_SECRET",
        "message_reference_exemptions_do_not_apply_to_staged_files_or_commit_headers",
        || {
            use std::io::Write;
            use std::process::Stdio;
            let hash = "0123456789abcdef0123456789abcdef01234567";
            let message = format!("token: {hash}");
            for format in ["sha1", "sha256"] {
                let (f, remote) = base(format);
                let repo = f.repo();
                f.write("config.yaml", &message);
                repo.stage_all(&c()).unwrap().result.unwrap();
                blocked(
                    repo.commit("ordinary subject", &c())
                        .unwrap()
                        .result
                        .as_ref()
                        .unwrap_err(),
                    "credential_detected",
                );
                f.commit("fixture with strict file contents");
                blocked(push(&f).result.as_ref().unwrap_err(), "credential_detected");
                assert!(refs(&f, &remote).is_empty());

                let (f, remote) = base(format);
                let original = f.run(&["cat-file", "commit", "HEAD"]);
                let split = original.windows(2).position(|w| w == b"\n\n").unwrap();
                let object = [
                    &original[..split],
                    format!("\nx-test token: {hash}\n\n{message}\n").as_bytes(),
                ]
                .concat();
                let mut child = f
                    .command(
                        &f.repository,
                        &["hash-object", "-t", "commit", "-w", "--stdin"],
                    )
                    .stdin(Stdio::piped())
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped())
                    .spawn()
                    .unwrap();
                child.stdin.take().unwrap().write_all(&object).unwrap();
                let output = child.wait_with_output().unwrap();
                assert!(output.status.success());
                let oid = String::from_utf8(output.stdout).unwrap();
                f.run(&["update-ref", "refs/heads/main", oid.trim()]);
                let result = push(&f);
                blocked(result.result.as_ref().unwrap_err(), "credential_detected");
                assert_eq!(result.remote_outcome, PushRemoteOutcome::NotAttempted);
                assert!(refs(&f, &remote).is_empty());
            }
        },
    );
}

#[test]
fn expanded_benign_files_and_historical_references_can_commit_and_push() {
    isolated(
        "ROCKET_SECRET",
        "expanded_benign_files_and_historical_references_can_commit_and_push",
        || {
            for format in ["sha1", "sha256"] {
                let (f, remote) = base(format);
                let repo = f.repo();
                let rust = "pub(crate) token: OAuth2AuthenticationTokenV3,\npub(in crate::auth) token: OAuth2AuthenticationTokenV3,\n";
                let ts = "readonly token: OAuth2AuthenticationTokenV3;\n";
                let json =
                    "{\"token\":\"#/definitions/AccessToken\",\"password\":\"请输入密码\"}\n";
                for (path, contents) in [
                    ("src/auth.rs", rust.as_bytes().to_vec()),
                    ("src/auth.ts", [b"\xef\xbb\xbf".as_slice(), ts.as_bytes()].concat()),
                    ("schema/config.json", utf16_fixture(json, true)),
                    ("schema/config-be.json", utf16_fixture(json, false)),
                    ("nested/.ENV.EXAMPLE", b"TOKEN=${GITHUB_TOKEN}\n".to_vec()),
                    ("nested/.env.sample", b"password: '{{ .Values.password }}'\n".to_vec()),
                    ("nested/.env.template", b"GITHUB_TOKEN=\n".to_vec()),
                    ("docs/keys.md", b"Public keys and private-key headers are documentation:\n-----BEGIN PRIVATE KEY-----\nComment: no material here\n\n".to_vec()),
                    (".envrc", b"export TOKEN=${GITHUB_TOKEN}\n".to_vec()),
                ] {
                    f.write(path, contents);
                }
                repo.stage_all(&c()).unwrap().result.unwrap();
                repo.commit("Add types and safe templates", &c())
                    .unwrap()
                    .result
                    .unwrap();
                // Outgoing historical blobs are checked even after the latest
                // snapshot no longer contains these type/reference expressions.
                f.run(&["rm", "--", "src/auth.rs", "src/auth.ts"]);
                repo.commit("Remove old source examples", &c())
                    .unwrap()
                    .result
                    .unwrap();
                let report = push(&f);
                report.result.unwrap();
                assert_eq!(report.remote_outcome, PushRemoteOutcome::Confirmed);
                assert_eq!(
                    f.run(&["rev-parse", "HEAD"]),
                    f.run_at(&remote, &["rev-parse", "main"])
                );
                assert_eq!(
                    f.run_at(&remote, &["show", "main^:src/auth.rs"]),
                    rust.as_bytes()
                );
            }
        },
    );
}

#[test]
fn encoded_credentials_and_key_metadata_refuse_commit_and_historical_push() {
    isolated(
        "ROCKET_SECRET",
        "encoded_credentials_and_key_metadata_refuse_commit_and_historical_push",
        || {
            for format in ["sha1", "sha256"] {
                let (f, remote) = base(format);
                let repo = f.repo();
                let safe_head = String::from_utf8(f.run(&["rev-parse", "HEAD"])).unwrap();
                let initial_objects = f.run_at(&remote, &["count-objects", "-v"]);
                let generic = "n6F0dL8pR2vW4sT9aQ3x";
                let stateless = format!("ghs_1_eyJhbGciOiJIUzI1NiJ9.e30.{}", "Q3-_".repeat(30));
                let body = "Q3x7A9b2N6f0L8p4R5v1W2s7T9a6D0c8";
                let rsa = format!(
                    "-----BEGIN RSA PRIVATE KEY-----\nProc-Type: 4,ENCRYPTED\nDEK-Info: AES-256-CBC,0123456789ABCDEF\n\n{body}\n-----END RSA PRIVATE KEY-----\n"
                );
                let pgp = format!(
                    "-----BEGIN PGP PRIVATE KEY BLOCK-----\r\nVersion: Rocket inert fixture\r\nComment: local test\r\n\r\n{body}\r\n-----END PGP PRIVATE KEY BLOCK-----\r\n"
                );
                let cases = [
                    (
                        utf16_fixture(&token(), true),
                        "github_token_detected",
                        token(),
                    ),
                    (
                        utf16_fixture(&stateless, false),
                        "github_token_detected",
                        stateless,
                    ),
                    (
                        [
                            b"\xef\xbb\xbf".as_slice(),
                            format!("api_key=\"{generic}\"").as_bytes(),
                        ]
                        .concat(),
                        "credential_detected",
                        generic.to_owned(),
                    ),
                    (rsa.into_bytes(), "private_key_detected", body.to_owned()),
                    (
                        utf16_fixture(&pgp, false),
                        "private_key_detected",
                        body.to_owned(),
                    ),
                    (
                        format!("{{\"private_key\":\"{}\"}}", pgp.replace("\r\n", "\\r\\n"))
                            .into_bytes(),
                        "private_key_detected",
                        body.to_owned(),
                    ),
                    (
                        format!(
                            "-----BEGIN PRIVATE KEY-----\n{}",
                            "Comment: -----BEGIN PRIVATE KEY-----\n".repeat(4096)
                        )
                        .into_bytes(),
                        "secret_scan_incomplete",
                        "never-present-fixture-sentinel".to_owned(),
                    ),
                ];
                for (contents, code, sentinel) in cases {
                    f.write("nested/.env.example", contents);
                    repo.stage_all(&c()).unwrap().result.unwrap();
                    let index_before = fs::read(f.repository.join(".git/index")).unwrap();
                    let report = repo.commit("fixture", &c()).unwrap();
                    let error = report.result.as_ref().unwrap_err();
                    blocked(error, code);
                    assert!(!error.to_string().contains(&sentinel));
                    assert!(!format!("{error:?}").contains(&sentinel));
                    assert_eq!(
                        fs::read(f.repository.join(".git/index")).unwrap(),
                        index_before
                    );
                    assert_eq!(f.run(&["rev-parse", "HEAD"]), safe_head.as_bytes());
                    // Manufacture pre-existing unsafe history with the fixture's
                    // raw Git helper; no real credential or remote is involved.
                    f.commit("pre-existing unsafe fixture");
                    f.run(&["rm", "--", "nested/.env.example"]);
                    f.commit("clean current snapshot");
                    let report = push(&f);
                    let error = report.result.as_ref().unwrap_err();
                    blocked(error, code);
                    assert!(!error.to_string().contains(&sentinel));
                    assert_eq!(report.remote_outcome, PushRemoteOutcome::NotAttempted);
                    assert!(refs(&f, &remote).is_empty());
                    assert_eq!(f.run_at(&remote, &["count-objects", "-v"]), initial_objects);
                    // Reset only this disposable fixture between independent
                    // history cases so an earlier finding cannot mask the next.
                    f.run(&["reset", "--hard", safe_head.trim()]);
                }
            }
        },
    );
}

#[test]
fn correcting_a_refused_staged_secret_allows_retry_without_publishing_its_blob() {
    isolated(
        "ROCKET_SECRET",
        "correcting_a_refused_staged_secret_allows_retry_without_publishing_its_blob",
        || {
            for format in ["sha1", "sha256"] {
                let (f, remote) = base(format);
                let repo = f.repo();
                f.write(".env.example", format!("GITHUB_TOKEN={}\n", token()));
                repo.stage_all(&c()).unwrap().result.unwrap();
                let rejected_blob =
                    String::from_utf8(f.run(&["rev-parse", ":.env.example"])).unwrap();
                blocked(
                    repo.commit("template", &c())
                        .unwrap()
                        .result
                        .as_ref()
                        .unwrap_err(),
                    "github_token_detected",
                );
                let clean = "GITHUB_TOKEN=${GITHUB_TOKEN}\npassword: '{{ .Values.password }}'\n";
                f.write(".env.example", clean);
                repo.stage_all(&c()).unwrap().result.unwrap();
                repo.commit("Add safe template", &c())
                    .unwrap()
                    .result
                    .unwrap();
                let report = push(&f);
                report.result.unwrap();
                assert_eq!(report.remote_outcome, PushRemoteOutcome::Confirmed);
                assert_eq!(
                    f.run_at(&remote, &["show", "main:.env.example"]),
                    clean.as_bytes()
                );
                assert!(
                    !f.output(&remote, &["cat-file", "-e", rejected_blob.trim()])
                        .status
                        .success()
                );
                f.write(".env.example", token());
                repo.stage_all(&c()).unwrap().result.unwrap();
                blocked(
                    repo.commit("still protected", &c())
                        .unwrap()
                        .result
                        .as_ref()
                        .unwrap_err(),
                    "github_token_detected",
                );
            }
        },
    );
}

#[test]
fn ordinary_source_templates_and_labels_can_be_committed_and_pushed() {
    isolated(
        "ROCKET_SECRET",
        "ordinary_source_templates_and_labels_can_be_committed_and_pushed",
        || {
            let (f, remote) = base("sha1");
            let repo = f.repo();
            for (path, text) in [
                (
                    "source.rs",
                    "pub token: CancellationToken,\npub token: Option<String>,\nlet token = cancellation_token;\n",
                ),
                (
                    "frontend.ts",
                    concat!(
                        "const token = existingToken;\nreadonly token: ",
                        "AccessTokenVersion123;\n"
                    ),
                ),
                (
                    "labels.json",
                    "{\"password\":\"Enter your password\",\"token\":\"Authentication\"}\n",
                ),
                (
                    "settings.yaml",
                    "password: \"{{ .Values.database.password }}\"\ntoken: '${{ secrets.GITHUB_TOKEN }}'\n",
                ),
                (".env.example", "GITHUB_TOKEN=${GITHUB_TOKEN}\nPASSWORD=\n"),
            ] {
                f.write(path, text);
            }
            repo.stage_all(&c()).unwrap().result.unwrap();
            repo.commit("Fix token: CancellationToken ownership and lifetime", &c())
                .unwrap()
                .result
                .unwrap();
            let pushed = push(&f);
            pushed.result.unwrap();
            assert_eq!(pushed.remote_outcome, PushRemoteOutcome::Confirmed);
            assert_eq!(
                f.run(&["rev-parse", "HEAD"]),
                f.run_at(&remote, &["rev-parse", "refs/heads/main"])
            );
        },
    );
}

#[test]
fn literal_credentials_and_tokens_in_templates_still_block_commits() {
    isolated(
        "ROCKET_SECRET",
        "literal_credentials_and_tokens_in_templates_still_block_commits",
        || {
            let (f, _) = base("sha1");
            let repo = f.repo();
            let before = f.run(&["rev-parse", "HEAD"]);
            let random = "n6F0dL8pR2vW4sT9aQ3x";
            for (text, code) in [
                (
                    format!("const token = \"{random}\";"),
                    "credential_detected",
                ),
                (format!("TOKEN={random}"), "credential_detected"),
                (
                    format!("token=\"example-{}\"", token()),
                    "github_token_detected",
                ),
                (
                    format!("token=\"${{{}}}\"", token()),
                    "github_token_detected",
                ),
                (
                    format!("token=\"{{{{ {} }}}}\"", token()),
                    "github_token_detected",
                ),
                (
                    format!("ghs_1_eyJhbGciOiJIUzI1NiJ9.e30.{}", "Q3-_".repeat(30)),
                    "github_token_detected",
                ),
            ] {
                f.write("configuration.cfg", &text);
                repo.stage_all(&c()).unwrap().result.unwrap();
                blocked(
                    repo.commit("fixture", &c())
                        .unwrap()
                        .result
                        .as_ref()
                        .unwrap_err(),
                    code,
                );
                assert_eq!(f.run(&["rev-parse", "HEAD"]), before);
            }
        },
    );
}

#[test]
fn amend_refuses_a_secret_message_without_rewriting_head() {
    isolated(
        "ROCKET_SECRET",
        "amend_refuses_a_secret_message_without_rewriting_head",
        || {
            let (f, _) = base("sha1");
            let repo = f.repo();
            let head = f.run(&["rev-parse", "HEAD"]);
            let review = repo
                .preview_commit_action(
                    CommitActionRequest {
                        operation: CommitOperation::Amend,
                        targets: vec![],
                        mainline: None,
                        recovery: None,
                    },
                    &c(),
                )
                .unwrap();
            let report = repo
                .apply_commit_action(&review, Some(&token()), &c())
                .unwrap();
            blocked(
                report.write.result.as_ref().unwrap_err(),
                "github_token_detected",
            );
            assert_eq!(head, f.run(&["rev-parse", "HEAD"]));
        },
    );
}

#[test]
fn environment_filename_is_checked_even_when_its_blob_already_has_a_clean_name() {
    isolated(
        "ROCKET_SECRET",
        "environment_filename_is_checked_even_when_its_blob_already_has_a_clean_name",
        || {
            let (f, remote) = base("sha1");
            f.write("a-clean-name", "same bytes\n");
            f.write("z/nested/.env", "same bytes\n");
            f.commit("aliased blob");
            blocked(
                push(&f).result.as_ref().unwrap_err(),
                "environment_file_blocked",
            );
            assert!(refs(&f, &remote).is_empty());
        },
    );
}

#[test]
fn staged_secret_is_blocked_without_running_commit_hooks_or_changing_index() {
    isolated(
        "ROCKET_SECRET",
        "staged_secret_is_blocked_without_running_commit_hooks_or_changing_index",
        || {
            let (f, _) = base("sha1");
            f.write("config.rs", token());
            f.run(&["add", "--", "config.rs"]);
            // The working copy is clean, but the index still contains the secret.
            f.write("config.rs", "clean working copy\n");
            fs::write(
                f.root.join("no-hooks/pre-commit"),
                "#!/bin/sh\necho ran > hook-ran\n",
            )
            .unwrap();
            let head = f.run(&["rev-parse", "HEAD"]);
            let index = fs::read(f.repository.join(".git/index")).unwrap();
            let report = f.repo().commit("safe message", &c()).unwrap();
            blocked(report.result.as_ref().unwrap_err(), "github_token_detected");
            assert_eq!(head, f.run(&["rev-parse", "HEAD"]));
            assert_eq!(index, fs::read(f.repository.join(".git/index")).unwrap());
            assert!(!f.repository.join("hook-ran").exists());
        },
    );
}

#[test]
fn unstaged_secret_does_not_block_a_clean_staged_snapshot() {
    isolated(
        "ROCKET_SECRET",
        "unstaged_secret_does_not_block_a_clean_staged_snapshot",
        || {
            let (f, _) = base("sha1");
            f.write("file", "clean staged\n");
            f.run(&["add", "--", "file"]);
            f.write("file", token());
            assert!(f.repo().commit("safe", &c()).unwrap().result.is_ok());
            assert_eq!(f.run(&["show", "HEAD:file"]), b"clean staged\n");
            assert_eq!(
                fs::read_to_string(f.repository.join("file")).unwrap(),
                token()
            );
        },
    );
}

#[test]
fn environment_files_and_messages_are_blocked_but_clean_templates_and_removal_work() {
    isolated(
        "ROCKET_SECRET",
        "environment_files_and_messages_are_blocked_but_clean_templates_and_removal_work",
        || {
            for name in [".env", "nested/.env.production", "production.env"] {
                let (f, _) = base("sha1");
                f.write(name, "");
                f.run(&["add", "--", name]);
                blocked(
                    f.repo()
                        .commit("safe", &c())
                        .unwrap()
                        .result
                        .as_ref()
                        .unwrap_err(),
                    "environment_file_blocked",
                );
            }
            let (f, _) = base("sha1");
            f.write(".env.example", "GITHUB_TOKEN=\n");
            f.run(&["add", "--all"]);
            blocked(
                f.repo()
                    .commit(&token(), &c())
                    .unwrap()
                    .result
                    .as_ref()
                    .unwrap_err(),
                "github_token_detected",
            );
            assert!(f.repo().commit("template", &c()).unwrap().result.is_ok());
            f.write(".env", "");
            f.commit("pre-existing env");
            f.run(&["rm", "--", ".env"]);
            assert!(
                f.repo()
                    .commit("remove tracked env", &c())
                    .unwrap()
                    .result
                    .is_ok()
            );
            f.write(".env.example", token());
            f.run(&["add", "--all"]);
            blocked(
                f.repo()
                    .commit("template", &c())
                    .unwrap()
                    .result
                    .as_ref()
                    .unwrap_err(),
                "github_token_detected",
            );
        },
    );
}

#[test]
fn user_can_disable_and_reenable_guard_and_request_clones_keep_the_preference() {
    isolated(
        "ROCKET_SECRET",
        "user_can_disable_and_reenable_guard_and_request_clones_keep_the_preference",
        || {
            let (f, _) = base("sha1");
            let repo = f.repo();
            f.write(".env", token());
            f.run(&["add", "--all"]);
            f.git.set_secret_protection(false);
            assert!(
                repo.commit("explicitly opted out", &c())
                    .unwrap()
                    .result
                    .is_ok()
            );
            let log = DiagnosticLog::open(f.root.join("logs")).unwrap();
            let request = f.git.for_request(CommandLimits::default(), log).unwrap();
            let push = request
                .open(&f.repository, &c())
                .unwrap()
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
            assert!(push.result.is_ok());
            f.git.set_secret_protection(true);
            f.write("readme.md", "changed\n");
            f.run(&["add", "--all"]);
            blocked(
                repo.commit("protected again", &c())
                    .unwrap()
                    .result
                    .as_ref()
                    .unwrap_err(),
                "environment_file_blocked",
            );
        },
    );
}

#[test]
fn push_checks_deleted_secrets_in_ancestors_for_sha1_and_sha256() {
    isolated(
        "ROCKET_SECRET",
        "push_checks_deleted_secrets_in_ancestors_for_sha1_and_sha256",
        || {
            for format in ["sha1", "sha256"] {
                let (f, remote) = base(format);
                f.write("old.cfg", token());
                f.commit("accidental secret");
                f.run(&["rm", "--", "old.cfg"]);
                f.commit("deleted later");
                let before = f.run_at(&remote, &["count-objects", "-v"]);
                let report = push(&f);
                blocked(report.result.as_ref().unwrap_err(), "github_token_detected");
                assert_eq!(report.remote_outcome, PushRemoteOutcome::NotAttempted);
                assert!(refs(&f, &remote).is_empty());
                assert_eq!(before, f.run_at(&remote, &["count-objects", "-v"]));
            }
        },
    );
}

#[test]
fn replacement_refs_cannot_hide_secret_history() {
    isolated(
        "ROCKET_SECRET",
        "replacement_refs_cannot_hide_secret_history",
        || {
            let (f, remote) = base("sha1");
            let clean = String::from_utf8(f.run(&["rev-parse", "HEAD"])).unwrap();
            f.write("secret.cfg", token());
            f.commit("bad ancestor");
            let bad = String::from_utf8(f.run(&["rev-parse", "HEAD"])).unwrap();
            f.run(&["rm", "--", "secret.cfg"]);
            f.commit("clean tip");
            f.run(&["replace", bad.trim(), clean.trim()]);
            blocked(
                push(&f).result.as_ref().unwrap_err(),
                "github_token_detected",
            );
            assert!(refs(&f, &remote).is_empty());
        },
    );
}

#[test]
fn force_atomic_and_annotated_tag_pushes_use_the_same_guard() {
    isolated(
        "ROCKET_SECRET",
        "force_atomic_and_annotated_tag_pushes_use_the_same_guard",
        || {
            let (f, remote) = base("sha1");
            f.run(&["push", "origin", "main"]);
            let before = refs(&f, &remote);
            f.write("secret.cfg", token());
            f.commit("bad update");
            let repo = f.repo();
            let atomic = repo
                .preview_atomic_push(
                    b"origin",
                    &[AtomicPushSelection {
                        local_branch: b"main".to_vec(),
                        remote_branch: b"main".to_vec(),
                    }],
                    &c(),
                )
                .unwrap();
            blocked(
                repo.apply_atomic_push(&atomic, &c()).as_ref().unwrap_err(),
                "github_token_detected",
            );
            f.run(&["reset", "--soft", "HEAD^"]);
            f.run(&["commit", "--amend", "--no-verify", "-m", "replacement"]);
            let force = repo
                .preview_force_push(b"origin", b"main", b"main", &c())
                .unwrap();
            blocked(
                repo.apply_force_push(&force, &c()).as_ref().unwrap_err(),
                "github_token_detected",
            );
            assert_eq!(before, refs(&f, &remote));
            let (f, remote) = base("sha1");
            f.run(&["tag", "--no-sign", "-a", "release", "-m", &token()]);
            let repo = f.repo();
            let tags = repo.remote_tags(b"origin", &c()).unwrap();
            let review = repo
                .preview_remote_tag(&tags, b"release", RemoteTagAction::Publish, &c())
                .unwrap();
            blocked(
                repo.apply_remote_tag(&review, &c()).as_ref().unwrap_err(),
                "github_token_detected",
            );
            assert!(refs(&f, &remote).is_empty());
        },
    );
}

#[test]
fn large_clean_objects_can_commit_and_push_with_protection_enabled() {
    isolated(
        "ROCKET_SECRET",
        "large_clean_objects_can_commit_and_push_with_protection_enabled",
        || {
            let (f, remote) = base("sha1");
            // Exercise both the old 8 MiB grouping threshold and the generic
            // runner's 32 MiB stdout default. These are inert fixture bytes.
            for (name, size) in [
                ("large.bin", 8 * 1024 * 1024 + 1),
                ("larger.bin", 40 * 1024 * 1024),
            ] {
                let file = fs::File::create(f.repository.join(name)).unwrap();
                file.set_len(size).unwrap();
            }
            let repo = f.repo();
            repo.stage_all(&c()).unwrap().result.unwrap();
            repo.commit("large clean fixture", &c())
                .unwrap()
                .result
                .unwrap();
            // Unchanged large blobs remain in the index when only a small file
            // changes, matching the IDM_EX publication that exposed the bug.
            f.write("readme.md", "updated documentation\n");
            repo.stage_all(&c()).unwrap().result.unwrap();
            repo.commit("documentation only", &c())
                .unwrap()
                .result
                .unwrap();
            push(&f).result.unwrap();
            assert_eq!(
                f.run(&["rev-parse", "HEAD"]),
                f.run_at(&remote, &["rev-parse", "refs/heads/main"])
            );
            assert!(f.run(&["status", "--porcelain"]).is_empty());
        },
    );
}

#[test]
fn scan_limits_and_missing_parent_objects_refuse_without_sending_a_push() {
    isolated(
        "ROCKET_SECRET",
        "scan_limits_and_missing_parent_objects_refuse_without_sending_a_push",
        || {
            let (f, remote) = base("sha1");
            f.git.set_secret_scan_mib(512).unwrap();
            let file = fs::File::create(f.repository.join("large.bin")).unwrap();
            file.set_len(128 * 1024 * 1024 + 1).unwrap();
            f.commit("large blob");
            let refusal = push(&f).result.unwrap_err();
            blocked(&refusal, "secret_scan_incomplete");
            assert!(refusal.to_string().contains("128 MiB per-object"));
            assert!(refs(&f, &remote).is_empty());
            let (f, remote) = base("sha1");
            let parent = String::from_utf8(f.run(&["rev-parse", "HEAD"])).unwrap();
            f.write("next", "next\n");
            f.commit("next");
            let parent = parent.trim();
            fs::remove_file(
                f.repository
                    .join(".git/objects")
                    .join(&parent[..2])
                    .join(&parent[2..]),
            )
            .unwrap();
            blocked(
                push(&f).result.as_ref().unwrap_err(),
                "secret_scan_incomplete",
            );
            assert!(refs(&f, &remote).is_empty());
        },
    );
}

#[test]
fn large_blob_secrets_after_eight_mib_block_commit_and_historical_push() {
    isolated(
        "ROCKET_SECRET",
        "large_blob_secrets_after_eight_mib_block_commit_and_historical_push",
        || {
            use std::io::{Seek, SeekFrom, Write};
            for format in ["sha1", "sha256"] {
                let (f, remote) = base(format);
                let repo = f.repo();
                let head = String::from_utf8(f.run(&["rev-parse", "HEAD"])).unwrap();
                let key = ["-----BEGIN ", "PRIVATE KEY-----\n", &"A".repeat(40)].concat();
                let cases = [
                    (token().into_bytes(), "github_token_detected", None),
                    (
                        b"api_key = \"a1B2c3D4e5F6g7H8i9J0k1L2m3N4\"\n".to_vec(),
                        "credential_detected",
                        None,
                    ),
                    (key.into_bytes(), "private_key_detected", None),
                    (
                        utf16_fixture(&token(), true)[2..].to_vec(),
                        "github_token_detected",
                        Some(true),
                    ),
                    (
                        utf16_fixture(&token(), false)[2..].to_vec(),
                        "github_token_detected",
                        Some(false),
                    ),
                ];
                for (payload, code, utf16) in cases {
                    let mut file = fs::File::create(f.repository.join("大きい-file.bin")).unwrap();
                    if let Some(little) = utf16 {
                        file.write_all(if little { &[0xff, 0xfe] } else { &[0xfe, 0xff] })
                            .unwrap();
                    }
                    // Cross the former file limit and a pipe read boundary.
                    let offset = 8 * 1024 * 1024 + if utf16.is_some() { 8190 } else { 8191 };
                    file.seek(SeekFrom::Start(offset)).unwrap();
                    file.write_all(&payload).unwrap();
                    drop(file);
                    repo.stage_all(&c()).unwrap().result.unwrap();
                    let index = fs::read(f.repository.join(".git/index")).unwrap();
                    blocked(
                        repo.commit("large fixture", &c())
                            .unwrap()
                            .result
                            .as_ref()
                            .unwrap_err(),
                        code,
                    );
                    assert_eq!(fs::read(f.repository.join(".git/index")).unwrap(), index);
                    assert_eq!(f.run(&["rev-parse", "HEAD"]), head.as_bytes());
                    // Raw Git only constructs pre-existing unsafe fixture history.
                    f.commit("synthetic historical secret");
                    f.run(&["rm", "--", "大きい-file.bin"]);
                    f.commit("remove synthetic fixture");
                    let report = push(&f);
                    blocked(report.result.as_ref().unwrap_err(), code);
                    assert_eq!(report.remote_outcome, PushRemoteOutcome::NotAttempted);
                    assert!(refs(&f, &remote).is_empty());
                    f.run(&["reset", "--hard", head.trim()]);
                }
            }
        },
    );
}

#[test]
fn cumulative_large_object_budget_remains_bounded() {
    isolated(
        "ROCKET_SECRET",
        "cumulative_large_object_budget_remains_bounded",
        || {
            let (f, remote) = base("sha1");
            f.git.set_secret_scan_mib(1).unwrap();
            for name in ["first.bin", "second.bin"] {
                let mut file = fs::File::create(f.repository.join(name)).unwrap();
                file.set_len(600 * 1024).unwrap();
                // Distinct objects must both be charged to the cumulative budget.
                use std::io::Write;
                file.write_all(name.as_bytes()).unwrap();
            }
            let repo = f.repo();
            repo.stage_all(&c()).unwrap().result.unwrap();
            let report = repo.commit("over total budget", &c()).unwrap();
            let error = report.result.as_ref().unwrap_err();
            blocked(error, "secret_scan_incomplete");
            assert!(error.to_string().contains("configured 1 MiB total"));
            f.commit("pre-existing over-budget history");
            blocked(
                push(&f).result.as_ref().unwrap_err(),
                "secret_scan_incomplete",
            );
            assert!(refs(&f, &remote).is_empty());
        },
    );
}

#[cfg(windows)]
#[test]
fn cancellation_while_reading_large_objects_stops_commit_and_push() {
    isolated(
        "ROCKET_SECRET",
        "cancellation_while_reading_large_objects_stops_commit_and_push",
        || {
            use std::{
                thread,
                time::{Duration, Instant},
            };
            let (f, remote) = base("sha1");
            fs::File::create(f.repository.join("large.bin"))
                .unwrap()
                .set_len(40 * 1024 * 1024)
                .unwrap();
            f.commit("large clean base");
            f.write("readme.md", "pending documentation\n");
            f.run(&["add", "--", "readme.md"]);
            let head = f.run(&["rev-parse", "HEAD"]);
            let index = fs::read(f.repository.join(".git/index")).unwrap();
            let gate = f.root.join("scan-gate");
            let marker = f.root.join("scan-marker");
            let relay = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../tools/helpers/bin/commit-switch/git.exe")
                .canonicalize()
                .unwrap();
            // This test runs in its own child process; set relay configuration before
            // starting any worker. Existing user environment/config is not changed.
            unsafe {
                std::env::set_var("ROCKET_FILE_TEST_GATE", &gate);
                std::env::set_var("ROCKET_FILE_TEST_MARKER", &marker);
                std::env::set_var("ROCKET_FILE_TEST_GIT", f.git.executable());
            }
            let log = DiagnosticLog::open(f.root.join("logs")).unwrap();
            let client =
                GitClient::new_with_log(relay, CommandLimits::default(), log.clone()).unwrap();
            let repo = client.open(&f.repository, &c()).unwrap();
            for action in ["commit", "push"] {
                if marker.exists() {
                    fs::remove_file(&marker).unwrap();
                }
                fs::write(&gate, "after:--batch").unwrap();
                let cancel = c();
                let watch_cancel = cancel.clone();
                let watch_marker = marker.clone();
                let watcher = thread::spawn(move || {
                    let started = Instant::now();
                    while !watch_marker.exists() && started.elapsed() < Duration::from_secs(20) {
                        thread::sleep(Duration::from_millis(5));
                    }
                    let observed = watch_marker.exists();
                    let requested = Instant::now();
                    watch_cancel.cancel();
                    (observed, requested)
                });
                let result = if action == "commit" {
                    repo.commit("cancelled large scan", &cancel)
                        .and_then(|r| r.result.map(|_| ()))
                } else {
                    repo.push(
                        "origin",
                        "main",
                        "main",
                        PushOptions {
                            set_upstream: false,
                        },
                        &cancel,
                    )
                    .and_then(|r| r.result.map(|_| ()))
                };
                let (observed, requested) = watcher.join().unwrap();
                fs::write(&gate, "").unwrap();
                assert!(observed, "did not reach content read");
                assert!(matches!(result, Err(Error::Cancelled)), "{result:?}");
                eprintln!(
                    "{action}: cancellation plus reconciliation {} ms",
                    requested.elapsed().as_millis()
                );
                assert_eq!(f.run(&["rev-parse", "HEAD"]), head);
                assert_eq!(fs::read(f.repository.join(".git/index")).unwrap(), index);
                assert_eq!(
                    fs::read_to_string(f.repository.join("readme.md")).unwrap(),
                    "pending documentation\n"
                );
                assert!(refs(&f, &remote).is_empty());
            }
            assert_eq!(log.health().dropped_events, 0);
            let text = fs::read_to_string(f.root.join("logs/rocket.jsonl")).unwrap();
            let stop_times: Vec<u64> = text
                .split("\"cancel_to_stop_us\":")
                .skip(1)
                .map(|tail| {
                    tail.chars()
                        .take_while(char::is_ascii_digit)
                        .collect::<String>()
                        .parse()
                        .unwrap()
                })
                .collect();
            assert_eq!(stop_times.len(), 2);
            for time in stop_times {
                eprintln!("owned process cancellation stop: {time} us");
                assert!(time < 2_000_000, "process stop exceeded two seconds");
            }
            assert!(!text.contains("large.bin"));
            assert!(!text.contains("pending documentation"));
        },
    );
}

#[test]
fn push_remains_bound_to_scanned_object_when_a_hook_moves_the_branch() {
    isolated(
        "ROCKET_SECRET",
        "push_remains_bound_to_scanned_object_when_a_hook_moves_the_branch",
        || {
            let (f, remote) = base("sha1");
            let safe = String::from_utf8(f.run(&["rev-parse", "HEAD"])).unwrap();
            f.write("hidden", token());
            f.commit("unsafe object");
            let unsafe_id = String::from_utf8(f.run(&["rev-parse", "HEAD"])).unwrap();
            f.run(&["reset", "--hard", safe.trim()]);
            fs::write(
                f.root.join("no-hooks/pre-push"),
                format!(
                    "#!/bin/sh\ngit update-ref refs/heads/main {}\n",
                    unsafe_id.trim()
                ),
            )
            .unwrap();
            let report = push(&f);
            assert!(report.result.is_ok(), "{report:?}");
            assert_eq!(f.run_at(&remote, &["rev-parse", "main"]), safe.as_bytes());
            assert_eq!(f.run(&["rev-parse", "main"]), unsafe_id.as_bytes());
            assert!(
                !f.output(&remote, &["cat-file", "-e", unsafe_id.trim()])
                    .status
                    .success()
            );
        },
    );
}

#[test]
fn diagnostics_record_only_fixed_refusal_codes_and_cancellation_stops_preflight() {
    isolated(
        "ROCKET_SECRET",
        "diagnostics_record_only_fixed_refusal_codes_and_cancellation_stops_preflight",
        || {
            let (f, remote) = base("sha1");
            f.write("PRIVATE-file.cfg", token());
            f.commit("private");
            let log = DiagnosticLog::open(f.root.join("logs")).unwrap();
            let repo = f
                .git
                .with_log(log.clone())
                .open(&f.repository, &c())
                .unwrap();
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
            blocked(report.result.as_ref().unwrap_err(), "github_token_detected");
            drop(repo);
            drop(log);
            for entry in fs::read_dir(f.root.join("logs")).unwrap() {
                let path = entry.unwrap().path();
                if !path.is_file() {
                    continue;
                }
                let bytes = fs::read(&path).unwrap();
                let text = String::from_utf8_lossy(&bytes);
                assert!(!text.contains(&token()));
                assert!(!text.contains("PRIVATE-file.cfg"));
            }
            let cancel = c();
            cancel.cancel();
            assert!(matches!(
                f.repo().push(
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
            assert!(refs(&f, &remote).is_empty());
        },
    );
}
