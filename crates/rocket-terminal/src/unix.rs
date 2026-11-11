//! Linux and macOS pseudoterminal sessions, through the C library only. The shell starts in a new
//! session whose controlling terminal is the pseudoterminal, so job control works as in
//! any terminal emulator: each job is a process group, and the kernel tells the
//! foreground group about Ctrl+C (SIGINT) and resizes (SIGWINCH).
//!
//! Ending a session hangs up every process group left in it, waits briefly, then kills
//! whatever remains, even after the shell exits by itself. That is what the Windows job
//! does for everything the shell started. A process that leaves the session itself
//! (setsid) is not followed, as in a Unix terminal; it only loses the terminal.
//!
//! OS-specific ABI, locale and process enumeration live in separate modules.
use super::*;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
use linux::*;
#[cfg(target_os = "macos")]
mod darwin;
#[cfg(target_os = "macos")]
use darwin::*;
use std::{
    collections::BTreeSet,
    ffi::{OsString, c_int, c_short, c_ulong, c_ushort},
    fs::File,
    io::{Read, Write},
    os::{
        fd::{AsRawFd, FromRawFd, OwnedFd},
        unix::process::{CommandExt, ExitStatusExt},
    },
    process::{Child, Command, Stdio},
    sync::mpsc::Receiver,
    time::Instant,
};

const POLLIN: c_short = 0x1;
const POLLOUT: c_short = 0x4;
const POLLNVAL: c_short = 0x20;
const EIO: i32 = 5;
const SIGHUP: c_int = 1;
const SIGKILL: c_int = 9;
const SIG_DFL: usize = 0;
/// How long the shell and its jobs get to exit by themselves after a hang-up, for
/// example so bash can save its history. Then they are killed.
const GRACE: Duration = Duration::from_millis(1000);
/// What the screen model answers to; xterm's other features are ignored safely.
const TERM: &str = "xterm-256color";

#[repr(C)]
struct WindowSize {
    rows: c_ushort,
    columns: c_ushort,
    width: c_ushort,
    height: c_ushort,
}
#[repr(C)]
struct PollEntry {
    fd: c_int,
    events: c_short,
    returned: c_short,
}
unsafe extern "C" {
    fn posix_openpt(flags: c_int) -> c_int;
    fn grantpt(fd: c_int) -> c_int;
    fn unlockpt(fd: c_int) -> c_int;
    fn ioctl(fd: c_int, request: c_ulong, ...) -> c_int;
    fn fcntl(fd: c_int, command: c_int, ...) -> c_int;
    fn poll(entries: *mut PollEntry, count: PollCount, milliseconds: c_int) -> c_int;
    fn tcgetattr(fd: c_int, termios: *mut Termios) -> c_int;
    fn tcsetattr(fd: c_int, when: c_int, termios: *const Termios) -> c_int;
    fn setsid() -> c_int;
    fn kill(pid: c_int, signal: c_int) -> c_int;
    fn signal(signal: c_int, handler: usize) -> usize;
}

fn checked(value: c_int) -> io::Result<c_int> {
    if value == -1 {
        Err(io::Error::last_os_error())
    } else {
        Ok(value)
    }
}

/// A new pseudoterminal: its master, nonblocking, and its other end for the shell.
/// Both are close-on-exec; the shell gets its copies as standard input and output.
fn open_terminal(columns: u16, rows: u16) -> io::Result<(File, OwnedFd)> {
    // SAFETY: plain flags; the result is a new descriptor or -1.
    let master = checked(unsafe { posix_openpt(O_RDWR | O_NOCTTY | O_CLOEXEC) })?;
    // SAFETY: `master` is the descriptor just opened and owned only here.
    let master = unsafe { OwnedFd::from_raw_fd(master) };
    let fd = master.as_raw_fd();
    // SAFETY: a live master descriptor; these only change its peer's permissions and lock.
    checked(unsafe { grantpt(fd) })?;
    checked(unsafe { unlockpt(fd) })?;
    let peer = open_peer(fd)?;
    set_size(fd, columns, rows)?;
    let mut settings = std::mem::MaybeUninit::<Termios>::uninit();
    // SAFETY: tcgetattr fills the whole struct on success, so it is read only then.
    checked(unsafe { tcgetattr(peer.as_raw_fd(), settings.as_mut_ptr()) })?;
    let mut settings = unsafe { settings.assume_init() };
    settings.input |= IUTF8;
    // SAFETY: a live terminal descriptor and the settings it just reported.
    checked(unsafe { tcsetattr(peer.as_raw_fd(), TCSANOW, &settings) })?;
    // The reader and writer wait with poll, so a stuck program can't block either.
    // SAFETY: F_GETFL/F_SETFL on a live descriptor; the flags are plain integers.
    let flags = checked(unsafe { fcntl(fd, F_GETFL) })?;
    checked(unsafe { fcntl(fd, F_SETFL, flags | O_NONBLOCK) })?;
    Ok((File::from(master), peer))
}

