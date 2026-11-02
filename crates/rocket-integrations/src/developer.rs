use crate::{Package, invalid};
use rocket_git::{CancellationToken, GitClient, IntegrationWorker, Result};
use serde_json::{Value, json};
use std::path::Path;

#[derive(Clone, Debug)]
pub struct Validation {
    pub syntax_valid: bool,
    pub line: Option<u64>,
}

/// Compile text only in the bounded worker. Neither module initialization nor
/// its returned callback executes; approval and draft files are not modified.
pub fn validate_package(client: &GitClient, worker: &IntegrationWorker, directory: &Path, cancel: &CancellationToken) -> Result<Validation> {
    cancel.check()?;
    let package = Package::load(directory)?;
    let input = serde_json::to_vec(&json!({"api":1,"source":package.source,"validate":true}))
        .map_err(|_| invalid("integration_request: Cannot encode validation input."))?;
    let output = client.integration_worker(worker, input, cancel)?;
    let result: Value = serde_json::from_slice(&output)
        .map_err(|_| invalid("integration_worker_response: Invalid validation result."))?;
    package.verify()?;
    cancel.check()?;
    match result["kind"].as_str() {
        Some("validated") => Ok(Validation { syntax_valid:true, line:None }),
        Some("error") if result["code"] == "syntax" => Ok(Validation {
            syntax_valid:false, line:result["line"].as_u64().filter(|n| (1..=65536).contains(n)),
        }),
        Some("error") => Err(crate::execution::lua_error(&result)),
        _ => Err(invalid("integration_worker_response: Unknown validation result.")),
    }
}
