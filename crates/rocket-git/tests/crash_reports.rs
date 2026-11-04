//! Failures are triggered only in disposable test subprocesses, never a user's app.
use rocket_git::{
    CancellationToken,
    crash_reports::{self, Component, Kind},
};
use std::{
    fs,
    path::PathBuf,
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

#[test]
#[ignore = "invoked only by the isolated crash tests"]
fn crash_probe() {
    let root = PathBuf::from(std::env::var_os("ROCKET_CRASH_FIXTURE").expect("isolated fixture"));
    assert!(root.is_absolute());
    let mode = std::env::var("ROCKET_CRASH_MODE").unwrap();
    if mode == "busy" {
        fs::create_dir_all(&root).unwrap();
        let lock = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(root.join("store.lock"))
            .unwrap();
        lock.lock().unwrap();
        assert!(crash_reports::install(Ok(root.clone()), Component::Cli).is_err());
        assert_eq!(crash_reports::directory(), Some(root.as_path()));
        assert_eq!(
            crash_reports::capture_error(),
            Some(std::io::ErrorKind::WouldBlock)
        );
        return;
    }
    crash_reports::install(Ok(root), Component::Cli).unwrap();
    match mode.as_str() {
        "panic" => {
            panic!("PRIVATE_CRASH_PAYLOAD: repository content and credentials must not be recorded")
        }
        "caught" => {
            let _ = std::panic::catch_unwind(|| panic!("PRIVATE_CRASH_PAYLOAD"));
        }
        "worker" => {
            let _ = std::thread::Builder::new()
                .name("PRIVATE_THREAD_NAME".into())
                .spawn(|| panic!("PRIVATE_CRASH_PAYLOAD"))
                .unwrap()
                .join();
        }
        "limit" => {
            for _ in 0..30 {
                let _ = std::panic::catch_unwind(|| panic!("PRIVATE_CRASH_PAYLOAD"));
            }
        }
        #[cfg(windows)]
        "native" => unsafe {
            unsafe extern "system" {
                fn SetErrorMode(mode: u32) -> u32;
                fn RaiseException(code: u32, flags: u32, count: u32, data: *const usize);
            }
            SetErrorMode(0x0002); // This child only; suppress an interactive OS crash dialog.
            RaiseException(0xe0425243, 1, 0, std::ptr::null());
        },
        _ => panic!("Unknown isolated probe mode"),
    }
}

#[test]
fn records_real_panics_workers_and_unhandled_exceptions_without_payloads() {
    let root = std::env::temp_dir().join(format!(
        "rocket-crash-process-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir(&root).unwrap();
    for mode in ["panic", "caught", "worker", "limit", "native", "busy"] {
        if mode == "native" && !cfg!(windows) {
            continue;
        }
        let directory = root.join(format!("报告-{mode}"));
        let output = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "crash_probe",
                "--ignored",
                "--nocapture",
                "--test-threads=1",
            ])
            .env("ROCKET_CRASH_FIXTURE", &directory)
            .env("ROCKET_CRASH_MODE", mode)
            .output()
            .unwrap();
        assert_eq!(
            output.status.success(),
            matches!(mode, "caught" | "worker" | "limit" | "busy"),
            "mode={mode}"
        );
        assert!(!String::from_utf8_lossy(&output.stderr).contains("PRIVATE_"));
        let s = crash_reports::read(&directory, &CancellationToken::new()).unwrap();
        assert_eq!(s.damaged, 0, "mode={mode}");
        assert_eq!(
            s.reports.len(),
            match mode {
                "limit" => 8,
                "busy" => 0,
                _ => 1,
            },
            "mode={mode}"
        );
        for r in s.reports {
            assert!(!r.json().contains("PRIVATE_"));
            assert!(!r.json().contains(&root.to_string_lossy().to_string()));
            assert_eq!(
                r.kind,
                if mode == "native" {
                    Kind::WindowsException
                } else {
                    Kind::RustPanic
                }
            );
            if mode != "native" {
                assert!(r.line > 0);
            }
        }
    }
    fs::remove_dir_all(root).unwrap();
}
