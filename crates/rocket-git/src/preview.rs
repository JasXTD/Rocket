//! Repository previews: bare clones without file contents in Rocket's cache, the
//! explicit downloads their views need, and Keep, which turns one into a normal clone.
//!
//! Every Git command keeps `--no-lazy-fetch`, so Git never fetches behind a read.
//! A read in a preview first asks for the objects it needs in one `git fetch
//! --stdin` from the preview's origin, through Rocket's own network path (transport
//! checks, progress, Stop). Only repositories the application registered as
//! previews download anything; other partial clones behave as before.
mod missing;
use crate::{
    CancellationToken, CloneFailure, CloneOptions, ClonePhase, CloneProgress, CloneReport,
    CloneSource, Error, FetchFailure, FetchPhase, FetchProgress, GitClient, Head, ObjectId,
    Repository, Result,
    clone::{directory_size, reparse_point, within_size_limit},
    runner::{CommandLimits, args},
};
use std::{
    collections::{HashMap, HashSet},
    ffi::OsString,
    fs,
    io,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
        mpsc::{self, SyncSender},
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

/// Previews stop downloading at 1 GiB unless the user lifts the limit.
pub const PREVIEW_LIMIT: u64 = 1 << 30;
/// Objects asked for in one request.
const BATCH: usize = 4_000;
const LOCK_FILE: &str = "rocket-preview.lock";
const REPOSITORY: &str = "repository.git";
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(30 * 60);
/// How often a running download measures the preview's size.
const SIZE_SAMPLE: Duration = Duration::from_millis(400);
/// How long a new preview folder may go without its lock file before cleanup
/// treats it as left behind.
const SETUP_GRACE: Duration = Duration::from_secs(60);

/// What a preview's reads have downloaded, shared with the application so it can
/// show downloads as they happen.
#[derive(Debug, Default)]
pub struct PreviewDownloads {
    limit: Option<u64>,
    running: AtomicUsize,
    /// Objects asked for by the downloads now running.
    requested: AtomicU64,
    /// Of those, how many have arrived.
    received: AtomicU64,
    /// The preview's size on disk, refreshed while something downloads.
    size: AtomicU64,
    /// The size when the downloads now running began.
    base: AtomicU64,
    /// Why the last download failed; cleared when one starts again.
    failure: Mutex<Option<FetchFailure>>,
}

/// A moment's view of [`PreviewDownloads`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DownloadStatus {
    pub running: bool,
    /// File versions the running downloads asked for.
    pub files: u64,
    /// Of `files`, how many have arrived.
    pub received: u64,
    pub size: u64,
    /// Bytes the running downloads added so far.
    pub downloaded: u64,
    /// Why the last download failed, until the next one starts.
    pub failure: Option<FetchFailure>,
}

impl PreviewDownloads {
    pub fn new(limit: Option<u64>) -> Arc<Self> {
        Arc::new(Self {
            limit,
            ..Default::default()
        })
    }
    pub fn limit(&self) -> Option<u64> {
        self.limit
    }
    pub fn status(&self) -> DownloadStatus {
        let running = self.running.load(Ordering::Acquire) > 0;
        let size = self.size.load(Ordering::Acquire);
        DownloadStatus {
            running,
            files: self.requested.load(Ordering::Acquire),
            received: self.received.load(Ordering::Acquire),
            size,
            downloaded: if running { size.saturating_sub(self.base.load(Ordering::Acquire)) } else { 0 },
            failure: self.failure.lock().ok().and_then(|failure| *failure),
        }
    }
    /// Remember how a download ended: a failure until the next download starts.
    /// Stopping isn't a failure.
    fn record(&self, result: &Result<()>) {
        let failure = match result {
            Ok(()) | Err(Error::Cancelled) => None,
            Err(error) => Some(error.fetch_failure().unwrap_or(FetchFailure::Other)),
        };
        if let Ok(mut slot) = self.failure.lock() {
            *slot = failure;
        }
    }
    /// Record the preview's size, for the banner.
    pub fn set_size(&self, bytes: u64) {
        self.size.store(bytes, Ordering::Release);
    }
}

