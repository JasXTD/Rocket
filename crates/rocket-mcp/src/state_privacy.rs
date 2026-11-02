//! Keep editor and shell contents out of the general MCP state by default.
use serde_json::{Value, json};

pub(crate) fn protect(mut state: Value, opted_in: bool) -> Value {
    state["private_state_opt_in"] = json!(opted_in);
    if opted_in {
        return state;
    }
    redact(&mut state["terminal"], &["tail", "grid", "error"]);
    if let Some(playground)=state.get_mut("integrations").and_then(|s|s.get_mut("playground")) {redact(playground,&["text"]);}
    redact(
        &mut state["external_tools"],
        &["arguments", "result", "error", "notice"],
    );
    if let Some(tabs) = state.get_mut("tabs").and_then(Value::as_array_mut) {
        for tab in tabs {
            redact(&mut tab["conflict_editor"], &["draft", "error", "notice"]);
            redact(&mut tab["branch_review"], &["payload", "result", "error", "notice"]);
        }
    }
    state
}

fn redact(value: &mut Value, fields: &[&str]) {
    if let Some(object) = value.as_object_mut() {
        for field in fields {
            object.remove(*field);
        }
        object.insert("text_redacted".into(), json!(true));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn private_text_is_absent_even_when_the_terminal_is_hidden() {
        let state = json!({"integrations":{"playground":{"text":{"source":"PRIVATE_LUA","state":"PRIVATE_CALLBACK"},"has_preview":true}},"terminal":{"visible":false,"tail":"PRIVATE_SHELL","grid":[{"text":"PRIVATE_GRID"}],"running":true},
            "external_tools":{"arguments":"PRIVATE_ARGUMENTS","result":"PRIVATE_RESULT","success":true},
            "tabs":[{"conflict_editor":{"draft":"PRIVATE_DRAFT","draft_bytes":13,"dirty":true},
                "branch_review":{"payload":"PRIVATE_DIFF","result":"PRIVATE_FINDING","error":"PRIVATE_CLI_ERROR","notice":"PRIVATE_NOTICE","preview":{"files":3}}}]});
        let public = protect(state.clone(), false);
        assert!(!public.to_string().contains("PRIVATE_"));
        assert_eq!(public["terminal"]["running"], true);
        assert_eq!(public["integrations"]["playground"]["has_preview"],true);
        assert_eq!(public["tabs"][0]["conflict_editor"]["draft_bytes"], 13);
        assert_eq!(public["tabs"][0]["branch_review"]["preview"]["files"], 3);
        let opted = protect(state, true);
        assert_eq!(opted["private_state_opt_in"], true);
        assert_eq!(opted["integrations"]["playground"]["text"]["source"],"PRIVATE_LUA");
        assert_eq!(opted["terminal"]["tail"], "PRIVATE_SHELL");
        assert_eq!(opted["tabs"][0]["branch_review"]["result"], "PRIVATE_FINDING");
    }

    #[test]
    fn consent_requires_the_exact_local_marker_and_revokes_immediately() {
        let root = std::env::temp_dir().join(format!(
            "rocket-private-state-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&root).unwrap();
        let policy = crate::policy::Policy { root: root.clone() };
        let marker = root.join("rocket-private-state");
        assert!(!policy.private_state_allowed());
        for invalid in ["true", "synthetic-content-only-v1\nextra", &"x".repeat(100)] {
            std::fs::write(&marker, invalid).unwrap();
            assert!(!policy.private_state_allowed());
        }
        std::fs::write(&marker, "synthetic-content-only-v1\n").unwrap();
        assert!(policy.private_state_allowed());
        std::fs::remove_file(&marker).unwrap();
        assert!(!policy.private_state_allowed());
        std::fs::remove_dir(root).unwrap();
    }
}
