//! The review digest and the eventual launch must describe the same bytes.
//! Windows keeps a deny-write/delete handle. Unix executes an owner-private copy,
//! never a source pathname that another process can replace after revalidation.
use super::{Command, invalid, io};
use rocket_git::{CancellationToken, Result, sha256_reader};
use std::{
    fs::{File, OpenOptions},
    path::{Path, PathBuf},
};

const MAX_PROGRAM: u64 = 512 * 1024 * 1024;

#[cfg(unix)]
use rocket_git::check_agent_executable_source;

fn open(path: &Path) -> Result<File> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.share_mode(1);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        // Refuse links and avoid blocking on a FIFO substituted after inspection.
        #[cfg(target_os = "linux")]
        options.custom_flags(0x20000 | 0x800); // O_NOFOLLOW | O_NONBLOCK
        #[cfg(target_os = "macos")]
        options.custom_flags(0x100 | 0x4);
    }
    let file = options
        .open(path)
        .map_err(|e| io("hold reviewed agent executable", e))?;
    let opened = file
        .metadata()
        .map_err(|e| io("inspect opened agent executable", e))?;
    if !opened.is_file() || opened.len() > MAX_PROGRAM {
        return Err(invalid(
            "Choose a regular executable no larger than 512 MiB.",
        ));
    }
    Ok(file)
}

pub struct Program {
    pub path: PathBuf,
    pub digest: String,
    // Retained by the terminal worker, including its asynchronous startup. On
    // Windows this prevents both content writes and pathname replacement.
    _file: File,
    #[cfg(unix)]
    snapshot: Snapshot,
}
impl Program {
    pub(super) fn inspect(command: &Command, cancel: &CancellationToken) -> Result<Self> {
        cancel.check()?;
        command.validate()?;
        let selected = Path::new(&command.program);
        let meta =
            std::fs::symlink_metadata(selected).map_err(|e| io("inspect agent executable", e))?;
        #[cfg(windows)]
        let redirected = {
            use std::os::windows::fs::MetadataExt;
            meta.file_attributes() & 0x400 != 0
        };
        #[cfg(not(windows))]
        let redirected = meta.file_type().is_symlink();
        if !meta.is_file() || redirected {
            return Err(invalid(
                "Choose a regular executable, not a link or directory.",
            ));
        }
        let path = selected
            .canonicalize()
            .map_err(|e| io("resolve agent executable", e))?;
        // Hold the canonical launch target, not an alias that can resolve to a
        // different file between opening it and resolving its final pathname.
        let mut file = open(&path)?;
        #[cfg(unix)]
        let (snapshot, digest) = {
            use std::os::unix::fs::MetadataExt;
            let opened = file
                .metadata()
                .map_err(|e| io("inspect opened agent executable", e))?;
            let current = std::fs::symlink_metadata(selected)
                .map_err(|e| io("inspect agent executable identity", e))?;
            if !current.is_file() || current.dev() != opened.dev() || current.ino() != opened.ino()
            {
                return Err(invalid(
                    "Executable changed during review. Review the session again.",
                ));
            }
            check_agent_executable_source(&file)?;
            let captured = Snapshot::capture(&mut file, &path, MAX_PROGRAM, cancel)?;
            // The source policy may have changed while the bytes were copied.
            check_agent_executable_source(&file)?;
            captured
        };
        #[cfg(not(unix))]
        let digest = sha256_reader(&mut file, MAX_PROGRAM, cancel)?;
        cancel.check()?;
        Ok(Self {
            path,
            digest,
            _file: file,
            #[cfg(unix)]
            snapshot,
        })
    }

    pub(super) fn recheck(&self, cancel: &CancellationToken) -> Result<()> {
        cancel.check()?;
        let mut current = open(&self.path)?;
        if sha256_reader(&mut current, MAX_PROGRAM, cancel)? != self.digest {
            return Err(invalid(
                "Executable changed after review. Review the session again.",
            ));
        }
        #[cfg(unix)]
        {
            // Check both the held source and today's path after hashing. Replacing
            // one with identical bytes must not erase a newly imposed restriction.
            check_agent_executable_source(&self._file)?;
            check_agent_executable_source(&current)?;
        }
        cancel.check()?;
        Ok(())
    }

    pub(super) fn launch_path(&self) -> &Path {
        #[cfg(unix)]
        {
            &self.snapshot.executable
        }
        #[cfg(not(unix))]
        {
            &self.path
        }
    }

