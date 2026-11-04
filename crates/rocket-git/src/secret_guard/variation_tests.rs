//! Cross-format examples use inert, generated values and never contact a host.
use super::{SecretKind, encoded, generated, scan, scan_message};
use std::collections::BTreeSet;

fn check_cases(cases: &[(&str, String)], expected: Option<SecretKind>) {
    let mut failures = Vec::new();
    for (label, source) in cases {
        for ending in ["\n", "\r\n"] {
            let source = format!("// fixture 資料\n{source}\n").replace('\n', ending);
            for encoding in 0..4 {
                let bytes = encoded(&source, encoding);
                for (context, result) in [("file", scan(&bytes)), ("message", scan_message(&bytes))]
                {
                    if result.map(|(kind, _)| kind) != expected {
                        // Labels describe syntax only; do not echo candidate values.
                        failures.push(format!(
                            "{label}: {context}, encoding={encoding}, crlf={}, got={result:?}",
                            ending.len() == 2
                        ));
                    }
                }
            }
        }
    }
    eprintln!(
        "variation examples: {} cases, {} checks, {} mismatches",
        cases.len(),
        cases.len() * 16,
        failures.len()
    );
    let groups = failures
        .iter()
        .filter_map(|failure| failure.split_once(':').map(|(label, _)| label))
        .collect::<BTreeSet<_>>();
    assert!(
        failures.is_empty(),
        "Scanner mismatches: {} checks across these syntax forms: {groups:?}",
        failures.len()
    );
}

fn assignment_formats(value: &str) -> Vec<(&'static str, String)> {
    vec![
        ("rust inferred", format!("let token = \"{value}\";")),
        ("rust typed", format!("let token: &str = \"{value}\";")),
        ("rust raw", format!("let token = r#\"{value}\"#;")),
        ("typescript inferred", format!("const token = \"{value}\";")),
        (
            "typescript typed",
            format!("const token: string = \"{value}\";"),
        ),
        ("python plain", format!("token = '{value}'")),
        ("python annotated", format!("token: str = '{value}'")),
        ("python raw", format!("token = r'{value}'")),
        ("python bytes", format!("token = b'{value}'")),
        ("go shorthand", format!("token := \"{value}\"")),
        ("go typed", format!("var token string = \"{value}\"")),
        ("go raw shorthand", format!("token := `{value}`")),
        ("csharp ordinary", format!("string token = \"{value}\";")),
        ("csharp verbatim", format!("string token = @\"{value}\";")),
        (
            "java ordinary",
            format!("private final String token = \"{value}\";"),
        ),
        ("kotlin typed", format!("val token: String = \"{value}\"")),
        ("swift typed", format!("let token: String = \"{value}\"")),
        ("powershell", format!("$token = '{value}'")),
        ("shell", format!("export TOKEN='{value}'")),
        ("json", format!("{{\"token\":\"{value}\"}}")),
        ("yaml", format!("token: '{value}' # worker message")),
        ("toml", format!("token = '{value}'")),
        ("ini", format!("token=\"{value}\"")),
        (
            "javascript comparison",
            format!("if (token === \"{value}\") {{ return; }}"),
        ),
    ]
}

#[test]
fn varied_protocol_names_and_formats_are_not_credentials() {
    // These application-independent examples were not in the original matrix.
    let values = [
        "BUILD_STEP_DONE_V8:compiler",
        "QUEUE_ITEM_READY_V42:local-store",
        "JOB_RUN_FINISHED:worker.pool",
        "HEALTH_CHECK_PASSED_V9999:service.ready",
        "IMAGE_SCAN_COMPLETE_V3:image_worker",
        "NETWORK_REQUEST_ACCEPTED_V12:local_client",
        "EDITOR_SESSION_STARTED_V7:syntax_service",
        "CACHE_ENTRY_SAVED_V2:disk",
        "TELEMETRY_EVENT_ACKNOWLEDGED_V23:telemetry",
        "DOWNLOAD_TASK_STARTED_V11:worker",
        "AUTH_CALLBACK_COMPLETE_V10:browser",
        "RENDER_FRAME_READY_V2:renderer",
    ];
    let mut failures = Vec::new();
    let mut checked = 0;
    for (name, value) in values.iter().enumerate() {
        for ending in ["", r"\n", r"\r\n"] {
            for (format, source) in assignment_formats(&format!("{value}{ending}")) {
                for encoding in 0..4 {
                    let bytes = encoded(&source, encoding);
                    for (context, result) in
                        [("file", scan(&bytes)), ("message", scan_message(&bytes))]
                    {
                        checked += 1;
                        if result.is_some() {
                            failures.push(format!("name={name}, {format}, ending={ending:?}, encoding={encoding}, {context}"));
                        }
                    }
                }
            }
        }
    }
    eprintln!(
        "unrelated protocol examples: {} names x 3 terminators x 24 formats x 4 encodings x 2 contexts = {checked}; mismatches={}",
        values.len(),
        failures.len()
    );
    let groups = failures
        .iter()
        .filter_map(|failure| failure.split_once(", ending=").map(|(label, _)| label))
        .collect::<BTreeSet<_>>();
    assert!(
        failures.is_empty(),
        "Unexpected protocol refusals: {} checks across these name/format pairs: {groups:?}",
        failures.len()
    );
}

