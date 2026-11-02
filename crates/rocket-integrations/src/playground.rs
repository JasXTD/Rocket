//! Offline scratch execution uses the same isolated, bounded Lua worker as packages.
use crate::{Node, Outcome, Request, invalid};
use rocket_git::{CancellationToken, GitClient, IntegrationWorker, Result};
use serde_json::{Value, json};
pub mod store;
pub const SOURCE_LIMIT: usize = 64 * 1024;
pub const STARTER: &str = r#"return function(ctx)
  local count = ctx.state.count or 0
  if ctx.event == 'increment' then count = count + 1 end
  return {
    state = { count = count },
    view = ui.page { title = 'My integration', children = {
      ui.input { id = 'name', label = 'Name', value = ctx.values.name or 'Rocket' },
      ui.button { id = 'increment', label = 'Update preview' },
      ui.text('Hello ' .. (ctx.values.name or 'Rocket') .. ' · updated ' .. count .. ' times')
    } }
  }
end
"#;
pub fn run(
    client: &GitClient,
    worker: &IntegrationWorker,
    source: &str,
    context: Value,
    cancel: &CancellationToken,
) -> Result<Outcome> {
    validate_source(source)?;
    if !context.is_object() || context.to_string().len() > 64 * 1024 {
        return Err(invalid(
            "playground_context: Context must be an object up to 64 KiB.",
        ));
    }
    cancel.check()?;
    let input =
        serde_json::to_vec(&json!({"api":1,"source":source,"context":context,"responses":{}}))
            .map_err(|_| invalid("playground_input: Cannot encode scratch input."))?;
    let output = client.integration_worker(worker, input, cancel)?;
    let result: Value = serde_json::from_slice(&output)
        .map_err(|_| invalid("playground_result: Invalid worker response."))?;
    cancel.check()?;
    match result["kind"].as_str() {
        Some("view") => {
            let node = Node::parse(&result["view"])?;
            let state = result.get("state").cloned().unwrap_or_else(|| json!({}));
            if !state.is_object() || state.to_string().len() > 32 * 1024 {
                return Err(invalid(
                    "playground_state: Script state must be an object up to 32 KiB.",
                ));
            }
            Ok(Outcome::View {
                node: Box::new(node),
                state,
            })
        }
        Some("request") => Request::parse(&result["request"]).map(Outcome::Review),
        Some("error") => Err(crate::execution::lua_error(&result)),
        _ => Err(invalid("playground_result: Unknown worker response.")),
    }
}
pub fn validate_source(source: &str) -> Result<()> {
    if source.len() > SOURCE_LIMIT || source.contains('\0') || source.starts_with('\u{1b}') {
        return Err(invalid(
            "playground_source: Use UTF-8 Lua source up to 64 KiB; binary chunks are refused.",
        ));
    }
    Ok(())
}
