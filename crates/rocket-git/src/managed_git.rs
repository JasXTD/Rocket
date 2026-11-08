//! The optional, reviewed Git for Windows runtime shipped beside Rocket.
//! Verification is blocking and cancellable: call from a worker. A live lease
//! keeps verified files open without Windows write/delete sharing. Client clones
//! reuse that lease instead of hashing the runtime for every Git command, but
//! directory membership is checked again immediately before each Git launch: from
//! the volume's change journal where it has one, otherwise by reading it.
use crate::{CancellationToken, Error, Result, advisory_tool::digest};
use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::OsString,
    fs::{self, File, OpenOptions},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, OnceLock, Weak,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    },
};

pub const MANAGED_GIT_VERSION: &str = "2.56.0.windows.4";
pub const MANAGED_GIT_MANIFEST_SHA256: &str =
    "94395c3c041d75fd66b5d768844e8545fc35010e368537aaede718eaf4e7ee96";
const MANIFEST: &str = include_str!("managed_git.manifest");
const MAX_FILES: usize = 20_000;
const MAX_BYTES: u64 = 2 * 1024 * 1024 * 1024;

#[cfg(windows)]
mod change_journal;
#[cfg(not(windows))]
mod change_journal {
    #[derive(Default)]
    pub(super) struct ChangeJournal;
    impl ChangeJournal {
        pub(super) fn stale(&mut self) -> Option<u64> {
            Some(0)
        }
        pub(super) fn read(&mut self, _: u64) {}
    }
}
#[cfg(all(test, windows))]
mod security_tests;
use change_journal::ChangeJournal;

#[derive(Clone, Default, Debug)]
pub struct ManagedGitProgress(Arc<Progress>);
#[derive(Default, Debug)]
struct Progress {
    checked: AtomicUsize,
    files: AtomicUsize,
    bytes: AtomicU64,
    total: AtomicU64,
}
impl ManagedGitProgress {
    pub fn snapshot(&self) -> (usize, usize, u64, u64) {
        (
            self.0.checked.load(Ordering::Relaxed),
            self.0.files.load(Ordering::Relaxed),
            self.0.bytes.load(Ordering::Relaxed),
            self.0.total.load(Ordering::Relaxed),
        )
    }
    fn begin(&self, files: usize, total: u64) {
        self.0.checked.store(0, Ordering::Relaxed);
        self.0.files.store(files, Ordering::Relaxed);
        self.0.bytes.store(0, Ordering::Relaxed);
        self.0.total.store(total, Ordering::Relaxed);
    }
}

pub struct ManagedGitRuntime {
    root: PathBuf,
    // Dropping the last client closes these handles and permits replacement.
    files: Vec<File>,
    directories: Vec<File>,
    // Exact child names and their directory bit, captured under the directory
    // handles. Per-directory lookup avoids rebuilding full UTF-8 paths per launch.
    directory_entries: BTreeMap<PathBuf, BTreeMap<OsString, bool>>,
    journal: Mutex<ChangeJournal>,
    #[cfg(test)]
    full_reads: AtomicUsize,
}
impl std::fmt::Debug for ManagedGitRuntime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ManagedGitRuntime")
            .field("version", &MANAGED_GIT_VERSION)
            .field("verified_files", &self.files.len())
            .finish()
    }
}
impl ManagedGitRuntime {
    pub fn executable(&self) -> PathBuf {
        self.root.join("cmd/git.exe")
    }

    /// File locks preserve verified bytes, but do not prevent adding new directory
    /// entries. Check the exact retained inventory before reusing or launching it,
    /// unless the volume's journal shows nothing since it was last read in full.
    pub(crate) fn revalidate(&self, cancel: &CancellationToken) -> Result<()> {
        cancel.check()?;
        normal_directory(&self.root)?;
        let Some(stale) = self.journal()?.stale() else {
            return cancel.check();
        };
        // Do not hold the lock during cancellable filesystem I/O.
        self.read_directories(cancel)?;
        self.journal()?.read(stale);
        Ok(())
    }

