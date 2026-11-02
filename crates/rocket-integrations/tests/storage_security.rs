//! Real process and OS-lock regressions for the protected integration store.
use rocket_integrations::{profiles, storage};
use serde_json::json;
use std::{
    fs::{self, OpenOptions},
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

const STORE: &str = "integration-accounts.dpapi";
const LOCK: &str = "integration-accounts.lock";
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "rocket-store-security-{}-{stamp}",
            std::process::id()
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn seed(&self) -> Vec<u8> {
        let mut value = profiles::empty();
        profiles::ensure(&mut value, "test");
        profiles::connection_mut(&mut value, "test", "default")["uncertain"] = json!(true);
        storage::save(&self.0, &value, None).unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
struct Worker(Child);
impl Worker {
    fn new(directory: &Path, action: &str) -> Self {
        Self(
            Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "storage_process_worker",
                    "--ignored",
                    "--nocapture",
                ])
                .env("ROCKET_STORAGE_SECURITY_DIR", directory)
                .env("ROCKET_STORAGE_SECURITY_ACTION", action)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap(),
        )
    }
    fn wait(&mut self) {
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            if let Some(status) = self.0.try_wait().unwrap() {
                let output = std::io::read_to_string(self.0.stdout.take().unwrap()).unwrap();
                let errors = std::io::read_to_string(self.0.stderr.take().unwrap()).unwrap();
                assert!(status.success(), "worker failed: {output}\n{errors}");
                break;
            }
            assert!(Instant::now() < deadline, "storage worker timed out");
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
#[ignore = "invoked in an isolated child process by the storage security tests"]
fn storage_process_worker() {
    let directory = PathBuf::from(std::env::var_os("ROCKET_STORAGE_SECURITY_DIR").unwrap());
    match std::env::var("ROCKET_STORAGE_SECURITY_ACTION")
        .unwrap()
        .as_str()
    {
        "save-while-busy" => {
            let (mut value, raw) = storage::load(&directory).unwrap();
            profiles::ensure(&mut value, "test");
            profiles::connection_mut(&mut value, "test", "default")["uncertain"] = json!(false);
            let error = storage::save(&directory, &value, raw.as_deref())
                .expect_err("competing save succeeded");
            assert!(
                error.to_string().contains("integration_store_busy:"),
                "{error}"
            );
        }
        "save-snapshot" => {
            let id = std::env::var("ROCKET_STORAGE_SECURITY_ID").unwrap();
            let (mut value, raw) = storage::load(&directory).unwrap();
            profiles::connection_mut(&mut value, "test", "default")["name"] = json!(id);
            fs::write(directory.join(format!("ready-{id}")), b"ready").unwrap();
            let mut line = String::new();
            BufReader::new(std::io::stdin())
                .read_line(&mut line)
                .unwrap();
            let outcome = match storage::save(&directory, &value, raw.as_deref()) {
                Ok(_) => "saved",
                Err(error) => {
                    let message = error.to_string();
                    assert!(
                        message.contains("integration_store_busy:")
                            || message.contains("integration_store_changed:"),
                        "{message}"
                    );
                    "rejected"
                }
            };
            fs::write(directory.join(format!("result-{id}")), outcome).unwrap();
        }
        "hold-lock" => {
            let file = OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(false)
                .open(directory.join(LOCK))
                .unwrap();
            file.try_lock().unwrap();
            fs::write(directory.join("lock-ready"), b"ready").unwrap();
            let mut line = String::new();
            BufReader::new(std::io::stdin())
                .read_line(&mut line)
                .unwrap();
            drop(file);
        }
        other => panic!("unknown worker mode: {other}"),
    }
}

#[test]
fn competing_process_cannot_clear_the_durable_uncertainty_guard() {
    let fixture = Fixture::new();
    let raw = fixture.seed();
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(fixture.0.join(LOCK))
        .unwrap();
    lock.try_lock().unwrap();
    let mut worker = Worker::new(&fixture.0, "save-while-busy");
    worker.wait();
    assert_eq!(fs::read(fixture.0.join(STORE)).unwrap(), raw);
    assert_eq!(
        profiles::connection(&storage::load(&fixture.0).unwrap().0, "test", "default")["uncertain"],
        true
    );
    assert!(!fs::read_dir(&fixture.0).unwrap().any(|entry| {
        entry
            .unwrap()
            .path()
            .extension()
            .is_some_and(|ext| ext == "tmp")
    }));
}

#[test]
fn first_save_obeys_the_same_process_lock() {
    let fixture = Fixture::new();
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .open(fixture.0.join(LOCK))
        .unwrap();
    lock.try_lock().unwrap();
    Worker::new(&fixture.0, "save-while-busy").wait();
    assert!(!fixture.0.join(STORE).exists());
    drop(lock);
    fixture.seed();
}

#[test]
fn concurrent_processes_from_one_snapshot_have_exactly_one_winner() {
    let fixture = Fixture::new();
    fixture.seed();
    let mut workers = Vec::new();
    for index in 0..4 {
        let id = format!("writer-{index}");
        let child = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "storage_process_worker",
                "--ignored",
                "--nocapture",
            ])
            .env("ROCKET_STORAGE_SECURITY_DIR", &fixture.0)
            .env("ROCKET_STORAGE_SECURITY_ACTION", "save-snapshot")
            .env("ROCKET_STORAGE_SECURITY_ID", &id)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        workers.push((id, Worker(child)));
    }
    let deadline = Instant::now() + Duration::from_secs(15);
    for (id, worker) in &mut workers {
        while !fixture.0.join(format!("ready-{id}")).exists() {
            assert!(
                worker.0.try_wait().unwrap().is_none(),
                "writer exited early"
            );
            assert!(Instant::now() < deadline, "writer timed out");
            std::thread::sleep(Duration::from_millis(5));
        }
    }
    for (_, worker) in &mut workers {
        worker.0.stdin.take().unwrap().write_all(b"save\n").unwrap();
    }
    let mut winners = Vec::new();
    for (id, worker) in &mut workers {
        worker.wait();
        if fs::read_to_string(fixture.0.join(format!("result-{id}"))).unwrap() == "saved" {
            winners.push(id.clone());
        }
    }
    assert_eq!(winners.len(), 1);
    let (value, _) = storage::load(&fixture.0).unwrap();
    assert_eq!(
        profiles::connection(&value, "test", "default")["name"],
        winners[0]
    );
    assert_eq!(
        profiles::connection(&value, "test", "default")["uncertain"],
        true
    );
}

