#![cfg(any(target_os = "linux", target_os = "macos"))]
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
        "rocket-unix-terminal-中文-{name}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
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
        assert!(
            start.elapsed() < Duration::from_secs(10),
            "{what}: {:?}",
            session.status()
        );
        thread::sleep(Duration::from_millis(20));
    }
}
/// State, process group and session of a live process, or None once it is gone.
#[cfg(target_os = "linux")]
fn stat(pid: u32) -> Option<(char, i32, i32)> {
    let text = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let mut fields = text[text.rfind(')')? + 1..].split_whitespace();
    let state = fields.next()?.chars().next()?;
    fields.next()?;
    Some((
        state,
        fields.next()?.parse().ok()?,
        fields.next()?.parse().ok()?,
    ))
}
#[cfg(target_os = "macos")]
fn stat(pid: u32) -> Option<(char, i32, i32)> {
    unsafe extern "C" {
        fn getsid(pid: i32) -> i32;
        fn getpgid(pid: i32) -> i32;
    }
    let result = Command::new("/bin/ps")
        .args(["-p", &pid.to_string(), "-o", "stat="])
        .output()
        .unwrap();
    let state = String::from_utf8(result.stdout)
        .unwrap()
        .trim()
        .chars()
        .next()?;
    Some((state, unsafe { getpgid(pid as i32) }, unsafe {
        getsid(pid as i32)
    }))
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
    wait(&session, "a command", |s| {
        s.text().lines().any(|l| l == "ROCKET_42") && prompt(s)
    });

    // Job control: a background job is its own process group in the shell's session.
    session.input(b"sleep 300 &\r").unwrap();
    let background = job(&wait(&session, "background job", |s| {
        s.text().contains("[1] ") && prompt(s)
    }));
    let (_, group, member) = stat(background).unwrap();
    assert_eq!((group, member), (background as i32, shell as i32));
    session.input(b"jobs\r").unwrap();
    wait(&session, "jobs", |s| {
        s.text().contains("Running") && prompt(s)
    });

    session.resize(76, 12);
    wait(&session, "resize", |s| (s.columns, s.rows) == (76, 12));
    session.input(b"stty size\r").unwrap();
    wait(&session, "terminal size", |s| {
        s.text().lines().any(|l| l == "12 76") && prompt(s)
    });

    // Ctrl+C reaches the foreground job, and the shell stays.
    session
        .input(b"sleep 30; echo ROCKET_NOT_INTERRUPTED\r")
        .unwrap();
    thread::sleep(Duration::from_millis(400));
    let asked = Instant::now();
    session.input(&[3]).unwrap();
    wait(&session, "prompt after Ctrl+C", prompt);
    assert!(asked.elapsed() < Duration::from_secs(3));
    session.input(b"echo ROCKET_STATUS_$?\r").unwrap();
    let screen = wait(&session, "interrupted status", |s| {
        s.text().contains("ROCKET_STATUS_130") && prompt(s)
    });
    assert!(
        !screen.text().lines().any(|l| l == "ROCKET_NOT_INTERRUPTED"),
        "the list ran on"
    );
    assert_eq!(session.status().pid, Some(shell));

    session
        .input("printf '%s\\n' 'é 中文 🚀'\r".as_bytes())
        .unwrap();
    wait(&session, "Unicode", |s| {
        s.text().lines().any(|l| l == "é 中文 🚀") && prompt(s)
    });

    // Saved after each command, in the owned file only.
    let saved = fs::read_to_string(&history).unwrap();
    assert!(
        saved.lines().any(|l| l == "echo ROCKET_$((6*7))"),
        "{saved}"
    );

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
    let session =
        Session::start_with_history(&root, 80, 12, Arc::new(|| {}), &root.join("history")).unwrap();
    wait(&session, "first prompt", prompt);
    session.input(b"sleep 300 &\r").unwrap();
    let background = job(&wait(&session, "background job", |s| {
        s.text().contains("[1] ") && prompt(s)
    }));
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
        arguments: vec![
            "-c".into(),
            "printf 'ROCKET_CUSTOM_%s\\n' \"$TERM\"; exit 3".into(),
        ],
    };
    let session = Session::start_shell(&root, 80, 12, Arc::new(|| {}), &custom).unwrap();
    closed(&session, "custom program");
    assert_eq!(session.status().exit_code, Some(3));
    let text = session.snapshot(0).unwrap().text();
    assert!(text.contains("ROCKET_CUSTOM_xterm-256color"), "{text}");

    for (program, message) in [
        (root.join("absent"), "was not found"),
        (PathBuf::from("sh"), "full path"),
    ] {
        let shell = Shell::Custom {
            program,
            arguments: vec![],
        };
        let session = Session::start_shell(&root, 80, 12, Arc::new(|| {}), &shell).unwrap();
        closed(&session, message);
        let status = session.status();
        assert!(
            status.error.as_deref().is_some_and(|e| e.contains(message)),
            "{status:?}"
        );
        assert_eq!(status.pid, None);
    }
    // Windows' own shells are refused, not looked for.
    let session =
        Session::start_shell(&root, 80, 12, Arc::new(|| {}), &Shell::CommandPrompt).unwrap();
    closed(&session, "Command Prompt");
    assert!(
        session
            .status()
            .error
            .is_some_and(|e| e.contains("only on Windows"))
    );
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
    let session =
        Session::start_openssh(&root, "ssh-keygen.exe", arguments, Arc::new(|| {})).unwrap();
    // Long Unicode paths can wrap the SSH prompt. Inspect the same exact suffix
    // after joining visual rows, without depending on the host's temp-path length.
    let ends = |text: &'static str| move |s: &Screen| s.text().replace('\n', "").trim_end().ends_with(text);
    wait(
        &session,
        "new passphrase",
        ends("(empty for no passphrase):"),
    );
    session.input(b"Rocket test phrase\r").unwrap();
    wait(
        &session,
        "confirmation",
        ends("Enter same passphrase again:"),
    );
    session.input(b"Rocket test phrase\r").unwrap();
    closed(&session, "ssh-keygen");
    let status = session.status();
    assert_eq!(
        (status.exit_code, status.error),
        (Some(0), None),
        "{}",
        session.snapshot(0).unwrap().text()
    );
    assert!(
        !session
            .snapshot(0)
            .unwrap()
            .text()
            .contains("Rocket test phrase"),
        "the passphrase was echoed"
    );
    use std::os::unix::fs::PermissionsExt;
    assert_eq!(
        fs::metadata(&key).unwrap().permissions().mode() & 0o777,
        0o600
    );
    fs::remove_dir_all(root).unwrap();
}

