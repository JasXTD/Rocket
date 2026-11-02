//! Per-profile scanner choice. Validation and durable writes run off the UI thread.
use super::{Rocket, scan};
use eframe::egui::Context;
use rocket_git::{CancellationToken, Error, ReadTask, Result, verify_osv_scanner};
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
};

const FILE: &str = "scanner-selection";
const LIMIT: u64 = 32 * 1024;
const MAGIC: &str = "ROCKET-OSV-1\n";

#[derive(Default)]
pub(super) struct Setup {
    pub loaded: bool,
    pub program: Option<PathBuf>,
    pub ready: bool,
    pub error: Option<String>,
    bytes: Option<Vec<u8>>,
    task: Option<ReadTask<Loaded>>,
}
struct Loaded {
    bytes: Option<Vec<u8>>,
    program: Option<PathBuf>,
    ready: bool,
    error: Option<String>,
}
impl Setup {
    pub fn running(&self) -> bool {
        self.task.is_some()
    }
    pub fn cancel(&self) {
        if let Some(task) = &self.task {
            task.cancel();
        }
    }
}
fn io(action: &'static str, source: std::io::Error) -> Error {
    Error::Io { action, source }
}
fn invalid(message: &str) -> Error {
    Error::InvalidInput(message.into())
}
fn linked(meta: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        meta.file_attributes() & 0x400 != 0
    }
    #[cfg(not(windows))]
    {
        meta.file_type().is_symlink()
    }
}
fn read(directory: &Path) -> Result<Option<Vec<u8>>> {
    let path = directory.join(FILE);
    let meta = match fs::symlink_metadata(&path) {
        Ok(meta) => meta,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(io("read scanner selection", e)),
    };
    if !meta.is_file() || linked(&meta) || meta.len() > LIMIT {
        return Err(invalid(
            "The saved scanner selection is not a normal, bounded settings file. Preserve it and check file access.",
        ));
    }
    let mut bytes = Vec::new();
    fs::File::open(&path)
        .and_then(|f| f.take(LIMIT + 1).read_to_end(&mut bytes))
        .map_err(|e| io("read scanner selection", e))?;
    if bytes.len() as u64 > LIMIT {
        return Err(invalid("The saved scanner selection is too large."));
    }
    Ok(Some(bytes))
}
fn encode(path: &Path) -> Result<Vec<u8>> {
    let text = path
        .to_str()
        .ok_or_else(|| invalid("The scanner path must be valid Unicode."))?;
    if !path.is_absolute()
        || text.chars().any(char::is_control)
        || (MAGIC.len() + text.len()) as u64 > LIMIT
    {
        return Err(invalid(
            "Choose an absolute path to the OSV-Scanner executable.",
        ));
    }
    Ok(format!("{MAGIC}{text}").into_bytes())
}
fn decode(bytes: Option<&[u8]>) -> Result<Option<PathBuf>> {
    let Some(bytes) = bytes else { return Ok(None) };
    let text = std::str::from_utf8(bytes)
        .ok()
        .and_then(|s| s.strip_prefix(MAGIC))
        .ok_or_else(|| {
            invalid("The saved scanner selection is invalid. Choose the scanner again.")
        })?;
    let path = PathBuf::from(text);
    encode(&path)?;
    Ok(Some(path))
}
fn message(error: &Error) -> String {
    match error {
        Error::InvalidInput(message) => message.clone(),
        Error::Cancelled => "Scanner setup cancelled. Choose a scanner or reload setup to try again.".into(),
        _ => "Could not read or save the scanner selection. Check file access, then choose a scanner or reload setup. Your previous selection was kept.".into(),
    }
}
fn load(
    directory: &Path,
    automatic: Option<PathBuf>,
    cancel: &CancellationToken,
) -> Result<Loaded> {
    cancel.check()?;
    let bytes = read(directory)?;
    let program = match decode(bytes.as_deref()) {
        Ok(saved) => saved.or_else(|| automatic.filter(|p| p.is_file())),
        Err(error) => {
            return Ok(Loaded {
                bytes,
                program: None,
                ready: false,
                error: Some(message(&error)),
            });
        }
    };
    let error = program
        .as_ref()
        .and_then(|p| verify_osv_scanner(p, cancel).err());
    cancel.check()?;
    let ready = program.is_some() && error.is_none();
    Ok(Loaded {
        bytes,
        program,
        ready,
        error: error.as_ref().map(message),
    })
}
fn save(
    directory: &Path,
    expected: Option<&[u8]>,
    path: &Path,
    cancel: &CancellationToken,
) -> Result<Loaded> {
    let bytes = encode(path)?;
    verify_osv_scanner(path, cancel)?;
    cancel.check()?;
    publish(directory, expected, &bytes, cancel)?;
    Ok(Loaded {
        bytes: Some(bytes),
        program: Some(path.to_path_buf()),
        ready: true,
        error: None,
    })
}
fn publish(
    directory: &Path,
    expected: Option<&[u8]>,
    bytes: &[u8],
    cancel: &CancellationToken,
) -> Result<()> {
    cancel.check()?;
    fs::create_dir_all(directory).map_err(|e| io("create scanner settings folder", e))?;
    let meta =
        fs::symlink_metadata(directory).map_err(|e| io("check scanner settings folder", e))?;
    if !meta.is_dir() || linked(&meta) {
        return Err(invalid(
            "Scanner settings must use a normal profile folder.",
        ));
    }
    let lock = directory.join("scanner-selection.lock");
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&lock)
        .map_err(|e| io("lock scanner selection", e))?;
    let result = (|| {
        if read(directory)?.as_deref() != expected {
            return Err(invalid(
                "Scanner selection changed in another window. Reload setup before choosing again.",
            ));
        }
        file.write_all(bytes)
            .and_then(|_| file.sync_all())
            .map_err(|e| io("save scanner selection", e))?;
        drop(file);
        cancel.check()?;
        if read(directory)?.as_deref() != expected {
            return Err(invalid(
                "Scanner selection changed in another window. Reload setup before choosing again.",
            ));
        }
        fs::rename(&lock, directory.join(FILE)).map_err(|e| io("publish scanner selection", e))
    })();
    if result.is_err() {
        let _ = fs::remove_file(&lock);
    }
    result
}

