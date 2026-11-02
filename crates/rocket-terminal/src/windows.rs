//! Windows SDK and the reviewed, bundled Microsoft ConPTY runtime.
//! The supervisor owns process/job/PTY handles. Separate threads service both
//! pipes, including while ClosePseudoConsole emits its final output frame.
use super::*;
mod environment;
mod runtime;
use runtime::Runtime;
use std::{
    ffi::{OsStr, c_void},
    fs::File,
    io::{Read, Write},
    mem::{ManuallyDrop, size_of},
    os::windows::{ffi::OsStrExt, io::FromRawHandle},
    ptr,
    sync::mpsc::Receiver,
};
type Handle = *mut c_void;
#[repr(C)]
#[derive(Clone, Copy)]
struct Coord {
    x: i16,
    y: i16,
}
#[repr(C)]
#[derive(Default)]
struct Startup {
    size: u32,
    reserved: *mut u16,
    desktop: *mut u16,
    title: *mut u16,
    x: u32,
    y: u32,
    width: u32,
    height: u32,
    columns: u32,
    rows: u32,
    fill: u32,
    flags: u32,
    show: u16,
    reserved_size: u16,
    reserved_bytes: *mut u8,
    input: Handle,
    output: Handle,
    error: Handle,
}
#[repr(C)]
struct StartupEx {
    startup: Startup,
    attributes: *mut c_void,
}
#[repr(C)]
#[derive(Default)]
struct ProcessInfo {
    process: Handle,
    thread: Handle,
    pid: u32,
    tid: u32,
}
#[repr(C)]
#[derive(Default)]
struct BasicLimits {
    process_time: i64,
    job_time: i64,
    flags: u32,
    min_working_set: usize,
    max_working_set: usize,
    active_processes: u32,
    affinity: usize,
    priority: u32,
    scheduling: u32,
}
#[repr(C)]
#[derive(Default)]
struct ExtendedLimits {
    basic: BasicLimits,
    counters: [u64; 6],
    process_memory: usize,
    job_memory: usize,
    peak_process_memory: usize,
    peak_job_memory: usize,
}
#[link(name = "kernel32")]
unsafe extern "system" {
    fn CreatePipe(read: *mut Handle, write: *mut Handle, security: *const c_void, size: u32)
    -> i32;
    fn CloseHandle(handle: Handle) -> i32;
    fn InitializeProcThreadAttributeList(
        list: *mut c_void,
        count: u32,
        flags: u32,
        size: *mut usize,
    ) -> i32;
    fn UpdateProcThreadAttribute(
        list: *mut c_void,
        flags: u32,
        attribute: usize,
        value: *mut c_void,
        size: usize,
        previous: *mut c_void,
        returned: *mut usize,
    ) -> i32;
    fn DeleteProcThreadAttributeList(list: *mut c_void);
    fn CreateProcessW(
        application: *const u16,
        command: *mut u16,
        process_attributes: *const c_void,
        thread_attributes: *const c_void,
        inherit: i32,
        flags: u32,
        environment: *const c_void,
        directory: *const u16,
        startup: *const StartupEx,
        info: *mut ProcessInfo,
    ) -> i32;
    fn TerminateProcess(process: Handle, exit_code: u32) -> i32;
    fn GetExitCodeProcess(process: Handle, exit_code: *mut u32) -> i32;
    fn WaitForSingleObject(handle: Handle, milliseconds: u32) -> u32;
    fn CreateJobObjectW(security: *const c_void, name: *const u16) -> Handle;
    fn SetInformationJobObject(job: Handle, class: i32, info: *const c_void, size: u32) -> i32;
    fn TerminateJobObject(job: Handle, exit_code: u32) -> i32;
    fn GetSystemDirectoryW(buffer: *mut u16, size: u32) -> u32;
}
struct Owned(Handle);
impl Drop for Owned {
    fn drop(&mut self) {
        // SAFETY: unique non-null kernel handle, closed once.
        unsafe {
            CloseHandle(self.0);
        }
    }
}
impl Owned {
    fn file(self) -> File {
        let handle = ManuallyDrop::new(self); // SAFETY: transfers exclusive pipe-handle ownership into File.
        unsafe { File::from_raw_handle(handle.0) }
    }
}
struct Console(Handle, Runtime);
impl Drop for Console {
    fn drop(&mut self) {
        // SAFETY: uniquely owned HPCON; output is drained by an independent reader.
        unsafe {
            (self.1.close)(self.0);
        }
    }
}
struct Process(Owned);
impl Drop for Process {
    fn drop(&mut self) {
        // SAFETY: live process handle of the shell started in its job.
        unsafe {
            TerminateProcess(self.0.0, 1);
        }
    }
}
struct Attributes(Vec<usize>);
impl Attributes {
    fn ptr(&mut self) -> *mut c_void {
        self.0.as_mut_ptr().cast()
    }
}
impl Drop for Attributes {
    fn drop(&mut self) {
        // SAFETY: allocated, initialized attribute list remains live until this call.
        unsafe {
            DeleteProcThreadAttributeList(self.ptr());
        }
    }
}
fn wide(value: &OsStr) -> io::Result<Vec<u16>> {
    let mut text: Vec<_> = value.encode_wide().collect();
    if text.contains(&0) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Path contains NUL",
        ));
    }
    text.push(0);
    Ok(text)
}
fn bool_result(value: i32) -> io::Result<()> {
    if value == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}
