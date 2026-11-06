//! Bounded local failure metadata. Never records panic payloads, arguments or memory.
//!
//! Install once in a host executable, after the privilege check. The hook records
//! panics before unwinding, including caught panics; it does not change recovery.
//! Windows exceptions are best effort, not a replacement for an external debugger.
use crate::{CancellationToken, Error, Result};
use std::{
    fmt::{self, Write as _},
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    path::{Path, PathBuf},
    sync::{
        OnceLock,
        atomic::{AtomicBool, AtomicU32, Ordering},
    },
    time::{SystemTime, UNIX_EPOCH},
};

mod disk;
#[cfg(test)]
mod tests;
#[cfg(windows)]
mod windows;

pub const MAX_FILES: usize = 20;
const MAX_RECORDS: u32 = 8;
const MAX_LINE: usize = 2048;
const MAX_BYTES: u64 = (MAX_LINE as u64) * (MAX_RECORDS as u64);
const MAX_FRAMES: usize = 24;
static RUNTIME: OnceLock<Runtime> = OnceLock::new();
static STATUS: OnceLock<io::Result<PathBuf>> = OnceLock::new();
static REQUESTED_DIRECTORY: OnceLock<PathBuf> = OnceLock::new();

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Component {
    Desktop,
    Cli,
    Mcp,
}
impl Component {
    pub fn code(self) -> &'static str {
        match self {
            Self::Desktop => "desktop",
            Self::Cli => "cli",
            Self::Mcp => "mcp",
        }
    }
    fn parse(s: &str) -> Option<Self> {
        match s {
            "desktop" => Some(Self::Desktop),
            "cli" => Some(Self::Cli),
            "mcp" => Some(Self::Mcp),
            _ => None,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    RustPanic,
    WindowsException,
}
impl Kind {
    pub fn code(self) -> &'static str {
        match self {
            Self::RustPanic => "rust-panic",
            Self::WindowsException => "windows-exception",
        }
    }
    fn parse(s: &str) -> Option<Self> {
        match s {
            "rust-panic" => Some(Self::RustPanic),
            "windows-exception" => Some(Self::WindowsException),
            _ => None,
        }
    }
}
/// A code offset in a fixed known module, never a stack value or absolute address.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Frame {
    pub module: u8,
    pub offset: u64,
}
impl Frame {
    pub fn module_name(self) -> &'static str {
        match self.module {
            1 => "rocket",
            2 => "ntdll",
            3 => "kernel32",
            4 => "kernelbase",
            5 => "ucrtbase",
            6 => "opengl32",
            _ => "unknown",
        }
    }
}
#[derive(Clone, Debug)]
pub struct Report {
    pub id: String,
    pub version: String,
    pub os: &'static str,
    pub arch: &'static str,
    pub component: Component,
    pub kind: Kind,
    pub time_ms: u64,
    pub pid: u32,
    pub thread: u32,
    pub source_file: String,
    pub line: u32,
    pub column: u32,
    pub exception_code: u32,
    pub fault: Frame,
    pub frames: Vec<Frame>,
}
impl Report {
    /// Serialization is from validated typed fields, never copies raw report bytes.
    pub fn json(&self) -> String {
        let frames = self
            .frames
            .iter()
            .map(|f| {
                format!(
                    "{{\"module\":\"{}\",\"offset\":{}}}",
                    f.module_name(),
                    f.offset
                )
            })
            .collect::<Vec<_>>()
            .join(",");
        format!(
            concat!(
                "{{\"schema\":1,\"id\":\"{}\",\"version\":\"{}\",\"os\":\"{}\",\"arch\":\"{}\",\"component\":\"{}\",\"kind\":\"{}\",",
                "\"time_unix_ms\":{},\"pid\":{},\"thread\":{},\"source_file\":\"{}\",\"line\":{},\"column\":{},",
                "\"exception_code\":{},\"fault\":{{\"module\":\"{}\",\"offset\":{}}},\"frames\":[{}],",
                "\"scope\":\"Local metadata only. No panic payload, repository content, credentials or memory dump.\"}}"
            ),
            escaped(&self.id),
            escaped(&self.version),
            escaped(self.os),
            escaped(self.arch),
            self.component.code(),
            self.kind.code(),
            self.time_ms,
            self.pid,
            self.thread,
            escaped(&self.source_file),
            self.line,
            self.column,
            self.exception_code,
            self.fault.module_name(),
            self.fault.offset,
            frames
        )
    }
}
#[derive(Default, Debug)]
pub struct Snapshot {
    pub reports: Vec<Report>,
    pub damaged: usize,
}

