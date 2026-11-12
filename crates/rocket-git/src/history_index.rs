//! Git's commit-graph, shown to people as the index of a repository's commits: one
//! optional file under `objects/info`, or a short chain of them, holding every
//! commit's parents and position. Git orders history several times faster with it.
//! It holds nothing the commits do not, so writing, updating or removing it changes
//! no commit, branch or file.
use crate::{runner::args, *};
use std::{
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
// At most 64 SHA-256 IDs, each followed by Git's LF terminator. Bound both the
// initial size check and the actual read: a file can grow after its metadata read.
const MAX_CHAIN_BYTES: u64 = (MAX_LAYERS * 65) as u64;

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
        let chain = match read_chain(&self.chain()) {
            Ok(chain) => chain,
            Err(Error::Io { source, .. }) if source.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(error),
        };
        let mut layers = Vec::new();
        for (number, name) in chain.split(|byte| *byte == b'\n').filter(|name| !name.is_empty()).enumerate() {
            if number == MAX_LAYERS {
                return Err(Error::parse("read commit index", "commit-graph chain exceeds 64 layers"));
            }
            // Object IDs only, so a chain file can never name a path outside its folder.
            let name = std::str::from_utf8(name).ok().filter(|name| {
                matches!(name.len(), 40 | 64) && name.bytes().all(|b| b.is_ascii_hexdigit())
            });
            let Some(name) = name else {
                return Ok(Vec::new());
            };
            let layer = self.chain_folder().join(format!("graph-{name}.graph"));
            if is_file(&layer)? {
                layers.push(layer);
            }
        }
        Ok(layers)
    }
}

fn read_chain(path: &Path) -> Result<Vec<u8>> {
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        // Refuse links and don't block on a FIFO substituted at the path.
        #[cfg(target_os = "linux")]
        options.custom_flags(0x20000 | 0x800); // O_NOFOLLOW | O_NONBLOCK
        #[cfg(target_os = "macos")]
        options.custom_flags(0x100 | 0x4);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.custom_flags(0x0020_0000); // FILE_FLAG_OPEN_REPARSE_POINT
    }
    let file = options.open(path).map_err(|e| Error::io("read commit index", e))?;
    let meta = file.metadata().map_err(|e| Error::io("inspect commit index chain", e))?;
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if meta.file_attributes() & 0x400 != 0 {
            return Err(Error::parse("read commit index", "commit-graph chain is a reparse point"));
        }
    }
    if !meta.is_file() || meta.len() > MAX_CHAIN_BYTES {
        return Err(Error::parse("read commit index", "commit-graph chain must be a regular file of at most 4160 bytes"));
    }
    read_chain_bytes(file)
}

fn read_chain_bytes(reader: impl Read) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    reader.take(MAX_CHAIN_BYTES + 1).read_to_end(&mut bytes)
        .map_err(|e| Error::io("read commit index", e))?;
    if bytes.len() as u64 > MAX_CHAIN_BYTES {
        return Err(Error::parse("read commit index", "commit-graph chain exceeds 4160 bytes"));
    }
    Ok(bytes)
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

