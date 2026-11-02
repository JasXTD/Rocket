#![cfg(windows)]
use rocket_terminal::{Screen, Session};
use std::{
    fs,
    path::Path,
    process::Command,
    sync::Arc,
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

fn wait(session: &Session, condition: impl Fn(&str) -> bool) -> Screen {
    let start = Instant::now();
    loop {
        assert!(session.status().error.is_none(), "{:?}", session.status());
        if let Some(screen) = session.snapshot(0)
            && condition(&screen.text())
        {
            return screen;
        }
        assert!(
            start.elapsed() < Duration::from_secs(15),
            "{}",
            session.snapshot(0).unwrap().text()
        );
        thread::sleep(Duration::from_millis(25));
    }
}
fn printed(text: &str, marker: &str) -> bool {
    text.lines().any(|line| line.trim() == marker)
}
fn prompt_line(text: &str) -> &str {
    text.lines()
        .rev()
        .find(|line| !line.trim().is_empty())
        .unwrap_or("")
}

#[test]
fn inherited_incompatible_modules_do_not_break_native_line_editing() {
    let root = std::env::temp_dir().join(format!(
        "rocket-readline-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let poison = root.join("modules/PSReadLine");
    fs::create_dir_all(&poison).unwrap();
    fs::write(poison.join("PSReadLine.psd1"), "@{ RootModule='PSReadLine.psm1'; ModuleVersion='99.0.0'; GUID='e330b2a9-1935-4bdf-b6a3-2b09650f261f' }").unwrap();
    fs::write(
        poison.join("PSReadLine.psm1"),
        "throw 'ROCKET_INCOMPATIBLE_MODULE_MUST_NOT_LOAD'",
    )
    .unwrap();
    fs::write(root.join("rocket-completion-中文.txt"), "unchanged fixture").unwrap();
    let output = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "psreadline_child", "--ignored", "--nocapture"])
        .env("ROCKET_READLINE_CHILD_ROOT", &root)
        .env("PSModulePath", root.join("modules"))
        .env("ROCKET_READLINE_UNICODE", "中文🚀")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let history = fs::read_to_string(root.join("history '中文'.log")).unwrap();
    assert!(history.contains("ROCKET_TYPED_OK"));
    assert_eq!(
        fs::read_to_string(root.join("rocket-completion-中文.txt")).unwrap(),
        "unchanged fixture"
    );
    // Only exact files created above and by this fixture's owned history are removed.
    for file in [
        poison.join("PSReadLine.psd1"),
        poison.join("PSReadLine.psm1"),
        root.join("rocket-completion-中文.txt"),
        root.join("history '中文'.log"),
    ] {
        fs::remove_file(file).unwrap();
    }
    fs::remove_dir(&poison).unwrap();
    fs::remove_dir(root.join("modules")).unwrap();
    fs::remove_dir(root).unwrap();
}

#[test]
#[ignore = "subprocess entrypoint; parent controls the inherited environment"]
fn psreadline_child() {
    let root = std::env::var_os("ROCKET_READLINE_CHILD_ROOT").expect("parent fixture");
    let root = Path::new(&root);
    let before = std::env::var_os("PSModulePath");
    let history = root.join("history '中文'.log");
    let session = Session::start_with_history(root, 160, 24, Arc::new(|| {}), &history).unwrap();
    let screen = wait(&session, |s| prompt_line(s).ends_with('>'));
    assert!(!screen.text().contains("Cannot load PSReadline"));
    assert!(!screen.text().contains("ROCKET_INCOMPATIBLE_MODULE"));
    session.input(b"Write-Output ('ROCKET_LOADED_' + [bool](Get-Module PSReadLine)); Write-Output ('ROCKET_CORE_' + [bool](Get-Command Get-ExecutionPolicy,Get-Item)); Write-Output ('ROCKET_ENV_' + $env:ROCKET_READLINE_UNICODE)\r").unwrap();
    wait(&session, |s| {
        printed(s, "ROCKET_LOADED_True")
            && printed(s, "ROCKET_CORE_True")
            && printed(s, "ROCKET_ENV_中文🚀")
    });
    session
        .input(b"Write-Output 'ROCKET_TYPED_OX'\x1b[D\x7fK\r")
        .unwrap();
    wait(&session, |s| printed(s, "ROCKET_TYPED_OK"));
    session
        .input(b"Write-Output 'ROCKET_HISTORY_SECOND'\r")
        .unwrap();
    wait(&session, |s| printed(s, "ROCKET_HISTORY_SECOND"));
    session.input(b"\x1b[A").unwrap();
    wait(&session, |s| {
        prompt_line(s).ends_with("Write-Output 'ROCKET_HISTORY_SECOND'")
    });
    session.input(b"\x1b[A").unwrap();
    wait(&session, |s| {
        prompt_line(s).ends_with("Write-Output 'ROCKET_TYPED_OK'")
    });
    session.input(b"\x1b[B\r").unwrap();
    wait(&session, |s| {
        s.lines()
            .filter(|l| l.trim() == "ROCKET_HISTORY_SECOND")
            .count()
            == 2
    });
    session.input(b"Write-Outp\t").unwrap();
    wait(&session, |s| {
        prompt_line(s).trim_end().ends_with("Write-Output")
    });
    session.input(b" 'ROCKET_COMPLETED'\r").unwrap();
    wait(&session, |s| printed(s, "ROCKET_COMPLETED"));
    session.input(b"Get-Item .\\rocket-comple\t").unwrap();
    wait(&session, |s| {
        prompt_line(s).contains("rocket-completion-中文.txt")
    });
    session.input(b"\x03").unwrap();
    wait(&session, |s| prompt_line(s).ends_with('>'));
    session
        .input(b"Write-Output 'ROCKET_AFTER_CANCEL'\r")
        .unwrap();
    wait(&session, |s| printed(s, "ROCKET_AFTER_CANCEL"));
    session.stop();
    let start = Instant::now();
    while !session.status().closed {
        assert!(start.elapsed() < Duration::from_secs(10));
        thread::sleep(Duration::from_millis(25));
    }
    assert_eq!(std::env::var_os("PSModulePath"), before);
}
