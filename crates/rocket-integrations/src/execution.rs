use crate::{Node, Package, Request, invalid};
use rocket_git::{CancellationToken, GitClient, IntegrationConnection, IntegrationWorker, Result};
use serde_json::{Value, json};

#[derive(Clone, Debug)]
pub enum Outcome {
    View { node: Box<Node>, state: Value },
    Review(Request),
}
pub fn execute(
    client: &GitClient,
    worker: &IntegrationWorker,
    package: &Package,
    mut context: Value,
    connection: Option<&IntegrationConnection>,
    allow_write: bool,
    cancel: &CancellationToken,
) -> Result<Outcome> {
    if !package.repository && let Some(context) = context.as_object_mut() {
        context.remove("repository");
        context.remove("target");
    }
    let mut cache = serde_json::Map::new();
    for _ in 0..9 {
        cancel.check()?;
        package.verify()?;
        let input = serde_json::to_vec(
            &json!({"api":1,"source":package.source,"context":context,"responses":cache}),
        )
        .map_err(|_| invalid("integration_request: Cannot encode worker input."))?;
        let out = client.integration_worker(worker, input, cancel)?;
        let result: Value = serde_json::from_slice(&out).map_err(|_| {
            invalid("integration_worker_response: Lua worker returned an invalid response.")
        })?;
        match result["kind"].as_str() {
            Some("view") => {
                let node = Node::parse(&result["view"])?;
                let state = result.get("state").cloned().unwrap_or_else(|| json!({}));
                if !state.is_object() || state.to_string().len() > 32 * 1024 {
                    return Err(invalid(
                        "integration_state_limit: Script state must be an object of at most 32 KiB.",
                    ));
                }
                package.verify()?;
                cancel.check()?;
                return Ok(Outcome::View {
                    node: Box::new(node),
                    state,
                });
            }
            Some("request") => {
                let request = Request::parse(&result["request"])?;
                if !package.network {
                    return Err(invalid(
                        "integration_permission: This extension has no approved network permission.",
                    ));
                }
                let connection = connection.ok_or_else(|| {
                    invalid("integration_connection: Configure and enable a connection first.")
                })?;
                if request.writes() {
                    if !package.writes || !allow_write {
                        return Err(invalid(
                            "integration_permission: Reviewed writes are off for this connection.",
                        ));
                    }
                    package.verify()?;
                    if request.check.is_some() && cache.len() >= 8 {
                        return Err(invalid("integration_request_limit: Leave one read available for the write's preflight check."));
                    }
                    check_request(client, package, connection, &request, cancel)?;
                    return Ok(Outcome::Review(request));
                }
                if cache.len() >= 8 {
                    return Err(invalid(
                        "integration_request_limit: An action may read at most eight requests. Use pagination and stable request paths.",
                    ));
                }
                // The files may have changed while the worker was running.
                package.verify()?;
                let response = client.integration_http(
                    connection,
                    &request.method,
                    &request.path,
                    None,
                    false,
                    cancel,
                )?;
                let body: Value = serde_json::from_slice(&response.body)
                    .map_err(|_| invalid("integration_json: The service returned invalid JSON."))?;
                cache.insert(request.key(), json!({"status":response.status,"json":body}));
                if serde_json::to_vec(&cache).map_or(true, |b| b.len() > 4 * 1024 * 1024) {
                    return Err(invalid(
                        "integration_response_limit: Responses exceed the action's 4 MiB limit.",
                    ));
                }
            }
            Some("error") => {
                return Err(lua_error(&result));
            }
            _ => {
                return Err(invalid(
                    "integration_worker_response: Unknown worker result.",
                ));
            }
        }
    }
    Err(invalid(
        "integration_request_limit: An action may read at most eight requests. Use pagination and stable request paths.",
    ))
}
pub fn submit(
    client: &GitClient,
    package: &Package,
    connection: &IntegrationConnection,
    request: &Request,
    cancel: &CancellationToken,
) -> Result<()> {
    package.verify()?;
    cancel.check()?;
    check_request(client, package, connection, request, cancel)?;
    let body = request.body.as_ref().map(Value::to_string);
    client.integration_http(
        connection,
        &request.method,
        &request.path,
        body.as_deref(),
        true,
        cancel,
    )?;
    Ok(())
}

fn check_request(client: &GitClient, package: &Package, connection: &IntegrationConnection, request: &Request, cancel: &CancellationToken) -> Result<()> {
    if let Some(check) = &request.check {
        let response = client.integration_http(connection, "GET", &check.path, None, false, cancel)?;
        let body: Value = serde_json::from_slice(&response.body)
            .map_err(|_| invalid("integration_json: The preflight response is not valid JSON."))?;
        if body.pointer(&check.pointer) != Some(&check.expected) {
            return Err(invalid("integration_stale: The remote item changed or its expected marker is missing. Nothing was sent. Reload the item and review your draft again."));
        }
        package.verify()?;
        cancel.check()?;
    }
    Ok(())
}

pub(crate) fn lua_error(result: &Value) -> rocket_git::Error {
    let (code, explanation) = match result["code"].as_str() {
        Some("syntax") => ("integration_lua_syntax", "Lua source could not be parsed. Check this line and its preceding block."),
        Some("memory") => ("integration_lua_memory", "The script exceeded its 32 MiB memory limit."),
        Some("instructions") => ("integration_lua_limit", "The script exceeded its instruction limit."),
        Some("result") => ("integration_lua_result", "Return a supported UI description and bounded plain data."),
        _ => ("integration_lua_error", "Script execution failed. Check the callback and API 1 contract; private error values are omitted."),
    };
    let phase = match result["phase"].as_str() { Some("compile") => "compile", Some("initialization") => "initialization", _ => "callback" };
    let location = result["line"].as_u64().filter(|n| (1..=65536).contains(n))
        .map(|n| format!("main.lua:{n}")).unwrap_or_else(|| "main.lua".into());
    invalid(&format!("{code}: {location} ({phase}). {explanation}"))
}
