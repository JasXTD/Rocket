#![cfg(target_os = "linux")]
//! Its own test binary: it changes this process's signal handling, which tests share.
use rocket_terminal::Session;
use std::{
    fs,
    sync::Arc,
    thread,
    time::{Duration, Instant},
};

unsafe extern "C" {
    fn signal(number: i32, handler: usize) -> usize;
}

#[test]
fn ctrl_c_interrupts_even_when_rocket_ignores_it() {
    // As when Rocket starts in the background of a script: ignored signals survive exec.
    // SAFETY: SIG_IGN (1) for SIGINT (2); this binary has no other test to disturb.
    unsafe { signal(2, 1) };
    let root = std::env::temp_dir().join(format!("rocket-unix-signals-{}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    let session = Session::start_with_history(&root, 80, 12, Arc::new(|| {}), &root.join("history")).unwrap();
    let text = |session: &Session| session.snapshot(0).map(|s| s.text()).unwrap_or_default();
    let until = |what: &str, done: &dyn Fn(&str) -> bool| {
        let start = Instant::now();
        while !done(&text(&session)) {
            assert!(start.elapsed() < Duration::from_secs(15), "{what}: {}", text(&session));
            thread::sleep(Duration::from_millis(20));
        }
    };
    let prompt = |t: &str| t.trim_end().lines().last() == Some(">");
    until("first prompt", &prompt);
    session.input(b"trap -p INT; echo ROCKET_TRAPS_LISTED; sleep 30\r").unwrap();
    until("sleep", &|t| t.contains("\nROCKET_TRAPS_LISTED"));
    thread::sleep(Duration::from_millis(300));
    session.input(&[3]).unwrap();
    until("prompt after Ctrl+C", &prompt);
    session.input(b"echo ROCKET_STATUS_$?\r").unwrap();
    until("status", &|t| t.contains("\nROCKET_STATUS_130") && prompt(t));
    // `trap -p` lists an ignored signal as trap -- '' SIGINT; the shell had none.
    assert!(!text(&session).contains("SIGINT"), "{}", text(&session));
    session.stop();
    let start = Instant::now();
    while !session.status().closed {
        assert!(start.elapsed() < Duration::from_secs(10));
        thread::sleep(Duration::from_millis(20));
    }
    fs::remove_dir_all(root).unwrap();
}