    fn journal(&self) -> Result<std::sync::MutexGuard<'_, ChangeJournal>> {
        self.journal
            .lock()
            .map_err(|_| invalid("Verification state is unavailable; restart Rocket"))
    }

    fn read_directories(&self, cancel: &CancellationToken) -> Result<()> {
        #[cfg(test)]
        self.full_reads.fetch_add(1, Ordering::Relaxed);
        // A listing shows a new reparse point only once the handle that set it closes,
        // and the journal has nothing more to say then. The held handles show it at once.
        for directory in &self.directories {
            cancel.check()?;
            let meta = directory
                .metadata()
                .map_err(|e| Error::io("recheck bundled Git directory", e))?;
            if linked(&meta) {
                return Err(invalid("The runtime contains a link or reparse point"));
            }
        }
        for (directory, expected) in &self.directory_entries {
            cancel.check()?;
            let mut seen = 0usize;
            for item in fs::read_dir(directory)
                .map_err(|e| Error::io("recheck bundled Git directory", e))?
            {
                cancel.check()?;
                let item = item.map_err(|e| Error::io("recheck bundled Git entry", e))?;
                // DirEntry metadata does not follow links and is supplied by the
                // Windows enumeration, avoiding a separate stat of every file.
                let meta = item
                    .metadata()
                    .map_err(|e| Error::io("recheck bundled Git file", e))?;
                if linked(&meta) {
                    return Err(invalid("The runtime contains a link or reparse point"));
                }
                if (!meta.is_file() && !meta.is_dir())
                    || expected.get(&item.file_name()) != Some(&meta.is_dir())
                {
                    return Err(invalid(
                        "Runtime contents changed after verification. Restore the complete reviewed bundle",
                    ));
                }
                seen += 1;
                if seen > expected.len() {
                    return Err(invalid("Runtime contents changed after verification"));
                }
            }
            if seen != expected.len() {
                return Err(invalid(
                    "A verified runtime entry is missing. Restore the complete reviewed bundle",
                ));
            }
        }
        cancel.check()
    }
}

/// Resolve only a runtime beside the current app, independent of drive letters,
/// the working directory, PATH and repository configuration.
pub fn managed_git_path() -> Result<PathBuf> {
    let app = std::env::current_exe().map_err(|e| Error::io("locate Rocket for bundled Git", e))?;
    Ok(app
        .parent()
        .ok_or_else(|| invalid("Rocket has no installation folder"))?
        .join("git")
        .join(MANAGED_GIT_VERSION)
        .join("cmd/git.exe"))
}

/// A packaged runtime takes priority over PATH. A published inventory also counts
/// as a bundle: a missing payload must report damage, not silently switch Git.
pub(crate) fn candidate() -> Result<Option<PathBuf>> {
    if !cfg!(all(windows, target_arch = "x86_64")) {
        return Ok(None);
    }
    let executable = managed_git_path()?;
    let root = executable.parent().unwrap().parent().unwrap();
    candidate_in(root.parent().unwrap())
}
fn candidate_in(directory: &Path) -> Result<Option<PathBuf>> {
    let root = directory.join(MANAGED_GIT_VERSION);
    for path in [&root, &directory.join("manifest.tsv")] {
        match fs::symlink_metadata(path) {
            Ok(_) => return Ok(Some(root.join("cmd/git.exe"))),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(Error::io("locate bundled Git", error)),
        }
    }
    Ok(None)
}

/// Force a full check for Preferences → Verify files. This never executes Git.
pub fn verify_managed_git(
    cancel: &CancellationToken,
    progress: &ManagedGitProgress,
) -> Result<Arc<ManagedGitRuntime>> {
    let path = managed_git_path()?;
    verify_root(
        path.parent().unwrap().parent().unwrap(),
        cancel,
        progress,
        false,
        None,
    )
}

