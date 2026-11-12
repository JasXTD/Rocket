//! Git's commit-graph, shown to people as the index of a repository's commits: one
//! optional file under `objects/info`, or a short chain of them, holding every
//! commit's parents and position. Git orders history several times faster with it.
//! It holds nothing the commits do not, so writing, updating or removing it changes
//! no commit, branch or file.
use crate::{runner::args, *};
use std::{
    ffi::OsString,
    fs,
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    sync::mpsc::SyncSender,
    time::SystemTime,
};

/// Rocket's own setting in the repository's config file: add fetched commits to
/// the index after every fetch Rocket runs there.
const ON_FETCH_KEY: &str = "rocket.commitGraphOnFetch";
/// Git merges layers long before a chain grows this long; a longer one is not Git's.
const MAX_LAYERS: usize = 64;

/// What Rocket can do to the index. Each one is a repository write.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HistoryIndexChange {
    /// Write the index, or add the commits an existing one lacks, and store whether
    /// fetches keep it up to date. `commits` is how many to expect, for progress only.
    Write { on_fetch: bool, commits: Option<u64> },
    Remove,
    /// Only store whether fetches keep it up to date.
    OnFetch(bool),
}

impl HistoryIndexChange {
    pub fn label(self) -> &'static str {
        match self {
            Self::Write { .. } => "history-index-write",
            Self::Remove => "history-index-remove",
            Self::OnFetch(_) => "history-index-setting",
        }
    }
}

/// Why Git reads no index in this repository, so writing one would change nothing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HistoryIndexUnused {
    /// A shallow clone: Git writes none.
    Shallow,
    /// Replaced or grafted commits: Git writes one and then ignores it.
    Replaced,
    /// `core.commitGraph` is false: Git writes none.
    TurnedOff,
}

/// The index on disk, read from its files without starting Git.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HistoryIndexFile {
    pub bytes: u64,
    /// How many commits it holds; `None` when a file is not in the format Rocket reads.
    pub commits: Option<u64>,
    pub written: Option<SystemTime>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HistoryIndex {
    pub file: Option<HistoryIndexFile>,
    pub unused: Option<HistoryIndexUnused>,
    /// Rocket adds fetched commits to the index after each of its fetches.
    pub on_fetch: bool,
}

/// Returned after an attempted change, even when it failed or was stopped: what
/// happened, and the index as it is now.
#[derive(Debug)]
#[must_use = "inspect the outcome and the index as it is now"]
pub struct HistoryIndexReport {
    pub result: Result<()>,
    pub index: Result<HistoryIndex>,
}

/// Where Git keeps the index in this repository's own object store.
struct Layout {
    info: PathBuf,
}

impl Layout {
    fn single(&self) -> PathBuf {
        self.info.join("commit-graph")
    }
    fn chain_folder(&self) -> PathBuf {
        self.info.join("commit-graphs")
    }
    fn chain(&self) -> PathBuf {
        self.chain_folder().join("commit-graph-chain")
    }
    /// The lock Git holds while it writes a single file, and the one for a chain.
    fn locks(&self) -> [PathBuf; 2] {
        [
            self.info.join("commit-graph.lock"),
            self.chain_folder().join("commit-graph-chain.lock"),
        ]
    }
    /// The files Git reads, in its own order of preference: the single file when
    /// there is one, otherwise the layers its chain file names.
    fn layers(&self) -> Result<Vec<PathBuf>> {
        if is_file(&self.single())? {
            return Ok(vec![self.single()]);
        }
        let chain = match fs::read(self.chain()) {
            Ok(chain) => chain,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(Error::io("read commit index", error)),
        };
        let mut layers = Vec::new();
        for name in chain.split(|byte| *byte == b'\n').filter(|name| !name.is_empty()) {
            // Object IDs only, so a chain file can never name a path outside its folder.
            let name = std::str::from_utf8(name).ok().filter(|name| {
                matches!(name.len(), 40 | 64) && name.bytes().all(|b| b.is_ascii_hexdigit())
            });
            let Some(name) = name else {
                return Ok(Vec::new());
            };
            if layers.len() == MAX_LAYERS {
                return Ok(Vec::new());
            }
            let layer = self.chain_folder().join(format!("graph-{name}.graph"));
            if is_file(&layer)? {
                layers.push(layer);
            }
        }
        Ok(layers)
    }
    /// Half-written layers of a chain. Git removes its own when it ends by itself.
    fn temporary(&self) -> Vec<OsString> {
        let Ok(entries) = fs::read_dir(self.chain_folder()) else {
            return Vec::new();
        };
        entries
            .filter_map(|entry| Some(entry.ok()?.file_name()))
            .filter(|name| name.to_string_lossy().starts_with("tmp_graph_"))
            .collect()
    }
}