fn hresult(value: i32) -> io::Result<()> {
    if value < 0 {
        Err(io::Error::other(format!(
            "ConPTY failed (HRESULT 0x{:08x})",
            value as u32
        )))
    } else {
        Ok(())
    }
}
fn pipe() -> io::Result<(Owned, Owned)> {
    let (mut read, mut write) = (ptr::null_mut(), ptr::null_mut());
    // SAFETY: valid out pointers, no inheritable handles or custom security.
    bool_result(unsafe { CreatePipe(&mut read, &mut write, ptr::null(), 0) })?;
    Ok((Owned(read), Owned(write)))
}
fn system_directory() -> io::Result<std::path::PathBuf> {
    let mut system = vec![0u16; 32768];
    // SAFETY: buffer has exactly the advertised capacity.
    let length = unsafe { GetSystemDirectoryW(system.as_mut_ptr(), system.len() as u32) } as usize;
    if length == 0 || length >= system.len() {
        return Err(io::Error::last_os_error());
    }
    use std::os::windows::ffi::OsStringExt;
    Ok(std::ffi::OsString::from_wide(&system[..length]).into())
}
pub(super) fn program(shell: &Shell) -> io::Result<std::path::PathBuf> {
    Ok(match shell {
        Shell::WindowsPowerShell => {
            system_directory()?.join(r"WindowsPowerShell\v1.0\powershell.exe")
        }
        Shell::CommandPrompt => system_directory()?.join("cmd.exe"),
        Shell::PowerShell(path) | Shell::GitBash(path) | Shell::Custom { program: path, .. } => {
            path.clone()
        }
    })
}
// The shell's whole command line. The startup helper hands it over unchanged.
fn shell_command(shell: &Shell, executable: &Path, history: Option<&Path>) -> io::Result<String> {
    let Some(arguments) = shell.arguments() else {
        // Test hosts can select an owned history file before PSReadLine's first read,
        // so acceptance tests neither read nor append the user's normal history.
        let history_setup = match history {
            Some(path) => {
                let text = path.to_str().filter(|p| path.is_absolute() && !p.contains(['\0', '\r', '\n', '"']))
                    .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "Terminal history needs an absolute Unicode path without quotes or line breaks"))?;
                format!(
                    "; if (Get-Module PSReadLine) {{ Set-PSReadLineOption -HistorySavePath '{}' }}",
                    text.replace('\'', "''")
                )
            }
            None => String::new(),
        };
        // ConPTY's pipe is UTF-8, but Windows PowerShell otherwise gives .NET console
        // writes and native-command pipelines the legacy OEM/ASCII encodings. Set
        // only this owned shell's encodings before its first interactive prompt.
        // PSReadLine 2.0 loses the initial input column when a wrapped prompt widens.
        // Keep the full location on its own line and editing at a stable column.
        // Resize never injects keys into pending commands or foreground applications.
        return Ok(format!(
            "\"{}\" -NoLogo -NoProfile -NoExit -Command \"[Console]::InputEncoding = [Console]::OutputEncoding = $OutputEncoding = [Text.UTF8Encoding]::new($false); function global:prompt {{ 'PS ' + $executionContext.SessionState.Path.CurrentLocation + [Environment]::NewLine + ('>' * ($nestedPromptLevel + 1)) + ' ' }}{history_setup}\"",
            executable.display()
        ));
    };
    let mut parts = vec![quote_argument(&executable.to_string_lossy())];
    parts.extend(arguments.iter().map(|a| shell_argument(a)));
    Ok(parts.join(" "))
}
fn spawn(
    console: &Console,
    directory: &Path,
    history: Option<&Path>,
    openssh: Option<&(String, Vec<String>)>,
    shell: &Shell,
) -> io::Result<(Process, Owned, u32)> {
    crate::process_privilege::check()?;
    let executable = if let Some((tool, _)) = openssh {
        system_directory()?.join("OpenSSH").join(tool)
    } else {
        program(shell)?
    };
    if openssh.is_none()
        && let Err(error) = std::fs::symlink_metadata(&executable)
        && error.kind() == io::ErrorKind::NotFound
    {
        // The helper could say so only on the screen, and without the path.
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("{} was not found.", executable.display()),
        ));
    }
    let environment = if openssh.is_some() {
        environment::for_openssh()?
    } else if matches!(shell, Shell::GitBash(_)) {
        environment::for_git_bash()?
    } else {
        environment::for_shell()?
    };
    let mut command = wide(OsStr::new(&shell_command(shell, &executable, history)?))?;
    if let Some((_, arguments)) = openssh {
        let mut parts = vec![quote_argument(&executable.to_string_lossy())];
        parts.extend(arguments.iter().map(|a| quote_argument(a)));
        command = wide(OsStr::new(&parts.join(" ")))?;
    }
    let app = if openssh.is_none() {
        let helper = std::env::current_exe()?
            .parent()
            .ok_or_else(|| io::Error::other("Missing executable directory"))?
            .join("rocket-terminal-host.exe");
        if !helper.is_file() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "Terminal startup helper missing. Rebuild Rocket or restore rocket-terminal-host.exe beside it.",
            ));
        }
        let shell_command = String::from_utf16(&command[..command.len() - 1])
            .map_err(|_| io::Error::other("Invalid Unicode shell command"))?;
        command = wide(OsStr::new(&format!(
            "{} {} {}",
            quote_argument(&helper.to_string_lossy()),
            quote_argument(&executable.to_string_lossy()),
            quote_argument(&shell_command)
        )))?;
        wide(helper.as_os_str())?
    } else {
        wide(executable.as_os_str())?
    };
    let mut directory = wide(directory.as_os_str())?;
    // Rust canonical paths use a verbatim prefix. Windows PowerShell 5.1 treats
    // it as a provider-qualified location and many ordinary commands misbehave.
    // Remove only the standard drive/UNC prefix, without lossy path conversion.
    let prefix: Vec<u16> = "\\\\?\\".encode_utf16().collect();
    let unc: Vec<u16> = "\\\\?\\UNC\\".encode_utf16().collect();
    if directory.starts_with(&unc) {
        directory.splice(..8, [b'\\' as u16, b'\\' as u16]);
    } else if directory.starts_with(&prefix) && directory.get(5) == Some(&(b':' as u16)) {
        directory.drain(..4);
    }
    // The job exists before the child does and the child is created inside it, so
    // it is never suspended or outside its job, however this process ends.
    // SAFETY: default security, unnamed job; handle is wrapped immediately.
    let handle = unsafe { CreateJobObjectW(ptr::null(), ptr::null()) };
    if handle.is_null() {
        return Err(io::Error::last_os_error());
    }
    let job = Owned(handle);
    let mut limits = ExtendedLimits::default();
    limits.basic.flags = 0x2000; // JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE
    // SAFETY: repr(C) matches JOBOBJECT_EXTENDED_LIMIT_INFORMATION; handles are live.
    bool_result(unsafe {
        SetInformationJobObject(
            job.0,
            9,
            (&limits as *const ExtendedLimits).cast(),
            size_of::<ExtendedLimits>() as u32,
        )
    })?;
    // The job list attribute points at this array, which must outlive the list.
    let mut jobs = [job.0];
    let mut size = 0;
    // SAFETY: sizing call; null list is explicitly supported.
    unsafe {
        InitializeProcThreadAttributeList(ptr::null_mut(), 2, 0, &mut size);
    }
    if size == 0 || size > 1024 * 1024 {
        return Err(io::Error::other("Invalid process attribute size"));
    }
    let mut storage = vec![0usize; size.div_ceil(size_of::<usize>())];
    // SAFETY: properly aligned storage large enough for the returned list size.
    bool_result(unsafe {
        InitializeProcThreadAttributeList(storage.as_mut_ptr().cast(), 2, 0, &mut size)
    })?;
    let mut attributes = Attributes(storage);
    // SAFETY: pseudoconsole attribute takes the HPCON value (not a pointer to it).
    bool_result(unsafe {
        UpdateProcThreadAttribute(
            attributes.ptr(),
            0,
            0x0002_0016,
            console.0,
            size_of::<Handle>(),
            ptr::null_mut(),
            ptr::null_mut(),
        )
    })?;
    // SAFETY: PROC_THREAD_ATTRIBUTE_JOB_LIST takes an array of job handles (Windows 10
    // or later, as ConPTY already requires); `jobs` outlives the attribute list.
    bool_result(unsafe {
        UpdateProcThreadAttribute(
            attributes.ptr(),
            0,
            0x0002_000D,
            jobs.as_mut_ptr().cast(),
            size_of_val(&jobs),
            ptr::null_mut(),
            ptr::null_mut(),
        )
    })?;
    // USESTDHANDLES with null handles is essential when Rocket itself has
    // redirected stdio (including MCP). Otherwise Windows can duplicate the
    // parent's pipes into PowerShell, bypassing ConPTY and corrupting MCP stdout.
    let startup = StartupEx {
        startup: Startup {
            size: size_of::<StartupEx>() as u32,
            flags: 0x101,
            show: 0,
            ..Default::default()
        },
        attributes: attributes.ptr(),
    };
    let mut info = ProcessInfo::default();
    // SAFETY: all UTF-16 strings, `jobs` and initialized structs are live for the call.
    bool_result(unsafe {
        CreateProcessW(
            app.as_ptr(),
            command.as_mut_ptr(),
            ptr::null(),
            ptr::null(),
            0,
            0x0008_0400, // EXTENDED_STARTUPINFO_PRESENT | CREATE_UNICODE_ENVIRONMENT
            environment.as_ptr().cast(),
            directory.as_ptr(),
            &startup,
            &mut info,
        )
    })?;
    // Test seam: end this whole process the moment the child exists.
    #[cfg(test)]
    if tests::EXIT_AFTER_CREATE.load(Ordering::Acquire) {
        std::process::exit(0);
    }
    let process = Process(Owned(info.process));
    drop(Owned(info.thread)); // only the process handle is needed
    Ok((process, job, info.pid))
}