/// Called before the initial --version probe, including explicit/custom paths.
/// Other installed Git copies retain their existing verification policy.
pub(crate) fn for_executable(
    path: &Path,
    cancel: &CancellationToken,
    log: Option<&crate::DiagnosticLog>,
) -> Result<Option<Arc<ManagedGitRuntime>>> {
    let Some(root) = runtime_root(path) else {
        return Ok(None);
    };
    let verify = || {
        let runtime = verify_root(root, cancel, &ManagedGitProgress::default(), true, log)?;
        let actual = path
            .canonicalize()
            .map_err(|e| Error::io("resolve bundled Git executable", e))?;
        if !actual.starts_with(&runtime.root) {
            return Err(invalid(
                "The executable leaves the reviewed runtime directory",
            ));
        }
        Ok(runtime)
    };
    match log {
        Some(log) => log.step("managed-git-verify", "verify_runtime", verify),
        None => verify(),
    }
    .map(Some)
}
fn runtime_root(path: &Path) -> Option<&Path> {
    let named = |p: &Path, expected: &str| {
        p.file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.eq_ignore_ascii_case(expected))
    };
    if !named(path, "git.exe") {
        return None;
    }
    // Portable Git also exposes bin/git.exe and ucrt64/bin/git.exe (older
    // distributions used mingw64). All receive the same inventory protection.
    // A custom selection of either must receive the same inventory protection.
    path.ancestors().skip(1).find(|root| {
        named(root, MANAGED_GIT_VERSION) && root.parent().is_some_and(|parent| named(parent, "git"))
    })
}
fn leases() -> &'static Mutex<Vec<Weak<ManagedGitRuntime>>> {
    static VALUE: OnceLock<Mutex<Vec<Weak<ManagedGitRuntime>>>> = OnceLock::new();
    VALUE.get_or_init(|| Mutex::new(Vec::new()))
}
fn verify_root(
    root: &Path,
    cancel: &CancellationToken,
    progress: &ManagedGitProgress,
    cached: bool,
    log: Option<&crate::DiagnosticLog>,
) -> Result<Arc<ManagedGitRuntime>> {
    cancel.check()?;
    if !cfg!(all(windows, target_arch = "x86_64")) {
        return Err(Error::Unsupported(
            "Bundled Git is available in Windows x64 builds".into(),
        ));
    }
    normal_directory(root)?;
    let root = root
        .canonicalize()
        .map_err(|e| Error::io("resolve bundled Git", e))?;
    if cached {
        let cached_lease = {
            let mut leases = leases()
                .lock()
                .map_err(|_| invalid("Verification state is unavailable; restart Rocket"))?;
            leases.retain(|lease| lease.strong_count() > 0);
            leases
                .iter()
                .filter_map(Weak::upgrade)
                .find(|lease| lease.root == root)
        };
        if let Some(lease) = cached_lease {
            // Do not hold the global cache mutex during cancellable filesystem I/O.
            lease.revalidate(cancel)?;
            return Ok(lease);
        }
    }
    if crate::sha256_bytes(MANIFEST.as_bytes())? != MANAGED_GIT_MANIFEST_SHA256 {
        return Err(invalid(
            "The compiled inventory does not match its reviewed digest",
        ));
    }
    let entries = inventory(MANIFEST)?;
    let runtime = Arc::new(verify_inventory_logged(
        &root, &entries, cancel, progress, log,
    )?);
    cancel.check()?;
    let mut leases = leases()
        .lock()
        .map_err(|_| invalid("Verification state is unavailable; restart Rocket"))?;
    leases.retain(|lease| lease.strong_count() > 0);
    leases.push(Arc::downgrade(&runtime));
    Ok(runtime)
}

