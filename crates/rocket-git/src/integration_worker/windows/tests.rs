use super::*;
use std::{
    net::{TcpListener, TcpStream},
    process::Command,
    sync::OnceLock,
    time::Duration,
};

fn fixture() -> &'static IntegrationWorker {
    static FIXTURE: OnceLock<IntegrationWorker> = OnceLock::new();
    FIXTURE.get_or_init(|| {
        let root =
            std::env::temp_dir().join(format!("rocket-worker-fixture-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        let source = root.join("probe.rs");
        let executable = root.join("rocket-lua-host.exe");
        fs::write(&source, include_str!("probe.rs")).unwrap();
        let result = Command::new("rustc")
            .args(["--edition=2024", "-C", "opt-level=1"])
            .arg(&source)
            .arg("-o")
            .arg(&executable)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        let digest = crate::advisory_tool::digest(
            &mut File::open(&executable).unwrap(),
            MAX_WORKER,
            &CancellationToken::new(),
        )
        .unwrap();
        IntegrationWorker::new(executable, &digest)
    })
}
fn limits() -> CommandLimits {
    CommandLimits {
        timeout: Duration::from_secs(5),
        stdout_bytes: 4096,
        stderr_bytes: 4096,
    }
}

#[test]
fn worker_hash_rejects_unbound_and_changed_files_before_start() {
    let original = fixture();
    let cancel = CancellationToken::new();
    for digest in ["", "00", &"0".repeat(64)] {
        let worker = IntegrationWorker::new(original.path.clone(), digest);
        let error = worker
            .run(b"echo\nshould-not-run".to_vec(), &limits(), &cancel)
            .err()
            .expect("must reject before launch");
        assert!(
            error.to_string().contains("worker_unbound")
                || error.to_string().contains("hash mismatch"),
            "{error}"
        );
    }
    let mutated = original.path.with_file_name("mutated.exe");
    let mut bytes = fs::read(&original.path).unwrap();
    bytes.push(0);
    fs::write(&mutated, bytes).unwrap();
    let worker = IntegrationWorker::new(mutated.clone(), &original.sha256);
    assert!(
        worker
            .run(Vec::new(), &limits(), &cancel)
            .err()
            .expect("must reject modified worker")
            .to_string()
            .contains("hash mismatch")
    );
    fs::remove_file(mutated).unwrap();
}

#[test]
fn windows_container_denies_private_files_network_children_and_executable_memory() {
    let worker = fixture();
    let root = worker.path.parent().unwrap();
    let private = root.join("private-fixture");
    let denied = root.join("must-not-exist");
    fs::write(&private, b"local synthetic data").unwrap();
    assert_eq!(fs::read(&private).unwrap(), b"local synthetic data");
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    // The target is reachable from the parent, so a denied child socket is meaningful.
    let baseline = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
    let _ = listener.accept().unwrap();
    drop(baseline);
    listener.set_nonblocking(true).unwrap();
    let sentinel = File::open(&private).unwrap();
    let unrelated: Vec<_> = (0..512)
        .map(|_| {
            let file = sentinel.try_clone().unwrap();
            // SAFETY: synthetic file only; test wide inheritance without risking an
            // invalid-handle exception when the child reuses a numeric handle slot.
            checked(unsafe { SetHandleInformation(file.as_raw_handle(), 1, 1) }).unwrap();
            file
        })
        .collect();
    let input = format!(
        "permissions\n{}\n{}\n{}\n{}",
        private.display(),
        denied.display(),
        listener.local_addr().unwrap(),
        sentinel.as_raw_handle() as usize
    );
    let output = worker
        .run(input.into_bytes(), &limits(), &CancellationToken::new())
        .unwrap();
    assert!(
        output.status.success(),
        "status={} stdout={} stderr={}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8(output.stdout).unwrap().trim(),
        "read=false;write=false;network=false;child=false;executable_memory=false;large_allocation=false;inherited=false"
    );
    assert!(matches!(
        String::from_utf8(output.stderr).unwrap().trim(),
        "network_error=10013" | "network_error=10107"
    ));
    assert!(!denied.exists());
    assert_eq!(fs::read(&private).unwrap(), b"local synthetic data");
    assert_eq!(
        listener.accept().unwrap_err().kind(),
        io::ErrorKind::WouldBlock
    );
    drop(sentinel);
    drop(unrelated);
    fs::remove_file(private).unwrap();
}

#[test]
fn isolated_worker_echo_limits_and_cancellation_reap_the_process() {
    let worker = fixture();
    let cancel = CancellationToken::new();
    let output = worker
        .run("echo\n資料 🚀".as_bytes().to_vec(), &limits(), &cancel)
        .unwrap();
    assert!(output.status.success(), "status={}", output.status);
    assert_eq!(output.stdout, "資料 🚀".as_bytes());
    assert!(matches!(
        worker.run(b"flood".to_vec(), &limits(), &cancel),
        Err(Error::OutputLimit { .. })
    ));
    let short = CommandLimits {
        timeout: Duration::from_millis(100),
        ..limits()
    };
    assert!(matches!(
        worker.run(b"wait".to_vec(), &short, &cancel),
        Err(Error::TimedOut)
    ));
    let mut observed = false;
    let result = run_observed(worker, b"wait".to_vec(), &limits(), &cancel, |handle| {
        // SAFETY: observer borrows the live contained process before collection starts.
        assert_eq!(unsafe { WaitForSingleObject(handle, 0) }, 0x102);
        observed = true;
        cancel.cancel();
    });
    assert!(observed);
    assert!(matches!(result, Err(Error::Cancelled)));
}

#[test]
fn worker_path_and_direct_execution_require_isolation() {
    for path in [
        r"\\server\share\worker.exe",
        r"C:\folder\worker.exe:stream",
        r"C:\folder\..\worker.exe",
    ] {
        assert!(local_path(Path::new(path)).is_err(), "{path}");
    }
    assert!(IntegrationWorker::enter_isolation().is_err());
}

#[link(name = "kernel32")]
unsafe extern "system" {
    fn GetProcessId(process: Handle) -> u32;
    fn OpenProcess(access: u32, inherit: i32, id: u32) -> Handle;
    fn QueryFullProcessImageNameW(
        process: Handle,
        flags: u32,
        name: *mut u16,
        length: *mut u32,
    ) -> i32;
}

/// The file a live contained process is running from: the stage's copy of the worker.
fn image_path(process: Handle) -> String {
    let mut path = vec![0u16; 32768];
    let mut length = path.len() as u32;
    // SAFETY: observer borrows the live contained process; buffer is writable.
    checked(unsafe { QueryFullProcessImageNameW(process, 0, path.as_mut_ptr(), &mut length) })
        .unwrap();
    String::from_utf16(&path[..length as usize]).unwrap()
}

#[test]
#[ignore = "invoked only by the parent-exit regression with an owned fixture"]
fn parent_exit_helper() {
    let worker = IntegrationWorker::new(
        PathBuf::from(std::env::var_os("ROCKET_CRASH_PROBE").unwrap()),
        &std::env::var("ROCKET_CRASH_HASH").unwrap(),
    );
    let _ = run_observed(
        &worker,
        b"wait".to_vec(),
        &limits(),
        &CancellationToken::new(),
        |process| {
            let path = image_path(process);
            // SAFETY: live process handle. Exit deliberately bypasses Rust destructors.
            println!("WORKER:{}", unsafe { GetProcessId(process) });
            println!("STAGE:{path}");
            std::io::stdout().flush().unwrap();
            std::process::exit(0);
        },
    );
    panic!("parent exit observer was not reached");
}

#[test]
fn abrupt_parent_exit_kills_the_contained_worker() {
    let worker = fixture();
    let output = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "integration_worker::windows::tests::parent_exit_helper",
            "--ignored",
            "--nocapture",
        ])
        .env("ROCKET_CRASH_PROBE", &worker.path)
        .env("ROCKET_CRASH_HASH", &worker.sha256)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = String::from_utf8(output.stdout).unwrap();
    let id: u32 = text
        .lines()
        .find_map(|line| line.strip_prefix("WORKER:"))
        .expect("worker started")
        .parse()
        .unwrap();
    assert_ne!(id, 0);
    // SAFETY: only synchronize access to the exact child PID reported by our helper.
    let handle = unsafe { OpenProcess(0x0010_0000, 0, id) };
    if handle.is_null() {
        assert_eq!(
            io::Error::last_os_error().raw_os_error(),
            Some(87),
            "worker exit must be confirmed, not an access denial"
        );
    } else {
        let handle = owned(handle).unwrap();
        // SAFETY: live owned process handle; a signaled process is terminated.
        assert_eq!(
            unsafe { WaitForSingleObject(handle.as_raw_handle(), 5000) },
            0,
            "orphan worker survived its parent"
        );
    }
    // A forced parent exit intentionally skips temporary-storage cleanup. Remove
    // only the exact, validated test artifacts after proving the child is gone.
    let executable = PathBuf::from(
        text.lines()
            .find_map(|line| line.strip_prefix("STAGE:"))
            .unwrap(),
    );
    local_path(&executable).unwrap();
    let directory = executable.parent().unwrap();
    assert_eq!(directory.parent().unwrap(), std::env::temp_dir());
    let nonce = directory
        .file_name()
        .unwrap()
        .to_str()
        .unwrap()
        .strip_prefix("rocket-lua-")
        .unwrap();
    assert!(nonce.len() == 32 && nonce.bytes().all(|c| c.is_ascii_hexdigit()));
    assert_eq!(executable.file_name().unwrap(), "rocket-lua-host.exe");
    // Kill-on-close and image-section release can finish on different kernel
    // paths. Retry only the transient sharing violation while cleaning up this
    // crash fixture; termination itself was already required above.
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        match fs::remove_file(&executable) {
            Ok(()) => break,
            Err(error)
                if error.raw_os_error() == Some(32) && std::time::Instant::now() < deadline =>
            {
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(error) => panic!("crash fixture cleanup failed: {error}"),
        }
    }
    fs::remove_dir(directory).unwrap();
    let name = wide(OsStr::new(&format!("Rocket.Lua.{nonce}"))).unwrap();
    // SAFETY: the exact random profile created by our now-exited helper.
    assert!(unsafe { DeleteAppContainerProfile(name.as_ptr()) } >= 0);
}

