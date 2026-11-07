//! The actual Cargo-built entry point, not a stand-in. Runs with the runtime tests:
//! cargo test --offline --locked -p rocket-integrations --features runtime --test linux_worker
#![cfg(all(target_os = "linux", target_arch = "x86_64", feature = "runtime"))]

use rocket_git::{CancellationToken, GitClient, IntegrationWorker, sha256_bytes};
use serde_json::json;
use std::{
    fs,
    path::PathBuf,
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};

// Always reap the owned child, including when a security assertion fails.
struct HeldWorker(Child);
impl Drop for HeldWorker {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
fn actual_entry_point_locks_down_before_reading_input() {
    let mut child = HeldWorker(
        Command::new(env!("CARGO_BIN_EXE_rocket-lua-host"))
            .env_clear()
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap(),
    );
    let parent = fs::read_to_string("/proc/self/status").unwrap();
    let field = |status: &str, name: &str| -> u64 {
        status
            .lines()
            .find_map(|line| line.strip_prefix(name))
            .unwrap()
            .trim()
            .parse()
            .unwrap()
    };
    let inherited_filters = field(&parent, "Seccomp_filters:");
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        assert!(
            child.0.try_wait().unwrap().is_none(),
            "worker exited before input"
        );
        let status = fs::read_to_string(format!("/proc/{}/status", child.0.id())).unwrap();
        let limits = fs::read_to_string(format!("/proc/{}/limits", child.0.id())).unwrap();
        if field(&status, "Seccomp_filters:") > inherited_filters {
            assert_eq!(field(&status, "NoNewPrivs:"), 1);
            assert_eq!(field(&status, "Seccomp:"), 2);
            for (name, bound) in [
                ("Max address space", 128 * 1024 * 1024),
                ("Max stack size", 2 * 1024 * 1024),
                ("Max locked memory", 0),
                ("Max pending signals", 32),
            ] {
                let row = limits
                    .lines()
                    .find_map(|line| line.strip_prefix(name))
                    .unwrap();
                assert!(
                    row.split_whitespace()
                        .take(2)
                        .all(|value| value.parse::<u64>().is_ok_and(|n| n <= bound)),
                    "{name}: {row}"
                );
            }
            break;
        }
        assert!(
            Instant::now() < deadline,
            "real worker read input without installing its filter"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    // Still no input: this proves the real entry point applied the sandbox first.
}

#[test]
fn actual_worker_starts_locked_down_and_preserves_runtime_limits() {
    let path = PathBuf::from(env!("CARGO_BIN_EXE_rocket-lua-host"));
    let hash = sha256_bytes(&fs::read(&path).unwrap()).unwrap();
    // Without a caller, the Linux entry point still locks down before parsing stdin.
    // Failed lock-down produces "sandbox", whereas empty input produces "input".
    let direct = Command::new(&path).stdin(Stdio::null()).output().unwrap();
    assert!(direct.status.success(), "{:?}", direct);
    assert_eq!(direct.stdout, b"{\"code\":\"input\",\"kind\":\"error\"}");
    let worker = IntegrationWorker::new(path.clone(), &hash);
    let client = GitClient::discover().unwrap();
    for (input, expected) in [
        (
            json!({"api":1,"source":"return function() assert(os==nil and io==nil and require==nil and package==nil); return ui.text('資料 🚀') end","context":{}}),
            json!({"kind":"view"}),
        ),
        (
            json!({"api":1,"source":"error('must not run during validation')","validate":true}),
            json!({"kind":"validated"}),
        ),
        (
            json!({"api":1,"source":"return function() while true do end end","context":{}}),
            json!({"kind":"error","code":"instructions"}),
        ),
        (
            json!({"api":1,"source":"return function() return string.rep('x',67108864) end","context":{}}),
            json!({"kind":"error"}),
        ),
        (
            json!({"api":1,"source":"\u{1b}Lua","context":{}}),
            json!({"kind":"error"}),
        ),
    ] {
        let output = client
            .integration_worker(
                &worker,
                serde_json::to_vec(&input).unwrap(),
                &CancellationToken::new(),
            )
            .unwrap();
        let result: serde_json::Value = serde_json::from_slice(&output).unwrap();
        for (key, value) in expected.as_object().unwrap() {
            assert_eq!(&result[key], value, "{result}");
        }
    }
    // Going through the actual playground consumer also checks typed Unicode UI data.
    let outcome = rocket_integrations::playground::run(
        &client,
        &worker,
        "return function() return ui.text('資料 🚀') end",
        json!({}),
        &CancellationToken::new(),
    )
    .unwrap();
    assert!(matches!(outcome, rocket_integrations::Outcome::View { .. }));
    let cancelled = CancellationToken::new();
    cancelled.cancel();
    assert!(matches!(
        client.integration_worker(&worker, b"{}".to_vec(), &cancelled),
        Err(rocket_git::Error::Cancelled)
    ));
    let wrong_hash = IntegrationWorker::new(path, &"0".repeat(64));
    assert!(
        client
            .integration_worker(&wrong_hash, b"{}".to_vec(), &CancellationToken::new())
            .unwrap_err()
            .to_string()
            .contains("hash mismatch")
    );
}