#[test]
fn ordinary_code_references_and_controls_pass_in_varied_formats() {
    let mut cases = assignment_formats("example");
    cases.extend([
        (
            "rust cancellation",
            "let token = cancel.child_token();".into(),
        ),
        (
            "rust generic types",
            "fn next(token: &Option<Arc<Mutex<CancellationToken>>>) {}".into(),
        ),
        (
            "typescript cancellation",
            "const token: AbortSignal = controller.signal;".into(),
        ),
        (
            "typescript narrowing",
            "if (token === undefined) return;".into(),
        ),
        (
            "python reference",
            "token: str = os.getenv('BUILD_ACCESS_TOKEN', '')".into(),
        ),
        ("python dataclass", "token: Optional[str] = None".into()),
        ("go context", "token := ctx.Done()".into()),
        (
            "csharp cancellation",
            "CancellationToken token = cancellation.Token;".into(),
        ),
        ("powershell env", "$token = $env:BUILD_ACCESS_TOKEN".into()),
        (
            "shell substitution",
            "TOKEN=$(read_token_from_agent)".into(),
        ),
        (
            "shell explicit reference",
            "TOKEN=\"${BUILD_ACCESS_TOKEN}\"".into(),
        ),
        (
            "json translated label",
            "{\"token\":\"接続コード 🔑\",\"password\":\"Choose a password\"}".into(),
        ),
        (
            "json schema reference",
            "{\"token\":\"#/components/schemas/SessionToken\"}".into(),
        ),
        ("toml example", "token = '<build-access-token>'".into()),
        (
            "yaml reference",
            "token: '${{ secrets.BUILD_ACCESS_TOKEN }}'".into(),
        ),
        (
            "log control flow",
            "logger.info(\"Request completed\", token_id);".into(),
        ),
        (
            "documentation type",
            "Fix token: CancellationToken cancellation handling".into(),
        ),
        (
            "sql bound parameter",
            "SELECT id FROM sessions WHERE token = @session_token;".into(),
        ),
    ]);
    check_cases(&cases, None);
}

#[test]
fn credential_like_values_are_blocked_in_varied_assignment_formats() {
    // A deterministic synthetic credential. It is not valid for any service.
    let value = format!("J7p9{}", generated(8907, 36));
    let mut cases = assignment_formats(&value);
    // A comparison is not a credential assignment. Provider signatures are
    // tested independently below, including their occurrence in comparisons.
    cases.retain(|(label, _)| *label != "javascript comparison");
    check_cases(&cases, Some(SecretKind::Credential));
}

#[test]
fn provider_tokens_remain_blocked_in_code_comments_and_raw_strings() {
    for family in ["ghp_", "gho_", "ghu_", "ghs_", "ghr_", "github_pat_"] {
        let value = format!("{family}{}", generated(8911, 40));
        let mut cases = assignment_formats(&value);
        cases.extend([
            ("comment", format!("// deployment note: {value}")),
            (
                "markdown",
                format!("```rust\nlet value = \"{value}\";\n```"),
            ),
            ("unrelated key", format!("{{\"message\":\"{value}\"}}")),
            (
                "after harmless protocol",
                format!("token = \"JOB_RUN_DONE_V2:worker\\n\"; other = \"{value}\";"),
            ),
            (
                "before harmless protocol",
                format!("other = \"{value}\"; token = \"JOB_RUN_DONE_V2:worker\\n\";"),
            ),
        ]);
        check_cases(&cases, Some(SecretKind::GithubToken));
    }
}

#[test]
fn private_keys_remain_blocked_beside_unrelated_normal_code() {
    let payload = generated(8917, 96);
    for label in [
        "PRIVATE KEY",
        "RSA PRIVATE KEY",
        "EC PRIVATE KEY",
        "OPENSSH PRIVATE KEY",
        "ENCRYPTED PRIVATE KEY",
    ] {
        let armor = format!("-----BEGIN {label}-----\n{payload}\n-----END {label}-----");
        let cases = [
            ("plain armor", armor.clone()),
            (
                "source documentation",
                format!("/* sample key\n{armor}\n*/"),
            ),
            (
                "after protocol",
                format!("let token = \"BUILD_STEP_DONE_V8:compiler\\r\\n\";\n{armor}"),
            ),
            (
                "before protocol",
                format!("{armor}\nconst token = \"QUEUE_ITEM_READY_V42:local-store\";"),
            ),
        ];
        check_cases(&cases, Some(SecretKind::PrivateKey));
    }
}