struct Runtime {
    file: File,
    component: Component,
    pid: u32,
    writing: AtomicBool,
    attempts: AtomicU32,
    failed: AtomicBool,
    #[cfg(windows)]
    modules: windows::Modules,
}
struct Buffer {
    bytes: [u8; MAX_LINE],
    len: usize,
}
impl Buffer {
    fn new() -> Self {
        Self {
            bytes: [0; MAX_LINE],
            len: 0,
        }
    }
}
impl fmt::Write for Buffer {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        let end = self
            .len
            .checked_add(s.len())
            .filter(|n| *n <= MAX_LINE)
            .ok_or(fmt::Error)?;
        self.bytes[self.len..end].copy_from_slice(s.as_bytes());
        self.len = end;
        Ok(())
    }
}
fn millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u64::MAX as u128) as u64
}
fn source_file(s: &str) -> &str {
    let base = s.rsplit(['/', '\\']).next().unwrap_or("");
    if !base.is_empty()
        && base.len() <= 96
        && base.ends_with(".rs")
        && base
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
    {
        base
    } else {
        "-"
    }
}
fn version(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 32
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b".-".contains(&b))
}
fn escaped(s: &str) -> String {
    let mut out = String::new();
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if c < ' ' => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out
}
impl Runtime {
    fn record(
        &self,
        kind: Kind,
        location: Option<&std::panic::Location<'_>>,
        code: u32,
        fault: Frame,
    ) {
        // No logger mutex, heap allocation, symbol loading or network work here.
        // Reentrant/concurrent reports may be dropped rather than block a faulting thread.
        if self.writing.swap(true, Ordering::AcqRel) {
            return;
        }
        struct Reset<'a>(&'a AtomicBool);
        impl Drop for Reset<'_> {
            fn drop(&mut self) {
                self.0.store(false, Ordering::Release);
            }
        }
        let _reset = Reset(&self.writing);
        if self.attempts.load(Ordering::Relaxed) >= MAX_RECORDS {
            return;
        }
        self.attempts.fetch_add(1, Ordering::Relaxed);
        let mut output = Buffer::new();
        let (file, line, column) = location.map_or(("-", 0, 0), |l| {
            (source_file(l.file()), l.line(), l.column())
        });
        #[cfg(windows)]
        let thread = windows::thread_id();
        #[cfg(not(windows))]
        let thread = 0;
        let result = write!(
            output,
            "ROCKET-CRASH-1\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}:{}\t",
            env!("CARGO_PKG_VERSION"),
            self.component.code(),
            kind.code(),
            millis(),
            self.pid,
            thread,
            file,
            line,
            column,
            code,
            fault.module,
            fault.offset
        );
        if result.is_err() {
            return;
        }
        #[cfg(windows)]
        if kind == Kind::RustPanic {
            let (frames, count) = self.modules.stack();
            for (i, frame) in frames.iter().take(count).enumerate() {
                if i > 0 && output.write_str(",").is_err() {
                    return;
                }
                if write!(output, "{}:{}", frame.module, frame.offset).is_err() {
                    return;
                }
            }
        }
        if writeln!(
            output,
            "\t{}\t{}",
            std::env::consts::OS,
            std::env::consts::ARCH
        )
        .is_err()
        {
            return;
        }
        let mut file = &self.file;
        // Files are pre-opened. This is best effort on disk errors; no second panic.
        if file
            .write_all(&output.bytes[..output.len])
            .and_then(|_| self.file.sync_data())
            .is_err()
        {
            self.failed.store(true, Ordering::Relaxed);
        }
    }
}

pub fn default_directory() -> io::Result<PathBuf> {
    #[cfg(windows)]
    let directory = std::env::var_os("LOCALAPPDATA").map(|p| PathBuf::from(p).join("Rocket/crashes"));
    #[cfg(not(windows))]
    let directory = crate::local_storage::directory().map(|p| p.join("crashes"));
    directory
        .filter(|p| p.is_absolute())
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                "Local application storage is unavailable",
            )
        })
}
/// Installs one per-process recorder. A failure remains visible through `capture_error`.
/// Call in the executable, not a library worker or an in-process test suite.
pub fn install(directory: io::Result<PathBuf>, component: Component) -> io::Result<()> {
    let status = STATUS.get_or_init(|| {
        let directory = directory?;
        // Retain the intended store even if a busy lock/full disk prevents capture.
        // The viewer must not silently switch to a different profile's reports.
        if directory.is_absolute() {
            let _ = REQUESTED_DIRECTORY.set(directory.clone());
        }
        let (directory, file) = disk::prepare(&directory)?;
        let runtime = Runtime {
            file,
            component,
            pid: std::process::id(),
            writing: AtomicBool::new(false),
            attempts: AtomicU32::new(0),
            failed: AtomicBool::new(false),
            #[cfg(windows)]
            modules: windows::Modules::new(),
        };
        RUNTIME
            .set(runtime)
            .map_err(|_| io::Error::other("Crash recorder already initialized"))?;
        std::panic::set_hook(Box::new(|info| {
            if let Some(runtime) = RUNTIME.get() {
                runtime.record(Kind::RustPanic, info.location(), 0, Frame::default());
            }
            // Deliberately omit the default hook's arbitrary payload and thread name.
            // Rust's existing unwind/abort behavior continues after this hook returns.
        }));
        #[cfg(windows)]
        windows::install();
        Ok(directory)
    });
    status
        .as_ref()
        .map(|_| ())
        .map_err(|e| io::Error::new(e.kind(), "Crash capture could not initialize"))
}
pub fn directory() -> Option<&'static Path> {
    STATUS
        .get()
        .and_then(|s| s.as_ref().ok())
        .or_else(|| REQUESTED_DIRECTORY.get())
        .map(PathBuf::as_path)
}
pub fn capture_error() -> Option<io::ErrorKind> {
    if RUNTIME
        .get()
        .is_some_and(|r| r.failed.load(Ordering::Relaxed))
    {
        return Some(io::ErrorKind::Other);
    }
    STATUS
        .get()
        .and_then(|s| s.as_ref().err())
        .map(io::Error::kind)
}