/// Run by the next test in its own process: start a session, report the shell's PID
/// once it prompts, then end the whole process without stopping the session.
#[test]
#[ignore = "subprocess fixture run by the next test"]
fn hang_up_fixture() {
    let root = PathBuf::from(std::env::var_os("ROCKET_TERMINAL_FIXTURE").unwrap());
    let session =
        Session::start_with_history(&root, 80, 12, Arc::new(|| {}), &root.join("history")).unwrap();
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
        assert!(
            start.elapsed() < Duration::from_secs(10),
            "the shell outlived its Rocket"
        );
        thread::sleep(Duration::from_millis(20));
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn reset_and_device_attribute_queries_do_not_stop_the_terminal() {
    let root = folder("reset");
    let session =
        Session::start_with_history(&root, 80, 12, Arc::new(|| {}), &root.join("history")).unwrap();
    wait(&session, "first prompt", prompt);
    // ESC c, as `reset` sends, with more text in the same write.
    session
        .input(b"printf '\\033cROCKET_AFTER_RESET\\n'\r")
        .unwrap();
    wait(&session, "reset", |s| {
        s.text().lines().any(|l| l == "ROCKET_AFTER_RESET") && prompt(s)
    });
    // A program asks which terminal this is and reads the answer from its input.
    session.input(b"printf '\\033[c'; IFS= read -rsn7 -t 5 answer; printf 'ROCKET_ANSWER_'; printf %s \"$answer\" | od -An -tx1 | tr -d ' \\n'; echo\r").unwrap();
    wait(&session, "device attributes", |s| {
        s.text().contains("ROCKET_ANSWER_1b5b3f313b3263") && prompt(s)
    });
    session.stop();
    closed(&session, "stop");
    assert_eq!(session.status().error, None);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn dropping_a_session_stops_jobs_that_ignore_hangup_without_touching_other_sessions() {
    let root = folder("drop");
    let session =
        Session::start_with_history(&root, 100, 18, Arc::new(|| {}), &root.join("history"))
            .unwrap();
    wait(&session, "first prompt", prompt);
    let shell = session.status().pid.unwrap();
    // A different session/process is a canary: teardown must never signal it.
    let mut canary = Command::new("/bin/sleep").arg("30").spawn().unwrap();
    session
        .input(b"/bin/sh -c 'trap \"\" HUP; exec sleep 60' &\r")
        .unwrap();
    let background = job(&wait(&session, "hangup-resistant job", |s| {
        s.text().contains("[1] ") && prompt(s)
    }));
    let start = Instant::now();
    drop(session);
    while !gone(shell) || !gone(background) {
        if start.elapsed() > Duration::from_secs(8) {
            let _ = canary.kill();
            let _ = canary.wait();
            panic!("Dropped terminal left shell/job alive");
        }
        thread::sleep(Duration::from_millis(20));
    }
    let unchanged = canary.try_wait().unwrap().is_none();
    canary.kill().unwrap();
    canary.wait().unwrap();
    assert!(unchanged, "An unrelated process was stopped");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn typing_history_completion_and_interruption_survive_repeated_resize() {
    let root = folder("editing");
    fs::write(root.join("rocket-completion-中文.txt"), "fixture").unwrap();
    let session =
        Session::start_with_history(&root, 100, 18, Arc::new(|| {}), &root.join("history"))
            .unwrap();
    wait(&session, "first prompt", prompt);
    session.input(b"printf 'ROCKET_%s\\n' HISTORY\r").unwrap();
    wait(&session, "history seed", |s| {
        s.text().lines().any(|l| l == "ROCKET_HISTORY") && prompt(s)
    });
    session.input(b"\x1b[A").unwrap();
    wait(&session, "history recall", |s| {
        s.text().trim_end().ends_with("HISTORY")
    });
    session.input(&[3]).unwrap();
    wait(&session, "cancel recalled line", prompt);
    session.input(b"ech\t").unwrap();
    wait(&session, "command completion", |s| {
        s.text().trim_end().ends_with("echo")
    });
    session.input(&[3]).unwrap();
    wait(&session, "cancel completion", prompt);
    session
        .input("printf 'ROCKET_%s\\n' '中文_🚀_é_OX'".as_bytes())
        .unwrap();
    for (columns, rows) in [(42, 8), (120, 24), (55, 12)] {
        session.resize(columns, rows);
        wait(&session, "resized screen", |s| {
            (s.columns, s.rows) == (usize::from(columns), usize::from(rows))
        });
    }
    session.input(b"\x1b[D\x7fK\r").unwrap();
    wait(&session, "edited Unicode command", |s| {
        s.text().contains("ROCKET_中文_🚀_é_OK") && prompt(s)
    });
    session.stop();
    closed(&session, "stop");
    assert_eq!(session.status().error, None);
    fs::remove_dir_all(root).unwrap();
}

#[test]
#[ignore = "bounded detached writer invoked only by its owned test launcher"]
fn detached_writer_fixture() {
    use std::io::Write;
    fs::write("writer-ready", std::process::id().to_string()).unwrap();
    let deadline = Instant::now() + Duration::from_secs(8);
    let mut output = std::io::stdout().lock();
    while Instant::now() < deadline {
        if output.write_all(&[b'x'; 8192]).and_then(|_| output.flush()).is_err() { break; }
    }
}

#[test]
#[ignore = "subprocess fixture invoked by bounded_output_drain"]
#[allow(clippy::zombie_processes, reason = "Fixture deliberately exits before its bounded detached writer; the parent test verifies that writer exits after PTY closure")]
fn detached_writer_launcher() {
    use std::os::unix::process::CommandExt;
    unsafe extern "C" { fn setsid() -> i32; }
    let mut command = Command::new(std::env::current_exe().unwrap());
    command.args(["--exact", "detached_writer_fixture", "--ignored", "--nocapture"]);
    // SAFETY: only an async-signal-safe session syscall before this child execs.
    unsafe { command.pre_exec(|| if setsid() < 0 { Err(std::io::Error::last_os_error()) } else { Ok(()) }); }
    let mut writer = command.spawn().unwrap();
    let deadline = Instant::now() + Duration::from_secs(3);
    while !std::path::Path::new("writer-ready").exists() {
        if Instant::now() >= deadline { let _=writer.kill(); let _=writer.wait(); panic!("Writer did not start"); }
        thread::sleep(Duration::from_millis(10));
    }
    // Deliberately leave this bounded descendant in its separate session.
}

#[test]
fn bounded_output_drain_closes_even_when_a_detached_process_keeps_writing() {
    let root = folder("drain");
    let start = Instant::now();
    let session = Session::start_shell(&root, 80, 12, Arc::new(|| {}), &Shell::Custom {
        program: std::env::current_exe().unwrap(),
        arguments: vec!["--exact".into(), "detached_writer_launcher".into(), "--ignored".into(), "--nocapture".into()],
    }).unwrap();
    closed(&session, "bounded output drain");
    assert!(start.elapsed() < Duration::from_secs(4), "Detached output blocked terminal shutdown");
    let writer: u32 = fs::read_to_string(root.join("writer-ready")).unwrap().parse().unwrap();
    let deadline = Instant::now() + Duration::from_secs(3);
    while !gone(writer) {
        assert!(Instant::now() < deadline, "Writer did not see PTY closure");
        thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(session.status().error, None);
    fs::remove_dir_all(root).unwrap();
}

#[cfg(target_os = "macos")]
#[test]
fn native_zsh_runs_in_the_unicode_directory_and_is_reaped() {
    let root = folder("zsh").canonicalize().unwrap();
    let session = Session::start_shell(&root, 100, 18, Arc::new(|| {}), &Shell::Custom {
        program: "/bin/zsh".into(), arguments: vec!["-f".into(), "-i".into()],
    }).unwrap();
    session.input(b"printf 'ROCKET_ZSH_%s\\n' \"$PWD\"; exit 0\r").unwrap();
    closed(&session, "native zsh");
    assert_eq!(session.status().error, None);
    assert_eq!(session.status().exit_code, Some(0));
    assert!(session.snapshot(0).unwrap().text().replace('\n', "").contains(&format!("ROCKET_ZSH_{}", root.canonicalize().unwrap().display())));
    assert!(gone(session.status().pid.unwrap()));
    fs::remove_dir_all(root).unwrap();
}