impl Rocket {
    pub(crate) fn choose_dependency_scanner(&mut self, path: PathBuf) {
        if !self.dependencies.editable
            || !self.dependencies.setup.loaded
            || self.dependencies.running()
            || self.quitting
        {
            return;
        }
        let directory = self.state_directory.join("dependency-security");
        let expected = self.dependencies.setup.bytes.clone();
        self.dependencies.setup.error = None;
        match crate::read_task::spawn(&self.context, move |cancel| {
            save(&directory, expected.as_deref(), &path, &cancel)
        }) {
            Ok(task) => self.dependencies.setup.task = Some(task),
            Err(error) => self.dependencies.setup.error = Some(message(&error)),
        }
    }
    pub(super) fn reload_dependency_setup(&mut self) {
        if self.dependencies.running() || self.quitting {
            return;
        }
        self.dependencies.setup.loaded = false;
        self.dependencies.setup.ready = false;
        self.dependencies.setup.error = None;
        self.dependencies.repositories.clear();
    }
    pub(super) fn poll_dependency_setup(&mut self, ctx: &Context) {
        if self.quitting {
            self.dependencies.setup.cancel();
        }
        let finished = self
            .dependencies
            .setup
            .task
            .as_mut()
            .and_then(ReadTask::try_finish);
        if let Some(result) = finished {
            self.dependencies.setup.task = None;
            self.dependencies.setup.loaded = true;
            match result {
                Ok(loaded) => {
                    self.dependencies.setup.bytes = loaded.bytes;
                    self.dependencies.setup.program = loaded.program;
                    self.dependencies.setup.ready = loaded.ready;
                    self.dependencies.setup.error = loaded.error;
                    self.dependencies.repositories.clear();
                }
                Err(error) => self.dependencies.setup.error = Some(message(&error)),
            }
            ctx.request_repaint();
        }
        if !self.dependencies.setup.loaded && !self.dependencies.running() && !self.quitting {
            let directory = self.state_directory.join("dependency-security");
            let automatic = scan::executable();
            #[cfg(test)]
            let automatic = self
                .dependencies
                .fixture_executable
                .clone()
                .unwrap_or(automatic);
            match crate::read_task::spawn(ctx, move |cancel| {
                load(&directory, Some(automatic), &cancel)
            }) {
                Ok(task) => self.dependencies.setup.task = Some(task),
                Err(error) => {
                    self.dependencies.setup.loaded = true;
                    self.dependencies.setup.error = Some(message(&error));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests;