/// Repositories whose reads may download, by Git directory. Shared by every client
/// made from the same one.
#[derive(Clone, Debug, Default)]
pub(crate) struct PreviewRegistry(Arc<Mutex<HashMap<PathBuf, Arc<PreviewDownloads>>>>);

impl PreviewRegistry {
    pub(crate) fn lookup(&self, git_dir: &Path) -> Option<Arc<PreviewDownloads>> {
        self.0.lock().ok()?.get(git_dir).cloned()
    }
}

impl GitClient {
    /// Let reads of the bare repository at `git_dir` download the file contents they
    /// need. Only previews Rocket created should be registered.
    pub fn register_preview(&self, git_dir: &Path, downloads: Arc<PreviewDownloads>) -> Result<()> {
        let git_dir = git_dir
            .canonicalize()
            .map_err(|e| Error::io("resolve preview repository", e))?;
        self.previews
            .0
            .lock()
            .map_err(|_| Error::InvalidInput("preview registry is unavailable".into()))?
            .insert(git_dir, downloads);
        Ok(())
    }

    /// Stop downloads for a preview that was discarded or kept.
    pub fn unregister_preview(&self, git_dir: &Path) {
        let key = git_dir.canonicalize().unwrap_or_else(|_| git_dir.to_path_buf());
        if let Ok(mut map) = self.previews.0.lock() {
            map.remove(&key);
            map.remove(git_dir);
        }
    }
}

/// Rocket's folder of previews. Each preview has its own subfolder holding a lock
/// file that its Rocket process keeps open; folders whose lock nobody holds were
/// left by a crash and are removed at the next start.
#[derive(Clone, Debug)]
pub struct PreviewCache {
    root: PathBuf,
}

/// One preview's folder, held for as long as this value lives. Dropping it releases
/// the lock but keeps the files; [`PreviewSlot::discard`] deletes them.
#[derive(Debug)]
pub struct PreviewSlot {
    directory: PathBuf,
    lock: Mutex<Option<fs::File>>,
}

static NEXT_SLOT: AtomicU64 = AtomicU64::new(0);

impl PreviewCache {
    pub fn open(root: impl Into<PathBuf>) -> Result<Self> {
        let root = root.into();
        fs::create_dir_all(&root).map_err(|e| Error::io("prepare preview cache", e))?;
        let meta = fs::symlink_metadata(&root).map_err(|e| Error::io("prepare preview cache", e))?;
        if !meta.is_dir() || reparse_point(&meta) {
            return Err(Error::InvalidInput(
                "the preview cache must be a plain folder".into(),
            ));
        }
        let root = root
            .canonicalize()
            .map_err(|e| Error::io("prepare preview cache", e))?;
        Ok(Self { root })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// A new, empty preview folder with its lock taken.
    pub fn reserve(&self) -> Result<PreviewSlot> {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        for _ in 0..16 {
            let directory = self.root.join(format!(
                "p{}-{stamp:x}-{}",
                std::process::id(),
                NEXT_SLOT.fetch_add(1, Ordering::Relaxed)
            ));
            match fs::create_dir(&directory) {
                Ok(()) => {
                    let lock = lock_file(&directory.join(LOCK_FILE), true)
                        .map_err(|e| Error::io("lock preview", e))?;
                    return Ok(PreviewSlot {
                        directory,
                        lock: Mutex::new(Some(lock)),
                    });
                }
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(Error::io("prepare preview", e)),
            }
        }
        Err(Error::InvalidInput("no free preview folder".into()))
    }

    /// Delete previews no running Rocket holds. Returns how many were removed;
    /// anything that can't be removed now is left for the next start.
    pub fn remove_stale(&self) -> usize {
        let Ok(entries) = fs::read_dir(&self.root) else {
            return 0;
        };
        let mut removed = 0;
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(meta) = fs::symlink_metadata(&path) else {
                continue;
            };
            if !meta.is_dir() || reparse_point(&meta) || !slot_name(&path) {
                continue;
            }
            // Holding the lock proves no other process has it open.
            match lock_file(&path.join(LOCK_FILE), false) {
                Ok(lock) => {
                    drop(lock);
                    if remove_tree(&self.root, &path).is_ok() {
                        removed += 1;
                    }
                }
                Err(e) if e.kind() == io::ErrorKind::NotFound => {
                    // A folder being reserved right now has no lock yet: leave young ones.
                    let young = meta
                        .modified()
                        .ok()
                        .and_then(|time| time.elapsed().ok())
                        .is_none_or(|age| age < SETUP_GRACE);
                    if !young && remove_tree(&self.root, &path).is_ok() {
                        removed += 1;
                    }
                }
                Err(_) => {} // In use by another Rocket.
            }
        }
        removed
    }
}