fn is_file(path: &Path) -> Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(meta) => Ok(meta.is_file()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(Error::io("inspect commit index", error)),
    }
}

/// Remove one of Git's index files. Git writes them read-only, which Windows
/// refuses to delete until the attribute is cleared.
// Clearing it widens a file's permissions elsewhere; this arm exists on Windows only.
#[cfg_attr(windows, allow(clippy::permissions_set_readonly_false))]
fn remove(path: &Path) -> std::io::Result<()> {
    match fs::remove_file(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        #[cfg(windows)]
        Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => {
            let mut permissions = fs::symlink_metadata(path)?.permissions();
            if !permissions.readonly() {
                return Err(error);
            }
            permissions.set_readonly(false);
            fs::set_permissions(path, permissions)?;
            fs::remove_file(path)
        }
        other => other,
    }
}

/// How many commits one index file holds: the last entry of its fanout table.
/// `None` for anything but the version 1 layout Git writes today.
fn layer_commits(path: &Path) -> Option<u64> {
    let mut file = fs::File::open(path).ok()?;
    let mut header = [0u8; 8];
    file.read_exact(&mut header).ok()?;
    if &header[..4] != b"CGPH" || header[4] != 1 {
        return None;
    }
    // One 12-byte entry for each chunk and one that ends the table.
    let mut table = vec![0u8; (usize::from(header[6]) + 1) * 12];
    file.read_exact(&mut table).ok()?;
    let fanout = table
        .as_chunks::<12>()
        .0
        .iter()
        .find(|entry| &entry[..4] == b"OIDF")
        .map(|entry| u64::from_be_bytes(entry[4..].try_into().expect("eight bytes")))?;
    file.seek(SeekFrom::Start(fanout.checked_add(255 * 4)?)).ok()?;
    let mut last = [0u8; 4];
    file.read_exact(&mut last).ok()?;
    Some(u64::from(u32::from_be_bytes(last)))
}

/// What was there before a write, so that only what a stopped write of ours left
/// behind is cleared afterwards.
struct Before {
    temporary: Vec<OsString>,
}

impl Before {
    fn observe(layout: &Layout) -> Result<Self> {
        for lock in layout.locks() {
            if fs::symlink_metadata(&lock).is_ok() {
                // Named from the .git folder: the full path is long and, on Windows,
                // in a form people do not type.
                let inside = Path::new("objects").join("info").join(lock.strip_prefix(&layout.info).unwrap_or(&lock));
                return Err(Error::Unsupported(format!(
                    "Another Git process is writing this repository's commit index, or one was interrupted. If no Git is running, delete {} in the repository's .git folder and try again.",
                    inside.display()
                )));
            }
        }
        Ok(Self {
            temporary: layout.temporary(),
        })
    }

    /// Call only after Rocket stopped its own Git, which is reaped by then. Git
    /// removes its lock when it ends by itself, but a stopped process cannot, and
    /// the lock would refuse every later write. No lock existed when this write
    /// began and Rocket's write gate was held throughout, so one that exists now is
    /// that process's. The index is a copy of what the commits say; nothing else in
    /// the repository is touched, and no other Git lock is ever removed here.
    fn clear_stopped_write(&self, layout: &Layout) {
        for lock in layout.locks() {
            let _ = remove(&lock);
        }
        for name in layout.temporary() {
            if !self.temporary.contains(&name) {
                let _ = remove(&layout.chain_folder().join(name));
            }
        }
    }
}