pub(super) fn run(
    directory: &Path,
    history: Option<&Path>,
    openssh: Option<&(String, Vec<String>)>,
    shell: &Shell,
    input: Receiver<Vec<u8>>,
    shared: &Arc<Shared>,
) -> io::Result<()> {
    let runtime = Runtime::load()?;
    let (input_read, input_write) = pipe()?;
    let (output_read, output_write) = pipe()?;
    let reader_state = shared.clone();
    let (reply_send, reply_receive) = mpsc::sync_channel::<Vec<u8>>(32);
    let mut output = output_read.file();
    let reader = std::thread::Builder::new()
        .name("rocket-terminal-output".into())
        .spawn(move || -> io::Result<()> {
            let mut buffer = [0u8; 8192];
            loop {
                match output.read(&mut buffer) {
                    Ok(0) => return Ok(()),
                    Ok(size) => {
                        let replies = {
                            let mut screen = reader_state.screen.lock().unwrap();
                            screen.feed(&buffer[..size]);
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
                    Err(error) if matches!(error.raw_os_error(), Some(109 | 232 | 995)) => {
                        return Ok(());
                    }
                    Err(error) => return Err(error),
                }
            }
        })?;
    let size = shared.size.load(Ordering::Acquire);
    let mut console = ptr::null_mut();
    // SAFETY: valid synchronous pipes and HPCON out pointer. No inherited cursor
    // flag, so teardown cannot wait for a cursor-query response from the host.
    hresult(unsafe {
        (runtime.create)(
            Coord {
                x: (size & 0xffff) as i16,
                y: (size >> 16) as i16,
            },
            input_read.0,
            output_write.0,
            0x08, // PSEUDOCONSOLE_GLYPH_WIDTH_GRAPHEMES; ambiguous width stays narrow.
            &mut console,
        )
    })?;
    let console = Console(console, runtime);
    let (process, job, pid) = spawn(&console, directory, history, openssh, shell)?;
    drop(input_read);
    drop(output_write);
    {
        let mut status = shared.status.lock().unwrap();
        status.pid = Some(pid);
        status.running = true;
    }
    (shared.wake)();
    let writer_state = shared.clone();
    let mut input_write = input_write.file();
    let writer = std::thread::Builder::new()
        .name("rocket-terminal-input".into())
        .spawn(move || -> io::Result<()> {
            while !writer_state.stop.load(Ordering::Acquire) {
                while let Ok(reply) = reply_receive.try_recv() {
                    input_write.write_all(&reply)?;
                }
                match input.recv_timeout(POLL) {
                    Ok(bytes) => input_write.write_all(&bytes)?,
                    Err(mpsc::RecvTimeoutError::Timeout) => {}
                    Err(mpsc::RecvTimeoutError::Disconnected) => break,
                }
            }
            Ok(())
        })?;
    let mut last_size = size;
    let result = (|| {
        loop {
            if shared.stop.load(Ordering::Acquire) {
                break;
            }
            let size = shared.size.load(Ordering::Acquire);
            if size != last_size {
                let (columns, rows) = ((size & 0xffff) as u16, (size >> 16) as u16);
                // Never hold the screen mutex across a ConPTY call: its output
                // pipe can fill, and the reader needs that mutex to keep draining.
                shared.screen.lock().unwrap().resize(columns, rows);
                hresult(unsafe {
                    (console.1.resize)(
                        console.0,
                        Coord {
                            x: columns as i16,
                            y: rows as i16,
                        },
                    )
                })?;
                last_size = size;
            }
            // SAFETY: process handle stays live throughout the supervisor loop.
            match unsafe { WaitForSingleObject(process.0.0, POLL.as_millis() as u32) } {
                0 => {
                    let mut code = 0;
                    bool_result(unsafe { GetExitCodeProcess(process.0.0, &mut code) })?;
                    shared.status.lock().unwrap().exit_code = Some(code);
                    break;
                }
                258 => {}
                _ => return Err(io::Error::last_os_error()),
            }
        }
        Ok(())
    })();
    shared.stop.store(true, Ordering::Release);
    // SAFETY: job contains only this shell and descendants. Always terminate it,
    // even after shell exit, to close inherited pipes and background children.
    let terminate = bool_result(unsafe { TerminateJobObject(job.0, 1) });
    drop(console); // Reader stays alive and drains teardown output.
    drop(process);
    drop(job);
    let read_result = reader
        .join()
        .map_err(|_| io::Error::other("Terminal output worker panicked"))?;
    let write_result = writer
        .join()
        .map_err(|_| io::Error::other("Terminal input worker panicked"))?;
    result?;
    terminate?;
    read_result?;
    if let Err(error) = write_result
        && !matches!(error.raw_os_error(), Some(109 | 232 | 995))
    {
        return Err(error);
    }
    Ok(())
}

// Windows CRT quoting: arguments never pass through a shell.
fn quote_argument(value: &str) -> String {
    let mut out = String::from("\"");
    let mut slashes = 0;
    for ch in value.chars() {
        if ch == '\\' {
            slashes += 1;
            continue;
        }
        out.extend(std::iter::repeat_n(
            '\\',
            slashes * if ch == '"' { 2 } else { 1 },
        ));
        slashes = 0;
        if ch == '"' {
            out.push('\\');
        }
        out.push(ch);
    }
    out.extend(std::iter::repeat_n('\\', slashes * 2));
    out.push('"');
    out
}

// cmd.exe reads its own command line: a quoted /K still works, but the text after it then
// keeps a stray quote. Quote only what needs it, as Command does.
fn shell_argument(value: &str) -> String {
    if value.is_empty() || value.contains([' ', '\t', '"']) {
        quote_argument(value)
    } else {
        value.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::Write,
        os::windows::io::AsRawHandle,
        process::{Command, Stdio},
        sync::atomic::AtomicBool,
        time::{Duration, Instant},
    };

    /// Set by the fixture: `spawn` ends the process as soon as the child exists.
    pub(super) static EXIT_AFTER_CREATE: AtomicBool = AtomicBool::new(false);

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn AssignProcessToJobObject(job: Handle, process: Handle) -> i32;
        fn QueryInformationJobObject(
            job: Handle,
            class: i32,
            info: *mut c_void,
            length: u32,
            returned: *mut u32,
        ) -> i32;
    }

    #[test]
    fn each_shell_gets_the_command_line_it_can_read() {
        let line = |shell: &Shell| shell_command(shell, &shell.program().unwrap(), None).unwrap();
        let bash = Shell::GitBash(r"C:\Program Files\Git\bin\bash.exe".into());
        assert_eq!(line(&bash), r#""C:\Program Files\Git\bin\bash.exe" --login -i"#);
        // A quoted /K would leave cmd.exe a stray quote in front of chcp.
        assert!(line(&Shell::CommandPrompt).ends_with(r#"cmd.exe" /d /K "chcp 65001>nul""#));
        let custom = Shell::Custom {
            program: r"D:\Tools\shell.exe".into(),
            arguments: vec!["--flag".into(), "two words".into(), String::new(), r#"say "hi""#.into()],
        };
        assert_eq!(line(&custom), r#""D:\Tools\shell.exe" --flag "two words" "" "say \"hi\"""#);
        let pwsh = Shell::PowerShell(r"C:\Program Files\PowerShell\7\pwsh.exe".into());
        assert!(line(&pwsh).starts_with(r#""C:\Program Files\PowerShell\7\pwsh.exe" -NoLogo -NoProfile -NoExit"#));
        assert!(line(&Shell::WindowsPowerShell).contains(r"\WindowsPowerShell\v1.0\powershell.exe"));
    }

    /// Processes still in `job`, including those of nested jobs.
    fn active_processes(job: &Owned) -> u32 {
        #[repr(C)]
        #[derive(Default)]
        struct Accounting {
            times: [i64; 4],
            page_faults: u32,
            total: u32,
            active: u32,
            terminated: u32,
        }
        let mut info = Accounting::default();
        // SAFETY: repr(C) matches JOBOBJECT_BASIC_ACCOUNTING_INFORMATION; live job.
        bool_result(unsafe {
            QueryInformationJobObject(
                job.0,
                1,
                (&mut info as *mut Accounting).cast(),
                size_of::<Accounting>() as u32,
                ptr::null_mut(),
            )
        })
        .unwrap();
        info.active
    }

    // A subprocess entry point, excluded from normal runs; the test below runs it.
    #[test]
    #[ignore = "subprocess fixture invoked by windows::tests"]
    fn start_fixture() {
        // Start only once the parent has put this process in its job.
        std::io::stdin().read_line(&mut String::new()).unwrap();
        EXIT_AFTER_CREATE.store(true, Ordering::Release);
        let _session =
            crate::Session::start(&std::env::temp_dir(), 80, 24, Arc::new(|| {})).unwrap();
        std::thread::sleep(Duration::from_secs(30));
        panic!("the shell was not created");
    }

    #[test]
    fn exiting_as_the_shell_is_created_leaves_nothing_running() {
        let mut fixture = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "windows::tests::start_fixture",
                "--ignored",
                "--nocapture",
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        // SAFETY: default security, unnamed job; wrapped immediately.
        let job = Owned(unsafe { CreateJobObjectW(ptr::null(), ptr::null()) });
        assert!(!job.0.is_null());
        // Ends anything the fixture left behind once this test finishes.
        let mut limits = ExtendedLimits::default();
        limits.basic.flags = 0x2000;
        // SAFETY: repr(C) matches JOBOBJECT_EXTENDED_LIMIT_INFORMATION; handles are live.
        bool_result(unsafe {
            SetInformationJobObject(
                job.0,
                9,
                (&limits as *const ExtendedLimits).cast(),
                size_of::<ExtendedLimits>() as u32,
            )
        })
        .unwrap();
        // SAFETY: both handles are live; the fixture waits for the line below.
        bool_result(unsafe { AssignProcessToJobObject(job.0, fixture.as_raw_handle()) }).unwrap();
        fixture.stdin.take().unwrap().write_all(b"go\n").unwrap();
        assert!(fixture.wait().unwrap().success());
        // Everything the fixture started is in this job too: the shell must not
        // outlive it, suspended or otherwise.
        let deadline = Instant::now() + Duration::from_secs(10);
        while active_processes(&job) > 0 {
            assert!(Instant::now() < deadline, "the shell outlived its parent");
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}
