//! The actual signed Pulley helper through Rocket's production consumer/protocol.
#![cfg(all(target_os = "macos", target_arch = "aarch64"))]
use rocket_git::{CancellationToken, GitClient, IntegrationWorker};
use serde_json::json;
use std::path::PathBuf;

#[test]
fn bundled_worker_supports_playground_errors_and_unicode() {
    let host = PathBuf::from(
        std::env::var_os("ROCKET_TEST_MAC_HOST").expect("Run the focused bundle integration build"),
    );
    let digest = std::env::var("ROCKET_LUA_WORKER_SHA256").unwrap();
    let worker = IntegrationWorker::for_host_executable(&host, &digest).unwrap();
    let client = GitClient::discover().unwrap();
    let cancel = CancellationToken::new();
    let outcome=rocket_integrations::playground::run(&client,&worker,
        "return function() assert(io==nil and os==nil and package==nil and debug==nil); return ui.text('資料 🚀') end",json!({}),&cancel).unwrap();
    assert!(matches!(outcome, rocket_integrations::Outcome::View { .. }));
    for source in [
        "return function() error('private payload') end",
        "return function() while true do end end",
        "invalid Lua !",
    ] {
        let error =
            rocket_integrations::playground::run(&client, &worker, source, json!({}), &cancel)
                .err()
                .unwrap()
                .to_string();
        assert!(!error.contains("private payload"));
        assert!(
            error.contains("lua_") || error.contains("integration_"),
            "{error}"
        );
    }
    cancel.cancel();
    assert!(matches!(
        rocket_integrations::playground::run(
            &client,
            &worker,
            "return function() return ui.text('unused') end",
            json!({}),
            &cancel
        ),
        Err(rocket_git::Error::Cancelled)
    ));
}
