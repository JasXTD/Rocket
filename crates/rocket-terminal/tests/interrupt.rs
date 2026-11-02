#![cfg(windows)]
use rocket_terminal::{Screen, Session};
use std::{
    fs,
    process::Command,
    sync::Arc,
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

fn wait(session: &Session, label: &str, predicate: impl Fn(&Screen) -> bool) -> Screen {
    let start = Instant::now();
    loop {
        let status = session.status();
        assert!(
            status.running && !status.closed && status.error.is_none(),
            "{label}: {status:?}"
        );
        if let Some(screen) = session.snapshot(0)
            && predicate(&screen)
        {
            return screen;
        }
        assert!(
            start.elapsed() < Duration::from_secs(8),
            "{label} timed out: {}",
            session.snapshot(0).unwrap().text()
        );
        thread::sleep(Duration::from_millis(20));
    }
}
fn printed(screen: &Screen, marker: &str) -> bool {
    screen.text().lines().any(|line| line.trim() == marker)
}
fn prompt(screen: &Screen) -> bool {
    screen.text().trim_end().ends_with("\n>")
}

#[test]
fn command_interrupt_is_independent_of_launcher_ctrl_c_policy() {
    for ignored in [false, true] {
        let output = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "interrupt_child", "--ignored", "--nocapture"])
            .env("ROCKET_INTERRUPT_IGNORE", if ignored { "1" } else { "0" })
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "launcher ignore={ignored}:\n{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        print!("{}", String::from_utf8_lossy(&output.stdout));
    }
}

#[test]
#[ignore = "subprocess entrypoint isolates the launcher control-handler policy"]
fn interrupt_child() {
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn SetConsoleCtrlHandler(handler: *const std::ffi::c_void, add: i32) -> i32;
        fn OpenProcess(access: u32, inherit: i32, pid: u32) -> *mut std::ffi::c_void;
        fn WaitForSingleObject(handle: *mut std::ffi::c_void, ms: u32) -> u32;
        fn CloseHandle(handle: *mut std::ffi::c_void) -> i32;
    }
    struct ChildHandle(*mut std::ffi::c_void);
    impl Drop for ChildHandle {
        fn drop(&mut self) {
            // SAFETY: unique synchronize-only handle to this fixture's child.
            unsafe {
                CloseHandle(self.0);
            }
        }
    }
    let ignored = std::env::var("ROCKET_INTERRUPT_IGNORE").unwrap() == "1";
    // This subprocess owns its policy. Never change the main test host or user shell.
    assert_ne!(
        unsafe { SetConsoleCtrlHandler(std::ptr::null(), i32::from(ignored)) },
        0
    );
    let root = std::env::temp_dir().join(format!(
        "rocket-interrupt-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir(&root).unwrap();
    let session =
        Session::start_with_history(&root, 150, 20, Arc::new(|| {}), &root.join("history.log"))
            .unwrap();
    let started = Instant::now();
    while !session.status().running {
        assert!(!session.status().closed, "{:?}", session.status());
        assert!(started.elapsed() < Duration::from_secs(10));
        thread::sleep(Duration::from_millis(20));
    }
    wait(&session, "initial prompt", prompt);
    let pid = session.status().pid;
    session.input(b"$global:RocketInterruptState=42; [IO.File]::WriteAllText('shell.pid',[string]$PID); [Console]::WriteLine('READY_'+'INTERRUPT'); Start-Sleep -Seconds 60; [Console]::WriteLine('MUST_NOT_'+'FINISH')\r").unwrap();
    wait(&session, "running sleep", |s| printed(s, "READY_INTERRUPT"));
    let interrupt = Instant::now();
    session.input(&[3]).unwrap();
    wait(&session, "prompt after interrupt", prompt);
    println!(
        "interrupt_ms={} ignore={ignored}",
        interrupt.elapsed().as_millis()
    );
    session
        .input(b"[Console]::WriteLine('PRESERVED_'+$global:RocketInterruptState)\r")
        .unwrap();
    let screen = wait(&session, "same-shell state", |s| printed(s, "PRESERVED_42"));
    assert!(!printed(&screen, "MUST_NOT_FINISH"));
    assert_eq!(session.status().pid, pid);
    wait(&session, "ready for external command", prompt);
    session.input(b"& ($PSHOME + '\\powershell.exe') -NoLogo -NoProfile -Command \"[IO.File]::WriteAllText('foreground.pid',[string]`$PID); [Console]::WriteLine('NATIVE_'+'READY'); Start-Sleep -Seconds 60; [Console]::WriteLine('NATIVE_'+'UNEXPECTED')\"\r").unwrap();
    wait(&session, "external command started", |s| {
        printed(s, "NATIVE_READY")
    });
    let child: u32 = fs::read_to_string(root.join("foreground.pid"))
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    // SAFETY: PID read from this fresh fixture. Pin process identity before
    // interrupting; no signal or termination is issued through this handle.
    let child = ChildHandle(unsafe { OpenProcess(0x0010_0000, 0, child) });
    assert!(!child.0.is_null());
    assert_eq!(unsafe { WaitForSingleObject(child.0, 0) }, 258);
    let interrupt = Instant::now();
    session.input(&[3]).unwrap();
    wait(&session, "prompt after external interrupt", prompt);
    assert_eq!(
        unsafe { WaitForSingleObject(child.0, 0) },
        0,
        "external command survived interruption"
    );
    println!(
        "external_interrupt_ms={} ignore={ignored}",
        interrupt.elapsed().as_millis()
    );
    session.input(b"[IO.File]::WriteAllText('after.pid',[string]$PID); [Console]::WriteLine('AFTER_NATIVE_'+$global:RocketInterruptState)\r").unwrap();
    let screen = wait(&session, "shell still usable after external command", |s| {
        printed(s, "AFTER_NATIVE_42")
    });
    assert!(!printed(&screen, "NATIVE_UNEXPECTED"));
    assert_eq!(
        fs::read(root.join("shell.pid")).unwrap(),
        fs::read(root.join("after.pid")).unwrap()
    );
    assert_eq!(session.status().pid, pid);
    session.stop();
    let stopped = Instant::now();
    while !session.status().closed {
        assert!(stopped.elapsed() < Duration::from_secs(10));
        thread::sleep(Duration::from_millis(20));
    }
    for file in ["history.log", "shell.pid", "after.pid", "foreground.pid"] {
        fs::remove_file(root.join(file)).unwrap();
    }
    fs::remove_dir(root).unwrap();
}