impl PreviewSlot {
    pub fn directory(&self) -> &Path {
        &self.directory
    }
    /// Where the preview's bare repository is published.
    pub fn repository(&self) -> PathBuf {
        self.directory.join(REPOSITORY)
    }
    /// Bytes on disk.
    pub fn size(&self) -> u64 {
        directory_size(&self.directory)
    }
    /// Remove everything but the lock, so the preview can be downloaded again.
    pub fn clear(&self) -> Result<()> {
        let entries = fs::read_dir(&self.directory).map_err(|e| Error::io("clear preview", e))?;
        for entry in entries.flatten() {
            if entry.file_name() == LOCK_FILE {
                continue;
            }
            remove_tree(&self.directory, &entry.path())?;
        }
        Ok(())
    }
    /// Delete the preview's files and release its lock. Links inside are removed,
    /// never followed. Files another process still holds are left for the next start.
    pub fn discard(&self) -> Result<()> {
        if let Ok(mut lock) = self.lock.lock() {
            drop(lock.take());
        }
        let parent = self
            .directory
            .parent()
            .map(Path::to_path_buf)
            .ok_or_else(|| Error::InvalidInput("preview folder has no parent".into()))?;
        remove_tree(&parent, &self.directory)
    }
}

fn slot_name(path: &Path) -> bool {
    path.file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| n.starts_with('p') && n.len() > 3 && n[1..].chars().all(|c| c.is_ascii_hexdigit() || c == '-'))
}

/// Open the lock file exclusively. `create` makes it; otherwise it must exist.
fn lock_file(path: &Path, create: bool) -> io::Result<fs::File> {
    let mut options = fs::OpenOptions::new();
    options.read(true).write(true);
    if create {
        options.create_new(true);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.share_mode(0);
    }
    options.open(path)
}

/// Delete `target` and everything below it. `target` must lie inside `root`. A
/// reparse point is removed as itself; read-only files are made writable first.
pub(crate) fn remove_tree(root: &Path, target: &Path) -> Result<()> {
    let fail = |e: io::Error| Error::io("delete preview", e);
    if !target.starts_with(root) || target == root
        || target.components().any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return Err(Error::InvalidInput("refusing to delete outside the preview cache".into()));
    }
    let meta = match fs::symlink_metadata(target) {
        Ok(meta) => meta,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(fail(e)),
    };
    if reparse_point(&meta) {
        // A junction or directory link is a directory entry; a file link is a file.
        return if meta.is_dir() { fs::remove_dir(target) } else { fs::remove_file(target) }.map_err(fail);
    }
    if !meta.is_dir() {
        writable(target, &meta);
        return fs::remove_file(target).map_err(fail);
    }
    for entry in fs::read_dir(target).map_err(fail)? {
        let entry = entry.map_err(fail)?;
        remove_tree(root, &entry.path())?;
    }
    fs::remove_dir(target).map_err(fail)
}

fn writable(path: &Path, meta: &fs::Metadata) {
    let mut permissions = meta.permissions();
    if permissions.readonly() {
        #[allow(clippy::permissions_set_readonly_false)]
        permissions.set_readonly(false);
        let _ = fs::set_permissions(path, permissions);
    }
}

/// What opening a preview produced.
#[derive(Debug)]
pub struct PreviewOpened {
    /// The published bare repository.
    pub repository: PathBuf,
    /// Its default branch, if it has one.
    pub branch: Option<String>,
    /// Bytes on disk afterwards.
    pub size: u64,
}

