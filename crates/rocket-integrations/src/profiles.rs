//! Versioned connection records and protected, non-executable panel drafts.
use crate::{Node, Package, id, invalid, object, text};
use rocket_git::Result;
use serde_json::{Value, json};
use std::collections::HashMap;

pub fn empty() -> Value {
    json!({"version":2,"profiles":{}})
}
pub fn default_connection(name: &str) -> Value {
    json!({"name":name,"enabled":false,"uncertain":false,"auth":"none",
        "origin":"","username":"","token":"","remember":false,"writes":false,
        "settings":{},"revision":"initial","credential":""})
}
pub fn selected<'a>(store: &'a Value, package: &str) -> &'a str {
    store["profiles"][package]["selected"]
        .as_str()
        .unwrap_or("default")
}
pub fn connection<'a>(store: &'a Value, package: &str, connection: &str) -> &'a Value {
    &store["profiles"][package]["connections"][connection]
}
pub fn ensure(store: &mut Value, package: &str) {
    if store["profiles"][package].is_null() {
        store["profiles"][package] =
            json!({"selected":"default","connections":{"default":default_connection("Default")}});
    }
}
pub fn connection_mut<'a>(store: &'a mut Value, package: &str, connection: &str) -> &'a mut Value {
    ensure(store, package);
    &mut store["profiles"][package]["connections"][connection]
}
/// The saved connections of one extension, by key.
fn connections<'a>(store: &'a Value, package: &str) -> impl Iterator<Item = (&'a String, &'a Value)> {
    store["profiles"][package]["connections"].as_object().into_iter().flatten()
}
/// Whether the extension is on: its switch reads the connections' saved state.
pub fn extension_enabled(store: &Value, package: &str) -> bool {
    connections(store, package).any(|(_, c)| c["enabled"] == true)
}
/// Whether an enabled connection was approved with other files than these.
pub fn needs_review(store: &Value, package: &Package) -> bool {
    connections(store, &package.id).any(|(_, c)| c["enabled"] == true && !package.approved(&c["approved"]))
}
/// Whether these exact files were approved before, so turning on needs no new review.
pub fn reviewed(store: &Value, package: &Package) -> bool {
    connections(store, &package.id).any(|(_, c)| package.approved(&c["approved"]))
}
/// The copy a connection last approved: the selected one if it has a copy.
pub fn approved_copy<'a>(store: &'a Value, package: &str) -> Option<&'a Value> {
    let approved = |c: &'a Value| c.get("approved").filter(|a| a.is_object());
    approved(connection(store, package, selected(store, package)))
        .or_else(|| connections(store, package).find_map(|(_, c)| approved(c)))
}
/// One review covers all of an extension's connections: each enabled or set-up connection
/// takes these exact files, or the selected one when none is. Access switches stay as saved.
pub fn approve(store: &mut Value, package: &Package) {
    ensure(store, &package.id);
    let snapshot = package.snapshot();
    let entry = &mut store["profiles"][&package.id];
    let chosen = entry["selected"].as_str().unwrap_or("default").to_owned();
    let Some(list) = entry["connections"].as_object_mut() else {
        return;
    };
    let usable = |c: &Value| {
        c["enabled"] == true || !package.network || c["origin"].as_str().is_some_and(|o| !o.is_empty())
    };
    let any = list.values().any(usable);
    for (key, connection) in list.iter_mut() {
        if usable(connection) || (!any && *key == chosen) {
            connection["enabled"] = json!(true);
            connection["approved"] = snapshot.clone();
        }
    }
}
/// Turns the extension off. Approvals stay, so turning it back on asks only if files changed.
pub fn disable(store: &mut Value, package: &str) {
    if let Some(list) = store["profiles"][package]["connections"].as_object_mut() {
        for connection in list.values_mut() {
            connection["enabled"] = json!(false);
        }
    }
}
/// Forgets an extension's connections, tokens and drafts.
pub fn remove(store: &mut Value, package: &str) {
    if let Some(list) = store["profiles"].as_object_mut() {
        list.remove(package);
    }
}
/// The name of a connection whose last write may have reached the service.
pub fn unchecked_write(store: &Value, package: &str) -> Option<String> {
    connections(store, package)
        .find(|(_, c)| c["uncertain"] == true)
        .map(|(_, c)| c["name"].as_str().unwrap_or("Default").to_owned())
}
/// The names of the connections that hold anything: a server, a token, settings or a draft.
pub fn saved_connections(store: &Value, package: &str) -> Vec<String> {
    connections(store, package)
        .filter(|(_, c)| {
            c["enabled"] == true
                || c.get("draft").is_some()
                || ["origin", "token", "credential"].iter().any(|k| c[*k].as_str().is_some_and(|s| !s.is_empty()))
        })
        .map(|(_, c)| c["name"].as_str().unwrap_or("Default").to_owned())
        .collect()
}
pub fn draft_count(store: &Value, package: &str) -> usize {
    connections(store, package).filter(|(_, c)| c.get("draft").is_some()).count()
}
pub fn upgrade(mut value: Value) -> Result<Value> {
    if value["version"] == 1 {
        let old = object(&value["profiles"])?.clone();
        value = empty();
        for (package, mut profile) in old {
            if !profile.is_object() {
                return Err(invalid("integration_store: Invalid legacy connection."));
            }
            profile["name"] = json!("Default");
            profile["revision"] = json!("legacy");
            profile["credential"] = json!(rocket_git::integration_credential_digest(
                profile["token"].as_str().unwrap_or("")
            )?);
            value["profiles"][package] =
                json!({"selected":"default","connections":{"default":profile}});
        }
    }
    validate(&value)?;
    Ok(value)
}
pub fn validate(value: &Value) -> Result<()> {
    if value["version"] != 2 || object(&value["profiles"])?.len() > 64 {
        return Err(invalid(
            "integration_store: Unsupported settings version or too many extensions.",
        ));
    }
    for (package, entry) in object(&value["profiles"])? {
        let connections = object(&entry["connections"])?;
        let mut names = std::collections::HashSet::new();
        if !id(package)
            || connections.is_empty()
            || connections.len() > 16
            || !entry["selected"]
                .as_str()
                .is_some_and(|s| connections.contains_key(s))
        {
            return Err(invalid(
                "integration_profile: Invalid connection selection or more than 16 connections.",
            ));
        }
        for (key, p) in connections {
            if !id(key)
                || !p.is_object()
                || !p["name"].as_str().is_some_and(|s| {
                    !s.trim().is_empty() && text(s, 256) && !s.chars().any(char::is_control)
                })
                || ["enabled", "uncertain", "remember", "writes", "repository"]
                    .iter()
                    .any(|k| p.get(*k).is_some_and(|v| !v.is_boolean()))
                || [
                    "origin",
                    "auth",
                    "username",
                    "token",
                    "revision",
                    "credential",
                ]
                .iter()
                .any(|k| p.get(*k).is_some_and(|v| !v.is_string()))
                || ["approved", "settings"]
                    .iter()
                    .any(|k| p.get(*k).is_some_and(|v| !v.is_object()))
            {
                return Err(invalid(
                    "integration_profile: Invalid saved connection fields; existing data was preserved.",
                ));
            }
            if !names.insert(p["name"].as_str().unwrap().trim().to_lowercase()) {
                return Err(invalid(
                    "integration_profile: Give each connection a distinct name.",
                ));
            }
            if let Some(digest) = p.get("credential")
                && !digest.as_str().is_some_and(|s| {
                    s.is_empty() || s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit())
                })
            {
                return Err(invalid("integration_profile: Invalid credential binding."));
            }
            if p["remember"] == true
                && !p["token"].as_str().unwrap_or("").is_empty()
                && p["credential"]
                    != rocket_git::integration_credential_digest(p["token"].as_str().unwrap())?
            {
                return Err(invalid(
                    "integration_profile: Saved credential binding changed; existing data was preserved.",
                ));
            }
            if let Some(draft) = p.get("draft") {
                validate_draft(draft)?;
            }
        }
    }
    Ok(())
}
/// No session-only token is persisted. The connection identity stays stable;
/// exact settings and the credential digest detect replacement or external edits.
pub fn binding(profile: &Value) -> Value {
    let mut v = json!({});
    for key in [
        "origin",
        "auth",
        "username",
        "settings",
        "revision",
        "credential",
    ] {
        v[key] = profile[key].clone();
    }
    // Preserve old API-1 draft bindings when the new permission is absent/off.
    if profile["repository"] == true { v["repository"] = json!(true); }
    v
}
/// Sharing more can't expose what a draft already holds, so turning the share on carries the
/// draft along. Any other change leaves it bound to what it was saved with.
pub fn carry_draft(draft: &mut Value, profile: &Value) {
    let mut bound = binding(profile);
    if profile["repository"] == true
        && let Some(keys) = bound.as_object_mut()
    {
        keys.remove("repository");
        if draft["binding"] == bound {
            draft["binding"] = binding(profile);
        }
    }
}
pub fn capture(
    package: &Package,
    profile: &Value,
    view: &Node,
    values: &HashMap<String, Value>,
    state: &Value,
) -> Result<Value> {
    let draft = json!({"source":package.snapshot(),"binding":binding(profile),
        "view":view.snapshot(),"values":values,"state":state});
    validate_draft(&draft)?;
    Ok(draft)
}
pub struct Restored {
    pub view: Node,
    pub values: HashMap<String, Value>,
    pub state: Value,
}
pub fn restore(package: &Package, profile: &Value) -> Result<Option<Restored>> {
    let Some(draft) = profile.get("draft") else {
        return Ok(None);
    };
    validate_draft(draft)?;
    if !package.approved(&draft["source"]) || draft["binding"] != binding(profile) {
        return Err(invalid(
            "integration_draft_changed: Saved draft belongs to different source or connection settings. It is preserved. Restore those settings/files, or explicitly clear this draft before starting again.",
        ));
    }
    Ok(Some(Restored {
        view: Node::parse(&draft["view"])?,
        values: object(&draft["values"])?
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect(),
        state: draft["state"].clone(),
    }))
}
fn validate_draft(draft: &Value) -> Result<()> {
    if !draft.is_object()
        || !draft["source"].is_object()
        || !draft["binding"].is_object()
        || !draft["state"].is_object()
        || serde_json::to_vec(&draft["state"]).map_or(true, |s| s.len() > 32 * 1024)
        || serde_json::to_vec(draft).map_or(true, |s| s.len() > 768 * 1024)
    {
        return Err(invalid(
            "integration_draft_limit: A saved draft exceeds its bounded data limits.",
        ));
    }
    let node = Node::parse(&draft["view"])?;
    let values = object(&draft["values"])?;
    if values.len() > 256 || serde_json::to_vec(values).map_or(true, |s| s.len() > 128 * 1024) {
        return Err(invalid(
            "integration_draft_limit: Draft fields exceed 128 KiB.",
        ));
    }
    if values.iter().any(|(key, v)| !node.accepts_value(key, v)) {
        return Err(invalid(
            "integration_draft: Saved fields do not match the saved panel.",
        ));
    }
    Ok(())
}