/// A stage nothing has run from, in a container of its own.
fn empty_stage() -> (Profile, Stage) {
    let time = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let nonce = format!("t{:x}{time:x}", std::process::id());
    let profile = Profile::new(&nonce).unwrap();
    let stage = Stage::new(b"not run", &profile.sid, &nonce).unwrap();
    (profile, stage)
}

#[test]
fn a_held_stage_goes_once_the_holder_lets_go() {
    let (mut profile, mut stage) = empty_stage();
    let directory = stage.directory.clone();
    // As a scanner does: an open handle that doesn't share delete access.
    let holder = locked_file(&stage.executable).unwrap();
    let release = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(100));
        drop(holder);
    });
    stage.cleanup().unwrap();
    release.join().unwrap();
    assert!(!directory.exists());
    profile.cleanup().unwrap();
}

#[test]
fn a_stage_that_stays_held_is_reported_and_waited_for_only_once() {
    let (mut profile, mut stage) = empty_stage();
    let wait = Duration::from_millis(300);
    stage.wait = wait;
    let holder = locked_file(&stage.executable).unwrap();
    let started = Instant::now();
    assert_eq!(stage.cleanup().unwrap_err().raw_os_error(), Some(32));
    assert!(started.elapsed() >= wait);
    // Drop tries again, and has already had its wait.
    let started = Instant::now();
    assert!(stage.cleanup().is_err());
    assert!(started.elapsed() < wait);
    drop(holder);
    stage.cleanup().unwrap();
    assert!(!stage.directory.exists());
    profile.cleanup().unwrap();
}

