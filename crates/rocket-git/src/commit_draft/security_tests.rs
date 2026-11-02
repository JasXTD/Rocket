use super::*;

const SUCCESS: &[u8] = b"{\"type\":\"item.completed\",\"item\":{\"type\":\"agent_message\",\"text\":\"Safe draft\"}}\n{\"type\":\"turn.completed\"}\n";

#[test]
fn successful_codex_turn_cannot_hide_config_drift_on_stderr() {
    for warning in [
        "WARNING: unknown config field: tools.shell\n",
        "Warning: unrecognized configuration key: agents.enabled\n",
        "WARNING: Unknown feature flag: shell_tool\n",
        "WARNING: Unrecognized feature key: code_mode_host\n",
        "Error loading config: unknown field `tools`\n",
        "WARNING: unsupported configuration setting: web_search\n",
    ] {
        assert!(
            matches!(
                codex_reply(true, SUCCESS, warning.as_bytes()),
                Err(Error::CommitDraft {
                    provider: DraftProvider::Codex,
                    failure: DraftFailure::CodexOutdated,
                    ..
                })
            ),
            "accepted configuration drift: {warning}"
        );
    }
}

#[test]
fn successful_codex_turn_cannot_hide_structured_config_warnings() {
    for event in [
        r#"{"type":"error","message":"Unrecognized config key: tools.shell"}"#,
        r#"{"type":"warning","message":"Unknown feature flag: shell_tool"}"#,
        r#"{"type":"item.completed","item":{"type":"error","message":"Unknown configuration field: tools"}}"#,
        r#"{"type":"turn.failed","error":{"message":"Unknown configuration field: tools"}}"#,
    ] {
        let output = [event.as_bytes(), b"\n", SUCCESS].concat();
        assert!(
            matches!(
                codex_reply(true, &output, b""),
                Err(Error::CommitDraft {
                    failure: DraftFailure::CodexOutdated,
                    ..
                })
            ),
            "accepted {event}"
        );
    }
}

#[test]
fn expected_codex_warnings_and_message_content_remain_usable() {
    assert_eq!(
        codex_reply(
            true,
            SUCCESS,
            b"WARNING: Code Mode is unavailable because code-mode host is disabled.\n"
        )
        .unwrap(),
        "Safe draft"
    );
    let output = br#"{"type":"item.completed","item":{"type":"agent_message","text":"Fix unknown configuration fields"}}
{"type":"turn.completed"}"#;
    assert_eq!(
        codex_reply(true, output, b"").unwrap(),
        "Fix unknown configuration fields"
    );
}
