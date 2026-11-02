#![cfg(windows)]
use rocket_terminal::{Screen, Session};
use std::{
    fs,
    sync::Arc,
    thread,
    time::{Duration, Instant},
};
fn wait(session: &Session, mut predicate: impl FnMut(&Screen) -> bool) -> Screen {
    let start = Instant::now();
    loop {
        let status = session.status();
        assert!(status.error.is_none(), "{status:?}");
        if let Some(screen) = session.snapshot(0)
            && predicate(&screen)
        {
            return screen;
        }
        assert!(
            start.elapsed() < Duration::from_secs(15),
            "terminal timed out: {status:?}\n{}",
            session.snapshot(0).unwrap().text()
        );
        thread::sleep(Duration::from_millis(30));
    }
}
#[test]
fn real_powershell_keeps_state_handles_input_resize_interrupt_and_owned_shutdown() {
    let root = std::env::temp_dir().join(format!(
        "rocket-terminal-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir(&root).unwrap();
    let session =
        Session::start_with_history(&root, 100, 18, Arc::new(|| {}), &root.join("history.log"))
            .unwrap();
    wait(&session, |s| s.text().contains("PS "));
    session.input(b"$global:RocketProbe=42\r").unwrap();
    wait(&session, |s| s.text().matches("PS ").count() >= 2);
    session
        .input(b"Write-Output ('ROCKET_VALUE_' + $global:RocketProbe)\r")
        .unwrap();
    wait(&session, |s| s.text().contains("ROCKET_VALUE_42"));
    session.resize(76, 12);
    thread::sleep(Duration::from_millis(100));
    session
        .input(b"[Console]::WriteLine([char]0x00E9)\r")
        .unwrap();
    let screen = wait(&session, |s| s.text().contains('é'));
    assert_eq!((screen.columns, screen.rows), (76, 12));
    session
        .input(b"$answer=Read-Host 'ROCKET_INPUT'; [Console]::WriteLine('ANSWER_'+$answer)\r")
        .unwrap();
    wait(&session, |s| s.text().contains("ROCKET_INPUT:"));
    session.input(b"yeX\x7fs\r").unwrap();
    wait(&session, |s| s.text().contains("ANSWER_yes"));
    session.input(b"Start-Sleep -Seconds 30\r").unwrap();
    thread::sleep(Duration::from_millis(500));
    session.input(&[3]).unwrap();
    thread::sleep(Duration::from_millis(150));
    session
        .input(b"Write-Output ('AFTER_'+'INTERRUPT')\r")
        .unwrap();
    wait(&session, |s| s.text().contains("AFTER_INTERRUPT"));
    session
        .input(b"1..1200 | ForEach-Object { [Console]::WriteLine('bounded') }\r")
        .unwrap();
    let screen = wait(&session, |s| s.scrollback.len() == 1000);
    assert!(screen.cells.len() <= 100);
    session.stop();
    let start = Instant::now();
    while !session.status().closed {
        assert!(
            start.elapsed() < Duration::from_secs(10),
            "shutdown stalled"
        );
        thread::sleep(Duration::from_millis(30));
    }
    assert!(session.input(b"x").is_err());
    assert!(!session.status().running);
    fs::remove_file(root.join("history.log")).unwrap();
    fs::remove_dir(&root).unwrap();
}

#[test]
fn dropping_session_reaps_background_children_even_with_queued_input() {
    let root = std::env::temp_dir().join(format!(
        "rocket-terminal-child-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir(&root).unwrap();
    let session =
        Session::start_with_history(&root, 100, 18, Arc::new(|| {}), &root.join("history.log"))
            .unwrap();
    wait(&session, |s| s.text().contains("PS "));
    session.input(b"$PID | Set-Content -LiteralPath 'shell.pid'; $child = Start-Process -FilePath ($PSHOME + '\\powershell.exe') -ArgumentList '-NoLogo','-NoProfile','-Command','Start-Sleep -Seconds 120' -PassThru -WindowStyle Hidden; $child.Id | Set-Content -LiteralPath 'child.pid'\r").unwrap();
    let start = Instant::now();
    let child = loop {
        if let Ok(text) = fs::read_to_string(root.join("child.pid"))
            && let Ok(pid) = text.trim().parse::<u32>()
        {
            break pid;
        }
        assert!(
            start.elapsed() < Duration::from_secs(10),
            "child not started: {}",
            session.snapshot(0).unwrap().text()
        );
        thread::sleep(Duration::from_millis(30));
    };
    unsafe extern "system" {
        fn OpenProcess(access: u32, inherit: i32, pid: u32) -> *mut std::ffi::c_void;
        fn WaitForSingleObject(handle: *mut std::ffi::c_void, ms: u32) -> u32;
        fn CloseHandle(handle: *mut std::ffi::c_void) -> i32;
    }
    let shell: u32 = fs::read_to_string(root.join("shell.pid"))
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    // Drop is asynchronous. Pin all owned process identities before dropping;
    // one child's exit is not proof that the shell/host released their CWD.
    // SAFETY: synchronize-only handles; never signal or terminate these PIDs.
    let handles: Vec<_> = [child, shell, session.status().pid.unwrap()]
        .into_iter()
        .map(|pid| unsafe { OpenProcess(0x0010_0000, 0, pid) })
        .collect();
    assert!(handles.iter().all(|h| !h.is_null()));
    session.input(b"Start-Sleep -Seconds 30\r").unwrap();
    thread::sleep(Duration::from_millis(200));
    for _ in 0..40 {
        let _ = session.input(&vec![b'x'; 8192]);
    }
    drop(session);
    // SAFETY: each handle remains live and is closed once after waiting.
    let results: Vec<_> = handles
        .into_iter()
        .map(|handle| unsafe {
            let ended = WaitForSingleObject(handle, 10_000);
            CloseHandle(handle);
            ended
        })
        .collect();
    assert!(
        results.iter().all(|&r| r == 0),
        "owned process survived terminal drop: {results:?}"
    );
    fs::remove_file(root.join("child.pid")).unwrap();
    fs::remove_file(root.join("shell.pid")).unwrap();
    fs::remove_file(root.join("history.log")).unwrap();
    fs::remove_dir(&root).unwrap();
}

#[test]
fn real_shell_uses_utf8_and_grapheme_cursor_positions() {
    let root = std::env::temp_dir().join(format!(
        "rocket-terminal-unicode-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir(&root).unwrap();
    let session =
        Session::start_with_history(&root, 80, 12, Arc::new(|| {}), &root.join("history.log"))
            .unwrap();
    wait(&session, |s| s.text().contains("PS "));
    session
        .input(
            "[Console]::Clear(); [Console]::Write('中é👩🏽‍💻|END'); Start-Sleep -Seconds 30\r"
                .as_bytes(),
        )
        .unwrap();
    let screen = wait(&session, |s| {
        s.cells[0][0].ch == '中' && s.text().contains("|END")
    });
    assert_eq!(screen.cursor, (9, 0));
    assert_eq!(screen.cells[0][0].width, 2);
    assert_eq!(screen.cells[0][2].text(), "é");
    assert_eq!(screen.cells[0][3].text(), "👩🏽‍💻");
    assert_eq!(screen.cells[0][5].ch, '|');
    session.stop();
    let start = Instant::now();
    while !session.status().closed {
        assert!(start.elapsed() < Duration::from_secs(10));
        thread::sleep(Duration::from_millis(20));
    }
    fs::remove_file(root.join("history.log")).unwrap();
    fs::remove_dir(root).unwrap();
}

#[test]
fn unavailable_working_directory_reports_startup_failure_without_a_live_shell() {
    let missing = std::env::temp_dir().join(format!(
        "rocket-terminal-missing-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    assert!(!missing.exists());
    let session = Session::start(&missing, 80, 12, Arc::new(|| {})).unwrap();
    let start = Instant::now();
    loop {
        let status = session.status();
        if status.closed {
            assert!(!status.running);
            assert!(status.pid.is_none());
            assert!(status.error.is_some());
            assert!(session.input(b"x").is_err());
            break;
        }
        assert!(start.elapsed() < Duration::from_secs(10));
        thread::sleep(Duration::from_millis(30));
    }
    assert!(!missing.exists());
}
