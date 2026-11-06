#![cfg(target_os = "linux")]
//! Linux pseudoterminal sessions with the real bash and OpenSSH, in private folders.
use rocket_terminal::{Screen, Session, Shell};
use std::{
    fs,
    io::{BufRead, BufReader},
    path::PathBuf,
    process::{Command, Stdio},
    sync::Arc,
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

fn folder(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "rocket-unix-terminal-{name}-{}-{}",
        std::process::id(),
        SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()
    ));
    fs::create_dir(&root).unwrap();
    root
}
fn wait(session: &Session, what: &str, mut predicate: impl FnMut(&Screen) -> bool) -> Screen {
    let start = Instant::now();
    loop {
        let status = session.status();
        assert!(status.error.is_none(), "{what}: {status:?}");
        if let Some(screen) = session.snapshot(0)
            && predicate(&screen)
        {
            return screen;
        }
        assert!(
            start.elapsed() < Duration::from_secs(15),
            "{what} timed out: {status:?}\n{}",
            session.snapshot(0).map(|s| s.text()).unwrap_or_default()
        );
        thread::sleep(Duration::from_millis(20));
    }
}
/// Rocket's own bash is idle: its prompt (the folder, then "> ") is the last line.
fn prompt(screen: &Screen) -> bool {
    screen.text().trim_end().lines().last() == Some(">")
}
fn closed(session: &Session, what: &str) {
    let start = Instant::now();
    while !session.status().closed {
        assert!(start.elapsed() < Duration::from_secs(10), "{what}: {:?}", session.status());
        thread::sleep(Duration::from_millis(20));
    }
}
/// State, process group and session of a live process, or None once it is gone.
fn stat(pid: u32) -> Option<(char, i32, i32)> {
    let text = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let mut fields = text[text.rfind(')')? + 1..].split_whitespace();
    let state = fields.next()?.chars().next()?;
    fields.next()?;
    Some((state, fields.next()?.parse().ok()?, fields.next()?.parse().ok()?))
}
fn gone(pid: u32) -> bool {
    stat(pid).is_none_or(|(state, ..)| state == 'Z')
}
/// The PID bash reports for its latest background job: "[1] 12345".
fn job(screen: &Screen) -> u32 {
    screen
        .text()
        .lines()
        .rev()
        .find_map(|line| line.strip_prefix("[1] ")?.trim().parse().ok())
        .expect("a background job")
}

