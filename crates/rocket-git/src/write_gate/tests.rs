use super::*;
use std::{
    fs,
    process::{Child, Command, Stdio},
    time::{Instant, SystemTime, UNIX_EPOCH},
};

pub(super) struct Fixture(pub(super) PathBuf);
fn fixture_base() -> PathBuf {
    // File-lock/rename semantics must be exercised on a native Unix filesystem,
    // even when the source lives on WSL's Windows mount.
    #[cfg(unix)]
    {
        std::env::temp_dir().join("rocket-write-gate-tests")
    }
    #[cfg(not(unix))]
    {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../work/write-gate-tests")
    }
}
impl Fixture {
    pub(super) fn new() -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let base = fixture_base();
        fs::create_dir_all(&base).unwrap();
        let path = base.join(format!(
            "{}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        fs::create_dir_all(path.join("checkout 文/.git")).unwrap();
        Self(path)
    }
    fn child(&self) -> Owner {
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args([
                "--ignored",
                "--exact",
                "write_gate::tests::process_owner",
                "--nocapture",
            ])
            .env("ROCKET_WRITE_GATE_FIXTURE", &self.0)
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x0800_0000);
        }
        let mut owner = Owner(command.spawn().unwrap());
        let deadline = Instant::now() + Duration::from_secs(10);
        while !self.0.join("ready").exists() {
            assert!(
                owner.0.try_wait().unwrap().is_none(),
                "owner exited before acquiring lock"
            );
            assert!(Instant::now() < deadline, "owner never acquired lock");
            std::thread::sleep(Duration::from_millis(10));
        }
        owner
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let base = fixture_base().canonicalize().unwrap();
        let target = self.0.canonicalize().unwrap();
        assert_eq!(target.parent(), Some(base.as_path()));
        fs::remove_dir_all(target).unwrap();
    }
}
struct Owner(Child);
impl Drop for Owner {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
#[ignore = "dedicated child of the write gate lifecycle tests"]
fn process_owner() {
    let root = PathBuf::from(std::env::var_os("ROCKET_WRITE_GATE_FIXTURE").expect("fixture only"));
    let gate = for_repository(&root.join("checkout 文/.git")).unwrap();
    let _permit = gate.acquire(&CancellationToken::new()).unwrap();
    fs::write(root.join("ready"), "owned").unwrap();
    let deadline = Instant::now() + Duration::from_secs(30);
    while !root.join("release").exists() {
        assert!(
            Instant::now() < deadline,
            "parent did not release fixture owner"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn process_ownership_survives_directory_move_and_is_released_after_exit_or_crash() {
    for crash in [false, true] {
        let f = Fixture::new();
        let mut owner = f.child();
        let checkout = f.0.join("checkout 文");
        let common = checkout.join(".git");
        let gate = for_repository(&common).unwrap();
        assert!(matches!(
            gate.acquire(&CancellationToken::new()),
            Err(Error::RepositoryBusy)
        ));
        let cancelled = CancellationToken::new();
        cancelled.cancel();
        assert!(matches!(gate.acquire(&cancelled), Err(Error::Cancelled)));
        let other = f.0.join("independent");
        fs::create_dir(&other).unwrap();
        let independent = for_repository(&other).unwrap();
        let _other_permit = independent.acquire(&CancellationToken::new()).unwrap();
        let moved = f.0.join("retained 文");
        // Safe submodule removal retains a whole checkout; keeping ownership must
        // not prevent that move or allow a second process through its new path.
        fs::rename(&checkout, &moved).unwrap();
        let moved_gate = for_repository(&moved.join(".git")).unwrap();
        assert!(matches!(
            moved_gate.acquire(&CancellationToken::new()),
            Err(Error::RepositoryBusy)
        ));
        if crash {
            owner.0.kill().unwrap();
        } else {
            fs::write(f.0.join("release"), "release").unwrap();
        }
        let status = owner.0.wait().unwrap();
        assert_eq!(status.success(), !crash);
        let _after_exit = moved_gate.acquire(&CancellationToken::new()).unwrap();
    }
}

#[test]
fn failed_os_acquisition_releases_local_queue_and_never_falls_back_to_unlocked_writes() {
    let f = Fixture::new();
    let common = f.0.join("checkout 文/.git");
    let gate = for_repository(&common).unwrap();
    fs::remove_dir(&common).unwrap();
    assert!(matches!(
        gate.acquire(&CancellationToken::new()),
        Err(Error::Io { .. })
    ));
    fs::create_dir(&common).unwrap();
    let _permit = gate.acquire(&CancellationToken::new()).unwrap();
}

#[cfg(unix)]
#[test]
fn ownership_refuses_redirected_lock_and_preserves_unrelated_file() {
    let f = Fixture::new();
    let common = f.0.join("checkout 文/.git");
    let target = f.0.join("unrelated");
    fs::write(&target, "keep").unwrap();
    let lock = common.join("rocket-write.lock");
    std::os::unix::fs::symlink(&target, &lock).unwrap();
    let gate = for_repository(&common).unwrap();
    assert!(matches!(
        gate.acquire(&CancellationToken::new()),
        Err(Error::Io { .. })
    ));
    assert_eq!(fs::read(&target).unwrap(), b"keep");
    fs::remove_file(&lock).unwrap();
    let _permit = gate.acquire(&CancellationToken::new()).unwrap();
}