#[test]
fn a_cleanup_failure_never_hides_how_the_run_ended() {
    let cancel = CancellationToken::new();
    let mut held = None;
    let result = run_observed(fixture(), b"wait".to_vec(), &limits(), &cancel, |process| {
        let path = PathBuf::from(image_path(process));
        held = Some((locked_file(&path).unwrap(), path));
        cancel.cancel();
    });
    assert!(matches!(result, Err(Error::Cancelled)), "{:?}", result.err());
    let (holder, executable) = held.unwrap();
    drop(holder);
    fs::remove_file(&executable).unwrap();
    fs::remove_dir(executable.parent().unwrap()).unwrap();
}

#[test]
#[ignore = "requires a built worker and ROCKET_TEST_LUA_WORKER / ROCKET_TEST_LUA_SHA256"]
fn actual_lua_worker_runs_only_in_the_verified_container() {
    let path =
        PathBuf::from(std::env::var_os("ROCKET_TEST_LUA_WORKER").expect("set test worker path"));
    let hash = std::env::var("ROCKET_TEST_LUA_SHA256")
        .expect("set independently computed test worker digest");
    let direct = Command::new(&path).output().unwrap();
    assert!(!direct.status.success());
    assert_eq!(direct.stdout, b"{\"kind\":\"error\",\"code\":\"sandbox\"}");
    let worker = IntegrationWorker::new(path, &hash);
    for (input, expected) in [
        (
            r#"{"api":1,"source":"return function() assert(os==nil and io==nil and require==nil and package==nil); return ui.text('資料 🚀') end","context":{}}"#,
            r#""kind":"view""#,
        ),
        (
            r#"{"api":1,"source":"error('must not run during validation')","validate":true}"#,
            r#""kind":"validated""#,
        ),
        (
            r#"{"api":1,"source":"return function() while true do end end","context":{}}"#,
            r#""code":"instructions""#,
        ),
        (
            r#"{"api":1,"source":"return function() return string.rep('x',67108864) end","context":{}}"#,
            r#""kind":"error""#,
        ),
        (
            r#"{"api":1,"source":"\u001bLua","context":{}}"#,
            r#""kind":"error""#,
        ),
    ] {
        let output = worker
            .run(
                input.as_bytes().to_vec(),
                &limits(),
                &CancellationToken::new(),
            )
            .unwrap();
        assert!(
            output.status.success(),
            "{} {}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        );
        let result = String::from_utf8(output.stdout).unwrap();
        assert!(result.contains(expected), "expected {expected} in {result}");
    }
}