impl Repository {
    fn index_layout(&self) -> Layout {
        Layout {
            info: self.info().common_dir.join("objects").join("info"),
        }
    }

    /// Whether Git has an index to read here: file checks only, no Git process.
    pub fn has_history_index(&self) -> bool {
        self.index_layout()
            .layers()
            .is_ok_and(|layers| !layers.is_empty())
    }

    fn index_file(&self) -> Result<Option<HistoryIndexFile>> {
        let layers = self.index_layout().layers()?;
        if layers.is_empty() {
            return Ok(None);
        }
        let mut file = HistoryIndexFile {
            bytes: 0,
            commits: Some(0),
            written: None,
        };
        for layer in &layers {
            let meta = fs::metadata(layer).map_err(|e| Error::io("inspect commit index", e))?;
            file.bytes += meta.len();
            file.written = file.written.max(meta.modified().ok());
            file.commits = file
                .commits
                .zip(layer_commits(layer))
                .map(|(sum, layer)| sum + layer);
        }
        Ok(Some(file))
    }

    fn config_bool(
        &self,
        scope: &[&str],
        key: &str,
        cancel: &CancellationToken,
    ) -> Result<Option<bool>> {
        let mut arguments = args(&["config"]);
        arguments.extend(args(scope));
        arguments.extend(args(&["--type=bool", "--get", key]));
        let output = self.runner.run(self.cwd(), &arguments, vec![], cancel)?;
        if output.status.code() == Some(1) && output.stdout.is_empty() {
            return Ok(None);
        }
        match output.success("read commit index settings")?.trim_ascii() {
            b"true" => Ok(Some(true)),
            b"false" => Ok(Some(false)),
            _ => Err(Error::parse("commit index settings", "not true or false")),
        }
    }

    /// Only the repository's own config file, which is the one `set_index_on_fetch` edits.
    fn index_on_fetch(&self, cancel: &CancellationToken) -> Result<bool> {
        Ok(self
            .config_bool(&["--local", "--no-includes"], ON_FETCH_KEY, cancel)?
            .unwrap_or(false))
    }

    fn index_unused(&self, cancel: &CancellationToken) -> Result<Option<HistoryIndexUnused>> {
        if self.info().is_shallow {
            return Ok(Some(HistoryIndexUnused::Shallow));
        }
        if self.config_bool(&[], "core.commitGraph", cancel)? == Some(false) {
            return Ok(Some(HistoryIndexUnused::TurnedOff));
        }
        if fs::symlink_metadata(self.info().common_dir.join("info/grafts")).is_ok() {
            return Ok(Some(HistoryIndexUnused::Replaced));
        }
        let replaced = self
            .runner
            .run(
                self.cwd(),
                &args(&["for-each-ref", "--count=1", "--format=%(refname)", "refs/replace/"]),
                vec![],
                cancel,
            )?
            .success("read replaced commits")?;
        Ok((!replaced.is_empty()).then_some(HistoryIndexUnused::Replaced))
    }

    /// The index as it is now, whether Git reads it, and the fetch setting.
    pub fn history_index(&self, cancel: &CancellationToken) -> Result<HistoryIndex> {
        cancel.check()?;
        Ok(HistoryIndex {
            file: self.index_file()?,
            unused: self.index_unused(cancel)?,
            on_fetch: self.index_on_fetch(cancel)?,
        })
    }

    /// Every commit a branch, tag or other reference reaches: what the index covers
    /// once written. Without an index this walks the whole history.
    pub fn count_commits(&self, cancel: &CancellationToken) -> Result<u64> {
        let output = self
            .runner
            .run(self.cwd(), &args(&["rev-list", "--count", "--all", "--"]), vec![], cancel)?
            .success("count commits")?;
        std::str::from_utf8(output.trim_ascii())
            .ok()
            .and_then(|count| count.parse().ok())
            .ok_or_else(|| Error::parse("count commits", "not a number"))
    }