fn invalid() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "A crash report is damaged or unsupported",
    )
}
fn frame(s: &str) -> io::Result<Frame> {
    let (module, offset) = s.split_once(':').ok_or_else(invalid)?;
    let value = Frame {
        module: module.parse().map_err(|_| invalid())?,
        offset: offset.parse().map_err(|_| invalid())?,
    };
    if value.module > 6
        || value.offset > u32::MAX as u64
        || (value.module == 0 && value.offset != 0)
    {
        return Err(invalid());
    }
    Ok(value)
}
fn parse(line: &str, id: String) -> io::Result<Report> {
    let fields: Vec<_> = line.split('\t').collect();
    if fields.len() != 15
        || fields[0] != "ROCKET-CRASH-1"
        || !version(fields[1])
        || !(fields[7] == "-" || source_file(fields[7]) == fields[7])
    {
        return Err(invalid());
    }
    let frames = if fields[12].is_empty() {
        vec![]
    } else {
        fields[12]
            .split(',')
            .map(frame)
            .collect::<io::Result<Vec<_>>>()?
    };
    if frames.len() > MAX_FRAMES {
        return Err(invalid());
    }
    Ok(Report {
        id,
        version: fields[1].into(),
        os: match fields[13] {
            "windows" => "windows",
            "linux" => "linux",
            "macos" => "macos",
            _ => return Err(invalid()),
        },
        arch: match fields[14] {
            "x86_64" => "x86_64",
            "aarch64" => "aarch64",
            "x86" => "x86",
            _ => return Err(invalid()),
        },
        component: Component::parse(fields[2]).ok_or_else(invalid)?,
        kind: Kind::parse(fields[3]).ok_or_else(invalid)?,
        time_ms: fields[4].parse().map_err(|_| invalid())?,
        pid: fields[5].parse().map_err(|_| invalid())?,
        thread: fields[6].parse().map_err(|_| invalid())?,
        source_file: fields[7].into(),
        line: fields[8].parse().map_err(|_| invalid())?,
        column: fields[9].parse().map_err(|_| invalid())?,
        exception_code: fields[10].parse().map_err(|_| invalid())?,
        fault: frame(fields[11])?,
        frames,
    })
}
fn io_error(source: io::Error) -> Error {
    Error::Io {
        action: "access local crash reports",
        source,
    }
}
pub fn read(directory: &Path, cancel: &CancellationToken) -> Result<Snapshot> {
    cancel.check()?;
    if !directory.try_exists().map_err(io_error)? {
        return Ok(Snapshot::default());
    }
    let _held = disk::pin_directory(directory).map_err(io_error)?;
    let paths = disk::entries(directory).map_err(io_error)?;
    if paths.len() > MAX_FILES {
        return Err(io_error(invalid()));
    }
    let mut snapshot = Snapshot::default();
    for path in paths {
        cancel.check()?;
        let result = (|| -> io::Result<()> {
            let mut file = disk::read_file(&path)?;
            if file.metadata()?.len() > MAX_BYTES {
                return Err(invalid());
            }
            let mut bytes = Vec::new();
            (&mut file).take(MAX_BYTES + 1).read_to_end(&mut bytes)?;
            if bytes.len() as u64 > MAX_BYTES {
                return Err(invalid());
            }
            let text = std::str::from_utf8(&bytes).map_err(|_| invalid())?;
            let complete = text.rsplit_once('\n').map_or("", |(s, _)| s);
            if !text.is_empty() && !text.ends_with('\n') {
                snapshot.damaged += 1;
            }
            if complete.is_empty() {
                return Ok(());
            }
            let stem = path
                .file_stem()
                .and_then(|s| s.to_str())
                .ok_or_else(invalid)?;
            for (index, line) in complete.split('\n').enumerate() {
                if index >= MAX_RECORDS as usize || line.len() > MAX_LINE {
                    return Err(invalid());
                }
                match parse(line, format!("{stem}-{}", index + 1)) {
                    Ok(report) => snapshot.reports.push(report),
                    Err(_) => snapshot.damaged += 1,
                }
            }
            Ok(())
        })();
        if result.is_err() {
            snapshot.damaged += 1;
        }
    }
    cancel.check()?;
    snapshot
        .reports
        .sort_by(|a, b| b.time_ms.cmp(&a.time_ms).then_with(|| a.id.cmp(&b.id)));
    Ok(snapshot)
}
/// Publish a new report copy only, with no overwrite and cancellation before publication.
pub fn export(report: &Report, destination: &Path, cancel: &CancellationToken) -> Result<()> {
    cancel.check()?;
    disk::export(report.json().as_bytes(), destination, || cancel.check())
}