struct Entry<'a> {
    path: &'a str,
    hash: &'a str,
    bytes: u64,
}
fn inventory(text: &str) -> Result<BTreeMap<String, Entry<'_>>> {
    let mut lines = text.lines();
    if lines.next() != Some("ROCKET-MANAGED-GIT-1\t2.56.0.windows.4") {
        return Err(invalid("The compiled runtime inventory is invalid"));
    }
    let mut entries = BTreeMap::new();
    let mut total = 0u64;
    for line in lines {
        let mut fields = line.split('\t');
        let hash = fields.next().unwrap_or("");
        let bytes = fields.next().and_then(|v| v.parse::<u64>().ok());
        let path = fields.next().unwrap_or("");
        if fields.next().is_some()
            || hash.len() != 64
            || !hash.bytes().all(|b| b.is_ascii_hexdigit())
            || path.is_empty()
            || path.contains(['\\', ':'])
            || path.chars().any(char::is_control)
            || path
                .split('/')
                .any(|part| matches!(part, "" | "." | "..") || part.ends_with(['.', ' ']))
        {
            return Err(invalid(
                "The compiled runtime inventory contains an invalid entry",
            ));
        }
        let bytes = bytes.ok_or_else(|| invalid("The runtime inventory has an invalid size"))?;
        total = total
            .checked_add(bytes)
            .filter(|n| *n <= MAX_BYTES)
            .ok_or_else(|| invalid("The runtime inventory exceeds its size limit"))?;
        if entries
            .insert(path.to_ascii_lowercase(), Entry { path, hash, bytes })
            .is_some()
            || entries.len() > MAX_FILES
        {
            return Err(invalid(
                "The runtime inventory contains duplicate or too many files",
            ));
        }
    }
    if !entries.contains_key("cmd/git.exe") {
        return Err(invalid("This build has no reviewed bundled Git inventory"));
    }
    Ok(entries)
}
fn invalid(message: &str) -> Error {
    Error::InvalidInput(format!("Bundled Git: {message}"))
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
fn normal_directory(path: &Path) -> Result<()> {
    let meta = fs::symlink_metadata(path).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            invalid("Not included in this build. Use installed Git or a complete Rocket bundle")
        } else {
            Error::io("inspect bundled Git directory", e)
        }
    })?;
    if linked(&meta) || !meta.is_dir() {
        return Err(invalid(
            "The runtime must be a normal directory, without links",
        ));
    }
    Ok(())
}
#[cfg(test)]
fn verify_inventory(
    root: &Path,
    entries: &BTreeMap<String, Entry<'_>>,
    cancel: &CancellationToken,
    progress: &ManagedGitProgress,
) -> Result<ManagedGitRuntime> {
    verify_inventory_logged(root, entries, cancel, progress, None)
}
fn verify_inventory_logged(
    root: &Path,
    entries: &BTreeMap<String, Entry<'_>>,
    cancel: &CancellationToken,
    progress: &ManagedGitProgress,
    log: Option<&crate::DiagnosticLog>,
) -> Result<ManagedGitRuntime> {
    let inventory_started = std::time::Instant::now();
    cancel.check()?;
    normal_directory(root)?;
    progress.begin(entries.len(), entries.values().map(|e| e.bytes).sum());
    let mut seen = BTreeSet::new();
    let mut directories = vec![root.to_owned()];
    // Only Windows holds the directories open and reads the volume's change journal.
    #[cfg(windows)]
    let (mut directory_guards, mut journal) = (Vec::new(), ChangeJournal::default());
    #[cfg(not(windows))]
    let (directory_guards, journal) = (Vec::new(), ChangeJournal::default());
    let mut directory_entries = BTreeMap::new();
    let mut visited = 0usize;
    while let Some(directory) = directories.pop() {
        cancel.check()?;
        normal_directory(&directory)?;
        let mut children = BTreeMap::new();
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            let file = OpenOptions::new()
                .read(true)
                .share_mode(3)
                .custom_flags(0x0220_0000)
                .open(&directory)
                .map_err(|e| Error::io("hold bundled Git directory during use", e))?;
            let meta = file
                .metadata()
                .map_err(|e| Error::io("inspect bundled Git directory handle", e))?;
            if linked(&meta) || !meta.is_dir() {
                return Err(invalid("The runtime directory changed during verification"));
            }
            if directory == root {
                journal = ChangeJournal::start(root, &file);
            }
            directory_guards.push(file);
        }
        for item in fs::read_dir(&directory).map_err(|e| Error::io("list bundled Git files", e))? {
            cancel.check()?;
            visited += 1;
            if visited > MAX_FILES * 4 {
                return Err(invalid("The runtime contains too many entries"));
            }
            let item = item.map_err(|e| Error::io("inspect bundled Git entry", e))?;
            let path = item.path();
            // Windows enumeration already supplies no-follow metadata. The
            // subsequent guarded file open checks the actual handle again.
            let meta = item
                .metadata()
                .map_err(|e| Error::io("inspect bundled Git file", e))?;
            if linked(&meta) {
                return Err(invalid("The runtime contains a link or reparse point"));
            }
            children.insert(item.file_name(), meta.is_dir());
            if meta.is_dir() {
                directories.push(path);
                continue;
            }
            let relative = path
                .strip_prefix(root)
                .ok()
                .and_then(|p| p.to_str())
                .ok_or_else(|| invalid("Invalid runtime path"))?
                .replace('\\', "/")
                .to_ascii_lowercase();
            if !meta.is_file() || !entries.contains_key(&relative) || !seen.insert(relative) {
                return Err(invalid(
                    "Unexpected file in the runtime. Restore the complete reviewed bundle",
                ));
            }
        }
        directory_entries.insert(directory, children);
    }
    if seen.len() != entries.len() {
        return Err(invalid(
            "A runtime file is missing. Restore the complete reviewed bundle",
        ));
    }
    #[cfg(windows)]
    journal.cover(&directory_guards);
    if let Some(log) = log {
        log.observation(
            "managed_git_inventory",
            "managed-git-verify",
            &Ok(()),
            inventory_started.elapsed(),
        );
    }
    let hashing_started = std::time::Instant::now();
    let files = verify_files(root, entries, cancel, progress);
    if let Some(log) = log {
        log.observation(
            "managed_git_hashes",
            "managed-git-verify",
            &files,
            hashing_started.elapsed(),
        );
    }
    let files = files?;
    cancel.check()?;
    Ok(ManagedGitRuntime {
        root: root.to_owned(),
        files,
        directories: directory_guards,
        directory_entries,
        journal: Mutex::new(journal),
        #[cfg(test)]
        full_reads: AtomicUsize::new(0),
    })
}

