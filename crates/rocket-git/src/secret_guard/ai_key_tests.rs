use super::{Policy, SecretKind, scan, scan_with_policy};

fn generated(length: usize, alphabet: &[u8]) -> String {
    let mut state = 0x54b87913u64;
    (0..length)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            alphabet[state as usize % alphabet.len()] as char
        })
        .collect()
}
const LETTERS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";
const ALNUM: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
const HEX: &[u8] = b"0123456789abcdef";
fn provider_keys() -> Vec<String> {
    let mut keys = vec![
        format!(
            "sk-{}T3BlbkFJ{}",
            generated(20, ALNUM),
            generated(20, ALNUM)
        ),
        format!("gsk_{}", generated(52, ALNUM)),
        format!("xai-{}", generated(80, ALNUM)),
        format!("hf_{}", generated(34, LETTERS)),
        format!("AIza{}", generated(35, ALNUM)),
        format!("sk-or-v1-{}", generated(64, HEX)),
    ];
    for prefix in ["sk-proj-", "sk-svcacct-", "sk-admin-"] {
        for before in [58, 74] {
            for after in [58, 74] {
                keys.push(format!(
                    "{prefix}{}T3BlbkFJ{}",
                    generated(before, ALNUM),
                    generated(after, ALNUM)
                ));
            }
        }
    }
    for prefix in ["sk-ant-api03-", "sk-ant-api01-", "sk-ant-admin01-"] {
        keys.push(format!("{prefix}{}AA", generated(93, ALNUM)));
    }
    keys
}
fn encode(text: &str, encoding: usize) -> Vec<u8> {
    match encoding {
        0 => text.as_bytes().to_vec(),
        1 => [b"\xef\xbb\xbf".as_slice(), text.as_bytes()].concat(),
        little => {
            let mut bytes = if little == 2 {
                vec![0xff, 0xfe]
            } else {
                vec![0xfe, 0xff]
            };
            bytes.extend(text.encode_utf16().flat_map(|n| {
                if little == 2 {
                    n.to_le_bytes()
                } else {
                    n.to_be_bytes()
                }
            }));
            bytes
        }
    }
}
fn detected(bytes: &[u8], message: bool, secrets: bool, ai_keys: bool) -> Option<SecretKind> {
    scan_with_policy(bytes, message, Policy { secrets, ai_keys }, &mut || Ok(()))
        .unwrap()
        .map(|(kind, _)| kind)
}

#[test]
fn ai_provider_formats_scan_all_text_and_encodings_without_variable_name_requirements() {
    for key in provider_keys() {
        for source in [
            key.clone(),
            format!("// example: {key}"),
            format!("const label = r#\"{key}\"#;"),
            format!("{{\"opaque\":\"{key}\"}}"),
            format!("Authorization: Bearer {key}"),
        ] {
            for encoding in 0..4 {
                for message in [false, true] {
                    assert_eq!(
                        detected(&encode(&source, encoding), message, false, true),
                        Some(SecretKind::AiApiKey)
                    );
                    assert_eq!(
                        detected(&encode(&source, encoding), message, false, false),
                        None
                    );
                }
            }
        }
    }
}

#[test]
fn ai_deepseek_and_glm_use_credential_context_not_generic_hash_or_dotted_id_detection() {
    for value in [
        format!("sk-{}", generated(32, HEX)),
        format!("{}.{}", generated(32, HEX), generated(16, ALNUM)),
        format!("{}.{}", generated(64, HEX), generated(64, ALNUM)),
    ] {
        for name in [
            "DEEPSEEK_API_KEY",
            "ZAI_API_KEY",
            "ZHIPUAI_API_KEY",
            "GLM_API_KEY",
            "apiKey",
            "api_key",
        ] {
            for source in [
                format!("{name}={value}"),
                format!("const {name}: &str = r#\"{value}\"#;"),
                format!("{{\"{name}\":\"{value}\"}}"),
                format!("{name}: '{value}'"),
            ] {
                for encoding in 0..4 {
                    for message in [false, true] {
                        assert_eq!(
                            detected(&encode(&source, encoding), message, false, true),
                            Some(SecretKind::AiApiKey)
                        );
                    }
                }
            }
        }
        for source in [
            value.clone(),
            format!("checksum = \"{value}\""),
            format!("const label = \"{value}\";"),
        ] {
            assert_eq!(detected(source.as_bytes(), false, false, true), None);
        }
    }
}

