use rocket_git::*;
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
    sync::atomic::{AtomicU64, Ordering},
};
static NEXT: AtomicU64 = AtomicU64::new(0);

#[path = "../support/fixture_root.rs"]
mod fixture_root;
use fixture_root::base as fixture_base;

// Re-exec mutation tests with child-only settings and file-only transports.
// Other test binaries include this module without needing the isolation helper.
#[allow(dead_code)]
pub fn isolated(case_variable: &str, name: &str, test: fn()) {
    if std::env::var(case_variable).as_deref() == Ok(name) {
        test();
        return;
    }
    let f = Fixture::new();
    let mut command = Command::new(std::env::current_exe().unwrap());
    // The parent already decided to run this case. Preserve explicit opt-in to
    // ignored integration tests instead of silently discovering zero in the child.
    command.args(["--exact", name, "--nocapture", "--include-ignored"]);
    for (key, _) in std::env::vars_os() {
        let name = key.to_string_lossy().to_ascii_uppercase();
        if name.starts_with("GIT_") || name.starts_with("SSH_") || name.ends_with("_PROXY") {
            command.env_remove(key);
        }
    }
    command
        .env(case_variable, name)
        .env("GIT_CONFIG_GLOBAL", f.root.join("empty.config"))
        .env("GIT_CONFIG_SYSTEM", f.root.join("empty.config"))
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_ALLOW_PROTOCOL", "file")
        .env("GIT_TEMPLATE_DIR", f.root.join("no-hooks"))
        .env("GIT_TERMINAL_PROMPT", "0");
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000);
    }
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

/// The system's ssh-keygen, for disposable fixture keys: Windows OpenSSH, or on Linux
/// the one in /usr/bin, as Rocket uses them.
#[allow(dead_code)]
pub fn ssh_keygen() -> Command {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let mut command = Command::new(
            PathBuf::from(std::env::var_os("WINDIR").unwrap()).join("System32/OpenSSH/ssh-keygen.exe"),
        );
        command.creation_flags(0x0800_0000);
        command
    }
    #[cfg(not(windows))]
    {
        Command::new("/usr/bin/ssh-keygen")
    }
}

pub struct Fixture {
    pub root: PathBuf,
    pub repository: PathBuf,
    pub git: GitClient,
}
impl Fixture {
    pub fn new() -> Self {
        Self::with_format("sha1")
    }
    pub fn with_format(format: &str) -> Self {
        let base = fixture_base();
        fs::create_dir_all(&base).unwrap();
        // Git's config environment does not accept Rust's Windows \\?\ verbatim prefix.
        // Keep this absolute, ordinary spelling when passing paths to Git; canonicalize
        // only when comparing identities or validating the cleanup boundary.
        let unique = format!(
            "rocket-{}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        );
        let root = base.join(unique);
        fs::create_dir(&root).unwrap();
        let repository = root.join("repo with spaces 文");
        fs::create_dir(&repository).unwrap();
        fs::create_dir(root.join("no-hooks")).unwrap();
        fs::write(root.join("empty.config"), b"").unwrap();
        let fixture = Self {
            root,
            repository,
            git: GitClient::discover().unwrap(),
        };
        fixture.run(&[
            "init",
            "--initial-branch=main",
            &format!("--object-format={format}"),
            ".",
        ]);
        fixture.run(&["config", "user.name", "Rocket Test"]);
        fixture.run(&["config", "user.email", "rocket@example.invalid"]);
        fixture.run(&["config", "core.autocrlf", "false"]);
        fixture.run(&["config", "commit.gpgsign", "false"]);
        fixture.run(&[
            "config",
            "core.hooksPath",
            fixture.root.join("no-hooks").to_str().unwrap(),
        ]);
        fixture
    }
    pub fn command(&self, at: &Path, arguments: &[&str]) -> Command {
        let mut command = Command::new(self.git.executable());
        command
            .current_dir(at)
            .args(arguments)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", self.root.join("empty.config"))
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GIT_AUTHOR_DATE", "2026-01-01T00:00:00+08:00")
            .env("GIT_COMMITTER_DATE", "2026-01-01T00:00:00+08:00");
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x0800_0000);
        }
        command
    }
    pub fn output(&self, at: &Path, arguments: &[&str]) -> Output {
        self.command(at, arguments).output().unwrap()
    }
    pub fn run_at(&self, at: &Path, arguments: &[&str]) -> Vec<u8> {
        let output = self.output(at, arguments);
        assert!(
            output.status.success(),
            "{arguments:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        output.stdout
    }
    pub fn run(&self, arguments: &[&str]) -> Vec<u8> {
        self.run_at(&self.repository, arguments)
    }
    pub fn write(&self, name: &str, bytes: impl AsRef<[u8]>) {
        let path = self.repository.join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, bytes).unwrap();
    }
    pub fn commit(&self, message: &str) {
        self.run(&["add", "--all", "--"]);
        self.run(&["commit", "--no-verify", "-m", message]);
    }
    pub fn repo(&self) -> Repository {
        self.git
            .open(&self.repository, &CancellationToken::new())
            .unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let base = fixture_base().canonicalize().unwrap();
        let target = self.root.canonicalize().unwrap();
        // Recursive deletion is confined to the uniquely created direct child of the fixture root.
        assert_eq!(target.parent(), Some(base.as_path()));
        if let Err(error) = fs::remove_dir_all(&target) {
            eprintln!("fixture cleanup failed for {}: {error}", target.display());
        }
    }
}
