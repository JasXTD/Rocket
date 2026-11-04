//! Counter labels are inert fixtures; no provider is contacted.
use super::{SecretKind, encoded, generated, scan, scan_message};

fn check(source: &str, expected: Option<SecretKind>, case: usize) {
    for ending in ["\n", "\r\n"] {
        for encoding in 0..4 {
            let bytes = encoded(&(source.to_owned() + ending), encoding);
            for result in [scan(&bytes), scan_message(&bytes)] {
                assert_eq!(
                    result.map(|(kind, _)| kind),
                    expected,
                    "case={case}, encoding={encoding}"
                );
            }
        }
    }
}

#[test]
fn counter_constants_use_structure_instead_of_names() {
    for (case, (namespace, label)) in [
        ("gho", "rocketFakeToken"), // The unchanged historical regression.
        ("svc", "transferBatchMarker"),
        ("build", "compilerSessionLabel"),
        ("trace", "NetworkRequestState"),
        ("APP", "commandWindowHandle"),
        ("ci", "previewCacheEntry"),
    ]
    .into_iter()
    .enumerate()
    {
        for start in 0..10u8 {
            for step in [1, 9] {
                let digits: String = (0..10)
                    .map(|i| char::from(b'0' + (start + i * step) % 10))
                    .collect();
                for separator in ['_', '-'] {
                    let value = format!("{namespace}{separator}{label}{separator}{digits}");
                    for key in ["TOKEN", "secret"] {
                        for source in [
                            format!("const {key}: &str = \"{value}\";"),
                            format!("pub(crate) const {key}: &'static str = r#\"{value}\"#;"),
                            format!("export const {key}: string = '{value}';"),
                            format!("const {key} = `{value}`; // descriptive counter"),
                        ] {
                            check(&source, None, case);
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn counter_constants_preserve_credential_fields_payloads_and_boundaries() {
    let marker = "svc_transferBatchMarker_0123456789";
    let opaque = generated(5172, 40);
    let mut cases = vec![
        format!("const TOKEN = \"{opaque}\";"),
        format!("const token = \"{marker}-{opaque}\";"),
        format!("const token = \"{opaque}_{marker}\";"),
        format!("const token = \"{marker}\" + \"{opaque}\";"),
        format!("const token = \"{marker}\".concat(\"{opaque}\");"),
        format!(
            "const token = \"{marker}\"{}+ \"{opaque}\";",
            " ".repeat(600)
        ),
        format!("const token = \"{marker}"),
        format!("const token = r##\"{marker}\"#;"),
        format!("const token = \"{marker}\" arbitrary_tail"),
        format!("const token = \"{marker}\"#extra"),
        format!("const token = \"{marker}\"; secret = \"{opaque}\";"),
        format!("let token = \"{marker}\";"),
        format!("token = '{marker}'"),
        format!("{{\"token\":\"{marker}\"}}"),
        format!("token: '{marker}'"),
        format!("// const token = \"{marker}\";"),
        format!("let const token = \"{marker}\";"),
        format!("const /* comment */ token = \"{marker}\";"),
        format!("{}const token = \"{marker}\";", " ".repeat(600)),
    ];
    for value in [
        "svc_transferBatchMarker_0123546789",
        "svc_transferBatchMarker_0012345678",
        "svc_transferBatchMarker_012345678",
        "svc_transferBatchMarker_01234567890",
        "svc_transferBatchMarker_012345a789",
        "svc_transferBatchMarker_0123456789_tail",
        "svc_transferBatchMarker_0123456789:extra",
        "svc_transferBatchMarker-0123456789",
        "SvC_transferBatchMarker_0123456789",
        "svc2_transferBatchMarker_0123456789",
        "svc_transferbatchmarker_0123456789",
        "svc_transferBAtchMarker_0123456789",
        "svc_abCdeFgh_0123456789",
        "svc_transferBatch2Marker_0123456789",
        "svc_qazwsxEdcrfvTgbyhn_0123456789",
        "svc_transferBatchMarkerExtraordinaryabcdefghijklmnopqrstuv_0123456789",
        "_transferBatchMarker_0123456789",
        "svc__0123456789",
    ] {
        cases.push(format!("const TOKEN: &str = \"{value}\";"));
    }
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
    for (case, source) in cases.iter().enumerate() {
        check(source, Some(SecretKind::Credential), case);
    }
}

#[test]
fn counter_labels_never_override_provider_keys_or_private_key_material() {
    let marker = "svc_transferBatchMarker_0123456789";
    let mut payloads = Vec::new();
    for prefix in ["ghp_", "gho_", "ghu_", "ghs_", "ghr_", "github_pat_"] {
        payloads.push((
            format!("{prefix}{}_0123456789", generated(5218, 40)),
            SecretKind::GithubToken,
        ));
    }
    // This label satisfies the counter grammar but has enough consecutive
    // token-alphabet bytes for the existing provider detector. It must still block.
    payloads.push((
        format!(
            "{}{}",
            "gho_", "transferBatchProcessingSampleMarker_0123456789"
        ),
        SecretKind::GithubToken,
    ));
    for value in [
        format!("gsk_{}", generated(5317, 52)),
        format!("sk-ant-api03-{}AA", generated(5321, 93)),
        format!("sk-{}", "0123456789abcdef".repeat(2)),
        format!("{}.{}", "0123456789abcdef".repeat(2), generated(5341, 16)),
    ] {
        payloads.push((value, SecretKind::AiApiKey));
    }
    payloads.push((
        format!(
            "-----BEGIN PRIVATE KEY-----\n{}\n-----END PRIVATE KEY-----",
            "Q3x7A9b2".repeat(8)
        ),
        SecretKind::PrivateKey,
    ));
    for (case, (payload, expected)) in payloads.iter().enumerate() {
        check(
            &format!("const TOKEN = \"{payload}\";"),
            Some(*expected),
            case,
        );
        check(
            &format!("const token = \"{marker}\";\nconst secret = \"{payload}\";"),
            Some(*expected),
            case,
        );
    }
}