#[test]
fn lock_release_and_process_exit_allow_retry_but_stale_snapshots_stay_rejected() {
    let fixture = Fixture::new();
    fixture.seed();
    for crash in [false, true] {
        let ready = fixture.0.join("lock-ready");
        if ready.exists() {
            fs::remove_file(&ready).unwrap();
        }
        let mut worker = Worker::new(&fixture.0, "hold-lock");
        let deadline = Instant::now() + Duration::from_secs(15);
        while !ready.exists() {
            assert!(
                worker.0.try_wait().unwrap().is_none(),
                "lock worker exited early"
            );
            assert!(Instant::now() < deadline, "lock worker timed out");
            std::thread::sleep(Duration::from_millis(5));
        }
        let (value, raw) = storage::load(&fixture.0).unwrap();
        let error =
            storage::save(&fixture.0, &value, raw.as_deref()).expect_err("locked save succeeded");
        assert!(
            error.to_string().contains("integration_store_busy:"),
            "{error}"
        );
        if crash {
            worker.0.kill().unwrap();
            worker.0.wait().unwrap();
        } else {
            worker
                .0
                .stdin
                .take()
                .unwrap()
                .write_all(b"release\n")
                .unwrap();
            worker.wait();
        }
        let next = storage::save(&fixture.0, &value, raw.as_deref()).unwrap();
        assert!(fixture.0.join(LOCK).is_file(), "lock identity must persist");
        assert_ne!(Some(next.as_slice()), raw.as_deref());
        let error =
            storage::save(&fixture.0, &value, raw.as_deref()).expect_err("stale save succeeded");
        assert!(
            error.to_string().contains("integration_store_changed:"),
            "{error}"
        );
        assert_eq!(fs::read(fixture.0.join(STORE)).unwrap(), next);
        // An early failed save must also release the OS lock.
        storage::save(&fixture.0, &value, Some(&next)).unwrap();
    }
}

#[test]
fn invalid_lock_path_preserves_protected_settings() {
    let fixture = Fixture::new();
    let raw = fixture.seed();
    let lock = fixture.0.join(LOCK);
    if lock.exists() {
        fs::remove_file(&lock).unwrap();
    }
    fs::create_dir(&lock).unwrap();
    let (value, _) = storage::load(&fixture.0).unwrap();
    assert!(storage::save(&fixture.0, &value, Some(&raw)).is_err());
    assert_eq!(fs::read(fixture.0.join(STORE)).unwrap(), raw);
}