    #[cfg(unix)]
    pub(super) fn check_launch_policy(&self) -> Result<()> {
        // Worktree preparation can take time after recheck. Revisit the cheap OS
        // policy checks just before handing the held snapshot to the terminal.
        check_agent_executable_source(&self._file)?;
        check_agent_executable_source(&open(&self.path)?)
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::{
        fs,
        io::{Cursor, Read},
        os::unix::fs::{MetadataExt, PermissionsExt},
    };

    struct Directory(PathBuf);
    impl Directory {
        fn new() -> Self {
            // macOS exposes its temporary directory through the /var alias.
            let parent = std::env::temp_dir().canonicalize().unwrap();
            let path = parent.join(format!(
                "rocket-agent-copy-test-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            rocket_git::local_storage::create_private_new(&path).unwrap();
            Self(path)
        }
        fn empty(&self) {
            assert_eq!(fs::read_dir(&self.0).unwrap().count(), 0);
        }
    }
    impl Drop for Directory {
        fn drop(&mut self) {
            let _ = fs::remove_dir(&self.0);
        }
    }

    #[test]
    fn snapshot_is_private_bounded_and_removed_without_touching_other_files() {
        let root = Directory::new();
        let original = root.0.join("original 文");
        fs::write(&original, b"original stays").unwrap();
        let bytes = b"reviewed executable bytes";
        let (snapshot, digest) = Snapshot::capture_in(
            &root.0,
            &mut Cursor::new(bytes),
            &original,
            100,
            &CancellationToken::new(),
        )
        .unwrap();
        assert_eq!(digest, rocket_git::sha256_bytes(bytes).unwrap());
        assert_eq!(fs::read(&snapshot.executable).unwrap(), bytes);
        assert_eq!(
            fs::metadata(&snapshot.directory).unwrap().mode() & 0o7777,
            0o700
        );
        assert_eq!(
            fs::metadata(&snapshot.executable).unwrap().mode() & 0o7777,
            0o500
        );
        assert_eq!(fs::metadata(&snapshot.executable).unwrap().nlink(), 1);
        let directory = snapshot.directory.clone();
        drop(snapshot);
        assert!(!directory.exists());
        assert_eq!(fs::read(&original).unwrap(), b"original stays");
        fs::remove_file(original).unwrap();
        root.empty();
    }

    #[test]
    fn snapshot_cancellation_and_oversize_leave_no_partial_executable() {
        struct CancelOnRead(CancellationToken);
        impl Read for CancelOnRead {
            fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
                out[..4].copy_from_slice(b"part");
                self.0.cancel();
                Ok(4)
            }
        }
        let root = Directory::new();
        let cancelled = CancellationToken::new();
        cancelled.cancel();
        assert!(
            Snapshot::capture_in(
                &root.0,
                &mut Cursor::new(b"data"),
                Path::new("agent"),
                32,
                &cancelled
            )
            .is_err()
        );
        root.empty();
        let during = CancellationToken::new();
        assert!(matches!(
            Snapshot::capture_in(
                &root.0,
                &mut CancelOnRead(during.clone()),
                Path::new("agent"),
                32,
                &during
            ),
            Err(rocket_git::Error::Cancelled)
        ));
        root.empty();
        assert!(
            Snapshot::capture_in(
                &root.0,
                &mut Cursor::new(b"larger than limit"),
                Path::new("agent"),
                4,
                &CancellationToken::new()
            )
            .is_err()
        );
        root.empty();
    }

    #[test]
    fn unsafe_snapshot_parent_and_source_links_are_refused() {
        let root = Directory::new();
        fs::set_permissions(&root.0, fs::Permissions::from_mode(0o777)).unwrap();
        let result = Snapshot::capture_in(
            &root.0,
            &mut Cursor::new(b"data"),
            Path::new("agent"),
            32,
            &CancellationToken::new(),
        );
        fs::set_permissions(&root.0, fs::Permissions::from_mode(0o700)).unwrap();
        assert!(result.is_err());
        root.empty();
        let source = root.0.join("source");
        let link = root.0.join("link");
        fs::write(&source, b"data").unwrap();
        std::os::unix::fs::symlink(&source, &link).unwrap();
        assert!(open(&link).is_err());
        assert!(open(&root.0).is_err());
        fs::remove_file(link).unwrap();
        fs::remove_file(source).unwrap();
        root.empty();
    }

    #[test]
    fn revoked_execute_permission_blocks_inspection_and_recheck() {
        let root = Directory::new();
        let source = root.0.join("agent 文");
        fs::copy(Path::new("/bin/sh").canonicalize().unwrap(), &source).unwrap();
        fs::set_permissions(&source, fs::Permissions::from_mode(0o700)).unwrap();
        let command = Command { program: source.to_str().unwrap().into(), arguments: String::new() };
        let program = Program::inspect(&command, &CancellationToken::new()).unwrap();
        let snapshot = program.launch_path().to_owned();
        program.recheck(&CancellationToken::new()).unwrap();
        fs::set_permissions(&source, fs::Permissions::from_mode(0o600)).unwrap();
        assert!(program.recheck(&CancellationToken::new()).is_err());
        assert!(Program::inspect(&command, &CancellationToken::new()).is_err());
        assert_eq!(fs::metadata(&source).unwrap().mode() & 0o777, 0o600);
        drop(program);
        assert!(!snapshot.exists());
        fs::remove_file(source).unwrap();
        root.empty();
    }

    #[cfg(target_os = "linux")]
    #[test]
    #[ignore = "needs ROCKET_AGENT_NOEXEC_TEST_ROOT on a disposable noexec mount"]
    fn noexec_mount_cannot_be_bypassed_by_the_agent_snapshot() {
        let parent = std::env::var_os("ROCKET_AGENT_NOEXEC_TEST_ROOT")
            .expect("provide a disposable noexec directory");
        let directory = PathBuf::from(parent).join(format!("agent-noexec-{}", std::process::id()));
        rocket_git::local_storage::create_private_new(&directory).unwrap();
        let source = directory.join("agent 文");
        fs::copy("/bin/true", &source).unwrap();
        fs::set_permissions(&source, fs::Permissions::from_mode(0o700)).unwrap();
        let before = fs::read(&source).unwrap();
        let direct = std::process::Command::new(&source).status();
        assert!(matches!(direct, Err(ref e) if e.kind() == std::io::ErrorKind::PermissionDenied), "fixture must really be noexec: {direct:?}");
        let result = Program::inspect(&Command { program: source.to_str().unwrap().into(), arguments: String::new() }, &CancellationToken::new());
        assert!(result.is_err(), "Rocket must not copy a noexec source into an executable directory");
        assert_eq!(fs::read(&source).unwrap(), before);
        assert_eq!(fs::metadata(&source).unwrap().mode() & 0o777, 0o700);
        fs::remove_file(source).unwrap();
        fs::remove_dir(directory).unwrap();
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn quarantine_blocks_snapshot_and_later_launch_without_changing_the_source() {
        let root = Directory::new();
        let source = root.0.join("agent 文");
        fs::copy("/bin/echo", &source).unwrap();
        fs::set_permissions(&source, fs::Permissions::from_mode(0o700)).unwrap();
        let before = fs::read(&source).unwrap();
        let command = Command { program: source.to_str().unwrap().into(), arguments: String::new() };
        let program = Program::inspect(&command, &CancellationToken::new()).unwrap();
        let snapshot = program.launch_path().to_owned();
        for marker in ["0083;00000000;Rocket security test;", "0041;00000000;approved-looking;", ""] {
            // Only Apple's installed utility and this test's disposable file.
            let set = std::process::Command::new("/usr/bin/xattr")
                .args(["-w", "com.apple.quarantine", marker]).arg(&source).output().unwrap();
            assert!(set.status.success(), "{}", String::from_utf8_lossy(&set.stderr));
            for result in [program.recheck(&CancellationToken::new()), program.check_launch_policy(), Program::inspect(&command, &CancellationToken::new()).map(|_| ())] {
                assert!(result.unwrap_err().to_string().contains("quarantine"));
            }
            let read = std::process::Command::new("/usr/bin/xattr")
                .args(["-p", "com.apple.quarantine"]).arg(&source).output().unwrap();
            assert!(read.status.success());
            assert_eq!(String::from_utf8(read.stdout).unwrap().trim_end(), marker);
        }
        assert_eq!(fs::read(&source).unwrap(), before);
        assert_eq!(fs::metadata(&source).unwrap().mode() & 0o777, 0o700);
        // The attribute still refuses the held source; Rocket never cleared it.
        assert!(check_agent_executable_source(&program._file).is_err());
        drop(program);
        assert!(!snapshot.exists());
        fs::remove_file(source).unwrap();
        root.empty();
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn enclosing_quarantine_blocks_review_recheck_and_launch_without_changing_metadata() {
        for bundle_name in ["Agent.app", "Agent.framework", "Runtime without suffix 文"] {
            let root = Directory::new();
            let bundle = root.0.join(bundle_name);
            let contents = bundle.join("Contents/MacOS");
            fs::create_dir_all(&contents).unwrap();
            let source = contents.join("agent 工具");
            fs::copy("/bin/echo", &source).unwrap();
            fs::set_permissions(&source, fs::Permissions::from_mode(0o700)).unwrap();
            let before = fs::read(&source).unwrap();
            let command = Command { program: source.to_str().unwrap().into(), arguments: String::new() };
            let program = Program::inspect(&command, &CancellationToken::new()).unwrap();
            let snapshot = program.launch_path().to_owned();
            for marker in ["0083;00000000;Rocket security test;", "0041;00000000;approved-looking;", ""] {
                let set = std::process::Command::new("/usr/bin/xattr")
                    .args(["-w", "com.apple.quarantine", marker]).arg(&bundle).output().unwrap();
                assert!(set.status.success(), "{}", String::from_utf8_lossy(&set.stderr));
                let leaf = std::process::Command::new("/usr/bin/xattr")
                    .args(["-p", "com.apple.quarantine"]).arg(&source).output().unwrap();
                assert!(!leaf.status.success(), "fixture must quarantine only the enclosing bundle");
                for result in [program.recheck(&CancellationToken::new()), program.check_launch_policy(), Program::inspect(&command, &CancellationToken::new()).map(|_| ())] {
                    let error = result.unwrap_err().to_string();
                    assert!(error.contains("quarantine") && error.contains("enclosing"), "{error}");
                }
                let read = std::process::Command::new("/usr/bin/xattr")
                    .args(["-p", "com.apple.quarantine"]).arg(&bundle).output().unwrap();
                assert!(read.status.success());
                assert_eq!(String::from_utf8(read.stdout).unwrap().trim_end(), marker);
            }
            // A same-bytes replacement at the reviewed pathname must not conceal
            // quarantine attached to the original, still-held bundle.
            let moved = root.0.join("moved bundle");
            fs::rename(&bundle, &moved).unwrap();
            fs::create_dir_all(&contents).unwrap();
            fs::write(&source, &before).unwrap();
            fs::set_permissions(&source, fs::Permissions::from_mode(0o700)).unwrap();
            assert!(program.recheck(&CancellationToken::new()).unwrap_err().to_string().contains("quarantine"));
            assert!(program.check_launch_policy().unwrap_err().to_string().contains("quarantine"));
            assert_eq!(fs::read(moved.join("Contents/MacOS/agent 工具")).unwrap(), before);
            drop(program);
            assert!(!snapshot.exists());
            fs::remove_dir_all(bundle).unwrap();
            fs::remove_dir_all(moved).unwrap();
            root.empty();
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    #[ignore = "native Gatekeeper diagnostic; requires a disposable Mac test environment"]
    fn bundle_quarantine_native_gatekeeper_probe() {
        use std::{os::unix::process::ExitStatusExt, process::Stdio, time::{Duration, Instant}};
        assert_eq!(std::env::var("ROCKET_MACOS_DISPOSABLE_GATEKEEPER_TEST").as_deref(), Ok("1"));
        let root = Directory::new();
        let bundle = root.0.join("Policy probe 文.app");
        let contents = bundle.join("Contents/MacOS");
        fs::create_dir_all(&contents).unwrap();
        let source = contents.join("probe");
        let code = root.0.join("probe.c");
        fs::write(&code, b"int main(void) { return 42; }\n").unwrap();
        let identifier = format!("dev.rocket.security.probe.{}.{}", std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos());
        fs::write(bundle.join("Contents/Info.plist"), format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?><plist version=\"1.0\"><dict><key>CFBundleIdentifier</key><string>{identifier}</string><key>CFBundleExecutable</key><string>probe</string><key>CFBundlePackageType</key><string>APPL</string></dict></plist>")).unwrap();
        let built = std::process::Command::new("/usr/bin/clang").arg(&code).arg("-o").arg(&source).output().unwrap();
        assert!(built.status.success(), "{}", String::from_utf8_lossy(&built.stderr));
        let before = fs::read(&source).unwrap();
        let run = || -> (&'static str, Option<i32>) {
            let mut child = match std::process::Command::new(&source)
                .stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn() {
                Ok(child) => child,
                Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => return ("blocked", None),
                Err(error) => panic!("native fixture could not start: {error}"),
            };
            let deadline = Instant::now() + Duration::from_secs(10);
            loop {
                if let Some(status) = child.try_wait().unwrap() {
                    let result = if status.code() == Some(42) { "allowed" }
                        else if status.signal() == Some(9) { "blocked" }
                        else { "unexpected_exit" };
                    return (result, status.code());
                }
                if Instant::now() >= deadline {
                    child.kill().unwrap(); child.wait().unwrap();
                    return ("timed_out", None);
                }
                std::thread::sleep(Duration::from_millis(10));
            }
        };
        assert_eq!(run(), ("allowed", Some(42)), "unquarantined fixture must run first");
        let command = Command { program: source.to_str().unwrap().into(), arguments: String::new() };
        let program = Program::inspect(&command, &CancellationToken::new()).unwrap();
        let set = std::process::Command::new("/usr/bin/xattr")
            .args(["-w", "com.apple.quarantine", "0083;00000000;Rocket security test;"])
            .arg(&bundle).output().unwrap();
        assert!(set.status.success());
        let leaf = std::process::Command::new("/usr/bin/xattr").args(["-p", "com.apple.quarantine"]).arg(&source).output().unwrap();
        assert!(!leaf.status.success(), "only the bundle must carry quarantine");
        let (direct, _) = run();
        // Gatekeeper behavior depends on native OS policy/cache. Record whether
        // its block was actually reproduced; a timeout is never called a block.
        // Rocket must refuse this context independently, even if macOS allows it.
        assert_ne!(direct, "unexpected_exit");
        for result in [program.check_launch_policy(), program.recheck(&CancellationToken::new()), Program::inspect(&command, &CancellationToken::new()).map(|_| ())] {
            assert!(result.unwrap_err().to_string().contains("quarantine"));
        }
        assert_eq!(fs::read(&source).unwrap(), before);
        let retained = std::process::Command::new("/usr/bin/xattr").args(["-p", "com.apple.quarantine"]).arg(&bundle).output().unwrap();
        assert!(retained.status.success());
        println!("ROCKET_GATEKEEPER_BUNDLE_PROBE={{\"baseline_exit\":42,\"direct_execution\":\"{direct}\",\"bundle_only_quarantine\":true,\"rocket_refused\":true,\"metadata_retained\":true}}");
        drop(program);
        fs::remove_dir_all(&bundle).unwrap();
        fs::remove_file(code).unwrap();
        root.empty();
    }

    #[test]
    fn native_snapshot_preserves_arguments_and_worktree_and_stop_releases_it() {
        use std::{
            sync::Arc,
            time::{Duration, Instant},
        };
        let root = Directory::new();
        let worktree = root.0.join("worktree 資料");
        rocket_git::local_storage::create_private_new(&worktree).unwrap();
        let executable = Path::new("/bin/sh").canonicalize().unwrap();
        let program = Arc::new(
            Program::inspect(
                &Command {
                    program: executable.to_string_lossy().into_owned(),
                    arguments: String::new(),
                },
                &CancellationToken::new(),
            )
            .unwrap(),
        );
        let snapshot = program.launch_path().to_owned();
        let shell = rocket_terminal::Shell::Custom {
            program: executable.clone(),
            // Fixed synthetic command; $1 must remain literal, not shell syntax.
            // sh -c consumes a separate $0; $2 is the expected full directory.
            arguments: vec![
                "-c".into(),
                // Compare the complete directory in the child, not a rendered
                // path that can wrap across terminal rows on macOS.
                "if [ \"$PWD\" = \"$2\" ]; then printf 'cwd-ok\\n'; else printf 'wrong-cwd\\n'; fi; printf 'arg=%s\\n' \"$1\"; IFS= read -r line".into(),
                "literal-argv0".into(),
                "$(touch should-not-exist); 文".into(),
                worktree.to_str().unwrap().to_owned(),
            ],
        };
        let session = rocket_terminal::Session::start_shell_held(
            &worktree,
            120,
            20,
            Arc::new(|| {}),
            &shell,
            &snapshot,
            program.clone(),
        )
        .unwrap();
        drop(program);
        let deadline = Instant::now() + Duration::from_secs(5);
        while !session.snapshot(0).unwrap().text().contains("arg=")
            && !session.status().closed
            && Instant::now() < deadline
        {
            std::thread::sleep(Duration::from_millis(10));
        }
        let text = session.snapshot(0).unwrap().text();
        assert!(text.lines().any(|line| line == "cwd-ok"), "{text}");
        assert!(text.contains("arg=$(touch should-not-exist); 文"), "{text}");
        assert!(snapshot.exists());
        assert!(!worktree.join("should-not-exist").exists());
        session.stop();
        let deadline = Instant::now() + Duration::from_secs(5);
        while !session.status().closed && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(session.status().closed, "{:?}", session.status());
        assert!(session.status().error.is_none(), "{:?}", session.status());
        assert!(!snapshot.exists(), "Stop retained the executable snapshot");
        fs::remove_dir(worktree).unwrap();
        root.empty();
    }
}

#[cfg(unix)]
struct Snapshot {
    directory: PathBuf,
    executable: PathBuf,
    device: u64,
    inode: u64,
}

#[cfg(unix)]
impl Snapshot {
    fn capture(
        source: &mut dyn std::io::Read,
        original: &Path,
        max: u64,
        cancel: &CancellationToken,
    ) -> Result<(Self, String)> {
        let root = std::env::temp_dir();
        #[cfg(target_os = "macos")]
        let root = rocket_git::local_storage::resolve_system_alias(&root)
            .map_err(|e| io("resolve agent snapshot storage", e))?;
        Self::capture_in(&root, source, original, max, cancel)
    }

    fn capture_in(
        root: &Path,
        source: &mut dyn std::io::Read,
        original: &Path,
        max: u64,
        cancel: &CancellationToken,
    ) -> Result<(Self, String)> {
        use std::{
            fs,
            io::{Read, Write},
            os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
        };
        cancel.check()?;
        let name = original
            .file_name()
            .ok_or_else(|| invalid("Agent executable has no file name."))?;
        let mut created = None;
        for _ in 0..8 {
            let mut random = [0u8; 16];
            File::open("/dev/urandom")
                .and_then(|mut f| f.read_exact(&mut random))
                .map_err(|e| io("name private agent snapshot", e))?;
            let id: String = random.iter().map(|b| format!("{b:02x}")).collect();
            let directory = root.join(format!("rocket-agent-executable-{id}"));
            match rocket_git::local_storage::create_private_new(&directory) {
                Ok(()) => {
                    created = Some(directory);
                    break;
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(io("create private agent snapshot", e)),
            }
        }
        let directory = created.ok_or_else(|| {
            invalid("Could not create a private agent snapshot. Existing files were preserved.")
        })?;
        let metadata = fs::symlink_metadata(&directory)
            .map_err(|e| io("inspect private agent snapshot", e))?;
        let snapshot = Self {
            executable: directory.join(name),
            directory,
            device: metadata.dev(),
            inode: metadata.ino(),
        };
        let mut output = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&snapshot.executable)
            .map_err(|e| io("copy reviewed agent executable", e))?;
        let mut total = 0u64;
        let mut bytes = [0u8; 64 * 1024];
        loop {
            cancel.check()?;
            let count = match source.read(&mut bytes) {
                Ok(count) => count,
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(io("read reviewed agent executable", e)),
            };
            if count == 0 {
                break;
            }
            total += count as u64;
            if total > max {
                return Err(invalid(
                    "Agent executable changed or exceeds its size limit.",
                ));
            }
            output
                .write_all(&bytes[..count])
                .map_err(|e| io("copy reviewed agent executable", e))?;
        }
        drop(output);
        // Only the owner may read/execute the verified snapshot. Never preserve
        // setuid/setgid or source ACLs; source files are not chmod'ed or changed.
        fs::set_permissions(&snapshot.executable, fs::Permissions::from_mode(0o500))
            .map_err(|e| io("protect agent executable snapshot", e))?;
        let mut verified = File::open(&snapshot.executable)
            .map_err(|e| io("verify agent executable snapshot", e))?;
        let digest = sha256_reader(&mut verified, max, cancel)?;
        cancel.check()?;
        Ok((snapshot, digest))
    }
}

#[cfg(unix)]
impl Drop for Snapshot {
    fn drop(&mut self) {
        use std::os::unix::fs::MetadataExt;
        let Ok(meta) = std::fs::symlink_metadata(&self.directory) else {
            return;
        };
        if !meta.is_dir() || meta.dev() != self.device || meta.ino() != self.inode {
            return;
        }
        // Only the one file and owned empty directory; never a recursive delete.
        let _ = std::fs::remove_file(&self.executable);
        let _ = std::fs::remove_dir(&self.directory);
    }
}
