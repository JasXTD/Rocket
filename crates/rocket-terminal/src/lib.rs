//! A bounded terminal model and an owned, asynchronous terminal session: a Windows
//! ConPTY, or a Linux/macOS pseudoterminal. No shell input or output is persisted or sent to
//! application diagnostics.
#[path = "../../rocket-git/src/process_privilege.rs"]
mod process_privilege;
#[cfg(all(test, windows))]
mod resize_tests;
mod screen;
#[cfg(any(target_os = "linux", target_os = "macos"))]
mod unix;
#[cfg(windows)]
mod windows;
pub use screen::{Cell, Row, Screen};
#[cfg(any(windows, any(target_os = "linux", target_os = "macos")))]
use std::time::Duration;
use std::{
    io,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU32, Ordering},
        mpsc::{self, SyncSender},
    },
};
#[cfg(any(target_os = "linux", target_os = "macos"))]
use unix as platform;
#[cfg(windows)]
use windows as platform;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Status {
    /// Owned session-process ID. For a shell this is the native startup host; the
    /// interactive shell and its commands remain inside that session's job.
    pub pid: Option<u32>,
    pub running: bool,
    pub closed: bool,
    pub exit_code: Option<u32>,
    pub error: Option<String>,
}
/// What a session runs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Shell {
    /// Windows PowerShell from the system folder.
    WindowsPowerShell,
    /// PowerShell 7 at this path, started the way Windows PowerShell is.
    PowerShell(PathBuf),
    /// Git for Windows' `bin\bash.exe`, as a login shell. That launcher sets PATH and
    /// MSYSTEM for its own Git; `usr\bin\bash.exe` would run whichever Git is on PATH.
    GitBash(PathBuf),
    /// Command Prompt from the system folder.
    CommandPrompt,
    /// A program by its full path, with one item per argument: the user's own, or on
    /// Linux an installed shell, which gets none and runs interactively on the terminal.
    Custom {
        program: PathBuf,
        arguments: Vec<String>,
    },
}
impl Shell {
    /// Command Prompt can't use a network path as its current folder. It would start in
    /// the Windows folder instead, and commands would run in the wrong place.
    pub fn runs_in(&self, directory: &Path) -> bool {
        use std::path::{Component, Prefix};
        !matches!(self, Self::CommandPrompt)
            || !matches!(
                directory.components().next(),
                Some(Component::Prefix(p))
                    if matches!(p.kind(), Prefix::UNC(..) | Prefix::VerbatimUNC(..))
            )
    }
    /// The file this shell runs. Nothing is looked up on PATH.
    pub fn program(&self) -> io::Result<PathBuf> {
        #[cfg(windows)]
        {
            windows::program(self)
        }
        #[cfg(not(windows))]
        {
            match self {
                Self::Custom { program, .. } => Ok(program.clone()),
                // Windows' own shells; on Linux the terminal refuses them.
                _ => Err(io::Error::from(io::ErrorKind::Unsupported)),
            }
        }
    }
    /// The arguments it starts with, or `None` for PowerShell's startup line, which is
    /// Rocket's own.
    pub fn arguments(&self) -> Option<Vec<String>> {
        match self {
            Self::WindowsPowerShell | Self::PowerShell(_) => None,
            Self::GitBash(_) => Some(vec!["--login".into(), "-i".into()]),
            // ConPTY's pipe is UTF-8; the console's own code page would mangle other text.
            Self::CommandPrompt => Some(vec!["/d".into(), "/K".into(), "chcp 65001>nul".into()]),
            Self::Custom { arguments, .. } => Some(arguments.clone()),
        }
    }
}
struct Shared {
    screen: Mutex<Screen>,
    status: Mutex<Status>,
    stop: AtomicBool,
    size: AtomicU32,
    wake: Arc<dyn Fn() + Send + Sync>,
}
pub struct Session {
    shared: Arc<Shared>,
    input: SyncSender<Vec<u8>>,
}
impl Session {
    /// Rocket's own shell: Windows PowerShell with Rocket's startup line, or on Linux bash
    /// without the user's startup files. Returns immediately; process creation, I/O and
    /// shutdown run on workers.
    pub fn start(
        directory: &Path,
        columns: u16,
        rows: u16,
        wake: Arc<dyn Fn() + Send + Sync>,
    ) -> io::Result<Self> {
        Self::start_session(directory, columns, rows, wake, None)
    }
    /// Use a separate history file (PSReadLine's, or bash's HISTFILE) for an owned test
    /// session. Selected before the first prompt; normal sessions keep the default.
    /// Give each concurrently active macOS session its own file: Apple's Bash 3.2
    /// needs snapshot saves when starting with empty history, rather than appends.
    pub fn start_with_history(
        directory: &Path,
        columns: u16,
        rows: u16,
        wake: Arc<dyn Fn() + Send + Sync>,
        history: &Path,
    ) -> io::Result<Self> {
        Self::start_session(directory, columns, rows, wake, Some(history))
    }
    /// Runs `shell` in `directory`. Whatever can't start is reported through `status`.
    pub fn start_shell(
        directory: &Path,
        columns: u16,
        rows: u16,
        wake: Arc<dyn Fn() + Send + Sync>,
        shell: &Shell,
    ) -> io::Result<Self> {
        if let Shell::Custom { arguments, .. } = shell
            && arguments.iter().any(|v| v.contains(['\0', '\r', '\n']))
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Invalid shell arguments",
            ));
        }
        Self::start_owned(directory, columns, rows, wake, None, None, shell.clone())
    }
    /// A session that never started, for a view that says why.
    pub fn refused(message: String) -> Self {
        let (input, _) = mpsc::sync_channel(1);
        Self {
            shared: Arc::new(Shared {
                screen: Mutex::new(Screen::new(20, 4)),
                status: Mutex::new(Status {
                    closed: true,
                    error: Some(message),
                    ..Default::default()
                }),
                stop: AtomicBool::new(true),
                size: AtomicU32::new(pack(20, 4)),
                wake: Arc::new(|| {}),
            }),
            input,
        }
    }
    /// Runs the system's OpenSSH directly, without a shell, history or askpass: Windows
    /// OpenSSH, or on Linux the same tool from /usr/bin. `tool` is its Windows file name.
    pub fn start_openssh(
        directory: &Path,
        tool: &str,
        arguments: Vec<String>,
        wake: Arc<dyn Fn() + Send + Sync>,
    ) -> io::Result<Self> {
        if !matches!(tool, "ssh-keygen.exe" | "ssh-add.exe")
            || arguments.iter().any(|v| v.contains(['\0', '\r', '\n']))
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Invalid OpenSSH invocation",
            ));
        }
        Self::start_owned(
            directory,
            120,
            12,
            wake,
            None,
            Some((tool.to_owned(), arguments)),
            Shell::WindowsPowerShell,
        )
    }
    fn start_session(
        directory: &Path,
        columns: u16,
        rows: u16,
        wake: Arc<dyn Fn() + Send + Sync>,
        history: Option<&Path>,
    ) -> io::Result<Self> {
        Self::start_owned(
            directory,
            columns,
            rows,
            wake,
            history,
            None,
            Shell::WindowsPowerShell,
        )
    }
    fn start_owned(
        directory: &Path,
        columns: u16,
        rows: u16,
        wake: Arc<dyn Fn() + Send + Sync>,
        history: Option<&Path>,
        openssh: Option<(String, Vec<String>)>,
        shell: Shell,
    ) -> io::Result<Self> {
        process_privilege::check()?;
        let columns = columns.clamp(20, 240);
        let rows = rows.clamp(4, 100);
        let shared = Arc::new(Shared {
            screen: Mutex::new(Screen::new(columns, rows)),
            status: Mutex::new(Status::default()),
            stop: AtomicBool::new(false),
            size: AtomicU32::new(pack(columns, rows)),
            wake,
        });
        let (input, receive) = mpsc::sync_channel::<Vec<u8>>(32);
        #[cfg(any(windows, any(target_os = "linux", target_os = "macos")))]
        {
            let worker = shared.clone();
            let directory = directory.to_path_buf();
            let history = history.map(Path::to_path_buf);
            std::thread::Builder::new()
                .name("rocket-terminal".into())
                .spawn(move || {
                    let result = platform::run(
                        &directory,
                        history.as_deref(),
                        openssh.as_ref(),
                        &shell,
                        receive,
                        &worker,
                    );
                    let mut status = worker.status.lock().unwrap();
                    status.running = false;
                    status.closed = true;
                    if let Err(error) = result {
                        status.error = Some(format!("Terminal: {error}"));
                    }
                    drop(status);
                    (worker.wake)();
                })?;
        }
        #[cfg(not(any(windows, any(target_os = "linux", target_os = "macos"))))]
        {
            let _ = (directory, receive, history, openssh, shell, shared, input);
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "Interactive terminal currently requires Windows 10 1809 or later, or Linux/macOS.",
            ));
        }
        #[cfg(any(windows, any(target_os = "linux", target_os = "macos")))]
        Ok(Self { shared, input })
    }
    pub fn input(&self, bytes: &[u8]) -> io::Result<()> {
        if bytes.len() > 8192 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Terminal input exceeds 8 KiB; paste a smaller block.",
            ));
        }
        if self.shared.stop.load(Ordering::Acquire) || self.status().closed {
            return Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "Terminal is closed.",
            ));
        }
        self.input.try_send(bytes.to_vec()).map_err(|error| {
            io::Error::new(
                io::ErrorKind::WouldBlock,
                match error {
                    mpsc::TrySendError::Full(_) => "Terminal input is busy; try again.",
                    mpsc::TrySendError::Disconnected(_) => "Terminal input is closed.",
                },
            )
        })
    }
    pub fn resize(&self, columns: u16, rows: u16) {
        self.shared.size.store(
            pack(columns.clamp(20, 240), rows.clamp(4, 100)),
            Ordering::Release,
        );
    }
    pub fn status(&self) -> Status {
        self.shared.status.lock().unwrap().clone()
    }
    pub fn snapshot(&self, revision: u64) -> Option<Screen> {
        let screen = self.shared.screen.try_lock().ok()?;
        (screen.revision != revision).then(|| screen.clone())
    }
    pub fn clear_scrollback(&self) {
        self.shared.screen.lock().unwrap().clear_scrollback();
        (self.shared.wake)();
    }
    /// Stops only this session's owned job. Drop never waits on the UI thread.
    pub fn stop(&self) {
        self.shared.stop.store(true, Ordering::Release);
    }
}
impl Drop for Session {
    fn drop(&mut self) {
        self.stop();
    }
}
fn pack(columns: u16, rows: u16) -> u32 {
    u32::from(columns) | (u32::from(rows) << 16)
}
#[cfg(any(windows, any(target_os = "linux", target_os = "macos")))]
const POLL: Duration = Duration::from_millis(30);