fn set_size(fd: c_int, columns: u16, rows: u16) -> io::Result<()> {
    let size = WindowSize {
        rows,
        columns,
        width: 0,
        height: 0,
    };
    // SAFETY: TIOCSWINSZ reads one live `struct winsize`; the kernel signals SIGWINCH.
    checked(unsafe { ioctl(fd, TIOCSWINSZ, &size as *const WindowSize) }).map(drop)
}

/// The events that are ready on `fd`, or none once `timeout` passes.
fn wait_for(fd: c_int, events: c_short, timeout: Duration) -> io::Result<c_short> {
    let mut entry = PollEntry {
        fd,
        events,
        returned: 0,
    };
    // SAFETY: one live entry, and the count says so.
    let ready = unsafe { poll(&mut entry, 1, timeout.as_millis() as c_int) };
    if ready < 0 {
        let error = io::Error::last_os_error();
        return if error.kind() == io::ErrorKind::Interrupted {
            Ok(0)
        } else {
            Err(error)
        };
    }
    if entry.returned & POLLNVAL != 0 {
        return Err(io::Error::other("Terminal descriptor is closed"));
    }
    Ok(entry.returned)
}

/// Runs in the shell's process between fork and exec, so only async-signal-safe calls.
fn child_setup() -> io::Result<()> {
    // Ignored signals survive exec. Rocket started in the background of a script ignores
    // SIGINT, and its shell would then ignore Ctrl+C, so start every signal at default.
    for number in 1..32 {
        if number != SIGKILL && number != SIGSTOP {
            // SAFETY: async-signal-safe; resetting a disposition cannot fail for these.
            unsafe { signal(number, SIG_DFL) };
        }
    }
    // A new session and process group, then this terminal as its controlling terminal
    // (standard input is already the terminal here).
    // SAFETY: both are async-signal-safe and act only on this new process.
    checked(unsafe { setsid() })?;
    checked(unsafe { ioctl(0, TIOCSCTTY, 0 as c_int) })?;
    Ok(())
}