fn verify_files(
    root: &Path,
    entries: &BTreeMap<String, Entry<'_>>,
    cancel: &CancellationToken,
    progress: &ManagedGitProgress,
) -> Result<Vec<File>> {
    let entries: Vec<_> = entries.values().collect();
    let workers = std::thread::available_parallelism()
        .map_or(1, usize::from)
        .min(4)
        .min(entries.len().max(1));
    let next = AtomicUsize::new(0);
    let stopped = AtomicBool::new(false);
    let verify = || {
        let result = (|| {
            let mut files = Vec::new();
            while !stopped.load(Ordering::Acquire) {
                cancel.check()?;
                let Some(entry) = entries.get(next.fetch_add(1, Ordering::Relaxed)) else {
                    break;
                };
                files.push(verify_file(root, entry, cancel)?);
                progress.0.bytes.fetch_add(entry.bytes, Ordering::Relaxed);
                progress.0.checked.fetch_add(1, Ordering::Relaxed);
            }
            Ok(files)
        })();
        if result.is_err() {
            stopped.store(true, Ordering::Release);
        }
        result
    };
    if workers == 1 {
        return verify();
    }
    // Bound I/O concurrency; keep every successfully hashed file locked. Join
    // all workers on cancellation, spawn failure or integrity failure before
    // returning, so a failed verification cannot leave handles or a live lease.
    std::thread::scope(|scope| {
        let mut handles = Vec::new();
        let mut failure = None;
        for _ in 0..workers {
            match std::thread::Builder::new()
                .name("git-verify".into())
                .spawn_scoped(scope, &verify)
            {
                Ok(handle) => handles.push(handle),
                Err(error) => {
                    stopped.store(true, Ordering::Release);
                    failure = Some(Error::io("start bundled Git verification worker", error));
                    break;
                }
            }
        }
        let mut files = Vec::with_capacity(entries.len());
        for handle in handles {
            let result = handle.join().unwrap_or_else(|_| {
                stopped.store(true, Ordering::Release);
                Err(invalid(
                    "A verification worker stopped unexpectedly; retry verification",
                ))
            });
            match result {
                Ok(mut verified) => files.append(&mut verified),
                Err(error) => {
                    failure.get_or_insert(error);
                }
            }
        }
        if let Some(error) = failure {
            return Err(error);
        }
        cancel.check()?;
        Ok(files)
    })
}