impl GitClient {
    /// Clone `source` into `slot` as a bare repository without file contents, then
    /// download the files of its default branch, all within `limit` bytes. Progress
    /// reports the clone's phases, then `ClonePhase::DownloadingFiles` and file
    /// counts. Nothing from the repository runs: no checkout, hooks or submodules.
    pub fn open_preview(
        &self,
        source: &CloneSource,
        slot: &PreviewSlot,
        limit: Option<u64>,
        meter: &PreviewDownloads,
        progress: Option<&SyncSender<CloneProgress>>,
        cancel: &CancellationToken,
    ) -> Result<PreviewOpened> {
        let mut client = self.clone();
        client.runner.limits.timeout = client.runner.limits.timeout.max(DOWNLOAD_TIMEOUT);
        let stop = AtomicBool::new(false);
        std::thread::scope(|scope| {
            // Keep the meter's size current for the banner while this runs.
            scope.spawn(|| {
                while !stop.load(Ordering::Acquire) {
                    meter.set_size(directory_size(slot.directory()));
                    for _ in 0..10 {
                        if stop.load(Ordering::Acquire) {
                            return;
                        }
                        std::thread::sleep(Duration::from_millis(50));
                    }
                }
            });
            let result = client.open_preview_inner(source, slot, limit, progress, cancel);
            stop.store(true, Ordering::Release);
            result
        })
        .inspect(|opened| meter.set_size(opened.size))
        .inspect_err(|_| {
            // A preview that didn't open keeps nothing; its lock stays for a retry.
            let _ = slot.clear();
            meter.set_size(0);
        })
    }

    fn open_preview_inner(
        &self,
        source: &CloneSource,
        slot: &PreviewSlot,
        limit: Option<u64>,
        progress: Option<&SyncSender<CloneProgress>>,
        cancel: &CancellationToken,
    ) -> Result<PreviewOpened> {
        let report = self.clone_repository_with(source, slot.repository(), &CloneOptions::preview(limit), progress, cancel)?;
        let repository = report_repository(report)?;
        cancel.check()?;
        let branch = match repository.head(cancel)? {
            Head::Branch { name, .. } => String::from_utf8(name).ok(),
            Head::Detached { .. } => None,
            Head::Unborn { .. } => {
                return Ok(PreviewOpened { repository: slot.repository(), branch: None, size: slot.size() });
            }
        };
        phase(progress, ClonePhase::DownloadingFiles);
        let missing = repository.missing_in_tree("HEAD", cancel)?;
        let total = missing.len() as u64;
        files(progress, 0, total);
        let (result, over) = within_size_limit(slot.directory(), limit, cancel, |cancel| {
            repository.fetch_objects(missing, cancel, |done| files(progress, done, total))
        });
        if over {
            return Err(Error::Clone(CloneFailure::SizeLimit));
        }
        result?;
        Ok(PreviewOpened { repository: slot.repository(), branch, size: slot.size() })
    }

    /// Make a normal clone of the preview at `preview` in `destination`: download
    /// every file version still missing, clone the preview with Rocket's verified
    /// clone, then point `origin` at `url`. The preview itself is unchanged apart
    /// from the downloaded files, so a stopped Keep can be tried again.
    pub fn keep_preview(
        &self,
        preview: &Path,
        url: &str,
        destination: &Path,
        progress: Option<&SyncSender<CloneProgress>>,
        cancel: &CancellationToken,
    ) -> Result<KeptPreview> {
        let mut client = self.clone();
        client.runner.limits.timeout = client.runner.limits.timeout.max(DOWNLOAD_TIMEOUT * 2);
        let repository = client.open(preview, cancel)?;
        if repository.info().worktree.is_some() {
            return Err(Error::InvalidInput("only a preview can be kept this way".into()));
        }
        phase(progress, ClonePhase::DownloadingFiles);
        let missing = repository.missing_everywhere(cancel)?;
        let total = missing.len() as u64;
        files(progress, 0, total);
        repository.fetch_objects(missing, cancel, |done| files(progress, done, total))?;
        if !repository.missing_everywhere(cancel)?.is_empty() {
            return Err(Error::InvalidInput(
                "some file versions are still missing after the download".into(),
            ));
        }
        let clone = client.clone_repository(&CloneSource::Local(preview.to_path_buf()), destination, progress, cancel)?;
        let origin = match &clone.repository {
            Some(Ok(kept)) => Some(kept.point_origin_at(url, cancel)),
            _ => None,
        };
        Ok(KeptPreview { clone, origin })
    }
}

