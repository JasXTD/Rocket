//! AWS access keys, Google OAuth client secrets, provider-prefixed client-secret
//! names and JWK private keys. Values are assembled at run time, so this file
//! holds no complete key.
use super::{SecretKind, encoded, generated, safe_path, scan, scan_message};

fn upper(seed: u64, length: usize) -> String {
    generated(seed, length).to_ascii_uppercase()
}

fn both(text: &str) -> [Option<(SecretKind, usize)>; 2] {
    [scan(text.as_bytes()), scan_message(text.as_bytes())]
}

#[test]
fn aws_access_key_ids_are_blocked_in_any_text_and_encoding() {
    for (index, prefix) in ["AKIA", "ASIA", "ABIA", "ACCA"].into_iter().enumerate() {
        for seed in 0..8 {
            let id = format!("{prefix}{}", upper(index as u64 * 8 + seed, 16));
            for source in [
                id.clone(),
                format!("aws_access_key_id = {id}"),
                format!("        \"AccessKeyId\": \"{id}\","),
                format!("accessKeyId: '{id}', // rotated"),
                format!("export AWS_ACCESS_KEY_ID={id}"),
                format!("X-Amz-Credential={id}%2F20261114%2Fus-east-1%2Fs3%2Faws4_request"),
                format!("legacy_key_{id}"),
                format!("Rotate {id} after the incident"),
            ] {
                for encoding in 0..4 {
                    let bytes = encoded(&source, encoding);
                    let expected = Some((SecretKind::AwsAccessKey, 1));
                    assert_eq!(scan(&bytes), expected, "{source}, encoding={encoding}");
                    assert_eq!(
                        scan_message(&bytes),
                        expected,
                        "{source}, encoding={encoding}"
                    );
                }
            }
            assert_eq!(safe_path(id.as_bytes()), "[sensitive filename hidden]");
        }
    }
    // Generated IDs can repeat characters; only filler is a placeholder.
    let repetitive = format!("AKIA{}", &"K7Q2M4".repeat(3)[..16]);
    assert_eq!(both(&repetitive), [Some((SecretKind::AwsAccessKey, 1)); 2]);
}

#[test]
fn aws_documentation_examples_other_ids_and_partial_values_are_not_keys() {
    let body = upper(3, 16);
    for source in [
        // AWS documentation examples end in EXAMPLE.
        format!("aws_access_key_id = AKIA{}EXAMPLE", "IOSFODNN7"),
        format!("\"AccessKeyId\": \"ASIA{}EXAMPLE\"", "I44QH8DHB"),
        // Placeholders, fragments and slices of longer identifiers.
        format!("AKIA{}", "X".repeat(16)),
        format!("ASIA{}", "AB".repeat(8)),
        format!("AKIA{}", &body[..15]),
        format!("AKIA{body}7"),
        format!("xAKIA{body}"),
        format!("AKIA{}", body.to_ascii_lowercase()),
        // Users, roles, groups and policies have IDs too; they are not credentials.
        format!("AIDA{body}"),
        format!("AROA{body}"),
        format!("AGPA{body}"),
        format!("ANPA{body}"),
        "Regions: ASIA PACIFIC and AKIA".into(),
    ] {
        assert_eq!(both(&source), [None, None], "{source}");
    }
}