fn verify_file(root: &Path, entry: &Entry<'_>, cancel: &CancellationToken) -> Result<File> {
    cancel.check()?;
    let path = root.join(entry.path);
    // Windows OPEN_REPARSE_POINT plus handle metadata replaces a redundant
    // path stat. Hash exactly that protected handle, never a later path open.
    #[cfg(not(windows))]
    {
        let meta =
            fs::symlink_metadata(&path).map_err(|e| Error::io("inspect bundled Git file", e))?;
        if linked(&meta) || !meta.is_file() || meta.len() != entry.bytes {
            return Err(invalid(&format!(
                "File changed: {}. Restore the reviewed bundle",
                entry.path
            )));
        }
    }
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.share_mode(1).custom_flags(0x0020_0000); // FILE_SHARE_READ, OPEN_REPARSE_POINT
    }
    let mut file = options
        .open(&path)
        .map_err(|e| Error::io("open bundled Git file for verification", e))?;
    let meta = file
        .metadata()
        .map_err(|e| Error::io("inspect opened bundled Git file", e))?;
    if linked(&meta) || !meta.is_file() || meta.len() != entry.bytes {
        return Err(invalid("Runtime changed while checking it"));
    }
    let actual = digest(&mut file, entry.bytes, cancel).map_err(|e| match e {
        Error::Cancelled => e,
        _ => invalid(&format!(
            "Could not verify {}. Restore the reviewed bundle",
            entry.path
        )),
    })?;
    if !actual.eq_ignore_ascii_case(entry.hash) {
        return Err(invalid(&format!(
            "File changed: {}. Restore the reviewed bundle",
            entry.path
        )));
    }
    Ok(file)
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    fn fixture() -> (PathBuf, String) {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../tools/maintenance/managed-git-20261030/unit-fixtures")
            .join(format!(
                "{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
        fs::create_dir_all(root.join("cmd")).unwrap();
        fs::write(root.join("cmd/git.exe"), b"synthetic, never executed").unwrap();
        let hash = crate::sha256_bytes(b"synthetic, never executed").unwrap();
        (
            root,
            format!("ROCKET-MANAGED-GIT-1\t{MANAGED_GIT_VERSION}\n{hash}\t25\tcmd/git.exe\n"),
        )
    }
    #[test]
    fn managed_inventory_rejects_unsafe_duplicate_and_incomplete_records() {
        let (_, text) = fixture();
        // The fixture length is derived below; these checks exercise record parsing.
        assert!(inventory(&text).is_ok());
        for value in [
            text.replace("cmd/git.exe", "../git.exe"),
            text.replace("cmd/git.exe", "cmd/../git.exe"),
            text.replace("cmd/git.exe", "C:/git.exe"),
            text.replace("cmd/git.exe", "cmd/git.exe "),
            format!("{text}{}", text.lines().nth(1).unwrap()),
            text.replace("\t25\t", "\t999999999999\t"),
            text.replace("cmd/git.exe", "other.exe"),
        ] {
            assert!(inventory(&value).is_err(), "accepted an invalid inventory");
        }
    }
    #[test]
    fn managed_file_checks_protect_bytes_and_release_on_drop() {
        let (root, text) = fixture();
        let text = text.replace(
            "\t25\t",
            &format!("\t{}\t", b"synthetic, never executed".len()),
        );
        let entries = inventory(&text).unwrap();
        let progress = ManagedGitProgress::default();
        let lease =
            verify_inventory(&root, &entries, &CancellationToken::new(), &progress).unwrap();
        assert_eq!(progress.snapshot().0, 1);
        assert!(
            fs::write(root.join("cmd/git.exe"), b"changed").is_err(),
            "verified bytes must be write-protected while in use"
        );
        drop(lease);
        fs::write(root.join("cmd/git.exe"), b"changed").unwrap();
        assert!(verify_inventory(&root, &entries, &CancellationToken::new(), &progress).is_err());
    }
    #[test]
    fn concurrent_verification_checks_all_files_and_releases_every_handle_on_failure() {
        let (root, text) = fixture();
        let mut text = text.replace(
            "\t25\t",
            &format!("\t{}\t", b"synthetic, never executed".len()),
        );
        fs::create_dir(root.join("資料")).unwrap();
        let mut total = b"synthetic, never executed".len() as u64;
        for n in 0..128 {
            let content = vec![n as u8; 8192 + n];
            let relative = format!("資料/file-{n:03}.dat");
            fs::write(root.join(&relative), &content).unwrap();
            text.push_str(&format!(
                "{}\t{}\t{relative}\n",
                crate::sha256_bytes(&content).unwrap(),
                content.len()
            ));
            total += content.len() as u64;
        }
        let entries = inventory(&text).unwrap();
        let progress = ManagedGitProgress::default();
        let check = || verify_inventory(&root, &entries, &CancellationToken::new(), &progress);
        let runtime = check().unwrap();
        assert_eq!(progress.snapshot(), (129, 129, total, total));
        for entry in entries.values() {
            assert!(
                OpenOptions::new()
                    .write(true)
                    .open(root.join(entry.path))
                    .is_err(),
                "{} was not protected",
                entry.path
            );
        }
        drop(runtime);
        // Same-length tampering must be detected, including files handled by
        // different workers. A failed attempt must release all partial leases.
        for n in [0, 63, 127] {
            let path = root.join(format!("資料/file-{n:03}.dat"));
            let original = fs::read(&path).unwrap();
            let mut changed = original.clone();
            changed[0] ^= 0xff;
            fs::write(&path, changed).unwrap();
            assert!(matches!(check(), Err(Error::InvalidInput(_))));
            for entry in entries.values() {
                OpenOptions::new()
                    .write(true)
                    .open(root.join(entry.path))
                    .unwrap();
            }
            fs::write(&path, original).unwrap();
        }
        assert!(check().is_ok());
    }
    #[test]
    fn managed_missing_extra_tampered_and_cancelled_files_are_not_accepted() {
        let (root, text) = fixture();
        let text = text.replace(
            "\t25\t",
            &format!("\t{}\t", b"synthetic, never executed".len()),
        );
        let entries = inventory(&text).unwrap();
        let check = |cancel: &CancellationToken| {
            verify_inventory(&root, &entries, cancel, &ManagedGitProgress::default())
        };
        fs::write(root.join("cmd/added.dll"), b"unexpected").unwrap();
        assert!(check(&CancellationToken::new()).is_err());
        fs::remove_file(root.join("cmd/added.dll")).unwrap();
        fs::write(
            root.join("cmd/git.exe"),
            vec![b'x'; b"synthetic, never executed".len()],
        )
        .unwrap();
        assert!(check(&CancellationToken::new()).is_err());
        fs::remove_file(root.join("cmd/git.exe")).unwrap();
        assert!(check(&CancellationToken::new()).is_err());
        let cancel = CancellationToken::new();
        cancel.cancel();
        assert!(matches!(check(&cancel), Err(Error::Cancelled)));
    }
    #[test]
    fn bundled_discovery_does_not_hide_an_incomplete_installation() {
        let (root, _) = fixture();
        let directory = root.join("app/git");
        assert!(candidate_in(&directory).unwrap().is_none());
        fs::create_dir_all(&directory).unwrap();
        fs::write(directory.join("manifest.tsv"), b"marker only").unwrap();
        let expected = directory.join(MANAGED_GIT_VERSION).join("cmd/git.exe");
        assert_eq!(candidate_in(&directory).unwrap(), Some(expected.clone()));
        fs::remove_file(directory.join("manifest.tsv")).unwrap();
        fs::create_dir_all(expected.parent().unwrap()).unwrap();
        // Even without the executable, discovery must select the damaged bundle
        // so verification reports the error instead of using an unrelated PATH Git.
        assert_eq!(candidate_in(&directory).unwrap(), Some(expected));
    }
    #[test]
    fn managed_path_recognition_preserves_installed_git_choices() {
        assert!(runtime_root(Path::new("D:/Rocket/git/2.56.0.windows.4/cmd/git.exe")).is_some());
        assert!(runtime_root(Path::new("D:/Rocket/git/2.56.0.windows.4/bin/git.exe")).is_some());
        assert!(
            runtime_root(Path::new(
                "D:/Rocket/git/2.56.0.windows.4/ucrt64/bin/git.exe"
            ))
            .is_some()
        );
        assert!(
            runtime_root(Path::new(
                "D:/Rocket/git/2.56.0.windows.4/mingw64/bin/git.exe"
            ))
            .is_some()
        );
        assert!(runtime_root(Path::new("C:/Program Files/Git/cmd/git.exe")).is_none());
        assert!(runtime_root(Path::new("D:/Rocket/git/other/cmd/git.exe")).is_none());
    }
}