/// What runs, with what, and how its environment differs from Rocket's.
struct Launch {
    program: PathBuf,
    arguments: Vec<String>,
    environment: Vec<(&'static str, Option<OsString>)>,
}

fn launch(
    directory: &Path,
    history: Option<&Path>,
    openssh: Option<&(String, Vec<String>)>,
    shell: &Shell,
) -> io::Result<Launch> {
    let mut environment = vec![
        ("TERM", Some(TERM.into())),
        // Inherited sizes would override the terminal's own for some programs.
        ("COLUMNS", None),
        ("LINES", None),
        ("PWD", Some(directory.as_os_str().to_owned())),
    ];
    environment.extend(utf8_locale(|name| std::env::var_os(name)));
    if let Some((tool, arguments)) = openssh {
        // The system's OpenSSH, by its fixed path. The passphrase goes to the terminal,
        // never to an askpass program; ssh-add reaches ssh-agent through SSH_AUTH_SOCK.
        environment.extend([
            ("SSH_ASKPASS", None),
            ("SSH_ASKPASS_REQUIRE", Some("never".into())),
        ]);
        return Ok(Launch {
            program: Path::new("/usr/bin").join(tool.trim_end_matches(".exe")),
            arguments: arguments.clone(),
            environment,
        });
    }
    match shell {
        // Rocket's own shell, for Session::start and MCP test windows: bash without the
        // user's startup files, with a fixed prompt that shows the folder, like the
        // Windows PowerShell one, and an owned history file when one is given.
        Shell::WindowsPowerShell => {
            environment.push(("PS1", Some("$PWD\\n> ".into())));
            if let Some(path) = history {
                if !path.is_absolute()
                    || path.to_str().is_none_or(|p| p.contains(['\0', '\r', '\n']))
                {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "Terminal history needs an absolute Unicode path without line breaks",
                    ));
                }
                // Saved after every command, so ending the session loses nothing.
                environment.push(("HISTFILE", Some(path.as_os_str().to_owned())));
                // Apple's Bash 3.2 skips `history -a` when a fresh session has no
                // older entries (bashhist.c: history_lines_this_session < where_history()).
                // This path belongs to one owned test session; normal/custom shells
                // never get this hook. Save its full history, including loaded entries.
                #[cfg(target_os = "macos")]
                environment.push(("PROMPT_COMMAND", Some("history -w".into())));
                #[cfg(not(target_os = "macos"))]
                environment.push(("PROMPT_COMMAND", Some("history -a".into())));
            }
            Ok(Launch {
                program: "/bin/bash".into(),
                arguments: vec!["--noprofile".into(), "--norc".into(), "-i".into()],
                environment,
            })
        }
        Shell::Custom { program, arguments } => {
            // This exact launch is the locally selected Mac acceptance shell. Normal
            // custom/login shells keep their own profiles, prompt and history.
            #[cfg(target_os = "macos")]
            if program == Path::new("/bin/zsh") && arguments == &["-dfi"]
                && let Some(path) = history {
                if !path.is_absolute() || path.to_str().is_none_or(|p| p.contains(['\0','\r','\n'])) {
                    return Err(io::Error::new(io::ErrorKind::InvalidInput,"Isolated zsh requires an absolute history path"));
                }
                environment.extend([
                    ("HISTFILE",Some(path.as_os_str().to_owned())),
                    ("ZDOTDIR",Some(path.parent().ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput,"History needs a parent folder"))?.as_os_str().to_owned())),
                    ("PS1",Some("%~\n> ".into())), ("RPROMPT",Some("".into())),
                    ("HISTSIZE",Some("100".into())), ("SAVEHIST",Some("100".into())),
                ]);
            }
            Ok(Launch { program: program.clone(), arguments: arguments.clone(), environment })
        },
        Shell::PowerShell(_) | Shell::GitBash(_) | Shell::CommandPrompt => Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "This shell is only on Windows. Choose another in Preferences → Terminal.",
        )),
    }
}

/// Rocket's screen reads UTF-8. With no locale set at all, a shell would run in the C
/// locale and show other text as escapes, so it gets the OS's UTF-8 locale then. A locale
/// the user set is kept, whatever it is.
fn utf8_locale(
    variable: impl Fn(&str) -> Option<OsString>,
) -> Option<(&'static str, Option<OsString>)> {
    let set = ["LC_ALL", "LC_CTYPE", "LANG"]
        .iter()
        .any(|name| variable(name).is_some_and(|value| !value.is_empty()));
    (!set).then(|| ("LC_CTYPE", Some(UTF8_LOCALE.into())))
}