impl GitClient {
    /// Ask Git's credential helper to sign in to `host` for a preview the host
    /// refused, with the helper's own window when it has one. Rocket keeps nothing:
    /// the next download asks the helper again.
    pub fn preview_sign_in(&self, host: &str, cancel: &CancellationToken) -> Result<()> {
        if host.is_empty()
            || host.len() > 255
            || !host.bytes().all(|b| b.is_ascii_alphanumeric() || b".-:[]".contains(&b))
        {
            return Err(Error::InvalidInput("this preview's host can't be signed in to".into()));
        }
        let cwd = crate::hosting::neutral_directory()?;
        let mut runner = self.runner.clone();
        // The helper may wait for its sign-in window.
        runner.limits.timeout = Duration::from_secs(300);
        runner.limits.stdout_bytes = 64 * 1024;
        let input = format!("protocol=https\nhost={host}\n\n").into_bytes();
        let output = self.runner.step("preview-sign-in", "read_credential", || runner.credential(&cwd, true, input, cancel))?;
        let signed = output.status.success() && output.stdout.split(|b| *b == b'\n').any(|line| line.starts_with(b"password="));
        if signed {
            Ok(())
        } else {
            Err(Error::Hosting(crate::HostingFailure::SignInRequired))
        }
    }
}

/// [`GitClient::keep_preview`]'s result: the clone's own report, and whether
/// `origin` now names the original URL (only once the clone opened).
#[derive(Debug)]
#[must_use = "inspect the clone and the origin change"]
pub struct KeptPreview {
    pub clone: CloneReport,
    pub origin: Option<Result<()>>,
}

fn report_repository(report: CloneReport) -> Result<Repository> {
    report.command?;
    match report.verification {
        Some(Ok(())) => {}
        Some(Err(e)) => return Err(e),
        None => return Err(Error::Clone(CloneFailure::VerificationFailed)),
    }
    match report.publication {
        Some(Ok(())) => {}
        Some(Err(e)) => return Err(e),
        None => return Err(Error::Clone(CloneFailure::PublicationFailed)),
    }
    let repository = report
        .repository
        .unwrap_or(Err(Error::Clone(CloneFailure::OpenFailed)))?;
    // The empty workspace and template are Rocket's own; a failed cleanup only
    // leaves them inside the preview's folder, which is deleted with it.
    let _ = report.cleanup;
    Ok(repository)
}

fn phase(progress: Option<&SyncSender<CloneProgress>>, value: ClonePhase) {
    if let Some(sender) = progress {
        let _ = sender.try_send(CloneProgress::Phase(value));
    }
}

fn files(progress: Option<&SyncSender<CloneProgress>>, done: u64, total: u64) {
    if let Some(sender) = progress {
        let _ = sender.try_send(CloneProgress::Files { done, total });
    }
}

impl Repository {
    /// This handle may download file contents for its reads.
    pub fn is_preview(&self) -> bool {
        self.downloads.is_some()
    }

