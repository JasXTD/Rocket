//! Exercise a real second process, including OS lock release after a crash.
use rocket_git::PreviewCache;
use std::{
    fs,
    io::Read,
    path::PathBuf,
    process::{Child, Command, Stdio},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

const CHILD_ROOT: &str = "ROCKET_TEST_PREVIEW_LOCK_CHILD";

#[test]
fn preview_owner_child() {
    let Some(root) = std::env::var_os(CHILD_ROOT).map(PathBuf::from) else {
        return;
    };
    let cache = PreviewCache::open(root.join("cache")).unwrap();
    let slot = cache.reserve().unwrap();
    fs::write(slot.directory().join("payload"), b"live preview").unwrap();
    fs::write(
        root.join("ready.tmp"),
        slot.directory().as_os_str().to_str().unwrap(),
    )
    .unwrap();
    fs::rename(root.join("ready.tmp"), root.join("ready")).unwrap();
    // The parent keeps this pipe open, then kills this process to simulate a crash.
    let _ = std::io::stdin().read_exact(&mut [0u8]);
}

struct Owner {
    child: Child,
    root: PathBuf,
}
impl Drop for Owner {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        fs::remove_dir_all(&self.root).unwrap();
    }
}

#[test]
fn another_process_keeps_its_preview_until_it_exits() {
    let root = std::env::temp_dir().join(format!(
        "rocket-preview-process-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir(&root).unwrap();
    let child = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "preview_owner_child", "--nocapture"])
        .env(CHILD_ROOT, &root)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .spawn()
        .unwrap();
    let mut owner = Owner { child, root };
    let deadline = Instant::now() + Duration::from_secs(15);
    while !owner.root.join("ready").exists() {
        assert!(
            owner.child.try_wait().unwrap().is_none(),
            "preview owner exited before readiness"
        );
        assert!(
            Instant::now() < deadline,
            "preview owner did not become ready"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    let slot = PathBuf::from(fs::read_to_string(owner.root.join("ready")).unwrap());
    let cache = PreviewCache::open(owner.root.join("cache")).unwrap();
    assert_eq!(cache.remove_stale(), 0, "a live process owns this preview");
    assert_eq!(fs::read(slot.join("payload")).unwrap(), b"live preview");
    owner.child.kill().unwrap();
    owner.child.wait().unwrap();
    assert_eq!(cache.remove_stale(), 1, "a crash releases the lock");
    assert!(!slot.exists());
}