/// Writes all of `bytes` unless the session stops first. A hung-up terminal takes
/// nothing more, which ends the writer the way a closed pipe does on Windows.
fn write_all(master: &File, mut bytes: &[u8], stop: &AtomicBool) -> io::Result<()> {
    while !bytes.is_empty() {
        if stop.load(Ordering::Acquire) {
            return Ok(());
        }
        if wait_for(master.as_raw_fd(), POLLOUT, POLL)? == 0 {
            continue;
        }
        match (&*master).write(bytes) {
            Ok(0) => return Err(io::Error::from(io::ErrorKind::WriteZero)),
            Ok(written) => bytes = &bytes[written..],
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                ) => {}
            Err(error) if error.raw_os_error() == Some(EIO) => return Ok(()),
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

fn exit_code(status: std::process::ExitStatus) -> Option<u32> {
    // A shell reports a signal as 128 plus its number; so does this.
    status
        .code()
        .map(|code| code as u32)
        .or_else(|| status.signal().map(|number| 128 + number as u32))
}

pub(super) fn run(
    directory: &Path,
    history: Option<&Path>,
    openssh: Option<&(String, Vec<String>)>,
    shell: &Shell,
    additions: &[(String, String)],
    held_path: Option<&Path>,
    input: Receiver<Vec<u8>>,
    shared: &Arc<Shared>,
) -> io::Result<()> {
    crate::process_privilege::check()?;
    let launch = launch(directory, history, openssh, shell)?;
    let executable = held_path.unwrap_or(&launch.program);
    // Command would look a bare name up on PATH; the shell is always named in full.
    if !executable.is_absolute() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "The shell needs a full path",
        ));
    }
    if let Err(error) = std::fs::symlink_metadata(executable)
        && error.kind() == io::ErrorKind::NotFound
    {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("{} was not found.", launch.program.display()),
        ));
    }
    let size = shared.size.load(Ordering::Acquire);
    let (master, peer) = open_terminal((size & 0xffff) as u16, (size >> 16) as u16)?;
    shared.screen.lock().unwrap().answer_device_attributes();
    let mut command = Command::new(executable);
    if held_path.is_some() {
        command.arg0(&launch.program);
    }
    command
        .args(&launch.arguments)
        .current_dir(directory)
        .stdin(Stdio::from(peer.try_clone()?))
        .stdout(Stdio::from(peer.try_clone()?))
        .stderr(Stdio::from(peer));
    for (name, value) in &launch.environment {
        match value {
            Some(value) => command.env(name, value),
            None => command.env_remove(name),
        };
    }
    command.envs(additions.iter().map(|(key,value)| (key,value)));
    // SAFETY: child_setup makes only async-signal-safe calls (see there).
    unsafe { command.pre_exec(child_setup) };
    let mut child = command.spawn()?;
    // Close this process's copies of the terminal's other end: the reader then sees the
    // end of output once every process in the session has closed it.
    drop(command);
    {
        let mut status = shared.status.lock().unwrap();
        status.pid = Some(child.id());
        status.running = true;
    }
    (shared.wake)();
    let master = Arc::new(master);
    // Set once the session has ended: whatever is still queued is read, then the reader
    // stops even if a process that left the session keeps the terminal open.
    let finished = Arc::new(AtomicBool::new(false));
    let (reply_send, reply_receive) = mpsc::sync_channel::<Vec<u8>>(32);
    let reader_state = shared.clone();
    let reader_master = master.clone();
    let reader_finished = finished.clone();
    let reader = std::thread::Builder::new()
        .name("rocket-terminal-output".into())
        .spawn(move || -> io::Result<()> {
            let mut buffer = [0u8; 8192];
            let mut drain_deadline = None;
            loop {
                if reader_finished.load(Ordering::Acquire) {
                    let deadline = drain_deadline
                        .get_or_insert_with(|| Instant::now() + Duration::from_millis(250));
                    if Instant::now() >= *deadline {
                        return Ok(());
                    }
                }
                if wait_for(reader_master.as_raw_fd(), POLLIN, POLL)? == 0 {
                    if reader_finished.load(Ordering::Acquire) {
                        return Ok(());
                    }
                    continue;
                }
                match (&*reader_master).read(&mut buffer) {
                    Ok(0) => return Ok(()),
                    Ok(read) => {
                        let replies = {
                            let mut screen = reader_state.screen.lock().unwrap();
                            screen.feed(&buffer[..read]);
                            screen.take_replies()
                        };
                        if !replies.is_empty() {
                            reply_send.try_send(replies).map_err(|_| {
                                reader_state.stop.store(true, Ordering::Release);
                                io::Error::other("Terminal reply queue unavailable")
                            })?;
                        }
                        (reader_state.wake)();
                    }
                    Err(error)
                        if matches!(
                            error.kind(),
                            io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                        ) => {}
                    // Every process in the session has closed the terminal.
                    Err(error) if error.raw_os_error() == Some(EIO) => return Ok(()),
                    Err(error) => return Err(error),
                }
            }
        });
    let writer_state = shared.clone();
    let writer_master = master.clone();
    let writer = std::thread::Builder::new()
        .name("rocket-terminal-input".into())
        .spawn(move || -> io::Result<()> {
            while !writer_state.stop.load(Ordering::Acquire) {
                while let Ok(reply) = reply_receive.try_recv() {
                    write_all(&writer_master, &reply, &writer_state.stop)?;
                }
                match input.recv_timeout(POLL) {
                    Ok(bytes) => write_all(&writer_master, &bytes, &writer_state.stop)?,
                    Err(mpsc::RecvTimeoutError::Timeout) => {}
                    Err(mpsc::RecvTimeoutError::Disconnected) => break,
                }
            }
            Ok(())
        });
    let mut exited = None;
    let mut last_size = size;
    let result = match (&reader, &writer) {
        (Ok(_), Ok(_)) => (|| {
            while !shared.stop.load(Ordering::Acquire) {
                let size = shared.size.load(Ordering::Acquire);
                if size != last_size {
                    let (columns, rows) = ((size & 0xffff) as u16, (size >> 16) as u16);
                    // As on Windows, never hold the screen lock across the resize call.
                    shared.screen.lock().unwrap().resize(columns, rows);
                    set_size(master.as_raw_fd(), columns, rows)?;
                    last_size = size;
                }
                if let Some(status) = poll_exit(&mut child)? {
                    shared.status.lock().unwrap().exit_code = exit_code(status);
                    exited = Some(status);
                    break;
                }
                std::thread::sleep(POLL);
            }
            Ok(())
        })(),
        _ => Err(io::Error::other("Terminal workers could not start")),
    };
    shared.stop.store(true, Ordering::Release);
    // Always end the session, even after the shell exits, so its background jobs can't
    // keep the terminal open or outlive it.
    let cleanup = end_session(&mut child, &mut exited);
    #[cfg(target_os = "macos")]
    if cleanup.is_err() {
        // A failed process-list query must not strand our directly owned child.
        // Keep the original diagnostic; collect the child after terminating it.
        let _ = child.kill();
        let _ = child.wait();
    }
    finished.store(true, Ordering::Release);
    let read_result = match reader {
        Ok(reader) => reader
            .join()
            .map_err(|_| io::Error::other("Terminal output worker panicked"))?,
        Err(error) => Err(error),
    };
    let write_result = match writer {
        Ok(writer) => writer
            .join()
            .map_err(|_| io::Error::other("Terminal input worker panicked"))?,
        Err(error) => Err(error),
    };
    result?;
    cleanup?;
    read_result?;
    write_result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(target_os = "linux")]
    fn stat_lines_give_state_group_and_session_even_with_odd_names() {
        assert_eq!(
            parse_stat("42 (bash) S 1 42 42 34816 42 4194560"),
            Some(('S', 42, 42))
        );
        // A program may name itself with spaces and parentheses.
        assert_eq!(
            parse_stat("77 (a (b) c) R 42 77 42 34816"),
            Some(('R', 77, 42))
        );
        assert_eq!(parse_stat("9 (zombie) Z 1 9 7"), Some(('Z', 9, 7)));
        assert_eq!(parse_stat("broken"), None);
        assert_eq!(parse_stat("1 (x) S 0"), None);
    }

    #[test]
    fn a_missing_locale_gets_utf8_characters_and_a_chosen_one_is_kept() {
        #[cfg(target_os = "linux")]
        let expected = "C.UTF-8";
        #[cfg(target_os = "macos")]
        let expected = "en_US.UTF-8";
        let none = |_: &str| None;
        assert_eq!(
            utf8_locale(none),
            Some(("LC_CTYPE", Some(expected.into())))
        );
        let empty = |_: &str| Some(OsString::new());
        assert_eq!(
            utf8_locale(empty),
            Some(("LC_CTYPE", Some(expected.into())))
        );
        for name in ["LC_ALL", "LC_CTYPE", "LANG"] {
            let chosen = |variable: &str| (variable == name).then(|| OsString::from("C"));
            assert_eq!(utf8_locale(chosen), None, "{name}");
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn isolated_zsh_preserves_normal_shell_configuration() {
        let shell = Shell::Custom { program: "/bin/zsh".into(), arguments: vec!["-dfi".into()] };
        let history = Path::new("/tmp/rocket-fixture/history");
        let isolated = launch(Path::new("/tmp/rocket-fixture"),Some(history),None,&shell).unwrap();
        assert_eq!(isolated.program,Path::new("/bin/zsh"));
        assert_eq!(isolated.arguments,["-dfi"]);
        assert!(isolated.environment.contains(&("HISTFILE",Some(history.into()))));
        assert!(isolated.environment.contains(&("ZDOTDIR",Some("/tmp/rocket-fixture".into()))));
        assert!(launch(Path::new("/tmp"),Some(Path::new("relative")),None,&shell).is_err());
        let normal = launch(Path::new("/tmp"),None,None,&shell).unwrap();
        assert!(!normal.environment.iter().any(|(key,_)|matches!(*key,"HISTFILE"|"ZDOTDIR"|"PS1"|"SAVEHIST")));
        let normal = Shell::Custom { program: "/bin/zsh".into(), arguments: vec![] };
        let normal = launch(Path::new("/tmp"),Some(history),None,&normal).unwrap();
        assert!(!normal.environment.iter().any(|(key,_)|*key=="HISTFILE"));
    }

    #[test]
    fn each_launch_names_its_program_and_environment() {
        let directory = Path::new("/tmp/rocket-launch");
        let value = |launch: &Launch, name: &str| {
            launch
                .environment
                .iter()
                .rev()
                .find(|(n, _)| *n == name)
                .map(|(_, v)| v.clone())
        };
        let own = launch(
            directory,
            Some(Path::new("/tmp/history")),
            None,
            &Shell::WindowsPowerShell,
        )
        .unwrap();
        assert_eq!(own.program, Path::new("/bin/bash"));
        assert_eq!(own.arguments, ["--noprofile", "--norc", "-i"]);
        assert_eq!(value(&own, "HISTFILE"), Some(Some("/tmp/history".into())));
        assert_eq!(value(&own, "TERM"), Some(Some(TERM.into())));
        assert_eq!(value(&own, "COLUMNS"), Some(None));
        assert_eq!(value(&own, "PWD"), Some(Some(directory.into())));
        assert!(
            launch(
                directory,
                Some(Path::new("relative")),
                None,
                &Shell::WindowsPowerShell
            )
            .is_err()
        );
        let tool = (
            "ssh-keygen.exe".to_owned(),
            vec!["-t".to_owned(), "ed25519".to_owned()],
        );
        let openssh = launch(directory, None, Some(&tool), &Shell::WindowsPowerShell).unwrap();
        assert_eq!(openssh.program, Path::new("/usr/bin/ssh-keygen"));
        assert_eq!(openssh.arguments, ["-t", "ed25519"]);
        assert_eq!(value(&openssh, "SSH_ASKPASS"), Some(None));
        assert_eq!(
            value(&openssh, "SSH_ASKPASS_REQUIRE"),
            Some(Some("never".into()))
        );
        assert_eq!(
            value(&openssh, "SSH_AUTH_SOCK"),
            None,
            "ssh-add needs the agent's socket"
        );
        let custom = Shell::Custom {
            program: "/usr/bin/zsh".into(),
            arguments: vec![],
        };
        let custom = launch(directory, None, None, &custom).unwrap();
        assert_eq!(
            (custom.program.as_path(), custom.arguments.len()),
            (Path::new("/usr/bin/zsh"), 0)
        );
        for windows in [
            Shell::CommandPrompt,
            Shell::GitBash("/x".into()),
            Shell::PowerShell("/x".into()),
        ] {
            assert_eq!(
                launch(directory, None, None, &windows)
                    .err()
                    .map(|e| e.kind()),
                Some(io::ErrorKind::Unsupported)
            );
        }
    }
}
