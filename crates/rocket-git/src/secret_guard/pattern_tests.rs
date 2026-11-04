//! Generated values are inert fixtures. These tests never contact a provider.
use super::{SecretKind, environment_file, safe_path, scan, scan_message};

#[path = "variation_tests.rs"]
mod variations;

#[test]
fn short_marker_constants_are_not_literal_credentials() {
    for (tag, suffix) in [
        ("XD-PRIVATE", "7f3a"),
        ("EXPORT-CANARY", "4b92"),
        ("CACHE-PROBE", "5ac9d1"),
        ("AUDIT_TRACE", "9e20c7a3"),
        ("QUEUE_ITEM", "c4f8"),
        ("NETWORK-SAMPLE", "a7c2"),
    ] {
        let separator = if tag.contains('_') { '_' } else { '-' };
        let marker = format!("{tag}{separator}{suffix}");
        for key in ["SECRET", "secret", "TOKEN", "token"] {
            for source in [
                format!("const {key}: &str = \"{marker}\";"),
                format!("pub const {key}: &'static str = r#\"{marker}\"#;"),
                format!("pub(crate) const {key}: &str = \"{marker}\";"),
                format!("export const {key}: string = '{marker}';"),
                format!("const {key} = `{marker}`; // visible marker"),
            ] {
                for ending in ["\n", "\r\n"] {
                    for encoding in 0..4 {
                        let bytes = encoded(&(source.clone() + ending), encoding);
                        assert_eq!(scan(&bytes), None, "marker constant, encoding={encoding}");
                        assert_eq!(
                            scan_message(&bytes),
                            None,
                            "marker in message, encoding={encoding}"
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn word_label_constants_are_not_literal_credentials() {
    for marker in [
        "fixture-token-DO-NOT-LOG",
        "local-parser-STATE-READY",
        "demo_session_EVENT_COMPLETE",
        "BUILD-SIGNAL-worker-ready",
        "queue-item-TRACE-ONLY",
        "preview-cache-LOCAL-READ",
        "disposable_PUBLISH_TOKEN_never_log",
        "offline_TRACE_EVENT_ignore_output",
    ] {
        for key in ["TOKEN", "token", "SECRET", "secret"] {
            for source in [
                format!("const {key}: &str = \"{marker}\";"),
                format!("pub const {key}: &'static str = r#\"{marker}\"#;"),
                format!("pub(crate) const {key}: &str = \"{marker}\";"),
                format!("export const {key}: string = '{marker}';"),
                format!("const {key} = `{marker}`; // visible label"),
            ] {
                for ending in ["\n", "\r\n"] {
                    for encoding in 0..4 {
                        let bytes = encoded(&(source.clone() + ending), encoding);
                        assert_eq!(scan(&bytes), None, "word label, encoding={encoding}");
                        assert_eq!(
                            scan_message(&bytes),
                            None,
                            "message label, encoding={encoding}"
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn marker_patterns_preserve_credential_fields_payloads_and_boundaries() {
    for marker in [
        "EXPORT-CANARY-4b92",
        "example-context-NEVER-TRANSMIT",
        "offline_TRACE_EVENT_ignore_output",
    ] {
        let opaque = format!("M8p4{}", generated(4017, 40));
        let mut cases = vec![
            format!("const SECRET: &str = \"{opaque}\";"),
            format!("const secret = r#\"{opaque}\"#;"),
            format!("const token = \"{opaque}\";"),
            format!("secret = \"{marker}\";"),
            format!("let secret: &str = \"{marker}\";"),
            format!("{{\"secret\":\"{marker}\"}}"),
            format!("secret: '{marker}'"),
            format!("// const secret = \"{marker}\";"),
            format!("\"const secret = '{marker}';\""),
            format!("let const secret = \"{marker}\";"),
            format!("const /* comment */ secret = \"{marker}\";"),
            format!("const secret = \"{marker}\"#extra"),
            format!("{}const secret = \"{marker}\";", " ".repeat(600)),
            format!("const secret = \"{marker}\" arbitrary_tail"),
            format!("const secret = \"{marker}-{opaque}\";"),
            format!("const secret = \"{marker}\" + \"{opaque}\";"),
            format!("const secret = \"{marker}"),
            format!("const secret = r##\"{marker}\"#;"),
            format!(
                "const secret = \"{marker}\"{}+ \"{opaque}\";",
                " ".repeat(600)
            ),
            format!("const secret = \"{marker}\"; password = \"{opaque}\";"),
            "const secret = \"QAZWSX-EDCRFV-TGBYHN-7f3a\";".into(),
            "const secret = \"EXPORT-CANARY-71bc95d2a4f6\";".into(),
            "const secret = \"EXPORT-CANARY-7f3a-92bc\";".into(),
            "const secret = \"EXPORT-CANARY-7f3a:more\";".into(),
            "const secret = \"EXPORT-CANARY-2A7c9F\";".into(),
            "const secret = \"EXPORT-CANARY-f9x2\";".into(),
            "const secret = \"EXPORT-CANARY-7938\";".into(),
            "const secret = \"example-context-NEVER-TRANSMIT-1234\";".into(),
            "const secret = \"qazwsx-edcrfv-TGBYHN-UJMIKO\";".into(),
            "const secret = \"example-context-NEVER-TrAnSmIt\";".into(),
            "const secret = \"example-context-NEVER_TRANSMIT\";".into(),
            "const secret = \"example-context-NEVER--TRANSMIT\";".into(),
            "const secret = \"example-CONTEXT-never-TRANSMIT\";".into(),
            "const secret = \"example-context-TOKEN-never-transmit\";".into(),
        ];
        for key in [
            "password",
            "passwd",
            "api_key",
            "api_secret",
            "client_secret",
            "auth_token",
            "access_token",
            "github_token",
            "db_password",
            "aws_secret_access_key",
        ] {
            cases.push(format!("const {key}: &str = \"{marker}\";"));
        }
        for (index, source) in cases.iter().enumerate() {
            for ending in ["\n", "\r\n"] {
                for encoding in 0..4 {
                    let bytes = encoded(&(source.clone() + ending), encoding);
                    assert_eq!(
                        scan(&bytes).map(|(kind, _)| kind),
                        Some(SecretKind::Credential),
                        "negative case={index}, encoding={encoding}"
                    );
                    assert_eq!(
                        scan_message(&bytes).map(|(kind, _)| kind),
                        Some(SecretKind::Credential),
                        "message negative case={index}, encoding={encoding}"
                    );
                }
            }
        }
        for (payload, expected) in [
            (
                format!("ghp_{}", generated(4021, 40)),
                SecretKind::GithubToken,
            ),
            (
                format!(
                    "-----BEGIN PRIVATE KEY-----\n{}\n-----END PRIVATE KEY-----",
                    "Q3x7A9b2".repeat(8)
                ),
                SecretKind::PrivateKey,
            ),
        ] {
            for source in [
                format!("const secret = \"{payload}\";"),
                format!("const secret = \"{marker}\";\n{payload}"),
            ] {
                for encoding in 0..4 {
                    let bytes = encoded(&source, encoding);
                    assert_eq!(scan(&bytes).map(|(kind, _)| kind), Some(expected));
                    assert_eq!(scan_message(&bytes).map(|(kind, _)| kind), Some(expected));
                }
            }
        }
    }
}

#[test]
fn protocol_identifiers_are_not_generic_token_credentials() {
    // Vary every component, including the absence of a version. The reported
    // value remains a regression example, not the detector's special case.
    for tag in [
        "IDM_EX_AV_REQUESTED",
        "UPLOAD_REQUEST_ACCEPTED",
        "CACHE_STATUS_READY",
        "PREVIEW_SESSION_STARTED",
    ] {
        for version in ["", "_V1", "_V27", "_V2026"] {
            for subject in [
                "malwarebytes",
                "another-engine",
                "file.sync",
                "local_worker",
            ] {
                let value = format!("{tag}{version}:{subject}");
                for source in [
                    format!("let token = \"{value}\";"),
                    format!("token = '{value}'"),
                    format!("const TOKEN = `{value}`; // request received"),
                    format!("{{\"token\":\"{value}\",\"status\":\"received\"}}"),
                    format!("token: \"{value}\""),
                ] {
                    for encoding in 0..4 {
                        let bytes = encoded(&source, encoding);
                        assert_eq!(scan(&bytes), None, "content: {source}, encoding {encoding}");
                        assert_eq!(
                            scan_message(&bytes),
                            None,
                            "message: {source}, encoding {encoding}"
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn protocol_identifiers_allow_one_escaped_line_ending() {
    for value in [
        "IDM_EX_AV_REQUESTED_V1:malwarebytes",
        "UPLOAD_REQUEST_ACCEPTED_V27:file.sync",
        "CACHE_STATUS_READY:local_worker",
        "PREVIEW_SESSION_STARTED_V2026:another-engine",
    ] {
        for ending in [r"\n", r"\r\n"] {
            for source in [
                format!("let token = \"{value}{ending}\";"),
                format!("token = '{value}{ending}'"),
                format!("const TOKEN = `{value}{ending}`; // request received"),
                format!("{{\"token\":\"{value}{ending}\",\"status\":\"received\"}}"),
                format!("token: \"{value}{ending}\""),
            ] {
                for encoding in 0..4 {
                    let bytes = encoded(&source, encoding);
                    assert_eq!(scan(&bytes), None, "content: {source}, encoding {encoding}");
                    assert_eq!(
                        scan_message(&bytes),
                        None,
                        "message: {source}, encoding {encoding}"
                    );
                }
            }
        }
    }
}

#[test]
fn protocol_line_endings_do_not_exempt_credentials_or_incomplete_literals() {
    let acknowledgment = "IDM_EX_AV_REQUESTED_V1:malwarebytes";
    let credential = "A9mK7pR4sT2vX8n6Q5bC3z";
    for ending in [r"\n", r"\r\n"] {
        let mut cases = [
            "password",
            "api_key",
            "auth_token",
            "github_token",
            "client_secret",
        ]
        .map(|key| format!("{key} = \"{acknowledgment}{ending}\""))
        .to_vec();
        cases.extend([
            format!("token = \"{credential}{ending}\""),
            format!("token = \"{acknowledgment}{credential}{ending}\""),
            format!("token = \"{acknowledgment}{ending}{credential}\""),
            format!("token = \"{acknowledgment}{ending}{ending}\""),
            format!("token = \"{acknowledgment}\\{ending}\""),
            format!("token = \"{acknowledgment}{ending}\" + \"{credential}\""),
            format!("token = \"{acknowledgment}{ending}\".concat(\"{credential}\")"),
            format!("token = \"{acknowledgment}{ending}"),
            format!(
                "token = \"{acknowledgment}{ending}\"{}+ \"{credential}\"",
                " ".repeat(600)
            ),
            format!("token = \"UPLOAD_REQUEST_ACCEPTED_V1:qazwsx.edcrfv.tgbyhn{ending}\""),
            format!("token = \"{acknowledgment}{ending}\"; password = \"{credential}\""),
            format!("token = \"{acknowledgment}{ending}\"; token = \"{credential}\""),
        ]);
        for source in cases {
            for encoding in 0..4 {
                let bytes = encoded(&source, encoding);
                assert_eq!(
                    scan(&bytes),
                    Some((SecretKind::Credential, 1)),
                    "content: {source}, encoding {encoding}"
                );
                assert_eq!(
                    scan_message(&bytes),
                    Some((SecretKind::Credential, 1)),
                    "message: {source}, encoding {encoding}"
                );
            }
        }
    }
    // No general unescaping, trimming, or multiple-terminator exemption.
    for suffix in [r"\r", r"\t", r"\0", r"\x0a", r"\u000a", r"\n\r", r"\n\t"] {
        let source = format!("token = \"{acknowledgment}{suffix}\"");
        assert_eq!(
            scan(source.as_bytes()),
            Some((SecretKind::Credential, 1)),
            "{source}"
        );
        assert_eq!(
            scan_message(source.as_bytes()),
            Some((SecretKind::Credential, 1)),
            "{source}"
        );
    }
}

#[test]
fn protocol_identifiers_do_not_exempt_credential_fields_or_opaque_values() {
    let acknowledgment = "IDM_EX_AV_REQUESTED_V1:malwarebytes";
    let credential = "A9mK7pR4sT2vX8n6Q5bC3z";
    let mut cases = [
        "password",
        "api_key",
        "auth_token",
        "github_token",
        "client_secret",
    ]
    .map(|key| format!("{key} = \"{acknowledgment}\""))
    .to_vec();
    cases.extend([
        format!("token = \"{acknowledgment}{credential}\""),
        format!("token = \"{credential}{acknowledgment}\""),
        "token = \"IDM_EX_AV_REQUESTED_V1:MALWAREBYTES\"".into(),
        "token = \"UPLOAD_REQUEST_ACCEPTED_V1:abcdefghijklmnoqrstu\"".into(),
        "token = \"UPLOAD_REQUEST_ACCEPTED_V1:qazwsx.edcrfv.tgbyhn\"".into(),
        "token = \"UPLOAD_REQUEST_ACCEPTED_V1:abcdefabcdefabcdefabcdefabcdef\"".into(),
        "token = \"QAZWSX_EDCRFV_TGBYHN_V1:local_worker\"".into(),
        "token = \"UPLOAD_REQUEST_ACCEPTED_V123456789:local_worker\"".into(),
        "token = \"UPLOAD_REQUEST_ACCEPTED_V1:engine_72b93d6a\"".into(),
        "token = \"UPLOAD_REQUEST_ACCEPTED_V1:engine:another\"".into(),
        "token = \"UPLOAD_REQUEST_ACCEPTED_V1:engine..another\"".into(),
        format!("token = \"{acknowledgment}\" + \"{credential}\""),
        format!("token = \"{acknowledgment}\".concat(\"{credential}\")"),
        format!("token = \"{acknowledgment}\", \"{credential}\""),
        format!(
            "token = \"{acknowledgment}\"{}+ \"{credential}\"",
            " ".repeat(600)
        ),
        format!("token = \"{acknowledgment}"),
    ]);
    for source in cases {
        for encoding in 0..4 {
            let bytes = encoded(&source, encoding);
            assert_eq!(
                scan(&bytes),
                Some((SecretKind::Credential, 1)),
                "content: {source}, encoding {encoding}"
            );
            assert_eq!(
                scan_message(&bytes),
                Some((SecretKind::Credential, 1)),
                "message: {source}, encoding {encoding}"
            );
        }
    }
}

#[test]
fn protocol_identifiers_do_not_hide_other_candidates() {
    let acknowledgment = "token = \"UPLOAD_REQUEST_ACCEPTED_V27:local_worker\\r\\n\";";
    let credential = "A9mK7pR4sT2vX8n6Q5bC3z";
    for source in [
        format!("{acknowledgment} token = \"{credential}\""),
        format!("{acknowledgment} password = \"{credential}\""),
        format!("password = \"{credential}\"; {acknowledgment}"),
    ] {
        assert_eq!(scan(source.as_bytes()), Some((SecretKind::Credential, 1)));
        assert_eq!(
            scan_message(source.as_bytes()),
            Some((SecretKind::Credential, 1))
        );
    }
    let github = format!("{acknowledgment} value = \"ghp_{}\";", generated(17, 36));
    let private_key = format!(
        "{acknowledgment}\n-----BEGIN PRIVATE KEY-----\n{}\n-----END PRIVATE KEY-----",
        generated(18, 64)
    );
    for (source, kind, line) in [
        (github, SecretKind::GithubToken, 1),
        (private_key, SecretKind::PrivateKey, 2),
    ] {
        assert_eq!(scan(source.as_bytes()), Some((kind, line)));
        assert_eq!(scan_message(source.as_bytes()), Some((kind, line)));
    }
}

#[test]
fn marker_prefixes_cannot_hide_literal_passwords() {
    for prefix in [
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
    ] {
        for separator in ["", "-"] {
            let candidate = format!("{prefix}{separator}A9mK7pR4sT2vX8n6Q5bC3z");
            for quote in ['\'', '"', '`'] {
                for encoding in 0..4 {
                    let bytes = encoded(&format!("password = {quote}{candidate}{quote}"), encoding);
                    assert_eq!(
                        scan(&bytes),
                        Some((SecretKind::Credential, 1)),
                        "prefix {prefix}, encoding {encoding}"
                    );
                    assert_eq!(
                        scan_message(&bytes),
                        Some((SecretKind::Credential, 1)),
                        "message prefix {prefix}, encoding {encoding}"
                    );
                }
            }
        }
    }
}

#[test]
fn password_literals_with_placeholder_prefixes_are_blocked() {
    let value = "A9mK7pR4sT2vX8n6Q5bC3z";
    let candidates = [
        format!("${value}"),
        format!("%{value}"),
        format!("<{value}"),
        format!("${{DATABASE_PASSWORD}}-{value}"),
        format!("%DATABASE_PASSWORD%-{value}"),
        format!("<database-password>-{value}"),
        format!("{{{{password}}}}-{value}"),
    ];
    let mut failures = Vec::new();
    for (case, candidate) in candidates.iter().enumerate() {
        for quote in ['\'', '"', '`'] {
            for encoding in 0..4 {
                let text = encoded(&format!("password = {quote}{candidate}{quote}"), encoding);
                if scan(&text) != Some((SecretKind::Credential, 1))
                    || scan_message(&text) != Some((SecretKind::Credential, 1))
                {
                    failures.push((case, quote, encoding));
                }
            }
        }
    }
    assert!(
        failures.is_empty(),
        "Missed literal password cases: {failures:?}"
    );
}

#[test]
fn password_literals_with_ascii_punctuation_are_blocked() {
    let value = "A9mK7pR4sT2vX8n6Q5bC3z";
    let mut failures = Vec::new();
    for punctuation in (b'!'..=b'~').filter(u8::is_ascii_punctuation) {
        for quote in ['\'', '"', '`'] {
            let candidate = format!("{value}{}{value}", char::from(punctuation))
                .replace('\\', "\\\\")
                .replace(quote, &format!("\\{quote}"));
            for encoding in 0..4 {
                let text = encoded(&format!("password = {quote}{candidate}{quote}"), encoding);
                if scan(&text) != Some((SecretKind::Credential, 1))
                    || scan_message(&text) != Some((SecretKind::Credential, 1))
                {
                    failures.push((punctuation, quote, encoding));
                }
            }
        }
    }
    assert!(
        failures.is_empty(),
        "Missed password punctuation cases: {failures:?}"
    );
}

#[test]
fn password_placeholders_and_references_remain_usable() {
    for value in [
        "${DATABASE_PASSWORD}",
        "${ProductionPasswordVersion123}",
        "${ProductionPasswordVersion123:-}",
        "${process.env.DATABASE_PASSWORD}",
        "$env:ProductionPasswordVersion123",
        "%ProductionPasswordVersion123%",
        "<production-password-v12>",
        "{{ ProductionPasswordVersion123 }}",
        "{{ .Values.database.password }}",
        "${{ secrets.ProductionPasswordVersion123 }}",
        "#/definitions/ProductionPasswordVersion123",
        "YOUR_PASSWORD",
        "example-password",
        "replace_me",
    ] {
        for quote in ['\'', '"', '`'] {
            for encoding in 0..4 {
                let text = encoded(&format!("password = {quote}{value}{quote}"), encoding);
                assert_eq!(
                    scan(&text),
                    None,
                    "placeholder: {value}, encoding {encoding}"
                );
                assert_eq!(scan_message(&text), None, "message placeholder: {value}");
            }
        }
    }
    for text in [
        "PASSWORD=$ProductionPasswordVersion123",
        "PASSWORD=${ProductionPasswordVersion123}",
        "PASSWORD=%ProductionPasswordVersion123%",
        "$password = $env:ProductionPasswordVersion123",
        "password = process.env.DATABASE_PASSWORD",
        "password = os.environ.get('DATABASE_PASSWORD')",
    ] {
        assert_eq!(scan(text.as_bytes()), None, "reference: {text}");
        assert_eq!(
            scan_message(text.as_bytes()),
            None,
            "message reference: {text}"
        );
    }
}

#[test]
fn binary_candidate_prefixes_cross_search_chunk_boundaries() {
    let prefixes = ["ghp_", "gho_", "ghu_", "ghs_", "ghr_", "github_pat_"];
    let headers = [
        "PRIVATE KEY",
        "ENCRYPTED PRIVATE KEY",
        "RSA PRIVATE KEY",
        "EC PRIVATE KEY",
        "OPENSSH PRIVATE KEY",
        "PGP PRIVATE KEY BLOCK",
    ];
    for offset in [64 * 1024 - 3, 64 * 1024 - 1, 64 * 1024, 64 * 1024 + 1] {
        for prefix in prefixes {
            let mut bytes = vec![0; offset];
            bytes[0] = b'\n';
            bytes.extend_from_slice(format!("{prefix}{}", generated(51, 40)).as_bytes());
            assert_eq!(
                scan(&bytes),
                Some((SecretKind::GithubToken, 2)),
                "{prefix} at {offset}"
            );
        }
        for header in headers {
            let mut bytes = vec![0; offset];
            bytes[0] = b'\n';
            bytes.extend_from_slice(
                format!("-----BEGIN {header}-----\n{}", "A".repeat(40)).as_bytes(),
            );
            assert_eq!(
                scan(&bytes),
                Some((SecretKind::PrivateKey, 2)),
                "{header} at {offset}"
            );
        }
    }
    // Dense false candidates exercise the same checks without an empty-chunk
    // shortcut; later real material must still be found at its exact line.
    for fill in *b"g-" {
        let mut bytes = vec![fill; 128 * 1024];
        assert_eq!(scan(&bytes), None);
        bytes.extend_from_slice(format!("\nghp_{}", generated(52, 40)).as_bytes());
        assert_eq!(scan(&bytes), Some((SecretKind::GithubToken, 2)));
    }
}

#[test]
fn large_content_checks_propagate_mid_scan_cancellation_and_deadlines() {
    let bytes = vec![b'x'; 9 * 1024 * 1024];
    for phase in 0..4 {
        let cancel = crate::CancellationToken::new();
        let mut checks = 0;
        let mut check = || {
            checks += 1;
            if checks == 3 {
                cancel.cancel();
            }
            cancel.check()
        };
        let result = match phase {
            0 => super::scan_with_check(&bytes, &mut check).map(|_| ()),
            1 => super::scan_message_with_check(&bytes, &mut check).map(|_| ()),
            2 => super::github(&bytes, &mut check).map(|_| ()),
            _ => super::assignment(
                &bytes,
                super::Context::Content,
                super::Policy::ALL,
                &mut check,
            )
            .map(|_| ()),
        };
        assert!(matches!(result, Err(crate::Error::Cancelled)));
        assert_eq!(checks, 3);
    }
    let mut checks = 0;
    let result = super::scan_with_check(&bytes, &mut || {
        checks += 1;
        if checks == 3 {
            Err(crate::Error::TimedOut)
        } else {
            Ok(())
        }
    });
    assert!(matches!(result, Err(crate::Error::TimedOut)));
    assert_eq!(checks, 3);
}

fn next(state: &mut u64) -> u64 {
    *state ^= *state << 13;
    *state ^= *state >> 7;
    *state ^= *state << 17;
    *state
}

fn generated(seed: u64, length: usize) -> String {
    let alphabet = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
    let mut state = seed + 0x183a_907d;
    (0..length)
        .map(|_| alphabet[next(&mut state) as usize % alphabet.len()] as char)
        .collect()
}

fn encoded(text: &str, encoding: usize) -> Vec<u8> {
    match encoding {
        0 => text.as_bytes().to_vec(),
        1 => [b"\xef\xbb\xbf".as_slice(), text.as_bytes()].concat(),
        2 | 3 => {
            let little = encoding == 2;
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
        _ => unreachable!(),
    }
}

#[test]
fn message_prose_allows_type_references_and_bare_object_ids() {
    // Assemble fixture assignments so scanning this source sees no credentials;
    // concat! preserves the exact bytes exercised by the stricter content scan.
    let messages = [
        "Fix GitHub token refresh".to_owned(),
        concat!("Fix token: ", "OAuth2AuthenticationTokenV3 ownership").to_owned(),
        "Rename token: \"AuthenticationTokenVersion2026\"".to_owned(),
        "token: OAuth2AuthenticationTokenV3".to_owned(),
        concat!(
            "Fix token: ",
            "X509CertificateAuthenticationProviderV2 lifetime"
        )
        .to_owned(),
        "Fix token: UserApiTokenV3 ownership".to_owned(),
        format!("token: {}", "0123456789abcdef".repeat(4)),
        "token: 0123456789abcdef0123456789abcdef01234567".to_owned(),
        "fix(token): support expiration".to_owned(),
        "Document token = process.env.GITHUB_TOKEN".to_owned(),
        "token: update GH-1234 and GH-5678".to_owned(),
        concat!(
            "Subject\n\nFix token: ",
            "OAuth2AuthenticationTokenV3 ownership"
        )
        .to_owned(),
        "```text\nordinary text\n```\ntoken: 0123456789abcdef0123456789abcdef01234567".to_owned(),
        "~~~~text\nordinary text\n~~~~\nRename token: \"AuthenticationTokenVersion2026\""
            .to_owned(),
    ];
    for (case, message) in messages.iter().enumerate() {
        for encoding in 0..4 {
            assert_eq!(
                scan_message(&encoded(message, encoding)),
                None,
                "case={case}, encoding={encoding}"
            );
        }
    }
    // These exceptions are a message policy, never a path/blob/header allowlist.
    for message in &messages[1..4] {
        assert!(matches!(
            scan(message.as_bytes()),
            Some((SecretKind::Credential, _))
        ));
    }
    eprintln!("message prose cases: {} x 4 encodings", messages.len());
}

#[test]
fn explicit_message_assignments_and_code_snippets_remain_strict() {
    let hash = "0123456789abcdef0123456789abcdef01234567";
    let messages = [
        format!("token={hash}"),
        format!("TOKEN = \"{hash}\""),
        format!("github_token: {hash}"),
        format!("api_key: {hash}"),
        format!("secret: {hash}"),
        format!("password: {hash}"),
        format!("token: \"{hash}\""),
        format!("{{\"token\":\"{hash}\"}}"),
        format!("Example {{ token: {hash} }}"),
        format!("Use `token: {hash}`"),
        format!("```yaml\ntoken: {hash}\n```"),
        format!("~~~yaml\ntoken: {hash}\n~~~"),
        format!("````yaml\n```\ntoken: {hash}\n````"),
        format!("```yaml\n~~~\ntoken: {hash}\n```"),
        format!("```yaml\n```not a closing fence\ntoken: {hash}\n```"),
        format!("    token: {hash}"),
        format!("\ttoken: {hash}"),
        "token: \"AuthenticationTokenVersion2026\"".to_owned(),
        "Rename token=\"AuthenticationTokenVersion2026\"".to_owned(),
        "password: \"CorrectHorseBatteryStaple\"".to_owned(),
    ];
    for (case, message) in messages.iter().enumerate() {
        for encoding in 0..4 {
            assert!(
                matches!(
                    scan_message(&encoded(message, encoding)),
                    Some((SecretKind::Credential, _))
                ),
                "case={case}, encoding={encoding}"
            );
        }
    }
    let value = "n6F0dL8pR2vW4sT9aQ3x";
    for message in [
        format!("token: {value}"),
        format!("Rotate token: \"{value}\""),
        format!("Rename token: \"{value}\""),
        format!("Subject\n\npassword: {value}"),
    ] {
        assert!(matches!(
            scan_message(message.as_bytes()),
            Some((SecretKind::Credential, _))
        ));
    }
    eprintln!(
        "strict message cases: {} x 4 encodings, plus four generic literals",
        messages.len()
    );
}

#[test]
fn repeated_message_clauses_remain_bounded_and_do_not_hide_a_later_secret() {
    let message = "Fix token: OAuth2AuthenticationTokenV3 ownership; ".repeat(16_384);
    let started = std::time::Instant::now();
    assert_eq!(scan_message(message.as_bytes()), None);
    let secret = format!("{message}\npassword=n6F0dL8pR2vW4sT9aQ3x");
    assert_eq!(
        scan_message(secret.as_bytes()),
        Some((SecretKind::Credential, 2))
    );
    eprintln!(
        "long message: {} bytes, {} ms for clean and secret variants",
        message.len(),
        started.elapsed().as_millis()
    );
}

#[test]
fn ordinary_language_configuration_and_documentation_examples_pass() {
    let examples = [
        (
            "rust crate visibility",
            concat!("pub(crate) token: ", "OAuth2AuthenticationTokenV3,"),
        ),
        (
            "rust parent visibility",
            concat!("pub(super) token: ", "OAuth2AuthenticationTokenV3,"),
        ),
        (
            "rust scoped visibility",
            concat!(
                "pub(in crate::auth) token: ",
                "OAuth2AuthenticationTokenV3,"
            ),
        ),
        ("rust lifetime", "token: &'a CancellationToken,"),
        (
            "rust nested generic",
            "token: Arc<Mutex<Option<CancellationToken>>>,",
        ),
        (
            "rust match",
            "Token::Password { password } => password.as_str(),",
        ),
        ("rust typed binding", "let token: Option<String> = None;"),
        (
            "rust raw label",
            "let password = r###\"Enter your password\"###;",
        ),
        ("rust closure", "let token = || state.token.clone();"),
        ("rust reference", "let token = &state.token;"),
        ("rust macro", "password: concat!(\"Enter \", \"password\"),"),
        ("typescript union", "token: string | undefined;"),
        ("typescript optional", "token?: AccessToken;"),
        (
            "typescript readonly",
            concat!("readonly token: ", "OAuth2AuthenticationTokenV3;"),
        ),
        ("typescript generic", "token: Promise<AccessToken>;"),
        ("typescript import", "import type { Token } from './token';"),
        (
            "typescript interpolation",
            "const token = `${process.env.GITHUB_TOKEN}`;",
        ),
        (
            "javascript destructuring",
            "const { token: existingToken } = session;",
        ),
        ("javascript alias", "export const token = existingToken;"),
        ("javascript default", "function login(password = '') {}"),
        (
            "javascript comparison",
            "if (password === suppliedPassword) return true;",
        ),
        (
            "javascript inequality",
            "if (token !== cachedToken) refresh();",
        ),
        ("python settings", "TOKEN = settings.GITHUB_TOKEN"),
        (
            "python getenv",
            "TOKEN = os.environ.get('GITHUB_TOKEN', '')",
        ),
        ("python type", "password: Optional[str] = None"),
        ("python bytes type", "token: bytes | None"),
        ("python parameter", "def login(password: str):"),
        ("python fstring", "token = f'{settings.GITHUB_TOKEN}'"),
        ("go selector", "token := config.AccessToken"),
        ("go pointer", "Token: &requestToken,"),
        ("go function", "token := os.Getenv(\"GITHUB_TOKEN\")"),
        (
            "csharp property",
            "public CancellationToken Token { get; set; }",
        ),
        ("csharp alias", "var token = existingCancellationToken;"),
        ("csharp expression", "Token => source.Token;"),
        ("java field", "private final String token = suppliedToken;"),
        (
            "java method",
            "String token = System.getenv(\"GITHUB_TOKEN\");",
        ),
        ("kotlin nullable", "val token: String? = null"),
        ("swift optional", "var token: String? = nil"),
        ("shell export", "export TOKEN=\"${GITHUB_TOKEN}\""),
        ("shell default", "TOKEN=${GITHUB_TOKEN:-}"),
        ("shell substitution", "TOKEN=$(get_token)"),
        ("powershell environment", "$token = $env:GITHUB_TOKEN"),
        (
            "powershell lookup",
            "$token = [Environment]::GetEnvironmentVariable('GITHUB_TOKEN')",
        ),
        (
            "json english label",
            "{\"password\": \"Choose a password with 12 characters\"}",
        ),
        ("json chinese label", "{\"password\": \"请输入密码\"}"),
        ("json japanese label", "{\"password\": \"パスワード\"}"),
        ("json arabic label", "{\"password\": \"كلمة المرور\"}"),
        ("json emoji label", "{\"token\": \"🔐 Access token\"}"),
        (
            "json escaped label",
            r#"{"password":"Enter \"your\" password"}"#,
        ),
        (
            "json schema",
            "{\"password\":{\"type\":\"string\",\"minLength\":12}}",
        ),
        ("json pointer", "{\"token\":\"#/definitions/AccessToken\"}"),
        ("yaml anchor", "password: *shared_database_password"),
        ("yaml reference", "password: !vault |"),
        ("yaml block", "password: |\n  ${DATABASE_PASSWORD}\n"),
        ("yaml helm", "password: '{{ .Values.database.password }}'"),
        ("yaml github actions", "token: ${{ secrets.GITHUB_TOKEN }}"),
        (
            "yaml erb",
            "password: <%= ENV.fetch('DATABASE_PASSWORD') %>",
        ),
        ("yaml boolean", "secret: false"),
        ("yaml null", "token: null"),
        ("toml environment", "token = '${GITHUB_TOKEN}'"),
        ("ini example", "api_key=YOUR_API_KEY"),
        ("ini redacted", "password=REDACTED"),
        ("ini masked", "password=********************************"),
        (
            "html password",
            "<input type='password' name='password' autocomplete='new-password'>",
        ),
        ("html binding", "<input :password=\"settings.password\">"),
        ("css variable", "--password: var(--input-placeholder);"),
        ("sql parameter", "WHERE password = @password"),
        ("sql prepared", "WHERE token = ?"),
        (
            "docs prefix",
            "Supported prefixes: ghp_, gho_, ghu_, ghs_, ghr_, github_pat_.",
        ),
        (
            "docs header",
            "A private PEM starts with -----BEGIN PRIVATE KEY----- and base64 data.",
        ),
        (
            "docs public key",
            "-----BEGIN PUBLIC KEY-----\nABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789\n-----END PUBLIC KEY-----",
        ),
        (
            "commit subject",
            "Fix token: CancellationToken propagation in background jobs",
        ),
    ];
    let mut failures = Vec::new();
    for (label, text) in examples {
        for encoding in 0..4 {
            if scan(&encoded(text, encoding)).is_some() {
                failures.push((label, encoding));
            }
        }
    }
    eprintln!(
        "expanded clean examples: {} x 4 encodings = {}",
        examples.len(),
        examples.len() * 4
    );
    assert!(
        failures.is_empty(),
        "Unexpected refusals (labels only): {failures:?}"
    );
}

#[test]
fn generated_generic_literals_are_detected_across_keys_and_formats() {
    let keys = [
        "TOKEN",
        "github_token",
        "gh-token",
        "GITHUB_PAT",
        "password",
        "passwd",
        "secret",
        "api_key",
        "api-secret",
        "accessToken",
        "auth_token",
        "client_secret",
        "AWS_SECRET_ACCESS_KEY",
        "database_password",
        "db-password",
    ];
    let mut count = 0;
    for seed in 0..32 {
        let start = if seed % 2 == 0 { 'n' } else { 'N' };
        let value = format!("{start}6F0{}", generated(seed, 40));
        for key in keys {
            let cases = [
                format!("{key}={value}"),
                format!("{key}: '{value}'"),
                format!("{{\"{key}\":\"{value}\"}}"),
                format!("{key} = \"{value}\" # settings"),
                format!("export {key}={value}"),
                format!("const {key} = '{value}';"),
                format!("{key} = `{value}`;"),
                format!("token=\"\"; {key}=\"{value}\""),
            ];
            for (format, text) in cases.iter().enumerate() {
                assert_eq!(
                    scan(text.as_bytes()),
                    Some((SecretKind::Credential, 1)),
                    "seed={seed}, key={key}, format={format}"
                );
                assert_eq!(
                    scan_message(text.as_bytes()),
                    Some((SecretKind::Credential, 1)),
                    "message seed={seed}, key={key}, format={format}"
                );
                count += 1;
            }
        }
    }
    eprintln!("generated generic literal cases: {count}");
}

#[test]
fn generated_github_tokens_survive_wrapping_and_encoding() {
    let mut count = 0;
    for seed in 0..16 {
        let body = generated(seed, 36);
        let mut tokens = ["ghp_", "gho_", "ghu_", "ghs_", "ghr_"]
            .map(|prefix| format!("{prefix}{body}"))
            .to_vec();
        tokens.push(format!(
            "github_pat_{}_{}",
            generated(seed, 22),
            generated(seed + 1, 59)
        ));
        tokens.push(format!(
            "ghs_1_eyJhbGciOiJIUzI1NiJ9.e30.{}-_",
            generated(seed, 480)
        ));
        for (family, token) in tokens.iter().enumerate() {
            let cases = [
                token.clone(),
                format!("// {token}"),
                format!("token=\"example-{token}\""),
                format!("token=\"dummy-{token}\""),
                format!("token=\"#/definitions/{token}\""),
                format!("token=\"${{{token}}}\""),
                format!("token=\"{{{{ {token} }}}}\""),
                format!("<value>{token}</value>"),
                format!("Authorization: Bearer {token}"),
                format!("See `{token}` in the fixture."),
                format!("r###\"{token}\"###"),
                format!("\0\u{1f}{token}\0"),
                format!("prefix\r\n{token}\r\nsuffix"),
            ];
            for (wrapper, text) in cases.iter().enumerate() {
                for encoding in 0..4 {
                    let expected_line = if wrapper == 12 { 2 } else { 1 };
                    assert_eq!(
                        scan(&encoded(text, encoding)),
                        Some((SecretKind::GithubToken, expected_line)),
                        "seed={seed}, family={family}, wrapper={wrapper}, encoding={encoding}"
                    );
                    assert_eq!(
                        scan_message(&encoded(text, encoding)),
                        Some((SecretKind::GithubToken, expected_line)),
                        "message seed={seed}, family={family}, wrapper={wrapper}, encoding={encoding}"
                    );
                    count += 1;
                }
            }
            assert_eq!(
                safe_path(format!("{}.cfg", token).as_bytes()),
                "[sensitive filename hidden]"
            );
        }
    }
    eprintln!("generated GitHub wrapper/encoding cases: {count}");
}

#[test]
fn private_keys_support_line_endings_and_armor_metadata() {
    let mut failures = Vec::new();
    let mut count = 0;
    for kind in [
        "PRIVATE KEY",
        "ENCRYPTED PRIVATE KEY",
        "RSA PRIVATE KEY",
        "EC PRIVATE KEY",
        "OPENSSH PRIVATE KEY",
        "PGP PRIVATE KEY BLOCK",
    ] {
        for newline in ["\n", "\r\n", "\\n", "\\r\\n"] {
            let metadata = if kind == "PGP PRIVATE KEY BLOCK" {
                vec![
                    newline.to_owned(),
                    format!(
                        "Version: Rocket synthetic fixture{newline}Comment: inert test data{newline}{newline}"
                    ),
                ]
            } else if kind == "RSA PRIVATE KEY" {
                vec![
                    String::new(),
                    format!(
                        "Proc-Type: 4,ENCRYPTED{newline}DEK-Info: AES-256-CBC,0123456789ABCDEF{newline}{newline}"
                    ),
                ]
            } else {
                vec![String::new()]
            };
            for (variant, metadata) in metadata.iter().enumerate() {
                let text = format!(
                    "-----BEGIN {kind}-----{newline}{metadata}{}{newline}-----END {kind}-----",
                    generated(7, 64)
                );
                for encoding in 0..4 {
                    if scan(&encoded(&text, encoding)) != Some((SecretKind::PrivateKey, 1))
                        || scan_message(&encoded(&text, encoding))
                            != Some((SecretKind::PrivateKey, 1))
                    {
                        failures.push((kind, newline, variant, encoding));
                    }
                    count += 1;
                }
            }
        }
    }
    eprintln!("private-key armor/encoding cases: {count}");
    assert!(
        failures.is_empty(),
        "Missed key shapes (metadata only): {failures:?}"
    );
}

#[test]
fn environment_names_and_similar_non_environment_names_are_distinguished() {
    let blocked = [
        ".env",
        ".ENV",
        ".env.local",
        ".env.production",
        ".env.test",
        ".env.example.bak",
        ".env.",
        "production.env",
        "test.ENV",
        "a.env",
        "my.env.env",
        ".envrc.env",
    ];
    let allowed = [
        ".env.example",
        ".ENV.EXAMPLE",
        ".env.sample",
        ".env.template",
        "environment.rs",
        "env",
        "env.json",
        ".envrc",
        "sample.env.example",
        "test.env.md",
        ".env-example",
        "env.template",
        "環境.env.example",
    ];
    for name in blocked {
        assert!(
            environment_file(name.as_bytes()),
            "blocked basename: {name}"
        );
    }
    for name in allowed {
        assert!(
            !environment_file(name.as_bytes()),
            "ordinary basename: {name}"
        );
    }
    eprintln!(
        "environment filename cases: {}",
        blocked.len() + allowed.len()
    );
}

#[test]
fn key_metadata_without_a_body_is_safe_but_truncated_material_still_blocks() {
    let header = "-----BEGIN PRIVATE KEY-----";
    for suffix in [
        "",
        "\n",
        "\nVersion: Rocket\nComment: documentation only\n\n",
        "\nProc-Type: 4,ENCRYPTED\nDEK-Info: AES-256-CBC,0123456789ABCDEF\n\n",
        "\nnot actual key data\n",
        "\nABCDEF\n-----END PRIVATE KEY-----",
    ] {
        assert_eq!(scan(format!("{header}{suffix}").as_bytes()), None);
    }
    for ending in ["", "\0", "\";", "\\n-----END PRIVATE KEY-----"] {
        let text = format!("{header}\n{}{ending}", generated(8, 64));
        assert_eq!(scan(text.as_bytes()), Some((SecretKind::PrivateKey, 1)));
    }
    let metadata = format!(
        "{header}\nComment: {}\n\n{}",
        "synthetic note ".repeat(500),
        generated(8, 64)
    );
    assert_eq!(scan(metadata.as_bytes()), Some((SecretKind::PrivateKey, 1)));
    // Repeated labels inside metadata must remain bounded and cannot invent a
    // key when no base64 body follows any of them.
    let misleading = format!("{header}\n{}", format!("Comment: {header}\n").repeat(256));
    assert_eq!(scan(misleading.as_bytes()), None);
    eprintln!("private-key negative/truncation/stress cases: 12");
}

#[test]
fn overlapping_armor_headers_refuse_when_scan_work_is_exhausted() {
    let header = "-----BEGIN PRIVATE KEY-----";
    let text = format!("{header}\n{}", format!("Comment: {header}\n").repeat(4096));
    let started = std::time::Instant::now();
    assert!(matches!(
        scan(text.as_bytes()),
        Some((SecretKind::Incomplete, _))
    ));
    assert!(matches!(
        scan_message(text.as_bytes()),
        Some((SecretKind::Incomplete, _))
    ));
    eprintln!(
        "overlapping armor refusal: {} bytes, {} ms",
        text.len(),
        started.elapsed().as_millis()
    );
}

#[test]
fn malformed_and_arbitrary_bytes_do_not_panic_or_expose_controls() {
    for bytes in [
        vec![],
        vec![0xff],
        vec![0xff, 0xfe, 0x00],
        vec![0xff, 0xfe, 0x00, 0xd8, 0x00],
        vec![0xfe, 0xff, 0xdc, 0x00],
        b"token=\"\\".to_vec(),
        b"password='".to_vec(),
    ] {
        assert_eq!(scan(&bytes), None);
    }
    let mut state = 0xa941_4f2d_8c70_35e9;
    let mut embedded = 0;
    for case in 0..10_000 {
        let length = next(&mut state) as usize % 1025;
        let bytes: Vec<u8> = (0..length).map(|_| next(&mut state) as u8).collect();
        if let Some((_, line)) = scan(&bytes) {
            assert!((1..=1 + bytes.iter().filter(|b| **b == b'\n').count()).contains(&line));
        }
        let safe = safe_path(&bytes);
        assert!(safe.len() <= 240 * 12);
        assert!(!safe.chars().any(char::is_control));
        if case % 64 == 0 {
            let token = format!("ghp_{}", generated(case, 36));
            let at = length / 2;
            let mixed = [&bytes[..at], token.as_bytes(), &bytes[at..]].concat();
            assert!(
                matches!(scan(&mixed), Some((SecretKind::GithubToken, _))),
                "embedded token case {case}"
            );
            assert_eq!(safe_path(&mixed), "[sensitive filename hidden]");
            embedded += 1;
        }
    }
    eprintln!("arbitrary byte inputs: 10000; embedded-token mutations: {embedded}");
}

#[test]
fn long_lines_and_candidate_boundaries_preserve_provider_detection() {
    let token = format!("ghp_{}", generated(8, 36));
    for offset in [
        0,
        1,
        29,
        30,
        35,
        36,
        255,
        256,
        510,
        511,
        512,
        513,
        4095,
        4096,
        1024 * 1024,
    ] {
        let text = format!("{}|{token}", "a".repeat(offset));
        assert_eq!(
            scan(text.as_bytes()),
            Some((SecretKind::GithubToken, 1)),
            "offset={offset}"
        );
    }
    let clean = "token: CancellationToken,\n".repeat(40_000);
    assert_eq!(scan(clean.as_bytes()), None);
    let text = format!("{clean}{token}");
    assert_eq!(
        scan(text.as_bytes()),
        Some((SecretKind::GithubToken, 40_001))
    );
    eprintln!("long-input cases: 15 offsets and two approximately 1 MiB files");
}