    /// Objects among `ids` that aren't in this repository. Never fetches.
    pub(crate) fn missing_objects(&self, ids: &[ObjectId], cancel: &CancellationToken) -> Result<Vec<ObjectId>> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let input: Vec<u8> = ids
            .iter()
            .flat_map(|id| id.as_str().bytes().chain(Some(b'\n')))
            .collect();
        let output = self
            .runner
            .run(self.cwd(), &args(&["cat-file", "--batch-check"]), input, cancel)?
            .success("preview-objects")?;
        let mut missing = Vec::new();
        for line in output.split(|b| *b == b'\n').filter(|l| !l.is_empty()) {
            if let Some(id) = line.strip_suffix(b" missing") {
                missing.push(crate::parse::oid(id, "preview-objects")?);
            }
        }
        Ok(missing)
    }

    /// Missing objects below the tree of `tree_ish` (a commit, tag or tree).
    pub(crate) fn missing_in_tree(&self, tree_ish: &str, cancel: &CancellationToken) -> Result<Vec<ObjectId>> {
        self.missing_listed(&args(&[
            "rev-list",
            "--objects",
            "--missing=print",
            "--no-object-names",
            &format!("{tree_ish}^{{tree}}"),
            "--",
        ]), cancel)
    }

    /// Every missing object reachable from any reference.
    pub(crate) fn missing_everywhere(&self, cancel: &CancellationToken) -> Result<Vec<ObjectId>> {
        self.missing_listed(&args(&[
            "rev-list",
            "--objects",
            "--missing=print",
            "--no-object-names",
            "--all",
            "--",
        ]), cancel)
    }

    /// The `?`-marked lines of a `rev-list --missing=print`, with independent
    /// output, record-size and missing-object budgets. Lifting the disk cap does
    /// not allow a hostile graph to consume unbounded memory.
    fn missing_listed(&self, arguments: &[OsString], cancel: &CancellationToken) -> Result<Vec<ObjectId>> {
        let mut missing = missing::MissingObjects::default();
        let mut failed = None;
        let mut sink = |chunk: &[u8]| -> bool {
            if let Err(error) = missing.push(chunk) {
                failed = Some(error);
                return false;
            }
            true
        };
        let limits = CommandLimits {
            timeout: DOWNLOAD_TIMEOUT,
            stdout_bytes: self.runner.limits.stdout_bytes.min(missing::OUTPUT_BYTES),
            ..self.runner.limits
        };
        let output = self
            .runner
            .read_streamed(self.cwd(), arguments, Vec::new(), cancel, &limits, &mut sink)?;
        if let Some(e) = failed {
            return Err(e);
        }
        if !output.status.success() {
            return Err(Error::Git {
                operation: "preview-objects",
                code: output.status.code(),
                stdout: Vec::new(),
                stderr: output.stderr,
            });
        }
        missing.finish()
    }

    /// Ask origin for `ids`, a batch per request. `done` hears how many have come.
    /// Download `ids` from origin in batches. `done` hears how many have arrived so
    /// far, from Git's progress within a batch and after each one.
    pub(crate) fn fetch_objects(
        &self,
        mut ids: Vec<ObjectId>,
        cancel: &CancellationToken,
        done: impl FnMut(u64) + Send,
    ) -> Result<()> {
        let mut seen = HashSet::new();
        ids.retain(|id| seen.insert(id.clone()));
        if ids.is_empty() {
            return Ok(());
        }
        let mut runner = self.runner.clone();
        runner.limits.timeout = runner.limits.timeout.max(DOWNLOAD_TIMEOUT);
        let arguments = args(&[
            "-c",
            "fetch.negotiationAlgorithm=noop",
            "fetch",
            "--progress",
            "--no-tags",
            "--no-write-fetch-head",
            "--recurse-submodules=no",
            "--filter=blob:none",
            "--stdin",
            "--",
            "origin",
        ]);
        let done = Mutex::new(done);
        let mut count = 0u64;
        for batch in ids.chunks(BATCH) {
            cancel.check()?;
            let input: Vec<u8> = batch
                .iter()
                .flat_map(|id| id.as_str().bytes().chain(Some(b'\n')))
                .collect();
            let (sender, updates) = mpsc::sync_channel::<FetchProgress>(256);
            let output = std::thread::scope(|scope| {
                let (done, before, limit) = (&done, count, batch.len() as u64);
                scope.spawn(move || {
                    for update in updates {
                        if update.phase == FetchPhase::Receiving
                            && let Ok(mut done) = done.lock()
                        {
                            done(before + update.current.min(limit));
                        }
                    }
                });
                let output = self.runner.observe("write_result", "fetch", || {
                    runner.fetch(self.cwd(), &arguments, input, cancel, Some(&sender))
                });
                drop(sender);
                output
            })?;
            if !output.status.success() {
                return Err(Error::Git {
                    operation: "fetch",
                    code: output.status.code(),
                    stdout: output.stdout,
                    stderr: output.stderr,
                });
            }
            count += batch.len() as u64;
            if let Ok(mut done) = done.lock() {
                done(count);
            }
        }
        Ok(())
    }

    /// Before a read in a preview: download `ids` that are missing.
    pub(crate) fn prepare_objects(&self, ids: &[ObjectId], cancel: &CancellationToken) -> Result<()> {
        let Some(downloads) = &self.downloads else {
            return Ok(());
        };
        let missing = self.missing_objects(ids, cancel)?;
        self.download_for_read(downloads, missing, cancel)
    }

    /// Before a read in a preview: download the files of `tree_ish`.
    pub(crate) fn prepare_tree(&self, tree_ish: &str, cancel: &CancellationToken) -> Result<()> {
        let Some(downloads) = &self.downloads else {
            return Ok(());
        };
        let missing = self.missing_in_tree(tree_ish, cancel)?;
        self.download_for_read(downloads, missing, cancel)
    }

    /// Before a read in a preview: download every version of `path` reachable from
    /// `tip`, for history and blame. Older names of a renamed file aren't included.
    pub(crate) fn prepare_path(&self, tip: &str, path: &crate::RepoPath, cancel: &CancellationToken) -> Result<()> {
        let Some(downloads) = &self.downloads else {
            return Ok(());
        };
        let mut arguments = args(&["rev-list", "--objects", "--missing=print", "--no-object-names", tip, "--"]);
        arguments.push(path.argument()?);
        let missing = self.missing_listed(&arguments, cancel)?;
        self.download_for_read(downloads, missing, cancel)
    }

    /// Before a read in a preview: download what `diff-tree` between `from` (or the
    /// commit's first parent, or nothing for a root commit) and `to` touches.
    pub(crate) fn prepare_changes(&self, from: Option<&str>, to: &str, paths: &[OsString], cancel: &CancellationToken) -> Result<()> {
        if self.downloads.is_none() {
            return Ok(());
        }
        let mut arguments = args(&["diff-tree", "-r", "-z", "--raw", "--no-abbrev", "--no-renames", "--no-ext-diff", "--no-textconv"]);
        match from {
            Some(from) => arguments.extend([OsString::from(from), OsString::from(to)]),
            None => {
                let parents = self
                    .runner
                    .run(self.cwd(), &args(&["rev-parse", "--verify", "--quiet", &format!("{to}^1")]), Vec::new(), cancel)?;
                if parents.status.success() {
                    arguments.extend([OsString::from(format!("{to}^1")), OsString::from(to)]);
                } else {
                    arguments.extend([OsString::from("--root"), OsString::from(to)]);
                }
            }
        }
        arguments.push("--".into());
        arguments.extend(paths.iter().cloned());
        let output = self
            .runner
            .run(self.cwd(), &arguments, Vec::new(), cancel)?
            .success("preview-objects")?;
        let ids = raw_blobs(&output)?;
        self.prepare_objects(&ids, cancel)
    }

    fn download_for_read(&self, downloads: &PreviewDownloads, missing: Vec<ObjectId>, cancel: &CancellationToken) -> Result<()> {
        if missing.is_empty() {
            return Ok(());
        }
        if let Some(limit) = downloads.limit
            && directory_size(&self.info().common_dir) > limit
        {
            return Err(Error::Clone(CloneFailure::SizeLimit));
        }
        let directory = &self.info().common_dir;
        if downloads.running.fetch_add(1, Ordering::AcqRel) == 0 {
            let size = directory_size(directory);
            downloads.base.store(size, Ordering::Release);
            downloads.set_size(size);
        }
        downloads.record(&Ok(()));
        let count = missing.len() as u64;
        downloads.requested.fetch_add(count, Ordering::AcqRel);
        let mut mine = 0u64;
        let mut sampled = Instant::now();
        let result = self.fetch_objects(missing, cancel, |arrived| {
            downloads.received.fetch_add(arrived.saturating_sub(mine), Ordering::AcqRel);
            mine = mine.max(arrived);
            if sampled.elapsed() >= SIZE_SAMPLE {
                sampled = Instant::now();
                downloads.set_size(directory_size(directory));
            }
        });
        downloads.received.fetch_sub(mine, Ordering::AcqRel);
        downloads.requested.fetch_sub(count, Ordering::AcqRel);
        downloads.set_size(directory_size(directory));
        downloads.record(&result);
        downloads.running.fetch_sub(1, Ordering::AcqRel);
        result
    }

    /// Point `origin` at `url` in a repository Rocket just created.
    fn point_origin_at(&self, url: &str, cancel: &CancellationToken) -> Result<()> {
        if url.is_empty() || url.contains('\0') || url.starts_with('-') {
            return Err(Error::InvalidInput("the preview's address is invalid".into()));
        }
        let output = self.runner.write(
            self.cwd(),
            &[OsString::from("remote"), OsString::from("set-url"), OsString::from("origin"), OsString::from(url)],
            Vec::new(),
            cancel,
        )?;
        if output.status.success() {
            Ok(())
        } else {
            Err(Error::Git { operation: "remote-set-url", code: output.status.code(), stdout: output.stdout, stderr: output.stderr })
        }
    }
}