#[test]
fn aws_secret_and_session_names_are_credential_assignments() {
    for seed in 0..16 {
        // Secret access keys are 40 base64 characters, often with / and +.
        let secret = format!(
            "{}9/{}4+{}",
            generated(seed, 12),
            generated(seed + 50, 12),
            generated(seed + 100, 12)
        );
        let session = format!("IQoJb3JpZ2luX2Vj{}", generated(seed, 600));
        for source in [
            format!("  secretAccessKey: '{secret}',"),
            format!("        \"SecretAccessKey\": \"{secret}\","),
            format!("AWS_SECRET_KEY={secret}"),
            format!("export MINIO_SECRET_ACCESS_KEY={secret}"),
            format!("aws_secret_access_key = {secret}"),
            format!("aws_session_token = {session}"),
            format!("export AWS_SESSION_TOKEN=\"{session}\""),
        ] {
            let expected = Some((SecretKind::Credential, 1));
            assert_eq!(both(&source), [expected; 2], "{source}");
        }
    }
    let example = format!("wJalrXUtnFEMI/K7MDENG/bPxRfiCY{}", "EXAMPLEKEY");
    for source in [
        format!("aws_secret_access_key = {example}"),
        format!("aws_secret_access_key = \"{example}\""),
        format!("\"SecretAccessKey\": \"{example}\""),
        format!("AWS_SECRET_KEY='{example}'"),
        "secretAccessKey: process.env.AWS_SECRET_ACCESS_KEY,".into(),
        "AWS_SESSION_TOKEN=${AWS_SESSION_TOKEN}".into(),
    ] {
        assert_eq!(both(&source), [None, None], "{source}");
    }
    // The documentation exemption covers AWS secret names only.
    assert_eq!(
        scan(format!("password = \"{example}\"").as_bytes()),
        Some((SecretKind::Credential, 1))
    );
}

#[test]
fn provider_prefixed_client_secret_names_are_credential_assignments() {
    for seed in 0..8 {
        let value = format!("n6F0{}", generated(seed, 36));
        // Microsoft Entra secrets contain ~ and . as well.
        let entra = format!(
            "{}8Q~{}",
            generated(seed, 3),
            generated(seed + 9, 34).replace('a', "~").replace('b', ".")
        );
        for key in [
            "OIDC_CLIENT_SECRET",
            "AZURE_CLIENT_SECRET",
            "googleClientSecret",
            "KEYCLOAK_CLIENT_SECRET",
            "auth0-client-secret",
            "GitHubClientSecret",
        ] {
            for source in [
                format!("{key}={value}"),
                format!("{key}: '{value}'"),
                format!("{{\"{key}\":\"{value}\"}}"),
                format!("{key}=\"{entra}\""),
            ] {
                let expected = Some((SecretKind::Credential, 1));
                assert_eq!(both(&source), [expected; 2], "{source}");
            }
        }
    }
    for source in [
        "OIDC_CLIENT_SECRET=${OIDC_CLIENT_SECRET}",
        "googleClientSecret: process.env.GOOGLE_CLIENT_SECRET,",
        "AZURE_CLIENT_SECRET=\"<azure-client-secret>\"",
        "const oidcClientSecret = config.clientSecret;",
        "readonly azureClientSecret: string;",
        "KEYCLOAK_CLIENT_SECRET=changeme",
    ] {
        assert_eq!(both(source), [None, None], "{source}");
    }
}

#[test]
fn google_oauth_client_secrets_are_blocked_without_a_credential_name() {
    for seed in 0..8 {
        let secret = format!("GOCSPX-{}-_", generated(seed, 26));
        for source in [
            secret.clone(),
            format!("const oauth = \"{secret}\";"),
            format!(
                "{{\"web\":{{\"client_id\":\"1234.apps.googleusercontent.com\",\"client_secret\":\"{secret}\"}}}}"
            ),
            format!("Rotated {secret} after review"),
        ] {
            for encoding in 0..4 {
                let bytes = encoded(&source, encoding);
                let expected = Some((SecretKind::Credential, 1));
                assert_eq!(scan(&bytes), expected, "{source}, encoding={encoding}");
                assert_eq!(
                    scan_message(&bytes),
                    expected,
                    "{source}, encoding={encoding}"
                );
            }
        }
    }
    let body = generated(1, 28);
    for source in [
        format!("GOCSPX-{}", &body[..27]),
        format!("GOCSPX-{body}x"),
        format!("xGOCSPX-{body}"),
        format!("GOCSPX-{}", "x".repeat(28)),
        "GOCSPX-...".into(),
    ] {
        assert_eq!(both(&source), [None, None], "{source}");
    }
}

