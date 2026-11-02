#![cfg(windows)]
//! The real Git Bash, from the bundle when it is here, else from an installed Git. HOME is
//! private and this is the only test in this binary, so nothing touches the user's files.
use rocket_terminal::{Screen, Session, Shell};
use std::{
    fs,
    path::PathBuf,
    sync::Arc,
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

fn bash() -> Option<PathBuf> {
    let bundled = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join(r"..\..\runtime\git\2.56.0.windows.4\bin\bash.exe");
    let installed = PathBuf::from(r"C:\Program Files\Git\bin\bash.exe");
    [bundled, installed].into_iter().find(|p| p.is_file())
}
fn wait(session: &Session, what: &str, mut predicate: impl FnMut(&Screen) -> bool) {
    let start = Instant::now();
    loop {
        if let Some(screen) = session.snapshot(0)
            && predicate(&screen)
        {
            return;
        }
        assert!(
            start.elapsed() < Duration::from_secs(40),
            "{what} timed out: {:?}\n{}",
            session.status(),
            session.snapshot(0).unwrap().text()
        );
        thread::sleep(Duration::from_millis(20));
    }
}
fn prompt(screen: &Screen) -> bool {
    screen.text().trim_end().ends_with('$')
}
fn line_is(screen: &Screen, text: &str) -> bool {
    screen.text().lines().any(|l| l.trim() == text)
}
fn closed(session: &Session) {
    let start = Instant::now();
    while !session.status().closed {
        assert!(start.elapsed() < Duration::from_secs(15), "{:?}", session.status());
        thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn git_bash_keeps_history_interrupts_commands_and_uses_its_own_git() {
    let Some(bash) = bash() else {
        eprintln!("skipped: no Git Bash here");
        return;
    };
    let root = std::env::temp_dir().join(format!(
        "rocket-git-bash-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let home = root.join("home");
    fs::create_dir_all(&home).unwrap();
    // SAFETY: the only test in this binary, before any session starts.
    unsafe {
        std::env::set_var("HOME", &home);
    }
    let shell = Shell::GitBash(bash.clone());
    let session = Session::start_shell(&root, 100, 18, Arc::new(|| {}), &shell).unwrap();
    wait(&session, "prompt", prompt);
    session.input(b"echo MARK_$((40+2)); command -v git\r").unwrap();
    wait(&session, "first command", |s| line_is(s, "MARK_42"));
    if bash.components().any(|c| c.as_os_str() == "2.56.0.windows.4") {
        wait(&session, "the bundled Git", |s| line_is(s, "/ucrt64/bin/git"));
    }
    session.input(b"\x1b[A").unwrap();
    thread::sleep(Duration::from_millis(200));
    session.input(b"\r").unwrap();
    wait(&session, "arrow up", |s| {
        s.text().lines().filter(|l| l.trim() == "MARK_42").count() >= 2
    });
    session.input(b"sleep 30; echo NOT_INTERRUPTED\r").unwrap();
    thread::sleep(Duration::from_secs(2));
    session.input(&[3]).unwrap();
    thread::sleep(Duration::from_millis(300));
    session.input(b"echo AFTER_$((2+3))\r").unwrap();
    wait(&session, "interrupt", |s| line_is(s, "AFTER_5"));
    session.input("printf 'é中🚀|END\\n'\r".as_bytes()).unwrap();
    wait(&session, "unicode", |s| line_is(s, "é中🚀|END"));
    // End session kills the shell. Each line must already be saved by then.
    session.input(b"echo KEPT_AFTER_END\r").unwrap();
    wait(&session, "last command", |s| line_is(s, "KEPT_AFTER_END"));
    thread::sleep(Duration::from_millis(300));
    session.stop();
    closed(&session);
    let saved = fs::read_to_string(home.join(".bash_history")).unwrap();
    assert!(saved.contains("echo KEPT_AFTER_END"), "{saved}");
    let session = Session::start_shell(&root, 100, 18, Arc::new(|| {}), &shell).unwrap();
    wait(&session, "second prompt", prompt);
    session.input(b"exit 7\r").unwrap();
    closed(&session);
    assert_eq!(session.status().exit_code, Some(7));
    let _ = fs::remove_dir_all(root);
}
