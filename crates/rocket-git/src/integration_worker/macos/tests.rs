use super::*;
use std::time::{Duration, Instant};

fn fixture() -> IntegrationWorker {
    let app = PathBuf::from(
        std::env::var_os("ROCKET_TEST_MAC_LUA_APP")
            .expect("Run the focused Mac Lua integration build with its signed helper"),
    );
    let path = app.join(FILES[0]);
    IntegrationWorker::new(
        path.clone(),
        &crate::sha256_bytes(&fs::read(path).unwrap()).unwrap(),
    )
}
fn limits() -> CommandLimits {
    CommandLimits {
        timeout: Duration::from_secs(5),
        stdout_bytes: 1024 * 1024,
        stderr_bytes: 4096,
    }
}
fn input() -> Vec<u8> {
    br#"{"api":1,"source":"return function() return ui.text('hello') end","context":{}}"#.to_vec()
}

#[test]
fn signed_helper_runs_and_snapshot_is_removed() {
    let worker = fixture();
    let parent = crate::local_storage::directory().unwrap().join("lua-runs");
    let before = fs::read_dir(&parent).map(|r| r.count()).unwrap_or(0);
    let out = worker
        .run(input(), &limits(), &CancellationToken::new())
        .unwrap();
    assert!(out.status.success());
    assert!(String::from_utf8(out.stdout).unwrap().contains("hello"));
    assert_eq!(fs::read_dir(parent).unwrap().count(), before);
}

#[test]
fn mismatched_digest_and_cancel_refuse_before_spawn() {
    let mut worker = fixture();
    worker.sha256 = "0".repeat(64);
    let error = worker
        .run(input(), &limits(), &CancellationToken::new())
        .err()
        .unwrap();
    assert!(error.to_string().contains("hash mismatch"), "{error}");
    let cancel = CancellationToken::new();
    cancel.cancel();
    assert!(matches!(
        worker.run(input(), &limits(), &cancel),
        Err(Error::Cancelled)
    ));
}

#[test]
fn cancellation_and_output_bounds_reap_actual_signed_helper() {
    let worker = fixture();
    let cancel = CancellationToken::new();
    let mut pid = None;
    let mut stopped = Instant::now();
    let result = run_observed(&worker, input(), &limits(), &cancel, |child| {
        pid = Some(child);
        stopped = Instant::now();
        cancel.cancel();
    });
    assert!(
        matches!(result, Err(Error::Cancelled)),
        "{:?}",
        result.err()
    );
    assert!(stopped.elapsed() < Duration::from_secs(1));
    assert_eq!(
        unsafe { kill(pid.unwrap() as i32, 0) },
        -1,
        "cancelled child was not reaped"
    );
    let bounded = CommandLimits {
        stdout_bytes: 4,
        ..limits()
    };
    assert!(matches!(
        worker.run(input(), &bounded, &CancellationToken::new()),
        Err(Error::OutputLimit { .. })
    ));
    let short = CommandLimits {
        timeout: Duration::from_nanos(1),
        ..limits()
    };
    assert!(matches!(
        worker.run(input(), &short, &CancellationToken::new()),
        Err(Error::TimedOut)
    ));
}

// Runs as a child test process only when invoked by the parent-exit test. Blocking
// in the spawn observation tests the real launcher's pre-input/startup interval.
#[test]
fn parent_fixture() {
    let Some(marker) = std::env::var_os("ROCKET_LUA_PARENT_TEST_MARKER") else {
        return;
    };
    run_observed(
        &fixture(),
        input(),
        &limits(),
        &CancellationToken::new(),
        |pid| {
            fs::write(&marker, pid.to_string()).unwrap();
            std::thread::sleep(Duration::from_secs(10));
        },
    )
    .unwrap();
}

#[test]
fn parent_exit_during_real_launcher_startup_stops_worker() {
    let marker =
        std::env::temp_dir().join(format!("rocket-lua-parent-{}.json", std::process::id()));
    assert!(!marker.exists());
    let child = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "integration_worker::macos::tests::parent_fixture",
            "--nocapture",
        ])
        .env("ROCKET_LUA_PARENT_TEST_MARKER", &marker)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    struct Owned(Child);
    impl Drop for Owned {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    let mut parent = Owned(child);
    let until = Instant::now() + Duration::from_secs(4);
    while !marker.exists() {
        assert!(Instant::now() < until);
        assert!(parent.0.try_wait().unwrap().is_none());
        std::thread::sleep(Duration::from_millis(5));
    }
    let pid = fs::read_to_string(&marker).unwrap();
    assert!(pid.parse::<u32>().unwrap() > 1);
    parent.0.kill().unwrap();
    parent.0.wait().unwrap();
    let stop = Instant::now();
    loop {
        let state = Command::new("/bin/ps")
            .args(["-p", &pid, "-o", "stat="])
            .output()
            .unwrap();
        let value = String::from_utf8(state.stdout).unwrap();
        if value.trim().is_empty() || value.trim().starts_with('Z') {
            break;
        }
        assert!(
            stop.elapsed() < Duration::from_secs(1),
            "worker outlived parent"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    fs::remove_file(marker).unwrap();
}

#[test]
fn sealed_metadata_and_linked_or_writable_files_are_refused() {
    let original = fixture();
    let source = original
        .path
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let root = crate::local_storage::directory()
        .unwrap()
        .join(format!("lua-test-{}.app", std::process::id()));
    crate::local_storage::create_private_new(&root).unwrap();
    let meta = fs::metadata(&root).unwrap();
    let copied = Snapshot {
        app: root.clone(),
        device: meta.dev(),
        inode: meta.ino(),
    };
    for name in FILES {
        fs::create_dir_all(root.join(name).parent().unwrap()).unwrap();
        fs::copy(source.join(name), root.join(name)).unwrap();
    }
    let path = root.join(FILES[0]);
    let worker = IntegrationWorker::new(path.clone(), &original.sha256);
    fs::write(root.join(FILES[1]), "changed metadata").unwrap();
    let e = worker
        .run(input(), &limits(), &CancellationToken::new())
        .err()
        .unwrap();
    assert!(
        e.to_string().contains("signature") || e.to_string().contains("sealed"),
        "{e}"
    );
    fs::copy(source.join(FILES[1]), root.join(FILES[1])).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o777)).unwrap();
    assert!(
        worker
            .run(input(), &limits(), &CancellationToken::new())
            .is_err()
    );
    fs::remove_file(&path).unwrap();
    std::os::unix::fs::symlink(&original.path, &path).unwrap();
    assert!(
        worker
            .run(input(), &limits(), &CancellationToken::new())
            .is_err()
    );
    drop(copied);
}