#[test]
fn rocket_bash_has_its_own_session_job_control_resize_interrupt_unicode_and_history() {
    let root = folder("bash");
    let history = root.join("history");
    let session = Session::start_with_history(&root, 100, 18, Arc::new(|| {}), &history).unwrap();
    wait(&session, "first prompt", prompt);
    let shell = session.status().pid.unwrap();
    // The shell leads a new session and process group, on its own terminal.
    let (_, group, member) = stat(shell).unwrap();
    assert_eq!((group, member), (shell as i32, shell as i32));
    assert_ne!(member, std::process::id() as i32);

    session.input(b"echo ROCKET_$((6*7))\r").unwrap();
    wait(&session, "a command", |s| s.text().lines().any(|l| l == "ROCKET_42") && prompt(s));

    // Job control: a background job is its own process group in the shell's session.
    session.input(b"sleep 300 &\r").unwrap();
    let background = job(&wait(&session, "background job", |s| s.text().contains("[1] ") && prompt(s)));
    let (_, group, member) = stat(background).unwrap();
    assert_eq!((group, member), (background as i32, shell as i32));
    session.input(b"jobs\r").unwrap();
    wait(&session, "jobs", |s| s.text().contains("Running") && prompt(s));

    session.resize(76, 12);
    wait(&session, "resize", |s| (s.columns, s.rows) == (76, 12));
    session.input(b"stty size\r").unwrap();
    wait(&session, "terminal size", |s| s.text().lines().any(|l| l == "12 76") && prompt(s));

    // Ctrl+C reaches the foreground job, and the shell stays.
    session.input(b"sleep 30; echo ROCKET_NOT_INTERRUPTED\r").unwrap();
    thread::sleep(Duration::from_millis(400));
    let asked = Instant::now();
    session.input(&[3]).unwrap();
    wait(&session, "prompt after Ctrl+C", prompt);
    assert!(asked.elapsed() < Duration::from_secs(3));
    session.input(b"echo ROCKET_STATUS_$?\r").unwrap();
    let screen = wait(&session, "interrupted status", |s| s.text().contains("ROCKET_STATUS_130") && prompt(s));
    assert!(!screen.text().lines().any(|l| l == "ROCKET_NOT_INTERRUPTED"), "the list ran on");
    assert_eq!(session.status().pid, Some(shell));

    session.input("printf '%s\\n' 'é 中文 🚀'\r".as_bytes()).unwrap();
    wait(&session, "Unicode", |s| s.text().lines().any(|l| l == "é 中文 🚀") && prompt(s));

    // Saved after each command, in the owned file only.
    let saved = fs::read_to_string(&history).unwrap();
    assert!(saved.lines().any(|l| l == "echo ROCKET_$((6*7))"), "{saved}");

    // Ending the session ends the shell and its background job.
    session.stop();
    closed(&session, "stop");
    assert!(gone(shell) && gone(background));
    assert_eq!(session.status().error, None);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn a_shell_that_exits_reports_its_code_and_takes_its_background_jobs_with_it() {
    let root = folder("exit");
    let session = Session::start_with_history(&root, 80, 12, Arc::new(|| {}), &root.join("history")).unwrap();
    wait(&session, "first prompt", prompt);
    session.input(b"sleep 300 &\r").unwrap();
    let background = job(&wait(&session, "background job", |s| s.text().contains("[1] ") && prompt(s)));
    session.input(b"exit 7\r").unwrap();
    closed(&session, "exit");
    let status = session.status();
    assert_eq!((status.exit_code, status.error), (Some(7), None));
    assert!(gone(background), "a background job outlived its shell");
    assert!(session.input(b"x").is_err());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn custom_programs_run_by_their_full_path_and_missing_ones_say_so() {
    let root = folder("custom");
    let custom = Shell::Custom {
        program: "/bin/sh".into(),
        arguments: vec!["-c".into(), "printf 'ROCKET_CUSTOM_%s\\n' \"$TERM\"; exit 3".into()],
    };
    let session = Session::start_shell(&root, 80, 12, Arc::new(|| {}), &custom).unwrap();
    closed(&session, "custom program");
    assert_eq!(session.status().exit_code, Some(3));
    let text = session.snapshot(0).unwrap().text();
    assert!(text.contains("ROCKET_CUSTOM_xterm-256color"), "{text}");

    for (program, message) in [(root.join("absent"), "was not found"), (PathBuf::from("sh"), "full path")] {
        let shell = Shell::Custom { program, arguments: vec![] };
        let session = Session::start_shell(&root, 80, 12, Arc::new(|| {}), &shell).unwrap();
        closed(&session, message);
        let status = session.status();
        assert!(status.error.as_deref().is_some_and(|e| e.contains(message)), "{status:?}");
        assert_eq!(status.pid, None);
    }
    // Windows' own shells are refused, not looked for.
    let session = Session::start_shell(&root, 80, 12, Arc::new(|| {}), &Shell::CommandPrompt).unwrap();
    closed(&session, "Command Prompt");
    assert!(session.status().error.is_some_and(|e| e.contains("only on Windows")));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn openssh_runs_from_usr_bin_and_takes_the_passphrase_on_the_terminal() {
    let root = folder("openssh");
    let key = root.join("id_ed25519");
    let arguments = ["-t", "ed25519", "-a", "16", "-C", "Rocket", "-f"]
        .into_iter()
        .map(str::to_owned)
        .chain([key.to_string_lossy().into_owned()])
        .collect();
    let session = Session::start_openssh(&root, "ssh-keygen.exe", arguments, Arc::new(|| {})).unwrap();
    let ends = |text: &'static str| move |s: &Screen| s.text().trim_end().ends_with(text);
    wait(&session, "new passphrase", ends("(empty for no passphrase):"));
    session.input(b"Rocket test phrase\r").unwrap();
    wait(&session, "confirmation", ends("Enter same passphrase again:"));
    session.input(b"Rocket test phrase\r").unwrap();
    closed(&session, "ssh-keygen");
    let status = session.status();
    assert_eq!((status.exit_code, status.error), (Some(0), None), "{}", session.snapshot(0).unwrap().text());
    assert!(!session.snapshot(0).unwrap().text().contains("Rocket test phrase"), "the passphrase was echoed");
    use std::os::unix::fs::PermissionsExt;
    assert_eq!(fs::metadata(&key).unwrap().permissions().mode() & 0o777, 0o600);
    fs::remove_dir_all(root).unwrap();
}

/// Run by the next test in its own process: start a session, report the shell's PID
/// once it prompts, then end the whole process without stopping the session.
#[test]
#[ignore = "subprocess fixture run by the next test"]
fn hang_up_fixture() {
    let root = PathBuf::from(std::env::var_os("ROCKET_TERMINAL_FIXTURE").unwrap());
    let session = Session::start_with_history(&root, 80, 12, Arc::new(|| {}), &root.join("history")).unwrap();
    wait(&session, "fixture prompt", prompt);
    println!("ROCKET_SHELL_PID {}", session.status().pid.unwrap());
    std::process::exit(0);
}

#[test]
fn a_shell_whose_rocket_ends_is_hung_up() {
    let root = folder("hang-up");
    let mut fixture = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "hang_up_fixture", "--ignored", "--nocapture"])
        .env("ROCKET_TERMINAL_FIXTURE", &root)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let shell: u32 = BufReader::new(fixture.stdout.take().unwrap())
        .lines()
        .map_while(Result::ok)
        .find_map(|line| line.strip_prefix("ROCKET_SHELL_PID ")?.parse().ok())
        .expect("the fixture's shell PID");
    assert!(fixture.wait().unwrap().success());
    let start = Instant::now();
    while !gone(shell) {
        assert!(start.elapsed() < Duration::from_secs(10), "the shell outlived its Rocket");
        thread::sleep(Duration::from_millis(20));
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn reset_and_device_attribute_queries_do_not_stop_the_terminal() {
    let root = folder("reset");
    let session = Session::start_with_history(&root, 80, 12, Arc::new(|| {}), &root.join("history")).unwrap();
    wait(&session, "first prompt", prompt);
    // ESC c, as `reset` sends, with more text in the same write.
    session.input(b"printf '\\033cROCKET_AFTER_RESET\\n'\r").unwrap();
    wait(&session, "reset", |s| s.text().lines().any(|l| l == "ROCKET_AFTER_RESET") && prompt(s));
    // A program asks which terminal this is and reads the answer from its input.
    session.input(b"printf '\\033[c'; IFS= read -rsn7 -t 5 answer; printf 'ROCKET_ANSWER_%q\\n' \"$answer\"\r").unwrap();
    wait(&session, "device attributes", |s| s.text().contains("ROCKET_ANSWER_$'\\E[?1;2c'") && prompt(s));
    session.stop();
    closed(&session, "stop");
    assert_eq!(session.status().error, None);
    fs::remove_dir_all(root).unwrap();
}