/// Blob IDs on either side of `diff-tree -r -z --raw --no-abbrev` records, without
/// submodule commits or the all-zero ID of an added or deleted side.
fn raw_blobs(output: &[u8]) -> Result<Vec<ObjectId>> {
    let mut ids = Vec::new();
    let mut fields = output.split(|b| *b == 0).filter(|f| !f.is_empty());
    while let Some(header) = fields.next() {
        let Some(header) = header.strip_prefix(b":") else {
            // A commit ID line before the records (diff-tree with one commit).
            continue;
        };
        let parts: Vec<&[u8]> = header.split(|b| *b == b' ').collect();
        if parts.len() < 5 {
            return Err(Error::parse("preview-objects", "invalid raw record"));
        }
        fields.next(); // the path
        for (mode, id) in [(parts[0], parts[2]), (parts[1], parts[3])] {
            if mode == b"160000" || id.iter().all(|b| *b == b'0') {
                continue;
            }
            ids.push(crate::parse::oid(id, "preview-objects")?);
        }
    }
    Ok(ids)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn raw_records_name_both_sides_but_not_submodules_or_zero_ids() {
        let a = "a".repeat(40);
        let b = "b".repeat(40);
        let z = "0".repeat(40);
        let c = "c".repeat(40);
        let raw = format!(
            ":100644 100644 {a} {b} M\0src/one.rs\0:000000 100644 {z} {c} A\0new.rs\0:160000 160000 {a} {b} M\0vendor\0"
        );
        let ids = raw_blobs(raw.as_bytes()).unwrap();
        let text: Vec<String> = ids.iter().map(|i| i.as_str().to_owned()).collect();
        assert_eq!(text, vec![a.clone(), b.clone(), c]);
        assert!(raw_blobs(b":100644 broken\0x\0").is_err());
    }

    #[test]
    fn deleting_stays_inside_the_cache_and_removes_read_only_files() {
        let base = std::env::temp_dir().join(format!("rocket-preview-unit-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        let cache = PreviewCache::open(&base).unwrap();
        let slot = cache.reserve().unwrap();
        let pack = slot.directory().join("repository.git/objects/pack");
        fs::create_dir_all(&pack).unwrap();
        let file = pack.join("pack-1.pack");
        fs::write(&file, b"data").unwrap();
        let mut permissions = fs::metadata(&file).unwrap().permissions();
        permissions.set_readonly(true);
        fs::set_permissions(&file, permissions).unwrap();
        assert!(slot.size() >= 4);
        // A live slot is not stale.
        assert_eq!(cache.remove_stale(), 0);
        assert!(remove_tree(cache.root(), cache.root()).is_err());
        assert!(remove_tree(&slot.directory().join("x"), slot.directory()).is_err());
        let folder = slot.directory().to_path_buf();
        slot.discard().unwrap();
        assert!(!folder.exists());
        // A folder left without a running owner is removed at the next start.
        let left = cache.reserve().unwrap();
        let left_folder = left.directory().to_path_buf();
        drop(left);
        assert_eq!(cache.remove_stale(), 1);
        assert!(!left_folder.exists());
        // A folder without its lock yet may be one another Rocket is reserving.
        let young = cache.root().join("p1-abc-0");
        fs::create_dir(&young).unwrap();
        assert_eq!(cache.remove_stale(), 0);
        assert!(young.exists());
        let _ = fs::remove_dir_all(&base);
    }
}