/// This is only a preflight check, not proof of ownership of any later lock.
/// External Git processes do not participate in Rocket's repository write gate.
fn ensure_unlocked(layout: &Layout) -> Result<()> {
    for lock in layout.locks() {
        match fs::symlink_metadata(&lock) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(Error::io("inspect commit index lock", error)),
            Ok(_) => {}
        }
        // Named from the common Git directory (also correct for linked worktrees).
        let inside = Path::new("objects").join("info")
            .join(lock.strip_prefix(&layout.info).unwrap_or(&lock));
        return Err(Error::Unsupported(format!(
            "Another Git process is writing this repository's commit index, or one was interrupted. If no Git is running, delete {} in the repository's common Git directory and try again.",
            inside.display()
        )));
    }
    Ok(())
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
        ensure_unlocked(&layout)?;
        let mut arguments = args(&["commit-graph", "write", "--reachable", "--progress"]);
        if !layout.layers()?.is_empty() {
            arguments.push("--split".into());
        }
        // Never unlink locks or tmp_graph_* after stopping Git: their appearance
        // since preflight cannot prove our child created them. Keep cancellation
        // typed, and let the next attempt explain any interrupted-writer lock.
        self.runner.history_index(
            self.cwd(),
            &arguments,
            cancel,
            progress.map(|sender| (sender, commits)),
        )?.success("write commit index").map(|_| ())
    }

    /// Caller holds the write permit.
    fn remove_index(&self, cancel: &CancellationToken) -> Result<()> {
        let layout = self.index_layout();
        // A lock means Git is writing the index right now.
        ensure_unlocked(&layout)?;
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
    fn oversized_chain_is_rejected_before_parsing() {
        let folder = std::env::temp_dir().join(format!("rocket-history-chain-size-{}", std::process::id()));
        let layout = Layout { info: folder };
        fs::create_dir_all(layout.chain_folder()).unwrap();
        // Just beyond the largest supported SHA-256 chain: enough to expose the
        // missing read bound without making the failing test allocate a huge file.
        fs::write(layout.chain(), vec![b'\n'; 64 * 65 + 1]).unwrap();
        let result = layout.layers();
        fs::remove_file(layout.chain()).unwrap();
        fs::remove_dir(layout.chain_folder()).unwrap();
        fs::remove_dir(&layout.info).unwrap();
        assert!(result.is_err(), "oversized chain must report its limit");
    }

    #[test]
    fn chain_read_stops_at_the_bound_even_if_the_file_grows() {
        struct Growing(usize);
        impl Read for Growing {
            fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
                self.0 += out.len();
                out.fill(b'a');
                Ok(out.len())
            }
        }
        let mut reader = Growing(0);
        assert!(read_chain_bytes(&mut reader).is_err());
        assert_eq!(reader.0 as u64, MAX_CHAIN_BYTES + 1);
        assert_eq!(read_chain_bytes(std::io::repeat(b'a').take(MAX_CHAIN_BYTES)).unwrap().len() as u64, MAX_CHAIN_BYTES);
    }

    #[test]
    fn chain_accepts_both_hash_sizes_and_counts_missing_layers_too() {
        let folder = std::env::temp_dir().join(format!("rocket-history-chain-layers-{}", std::process::id()));
        let layout = Layout { info: folder };
        fs::create_dir_all(layout.chain_folder()).unwrap();
        for width in [40, 64] {
            let names: Vec<_> = (0..MAX_LAYERS).map(|n| format!("{n:0width$x}")).collect();
            for name in &names {
                fs::write(layout.chain_folder().join(format!("graph-{name}.graph")), b"layer").unwrap();
            }
            for ending in ["", "\n"] {
                fs::write(layout.chain(), names.join("\n") + ending).unwrap();
                assert_eq!(layout.layers().unwrap().len(), MAX_LAYERS);
            }
            for name in &names {
                fs::remove_file(layout.chain_folder().join(format!("graph-{name}.graph"))).unwrap();
            }
        }
        // SHA-1 is short enough that 65 names fit within the byte limit. Missing
        // layers must not let a chain sidestep the independent count limit.
        fs::write(layout.chain(), ("a".repeat(40) + "\n").repeat(MAX_LAYERS + 1)).unwrap();
        assert!(layout.layers().is_err());
        fs::write(layout.chain(), b"../../outside\n").unwrap();
        assert!(layout.layers().unwrap().is_empty());
        fs::remove_file(layout.chain()).unwrap();
        fs::remove_dir(layout.chain_folder()).unwrap();
        fs::remove_dir(&layout.info).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn chain_refuses_links_and_non_regular_files() {
        let folder = std::env::temp_dir().join(format!("rocket-history-chain-kind-{}", std::process::id()));
        fs::create_dir(&folder).unwrap();
        let target = folder.join("target");
        let link = folder.join("chain");
        fs::write(&target, b"keep").unwrap();
        std::os::unix::fs::symlink(&target, &link).unwrap();
        assert!(read_chain(&link).is_err());
        assert!(read_chain(&folder).is_err());
        assert_eq!(fs::read(&target).unwrap(), b"keep");
        fs::remove_file(link).unwrap();
        fs::remove_file(target).unwrap();
        fs::remove_dir(folder).unwrap();
    }

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
    fn preflight_preserves_external_locks_and_temporary_layers() {
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
        ensure_unlocked(&layout).unwrap();
        let external = layout.chain_folder().join("tmp_graph_external");
        for path in layout.locks().iter().chain([&external]) {
            read_only(path);
        }
        // While a lock is there, a new write is refused rather than started.
        assert!(matches!(ensure_unlocked(&layout), Err(Error::Unsupported(_))));
        for path in layout.locks().iter().chain([&external]) {
            assert_eq!(fs::read(path).unwrap(), b"x", "{}", path.display());
            remove(path).unwrap();
        }
        assert!(older.exists() && layout.single().exists());
        assert!(ensure_unlocked(&layout).is_ok());
        for path in [&older, &layout.single()] {
            remove(path).unwrap();
        }
        fs::remove_dir_all(&folder).unwrap();
    }
}