#[cfg(all(test, windows))]
mod privilege_tests {
    use super::*;
    #[test]
    fn terminal_and_openssh_refuse_before_starting_workers() {
        for state in [
            Err(()),
            Ok((true, 0x2000)),
            Ok((false, 0x3000)),
            Ok((true, 0x4000)),
        ] {
            process_privilege::with_test_token(state, || {
                let directory = Path::new(r"C:\rocket-absent-privilege-fixture");
                let shell = Shell::Custom {
                    program: directory.join("absent.exe"),
                    arguments: vec![],
                };
                let denied = Session::start_shell(directory, 80, 24, Arc::new(|| {}), &shell)
                    .err()
                    .expect("must not start a session worker");
                assert_eq!(denied.kind(), io::ErrorKind::PermissionDenied);
                let denied =
                    Session::start_openssh(directory, "ssh-keygen.exe", vec![], Arc::new(|| {}))
                        .err()
                        .expect("must not start OpenSSH");
                assert_eq!(denied.kind(), io::ErrorKind::PermissionDenied);
            });
        }
    }
    #[test]
    fn command_prompt_disables_autorun_before_startup_command() {
        assert_eq!(
            Shell::CommandPrompt.arguments().unwrap(),
            ["/d", "/K", "chcp 65001>nul"]
        );
    }
}