#[test]
fn jwk_private_and_secret_keys_are_private_keys_in_common_notations() {
    let (n, d) = (generated(1, 342), generated(2, 342));
    let (x, y, small) = (generated(3, 43), generated(4, 43), generated(5, 43));
    let (p, q) = (generated(6, 171), generated(7, 171));
    let cases = [
        // RSA in compact JSON.
        (
            format!(
                r#"{{"kty":"RSA","kid":"k1","n":"{n}","e":"AQAB","d":"{d}","p":"{p}","q":"{q}"}}"#
            ),
            1,
        ),
        // EC, pretty-printed: the line of "d" is reported.
        (
            format!(
                "{{\n  \"kty\": \"EC\",\n  \"crv\": \"P-256\",\n  \"x\": \"{x}\",\n  \"y\": \"{y}\",\n  \"d\": \"{small}\"\n}}"
            ),
            6,
        ),
        // OKP with sorted members: "d" comes before "kty".
        (
            format!(r#"{{"crv":"Ed25519","d":"{small}","kty":"OKP","x":"{x}"}}"#),
            1,
        ),
        // Escaped JSON in an environment variable.
        (
            format!(r#"JWT_PRIVATE_KEY="{{\"kty\":\"EC\",\"crv\":\"P-256\",\"d\":\"{small}\"}}""#),
            1,
        ),
        // JavaScript object and YAML.
        (
            format!("const key = {{ kty: 'EC', crv: 'P-256', x: '{x}', y: '{y}', d: '{small}' }};"),
            1,
        ),
        (
            format!("signing_key:\n  kty: RSA\n  n: {n}\n  e: AQAB\n  d: {d}\n"),
            5,
        ),
        // A symmetric key in Python keyword arguments.
        (format!("key = jwk.JWK(kty=\"oct\", k=\"{small}\")"), 1),
        // A key set whose second key is private.
        (
            format!(
                "{{\"keys\":[{{\"kty\":\"EC\",\"x\":\"{x}\",\"y\":\"{y}\"}},\n{{\"kty\":\"EC\",\"x\":\"{x}\",\"y\":\"{y}\",\"d\":\"{small}\"}}]}}"
            ),
            2,
        ),
    ];
    for (source, line) in cases {
        for encoding in 0..4 {
            let bytes = encoded(&source, encoding);
            let expected = Some((SecretKind::PrivateKey, line));
            assert_eq!(scan(&bytes), expected, "{source}, encoding={encoding}");
            assert_eq!(
                scan_message(&bytes),
                expected,
                "{source}, encoding={encoding}"
            );
        }
    }
}

#[test]
fn jwk_private_members_accept_every_json_whitespace_position() {
    let material = generated(51, 64);
    let whitespace = ["", " ", "\t", "\r", "\n", "\r\n", " \t\r\n "];
    for (kind, member) in [("RSA", "d"), ("EC", "d"), ("OKP", "d"), ("oct", "k")] {
        for reverse in [false, true] {
            for gap in whitespace {
                // Each side of each colon independently, then all four together.
                for position in 0..=4 {
                    let mut gaps = [""; 4];
                    if position == 4 {
                        gaps.fill(gap);
                    } else {
                        gaps[position] = gap;
                    }
                    let key_type = format!("\"kty\"{}:{}\"{kind}\"", gaps[0], gaps[1]);
                    let private = format!("\"{member}\"{}:{}\"{material}\"", gaps[2], gaps[3]);
                    let (first, last) = if reverse {
                        (&private, &key_type)
                    } else {
                        (&key_type, &private)
                    };
                    let source = format!("\r\n{{\n  {first},\n  {last}\n}}\n");
                    let at = source.find(&format!("\"{member}\"")).unwrap();
                    let line = source.as_bytes()[..at]
                        .iter()
                        .filter(|&&b| b == b'\n')
                        .count()
                        + 1;
                    for encoding in 0..4 {
                        let bytes = encoded(&source, encoding);
                        let expected = Some((SecretKind::PrivateKey, line));
                        assert_eq!(
                            scan(&bytes),
                            expected,
                            "kind={kind} reverse={reverse} gap={gap:?} position={position} encoding={encoding}"
                        );
                        assert_eq!(scan_message(&bytes), expected);
                    }
                }
            }
        }
    }
}

#[test]
fn jwk_whitespace_lookahead_remains_cancellable() {
    let material = generated(52, 64);
    for position in 0..4 {
        // Spaces/tabs also exercise the old uninterruptible lookahead; line
        // breaks are covered by the format matrix above.
        let padding = " \t".repeat(512 * 1024);
        let mut gaps = [""; 4];
        gaps[position] = &padding;
        let source = format!(
            "{{\"kty\"{}:{}\"EC\",\"d\"{}:{}\"{material}\"}}",
            gaps[0], gaps[1], gaps[2], gaps[3]
        );
        let mut checks = 0;
        let result = super::super::jwk::find(source.as_bytes(), &mut || {
            checks += 1;
            if checks == 2 {
                Err(crate::Error::Cancelled)
            } else {
                Ok(())
            }
        });
        assert!(
            matches!(result, Err(crate::Error::Cancelled)),
            "whitespace position={position}"
        );
        assert_eq!(checks, 2);
    }
}

#[test]
fn public_jwks_schemas_and_distant_or_short_members_are_not_private_keys() {
    let (n, x, y, d) = (
        generated(11, 342),
        generated(12, 43),
        generated(13, 43),
        generated(14, 43),
    );
    let far = " ".repeat(17 * 1024);
    for source in [
        format!(r#"{{"keys":[{{"kty":"RSA","use":"sig","kid":"k1","n":"{n}","e":"AQAB"}}]}}"#),
        format!(r#"{{"kty":"EC","crv":"P-256","x":"{x}","y":"{y}"}}"#),
        format!("{{\"kty\"\r\n:\t\n\"EC\",\"x\"\n:\r\"{x}\",\"y\":\"{y}\"}}"),
        // A long "d" without a key type, or too far from one.
        format!(r#"{{"d":"{d}"}}"#),
        format!(r#"{{"kty":"EC","x":"{x}"}}{far}{{"d":"{d}"}}"#),
        // Short or repetitive private values.
        format!(r#"{{"kty":"EC","d":"{}"}}"#, &d[..39]),
        format!(r#"{{"kty":"RSA","d":"{}"}}"#, "A".repeat(342)),
        // Members that do not belong to the key type.
        format!(r#"{{"kty":"RSA","k":"{d}"}}"#),
        format!(r#"{{"kty":"oct","d":"{d}"}}"#),
        // Schemas describe members without values.
        r#"{"kty": {"type": "string", "enum": ["RSA", "EC"]}, "d": {"type": "string"}}"#.into(),
    ] {
        assert_eq!(both(&source), [None, None], "{source}");
    }
}

#[test]
fn jwk_check_is_cancellable_and_finds_a_private_key_after_many_public_keys() {
    let bytes = vec![b'x'; 9 * 1024 * 1024];
    let mut checks = 0;
    let result = super::super::jwk::find(&bytes, &mut || {
        checks += 1;
        if checks == 3 {
            Err(crate::Error::Cancelled)
        } else {
            Ok(())
        }
    });
    assert!(matches!(result, Err(crate::Error::Cancelled)));
    assert_eq!(checks, 3);
    let public = format!(r#"{{"kty":"EC","x":"{}"}},"#, generated(20, 43)).repeat(20_000);
    let private = format!(r#"{{"kty":"EC","d":"{}"}}"#, generated(21, 43));
    assert_eq!(
        scan(format!("[{public}\n{private}]").as_bytes()),
        Some((SecretKind::PrivateKey, 2))
    );
}

#[test]
fn new_general_rules_follow_the_secret_protection_switch() {
    use super::super::{Policy, scan_with_policy};
    for source in [
        format!("AKIA{}", upper(40, 16)),
        format!("GOCSPX-{}", generated(41, 28)),
        format!("OIDC_CLIENT_SECRET=n6F0{}", generated(42, 36)),
        format!(r#"{{"kty":"EC","d":"{}"}}"#, generated(43, 43)),
    ] {
        let off = Policy {
            secrets: false,
            ai_keys: true,
        };
        assert_eq!(
            scan_with_policy(source.as_bytes(), false, off, &mut || Ok(())).unwrap(),
            None,
            "{source}"
        );
        assert!(scan(source.as_bytes()).is_some(), "{source}");
    }
}
