#![cfg(windows)]
use rocket_terminal::{Screen, Session, Shell};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::Arc,
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

fn folder(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "rocket-shells-{name}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir(&root).unwrap();
    root
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
            start.elapsed() < Duration::from_secs(20),
            "{what} timed out: {:?}\n{}",
            session.status(),
            session.snapshot(0).unwrap().text()
        );
        thread::sleep(Duration::from_millis(20));
    }
}
fn line_is(screen: &Screen, text: &str) -> bool {
    screen.text().lines().any(|l| l.trim() == text)
}
fn closed(session: &Session) {
    let start = Instant::now();
    while !session.status().closed {
        assert!(start.elapsed() < Duration::from_secs(10), "{:?}", session.status());
        thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn only_command_prompt_refuses_a_network_folder() {
    let bash = Shell::GitBash(r"C:\Git\bin\bash.exe".into());
    for network in [r"\\server\share\repo", r"\\?\UNC\server\share\repo"] {
        assert!(!Shell::CommandPrompt.runs_in(Path::new(network)), "{network}");
        assert!(bash.runs_in(Path::new(network)), "{network}");
        assert!(Shell::WindowsPowerShell.runs_in(Path::new(network)), "{network}");
    }
    for local in [r"C:\Code\repo", r"\\?\C:\Code\repo"] {
        assert!(Shell::CommandPrompt.runs_in(Path::new(local)), "{local}");
    }
}

#[test]
fn command_prompt_starts_in_the_folder_in_utf8_and_keeps_its_exit_code() {
    let root = folder("cmd");
    fs::write(root.join("note.txt"), "é 中 🚀\r\n").unwrap();
    let session =
        Session::start_shell(&root, 100, 18, Arc::new(|| {}), &Shell::CommandPrompt).unwrap();
    wait(&session, "prompt", |s| s.text().trim_end().ends_with('>'));
    session.input(b"chcp & echo MARK_%CD:~-3%\r").unwrap();
    wait(&session, "code page", |s| s.text().contains("65001"));
    session.input(b"type note.txt\r").unwrap();
    wait(&session, "UTF-8 file", |s| line_is(s, "é 中 🚀"));
    session.input(b"ping -t 127.0.0.1\r").unwrap();
    wait(&session, "ping", |s| s.text().contains("Reply from"));
    session.input(&[3]).unwrap();
    thread::sleep(Duration::from_millis(300));
    session.input(b"echo AFTER_INTERRUPT\r").unwrap();
    wait(&session, "interrupt", |s| line_is(s, "AFTER_INTERRUPT"));
    session.input(b"exit 7\r").unwrap();
    closed(&session);
    assert_eq!(session.status().exit_code, Some(7));
    fs::remove_file(root.join("note.txt")).unwrap();
    fs::remove_dir(root).unwrap();
}

#[test]
fn a_custom_program_gets_its_arguments_and_its_output_stays_on_screen() {
    let root = folder("custom");
    let cmd = Shell::CommandPrompt.program().unwrap();
    let custom = Shell::Custom {
        program: cmd,
        arguments: vec!["/C".into(), "echo CUSTOM_RAN & exit 3".into()],
    };
    let session = Session::start_shell(&root, 100, 18, Arc::new(|| {}), &custom).unwrap();
    closed(&session);
    assert_eq!(session.status().exit_code, Some(3));
    assert!(session.status().pid.is_some());
    assert!(session.snapshot(0).unwrap().text().contains("CUSTOM_RAN"));
    fs::remove_dir(root).unwrap();
}

#[test]
fn a_missing_program_says_which_file_and_starts_nothing() {
    let root = folder("missing");
    let program = root.join("no-such-shell.exe");
    let custom = Shell::Custom {
        program: program.clone(),
        arguments: Vec::new(),
    };
    let session = Session::start_shell(&root, 100, 18, Arc::new(|| {}), &custom).unwrap();
    closed(&session);
    let status = session.status();
    assert!(status.pid.is_none(), "{status:?}");
    let error = status.error.unwrap();
    assert!(error.contains(&program.display().to_string()) && error.ends_with("was not found."), "{error}");
    assert!(session.input(b"x").is_err());
    fs::remove_dir(root).unwrap();
}

#[test]
fn line_breaks_in_custom_arguments_are_refused() {
    let root = folder("arguments");
    let custom = Shell::Custom {
        program: Shell::CommandPrompt.program().unwrap(),
        arguments: vec!["one\ntwo".into()],
    };
    assert!(Session::start_shell(&root, 100, 18, Arc::new(|| {}), &custom).is_err());
    fs::remove_dir(root).unwrap();
}

#[test]
fn a_refused_session_is_closed_and_says_why() {
    let session = Session::refused("Not here".into());
    let status = session.status();
    assert!(status.closed && !status.running && status.pid.is_none());
    assert_eq!(status.error.as_deref(), Some("Not here"));
    assert!(session.input(b"x").is_err());
}
