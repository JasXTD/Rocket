#![forbid(unsafe_code)]
// Standard handles are explicitly supplied pipes. A console subsystem would
// require console initialization that the LPAC/Win32k restrictions forbid.
#![cfg_attr(windows, windows_subsystem = "windows")]
use std::io::{Read, Write};
fn main() {
    if rocket_git::IntegrationWorker::enter_isolation().is_err() {
        let _ = std::io::stdout().write_all(b"{\"kind\":\"error\",\"code\":\"sandbox\"}");
        std::process::exit(1);
    }
    let mut bytes = vec![];
    let input = std::io::stdin()
        .take(8 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .ok()
        .filter(|_| bytes.len() <= 8 * 1024 * 1024)
        .and_then(|_| serde_json::from_slice(&bytes).ok());
    let output = input
        .map(rocket_integrations::runtime::run)
        .unwrap_or_else(|| serde_json::json!({"kind":"error","code":"input"}));
    let bytes = serde_json::to_vec(&output)
        .ok()
        .filter(|b| b.len() <= 1024 * 1024)
        .unwrap_or_else(|| b"{\"kind\":\"error\",\"code\":\"result\"}".to_vec());
    if std::io::stdout().write_all(&bytes).is_err() {
        std::process::exit(1);
    }
}