#[test]
fn expanded_literal_patterns_keep_boundaries_and_strong_fields() {
    let secret = format!("K8v2{}", generated(8923, 40));
    let protocol = "NETWORK_REQUEST_ACCEPTED_V12:local_client";
    let mut cases = vec![
        (
            "single-quoted equals data",
            format!("token: 'prefix={secret}'"),
        ),
        ("single-quoted colon data", format!("token: 'tag:{secret}'")),
        (
            "rust lifetime typed",
            format!("let password: &'static str = \"{secret}\";"),
        ),
        (
            "python qualified annotation",
            format!("token: typing.Optional[str] = '{secret}'"),
        ),
        (
            "typescript union annotation",
            format!("const token: string | undefined = '{secret}';"),
        ),
        (
            "go named type",
            format!("var token auth.Value = \"{secret}\""),
        ),
        (
            "rust multi-hash raw",
            format!("let token = r###\"{secret}\"###;"),
        ),
        ("rust raw bytes", format!("let token = br#\"{secret}\"#;")),
        ("python raw bytes", format!("token = rb'{secret}'")),
        ("python uppercase prefix", format!("token = BR'{secret}'")),
        (
            "csharp escaped quote",
            format!("string token = @\"prefix\"\"{secret}\";"),
        ),
        (
            "incomplete raw credential",
            format!("let token = r##\"{secret}"),
        ),
        (
            "raw protocol missing hashes",
            format!("let token = r##\"{protocol}\"#;"),
        ),
        (
            "raw protocol extra hashes",
            format!("let token = r#\"{protocol}\"##;"),
        ),
        (
            "raw protocol concatenation",
            format!("let token = r#\"{protocol}\"# + \"{secret}\";"),
        ),
        (
            "typed protocol concatenation",
            format!("const token: string = \"{protocol}\" + \"{secret}\";"),
        ),
        (
            "shorthand protocol concatenation",
            format!("token := \"{protocol}\" + \"{secret}\""),
        ),
        (
            "raw protocol followed by credential",
            format!("let token = r#\"{protocol}\"#; password = \"{secret}\";"),
        ),
        (
            "annotation window boundary",
            format!(
                "let token: &str = \"{protocol}\"{}+ \"{secret}\";",
                " ".repeat(600)
            ),
        ),
    ];
    for (name, source) in assignment_formats(protocol) {
        if name != "javascript comparison" {
            cases.push((
                name,
                source
                    .replace("token", "api_key")
                    .replace("TOKEN", "API_KEY"),
            ));
        }
    }
    check_cases(&cases, Some(SecretKind::Credential));
}

#[test]
fn annotations_do_not_capture_adjacent_values_or_comparisons() {
    let value = generated(8929, 40);
    let cases = [
        (
            "rust neighboring field",
            format!("token: CancellationToken, label = \"{value}\";"),
        ),
        (
            "typescript neighboring field",
            format!("token: AbortSignal; label = \"{value}\";"),
        ),
        (
            "commented value",
            format!("token: CancellationToken // label = \"{value}\";"),
        ),
        (
            "rust type reference",
            "let token: &'static str = EXISTING_ACCESS_TOKEN;".into(),
        ),
        (
            "go variable reference",
            "var token auth.Value = existingAccessToken".into(),
        ),
        (
            "python union reference",
            "token: str | None = existing_token".into(),
        ),
        (
            "go shorthand reference",
            "token := existingAccessToken".into(),
        ),
        (
            "normal comparison",
            format!("if (token == \"{value}\") {{ return; }}"),
        ),
        ("typed comparison", format!("token: Type == \"{value}\"")),
        ("function expression", "token: (value) => value".into()),
    ];
    check_cases(&cases, None);
}

#[test]
fn compound_protocol_patterns_still_reject_opaque_payloads() {
    let cases = [
        (
            "opaque joined subject",
            "token = \"NETWORK_REQUEST_ACCEPTED_V12:qazwsx.edcrfv.tgbyhn\"".into(),
        ),
        (
            "opaque joined tag",
            "token = \"QAZWSX_EDCRFV_TGBYHN_V7:local_client\"".into(),
        ),
        (
            "opaque single subject",
            "token = \"JOB_RUN_FINISHED:abcdefghijklmnopqrstuv\"".into(),
        ),
        (
            "digit-bearing subject",
            "token = \"TELEMETRY_EVENT_ACKNOWLEDGED_V23:worker_709b1c4d\"".into(),
        ),
        (
            "extra colon",
            "token = \"NETWORK_REQUEST_ACCEPTED_V12:local_client:A6kP9vW3\"".into(),
        ),
        (
            "appended opaque data",
            format!(
                "token = \"JOB_RUN_FINISHED:worker.pool{}\"",
                generated(8931, 40)
            ),
        ),
        (
            "specific auth field",
            "auth_token = \"NETWORK_REQUEST_ACCEPTED_V12:local_client\"".into(),
        ),
    ];
    check_cases(&cases, Some(SecretKind::Credential));
}