    /// Write, update or remove the index, or store the fetch setting, then read the
    /// index again. Progress counts the commits Git has read so far.
    pub fn change_history_index(
        &self,
        change: HistoryIndexChange,
        progress: Option<&SyncSender<FetchProgress>>,
        cancel: &CancellationToken,
    ) -> Result<HistoryIndexReport> {
        let _permit = self.acquire_write(cancel)?;
        let result = self
            .runner
            .observe("write_result", change.label(), || match change {
                HistoryIndexChange::Write { on_fetch, commits } => {
                    match self.index_unused(cancel)? {
                        Some(HistoryIndexUnused::Shallow) => return Err(Error::Unsupported(
                            "Git keeps no commit index for a shallow clone.".into(),
                        )),
                        Some(HistoryIndexUnused::TurnedOff) => return Err(Error::Unsupported(
                            "Git's core.commitGraph setting is off, so Git would write no commit index.".into(),
                        )),
                        Some(HistoryIndexUnused::Replaced) => return Err(Error::Unsupported(
                            "Git reads no commit index while the repository has replaced or grafted commits.".into(),
                        )),
                        None => {}
                    }
                    // The setting first: if it cannot be stored, nothing is written.
                    self.set_index_on_fetch(on_fetch, cancel)?;
                    self.write_index(commits, progress, cancel)?;
                    if !self.has_history_index() {
                        return Err(Error::Unsupported(
                            "Git finished without writing a commit index for this repository.".into(),
                        ));
                    }
                    Ok(())
                }
                HistoryIndexChange::Remove => self.remove_index(cancel),
                HistoryIndexChange::OnFetch(on) => self.set_index_on_fetch(on, cancel),
            });
        let index = self.runner.observe("refresh", "history-index-read", || {
            self.history_index(&CancellationToken::new())
        });
        Ok(HistoryIndexReport { result, index })
    }

    /// Caller holds the write permit. A first write makes the single file; a later
    /// one adds a small layer with the commits the index lacks, as Git's own
    /// fetch.writeCommitGraph does, and does nothing when it lacks none.
    fn write_index(
        &self,
        commits: Option<u64>,
        progress: Option<&SyncSender<FetchProgress>>,
        cancel: &CancellationToken,
    ) -> Result<()> {
        let layout = self.index_layout();
        let before = Before::observe(&layout)?;
        let mut arguments = args(&["commit-graph", "write", "--reachable", "--progress"]);
        if !layout.layers()?.is_empty() {
            arguments.push("--split".into());
        }
        match self.runner.history_index(
            self.cwd(),
            &arguments,
            cancel,
            progress.map(|sender| (sender, commits)),
        ) {
            Ok(output) => output.success("write commit index").map(|_| ()),
            Err(error @ (Error::Cancelled | Error::TimedOut)) => {
                before.clear_stopped_write(&layout);
                Err(error)
            }
            Err(error) => Err(error),
        }
    }

    /// Caller holds the write permit.
    fn remove_index(&self, cancel: &CancellationToken) -> Result<()> {
        let layout = self.index_layout();
        // A lock means Git is writing the index right now.
        Before::observe(&layout)?;
        cancel.check()?;
        // The chain file goes before its layers: without it Git reads none of them.
        let mut files = vec![layout.single(), layout.chain()];
        if let Ok(entries) = fs::read_dir(layout.chain_folder()) {
            files.extend(entries.filter_map(|entry| {
                let entry = entry.ok()?;
                let name = entry.file_name();
                let name = name.to_string_lossy();
                (name.starts_with("graph-") && name.ends_with(".graph")).then(|| entry.path())
            }));
        }
        for file in files {
            remove(&file).map_err(|e| Error::io("remove commit index", e))?;
        }
        // Only an empty folder goes; anything Rocket does not know stays.
        let _ = fs::remove_dir(layout.chain_folder());
        Ok(())
    }

