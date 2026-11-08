//! Native macOS refusal checks. No ignored test or external worker path: Cargo builds
//! the actual entry point. This proves refusal, not a functioning macOS sandbox.
#![cfg(all(target_os = "macos", feature = "runtime"))]

use std::{
    io::{Read, Write},
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};

struct HeldWorker(Child);
impl Drop for HeldWorker {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn check_refusal(input: Option<&[u8]>) {
    let mut child = HeldWorker(
        Command::new(env!("CARGO_BIN_EXE_rocket-lua-host"))
            .env_clear()
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("start the actual Cargo-built worker"),
    );
    // Keep stdin open: waiting for EOF would show that untrusted input is reached
    // before isolation has been established. A fast refusal may close it already.
    if let Some(input) = input
        && let Err(error) = child.0.stdin.as_mut().unwrap().write_all(input)
    {
        assert_eq!(error.kind(), std::io::ErrorKind::BrokenPipe);
    }
    let deadline = Instant::now() + Duration::from_secs(3);
    let status = loop {
        if let Some(status) = child.0.try_wait().unwrap() {
            break status;
        }
        assert!(
            Instant::now() < deadline,
            "unsupported worker waited for input or ran a script"
        );
        std::thread::sleep(Duration::from_millis(10));
    };
    assert_eq!(status.code(), Some(1));
    let mut output = Vec::new();
    child
        .0
        .stdout
        .take()
        .unwrap()
        .take(4097)
        .read_to_end(&mut output)
        .unwrap();
    assert_eq!(output, br#"{"kind":"error","code":"sandbox"}"#);
}

#[test]
fn actual_entry_point_refuses_before_input() {
    check_refusal(None);
}

#[test]
fn actual_entry_point_refuses_supplied_script() {
    check_refusal(Some(
        br#"{"api":1,"source":"return function() return {kind='must-not-run'} end","context":{}}"#,
    ));
}