#[test]
fn ai_placeholders_references_identifiers_and_wrong_shapes_do_not_match() {
    let mut cases = vec![
        "OPENAI_API_KEY=${OPENAI_API_KEY}".into(),
        "ZAI_API_KEY=process.env.ZAI_API_KEY".into(),
        "GLM_API_KEY='${GLM_API_KEY}'".into(),
        "DEEPSEEK_API_KEY=sk-your-api-key".into(),
        "api_key='YOUR_API_KEY'".into(),
        "api_key='id.secret'".into(),
        "fn hf_requiredCharacteristicTypesForDisplayMetadata() {}".into(),
        "const SECRET: &str = \"XD-PRIVATE-7f3a\";".into(),
        "let token = \"IDM_EX_AV_REQUESTED_V1:malwarebytes\\n\";".into(),
    ];
    for prefix in [
        "sk-proj-",
        "sk-svcacct-",
        "sk-admin-",
        "sk-ant-api03-",
        "sk-ant-admin01-",
        "gsk_",
        "xai-",
        "hf_",
        "AIza",
        "sk-or-v1-",
    ] {
        cases.push(format!("{prefix}..."));
        cases.push(format!("{prefix}YOUR_API_KEY"));
        cases.push(format!("{prefix}{}", "x".repeat(160)));
    }
    for key in provider_keys() {
        cases.push(format!("identifier_{key}"));
        cases.push(format!("{key}_extra"));
        cases.push(key[..key.len() - 1].into());
    }
    for source in cases {
        for encoding in 0..4 {
            assert_eq!(
                detected(&encode(&source, encoding), false, false, true),
                None,
                "ordinary source, encoding={encoding}"
            );
        }
    }
    // Both guards still allow the actual earlier false positives.
    assert_eq!(scan(b"const SECRET: &str = \"XD-PRIVATE-7f3a\";"), None);
    assert_eq!(
        scan(b"let token = \"IDM_EX_AV_REQUESTED_V1:malwarebytes\\n\";"),
        None
    );
}

#[test]
fn ai_policy_independence_preserves_general_protection_and_redacted_locations() {
    let password = format!("password=\"{}\"", generated(40, ALNUM));
    assert_eq!(
        detected(password.as_bytes(), false, true, false),
        Some(SecretKind::Credential)
    );
    assert_eq!(detected(password.as_bytes(), false, false, true), None);
    let provider = provider_keys().remove(1);
    assert_eq!(
        super::safe_path(provider.as_bytes()),
        "[sensitive filename hidden]"
    );
    let key_assignment = format!("api_key=\"{provider}\"");
    assert_eq!(
        detected(key_assignment.as_bytes(), false, true, false),
        Some(SecretKind::Credential)
    );
    assert_eq!(
        detected(key_assignment.as_bytes(), false, false, true),
        Some(SecretKind::AiApiKey)
    );
}

#[test]
fn ai_scan_is_cancellable_and_matches_across_chunk_boundaries() {
    let key = provider_keys().remove(0);
    let mut bytes = vec![b' '; 64 * 1024 - 2];
    bytes.extend(key.as_bytes());
    assert_eq!(
        detected(&bytes, false, false, true),
        Some(SecretKind::AiApiKey)
    );
    let bytes = vec![b' '; 8 * 1024 * 1024];
    let mut checks = 0;
    let error = scan_with_policy(
        &bytes,
        false,
        Policy {
            secrets: false,
            ai_keys: true,
        },
        &mut || {
            checks += 1;
            if checks >= 4 {
                Err(crate::Error::Cancelled)
            } else {
                Ok(())
            }
        },
    )
    .unwrap_err();
    assert!(matches!(error, crate::Error::Cancelled));
    assert_eq!(checks, 4);
}