    /// Caller holds the write permit.
    fn set_index_on_fetch(&self, on: bool, cancel: &CancellationToken) -> Result<()> {
        let observed = local_files::read_config(&self.info().common_dir.join("config"))?;
        if self.index_on_fetch(cancel)? == on {
            return Ok(());
        }
        let edit = if on {
            args(&["--replace-all", ON_FETCH_KEY, "true"])
        } else {
            args(&["--unset-all", ON_FETCH_KEY])
        };
        self.rewrite_local_config(&observed, || Ok(()), &[edit], cancel)
            .map(|_| ())
    }

    /// After a fetch that brought something: add the fetched commits to the index,
    /// where the repository has one and asks for it. The fetch has succeeded and
    /// stays so whatever happens here; a failure is recorded in the diagnostic log.
    /// Caller holds the write permit.
    pub(crate) fn update_history_index_after_fetch(&self, cancel: &CancellationToken) {
        if !self.has_history_index() {
            return;
        }
        let _ = self.runner.observe("write_result", "history-index-write", || {
            if !self.index_on_fetch(cancel)? {
                return Ok(());
            }
            self.write_index(None, None, cancel)
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layer_count_reads_the_fanout_and_refuses_other_layouts() {
        let folder = std::env::temp_dir().join(format!("rocket-history-index-{}", std::process::id()));
        fs::create_dir_all(&folder).unwrap();
        let path = folder.join("layer");
        // Header, a table of two chunks and its end, then a fanout whose last
        // entry says 70,000 commits.
        let mut bytes = b"CGPH\x01\x01\x02\x00".to_vec();
        bytes.extend(b"OIDL");
        bytes.extend(2000u64.to_be_bytes());
        bytes.extend(b"OIDF");
        bytes.extend(44u64.to_be_bytes());
        bytes.extend([0u8; 12]);
        assert_eq!(bytes.len(), 44);
        bytes.extend([0u8; 255 * 4]);
        bytes.extend(70_000u32.to_be_bytes());
        fs::write(&path, &bytes).unwrap();
        assert_eq!(layer_commits(&path), Some(70_000));
        for (at, byte) in [(0, b'X'), (4, 2)] {
            let mut other = bytes.clone();
            other[at] = byte;
            fs::write(&path, &other).unwrap();
            assert_eq!(layer_commits(&path), None);
        }
        fs::write(&path, &bytes[..bytes.len() - 1]).unwrap();
        assert_eq!(layer_commits(&path), None);
        let mut missing = bytes.clone();
        missing[20..24].copy_from_slice(b"XXXX");
        fs::write(&path, &missing).unwrap();
        assert_eq!(layer_commits(&path), None);
        fs::remove_dir_all(&folder).unwrap();
    }

    #[test]
    fn a_stopped_write_loses_its_lock_and_half_written_layer_and_nothing_older() {
        let folder =
            std::env::temp_dir().join(format!("rocket-history-index-stop-{}", std::process::id()));
        let layout = Layout { info: folder.clone() };
        fs::create_dir_all(layout.chain_folder()).unwrap();
        let read_only = |path: &Path| {
            fs::write(path, b"x").unwrap();
            let mut permissions = fs::metadata(path).unwrap().permissions();
            permissions.set_readonly(true);
            fs::set_permissions(path, permissions).unwrap();
        };
        // Left by something earlier, and the index itself: neither is the stopped write's.
        let older = layout.chain_folder().join("tmp_graph_older");
        read_only(&older);
        read_only(&layout.single());
        let before = Before::observe(&layout).unwrap();
        let ours = layout.chain_folder().join("tmp_graph_ours");
        for path in layout.locks().iter().chain([&ours]) {
            read_only(path);
        }
        // While a lock is there, a new write is refused rather than started.
        assert!(matches!(Before::observe(&layout), Err(Error::Unsupported(_))));
        before.clear_stopped_write(&layout);
        for path in layout.locks().iter().chain([&ours]) {
            assert!(!path.exists(), "{}", path.display());
        }
        assert!(older.exists() && layout.single().exists());
        assert!(Before::observe(&layout).is_ok());
        for path in [&older, &layout.single()] {
            remove(path).unwrap();
        }
        fs::remove_dir_all(&folder).unwrap();
    }
}
