//! Explicit LFS downloads and single-file pointer checkout. No history migration.
mod attributes;
mod disk;
mod helper;
mod locks;
mod prune;
pub use prune::{LfsPrunePreview,LfsPruneReport};
mod upload;
use crate::{
    local_files, parse,
    runner::{args, git_path},
    *,
};
pub use attributes::{LfsAttributeEdit, LfsTrackingRule};
pub use locks::{
    LfsLock, LfsLockAction, LfsLockFailure, LfsLockOutcome, LfsLockPreview, LfsLockReport,
    LfsLocks, is_lfs_lock_fixture_url,
};
use std::{
    collections::BTreeMap,
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
};
pub use upload::{LfsPushFailure, LfsUploadOutcome};

pub const LFS_FILE_LIMIT: u64 = 512 * 1024 * 1024;
const ENTRY_LIMIT: usize = 2000;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LfsFailure {
    HelperMissing,
    HelperVersion,
    Configuration,
    Layout,
    Busy,
    Stale,
    Modified,
    MissingObject,
    InvalidObject,
    Limit,
    Verification,
    Attributes,
    Pattern,
    NoChange,
    ReadOnly,
    NoObjects,
    LockResponse,
    LockLimit,
    LockStale,
    LockOwned,
    LockExists,
    LockUnsupported,
    LockVerification,
}
impl LfsFailure {
    pub fn code(self) -> &'static str {
        match self {
            Self::HelperMissing => "lfs_helper_missing",
            Self::HelperVersion => "lfs_helper_version",
            Self::Configuration => "lfs_configuration",
            Self::Layout => "lfs_unsupported_layout",
            Self::Busy => "lfs_operation_active",
            Self::Stale => "lfs_stale_preview",
            Self::Modified => "lfs_modified_file",
            Self::MissingObject => "lfs_object_missing",
            Self::InvalidObject => "lfs_invalid_object",
            Self::Limit => "lfs_limit",
            Self::Verification => "lfs_verification_failed",
            Self::Attributes => "lfs_attributes_unsupported",
            Self::Pattern => "lfs_invalid_pattern",
            Self::NoChange => "lfs_rule_unchanged",
            Self::ReadOnly => "lfs_attributes_read_only",
            Self::NoObjects => "lfs_no_objects",
            Self::LockResponse => "lfs_lock_invalid_response",
            Self::LockLimit => "lfs_lock_limit",
            Self::LockStale => "lfs_lock_stale",
            Self::LockOwned => "lfs_lock_owned_by_other",
            Self::LockExists => "lfs_already_locked",
            Self::LockUnsupported => "lfs_lock_unsupported",
            Self::LockVerification => "lfs_lock_verification_failed",
        }
    }
    pub fn guidance(self) -> &'static str {
        match self {
            Self::LockResponse => {
                "The helper returned malformed, incomplete or inconsistent lock data. Refresh locks; Rocket will not infer ownership or an empty list from this response."
            }
            Self::LockLimit => {
                "This view supports up to 1,000 locks and 2 MiB of lock data. Inspect larger lock sets with Git LFS directly."
            }
            Self::LockStale => {
                "The selected lock, file, remote or repository changed after review. Refresh and review again; inspect the operation's remote outcome before retrying."
            }
            Self::LockOwned => {
                "This lock belongs to another authenticated account. A separately reviewed force unlock is required, and the server must permit it."
            }
            Self::LockExists => {
                "This path is already locked. Refresh locks to inspect its owner before taking another action."
            }
            Self::LockUnsupported => {
                "The remote does not provide the required LFS locking API, or uses an unsupported endpoint. File remotes do not provide a lock server. Use a standard HTTPS or SSH remote with locking support."
            }
            Self::LockVerification => {
                "The local or server lock post-check could not confirm the reviewed result. Inspect your file and refresh server locks before retrying. A remote change may already have completed; Rocket did not roll it back."
            }
            Self::NoObjects => {
                "The reviewed commit contains no LFS pointer objects. Commit the intended pointers before reviewing an upload."
            }
            Self::Attributes => {
                "Tracking edits require a regular UTF-8 root .gitattributes up to 1 MiB, with ordinary non-conflicted index entries. Deleted tracked files, malformed rules, links and sparse/hidden entries are preserved. Inspect the file before retrying."
            }
            Self::Pattern => {
                "Enter one repository-relative Git attributes pattern, or select literal file path. Empty, control-containing, absolute, parent-directory, macro, negative and directory-only patterns are refused."
            }
            Self::NoChange => {
                "This rule already exists or no selected LFS attributes remain. Refresh the tracking rules and review the intended rule."
            }
            Self::ReadOnly => {
                "The root .gitattributes is read-only. Rocket preserved it; change its permissions yourself if you intend to edit it."
            }
            Self::HelperMissing => {
                "Install Git LFS alongside the selected Git installation, then refresh. Rocket does not download or install helpers automatically."
            }
            Self::HelperVersion => {
                "Use Git LFS 3.8.1 or newer, which includes the October 2026 security fixes. Update Git LFS and refresh."
            }
            Self::Configuration => {
                "This first LFS workflow requires standard storage and remote endpoints. Inspect .lfsconfig, LFS endpoint/storage overrides, URL rewrite rules, custom transfers and extensions with Git LFS before retrying; Rocket does not change those settings."
            }
            Self::Layout => {
                "Use regular files and normal folders within this worktree. Links, junctions, nested repositories, sparse/hidden index entries and unsafe paths are refused."
            }
            Self::Busy => {
                "Finish the active Git operation and inspect existing locks before retrying. Rocket does not remove locks."
            }
            Self::Stale => {
                "The repository, configuration or selected file changed after review. Refresh and review again; existing edits are preserved."
            }
            Self::Modified => {
                "Checkout requires an unchanged LFS pointer matching the index. Existing content, local edits, missing files and read-only files are preserved. Restore the intended pointer with Git before reviewing again."
            }
            Self::MissingObject => {
                "A required object is not cached at its expected size. Fetch or recover the original object, then refresh and review again."
            }
            Self::InvalidObject => {
                "The pointer or cached object is invalid, unsupported, or fails its SHA-256/size check. Inspect it with Git LFS before retrying."
            }
            Self::Limit => {
                "This workflow supports 2,000 LFS entries, a 20,000-file tree/index, objects up to 512 MiB and 2 GiB per reviewed transfer. Use Git LFS directly for larger operations."
            }
            Self::Verification => {
                "Inspect the refreshed state and retained recovery files before retrying. The operation may have completed partly; cancellation is not rollback."
            }
        }
    }
}
fn fail<T>(f: LfsFailure) -> Result<T> {
    Err(Error::Lfs(f))
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LfsPointer {
    pub oid: String,
    pub size: u64,
}
impl LfsPointer {
    pub(crate) fn parse(bytes: &[u8]) -> Result<Self> {
        let s = std::str::from_utf8(bytes).map_err(|_| Error::Lfs(LfsFailure::InvalidObject))?;
        let mut lines = s.lines();
        if lines.next() != Some("version https://git-lfs.github.com/spec/v1") {
            return fail(LfsFailure::InvalidObject);
        }
        let oid = lines
            .next()
            .and_then(|s| s.strip_prefix("oid sha256:"))
            .ok_or(Error::Lfs(LfsFailure::InvalidObject))?;
        let size = lines
            .next()
            .and_then(|s| s.strip_prefix("size "))
            .ok_or(Error::Lfs(LfsFailure::InvalidObject))?;
        if oid.len() != 64
            || !oid
                .bytes()
                .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
            || size.is_empty()
            || !size.bytes().all(|c| c.is_ascii_digit())
            || lines.next().is_some()
        {
            return fail(LfsFailure::InvalidObject);
        }
        let size = size
            .parse()
            .map_err(|_| Error::Lfs(LfsFailure::InvalidObject))?;
        Ok(Self {
            oid: oid.into(),
            size,
        })
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LfsFileState {
    Pointer,
    Content,
    Missing,
    Unsupported,
}
impl LfsFileState {
    pub fn label(self) -> &'static str {
        match self {
            Self::Pointer => "Pointer",
            Self::Content => "Content / local edits",
            Self::Missing => "Missing working file",
            Self::Unsupported => "Unsupported path",
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LfsEntry {
    pub path: RepoPath,
    pub object: LfsPointer,
    pub cached: bool,
    pub state: LfsFileState,
    bytes: Vec<u8>,
}
#[derive(Clone, Debug)]
pub struct LfsStatus {
    pub version: String,
    pub head: Head,
    pub entries: Vec<LfsEntry>,
    pub remotes: Vec<String>,
    pub tracking: std::result::Result<Vec<LfsTrackingRule>, String>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LfsAction {
    Fetch(String),
    /// Offer only the unique objects in the reviewed HEAD tree, not its ancestry.
    Push(String),
    Checkout(RepoPath),
    Track {
        pattern: String,
        literal: bool,
    },
    Untrack {
        line: usize,
        expected: String,
    },
}
impl LfsAction {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Fetch(_) => "lfs-fetch",
            Self::Push(_) => "lfs-push",
            Self::Checkout(_) => "lfs-checkout",
            Self::Track { .. } => "lfs-track",
            Self::Untrack { .. } => "lfs-untrack",
        }
    }
}
#[derive(Clone, PartialEq, Eq)]
struct Context {
    head: Head,
    index: Vec<u8>,
    flags: Vec<u8>,
    config: Vec<u8>,
}
impl Context {
    fn matches_after_fetch(&self, observed: &Self, source: Option<&str>) -> bool {
        if self == observed {
            return true;
        }
        if self.head != observed.head
            || self.index != observed.index
            || self.flags != observed.flags
        {
            return false;
        }
        // Git LFS initializes a format marker and caches the selected endpoint's
        // authentication mode. Neither contains credentials. All other records,
        // including another endpoint's auth mode, must remain byte-for-byte equal.
        let before: Vec<_> = self
            .config
            .split(|b| *b == 0)
            .filter(|r| !r.is_empty())
            .collect();
        let after: Vec<_> = observed
            .config
            .split(|b| *b == 0)
            .filter(|r| !r.is_empty())
            .collect();
        let key = source.and_then(helper::access_key);
        let auth = |r: &&[u8]| {
            key.as_deref().is_some_and(|k| {
                r.split_once_byte(b'\n')
                    .is_some_and(|(name, _)| name == k.as_bytes())
            })
        };
        let format = |r: &&[u8]| r.starts_with(b"lfs.repositoryformatversion\n");
        let before_format: Vec<_> = before.iter().copied().filter(format).collect();
        let after_format: Vec<_> = after.iter().copied().filter(format).collect();
        if before_format != after_format
            && !(before_format.is_empty() && after_format == [b"lfs.repositoryformatversion\n0"])
        {
            return false;
        }
        let before_auth: Vec<_> = before.iter().copied().filter(auth).collect();
        let after_auth: Vec<_> = after.iter().copied().filter(auth).collect();
        if before_auth.len() > 1
            || after_auth.len() > 1
            || after_auth
                .iter()
                .any(|r| helper::auth_record_key(r).is_none())
        {
            return false;
        }
        before
            .iter()
            .copied()
            .filter(|r| !format(r) && !auth(r))
            .eq(after.iter().copied().filter(|r| !format(r) && !auth(r)))
    }
}
/// Development transfer boundary. Network sources must match a complete URL;
/// local sources still undergo the fixture-root and directory checks.
#[derive(Clone)]
pub struct LfsSourceScope {
    pub root: PathBuf,
    pub network_sources: Vec<String>,
}
#[derive(Clone)]
pub struct LfsPreview {
    action: LfsAction,
    head: Head,
    entries: Vec<LfsEntry>,
    source: Option<String>,
    local_source: Option<PathBuf>,
    root: PathBuf,
    context: Context,
    helper: PathBuf,
    source_scope: Option<LfsSourceScope>,
    attributes: Option<LfsAttributeEdit>,
}
impl LfsPreview {
    pub fn action(&self) -> &LfsAction {
        &self.action
    }
    pub fn attributes(&self) -> Option<&LfsAttributeEdit> {
        self.attributes.as_ref()
    }
    pub fn head(&self) -> &Head {
        &self.head
    }
    pub fn entries(&self) -> &[LfsEntry] {
        &self.entries
    }
    pub fn source(&self) -> Option<&str> {
        self.source.as_deref()
    }
    pub fn local_source(&self) -> Option<&Path> {
        self.local_source.as_deref()
    }
    pub fn object_count(&self) -> usize {
        self.entries
            .iter()
            .map(|e| &e.object.oid)
            .collect::<std::collections::BTreeSet<_>>()
            .len()
    }
    pub fn total_bytes(&self) -> u64 {
        self.entries
            .iter()
            .map(|e| (&e.object.oid, e.object.size))
            .collect::<BTreeMap<_, _>>()
            .values()
            .sum()
    }
}
impl std::fmt::Debug for LfsPreview {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LfsPreview")
            .field("operation", &self.action.label())
            .finish_non_exhaustive()
    }
}
pub struct LfsWriteReport {
    pub result: Result<()>,
    pub verification: Result<()>,
    pub observed: Result<LfsStatus>,
    pub recovery_directory: Option<PathBuf>,
    pub completed_steps: usize,
    /// None for operations that do not upload. This is separate from local verification.
    pub upload_outcome: Option<LfsUploadOutcome>,
}

impl Repository {
    pub(crate) fn conflict_lfs_content(&self, pointer: &LfsPointer, c: &CancellationToken) -> Result<Vec<u8>> {
        self.lfs_context(c)?;
        let (helper, _) = helper::locate(self, c)?;
        disk::conflict_content(self, &helper, pointer, c)
    }
    fn lfs_read(&self, a: &[OsString], input: Vec<u8>, c: &CancellationToken) -> Result<Vec<u8>> {
        self.runner
            .run(self.cwd(), a, input, c)?
            .success("lfs-status")
    }
    fn lfs_context(&self, c: &CancellationToken) -> Result<Context> {
        self.require_worktree(c)?;
        let head = self.head(c)?;
        let index = self.lfs_read(&args(&["ls-files", "--stage", "-z"]), Vec::new(), c)?;
        if index
            .split(|b| *b == 0)
            .any(|r| r.ends_with(b"\t.lfsconfig"))
        {
            return fail(LfsFailure::Configuration);
        }
        if let Some(id) = head_id(&head)
            && !self
                .lfs_read(
                    &args(&["ls-tree", "-z", id.as_str(), "--", ".lfsconfig"]),
                    Vec::new(),
                    c,
                )?
                .is_empty()
        {
            return fail(LfsFailure::Configuration);
        }
        let config = self.lfs_read(
            &args(&["config", "--null", "--list", "--includes"]),
            Vec::new(),
            c,
        )?;
        helper::validate_config(self, &config)?;
        Ok(Context {
            head,
            index,
            flags: self.lfs_read(&args(&["ls-files", "-v", "-z"]), Vec::new(), c)?,
            config,
        })
    }
    pub fn lfs_status(&self, c: &CancellationToken) -> Result<LfsStatus> {
        self.runner.step("lfs-status", "inspect_lfs", || {
            let context = self.lfs_context(c)?;
            let (_, version) = helper::locate(self, c)?;
            let entries = self.lfs_entries(None, &context.index, c)?;
            let remotes = self
                .remotes(c)?
                .into_iter()
                .filter_map(|r| String::from_utf8(r.name).ok())
                .collect();
            c.check()?;
            Ok(LfsStatus {
                version,
                head: context.head.clone(),
                entries,
                remotes,
                tracking: self
                    .runner
                    .step("lfs-status", "inspect_lfs_attributes", || {
                        attributes::list(self, &context)
                    })
                    .map_err(|e| e.to_string()),
            })
        })
    }
    fn lfs_entries(
        &self,
        head: Option<&ObjectId>,
        index: &[u8],
        c: &CancellationToken,
    ) -> Result<Vec<LfsEntry>> {
        let tree;
        let records = if let Some(id) = head {
            tree = self.lfs_read(
                &args(&["ls-tree", "-rz", "--full-tree", id.as_str()]),
                Vec::new(),
                c,
            )?;
            &tree[..]
        } else {
            index
        };
        let mut paths = BTreeMap::new();
        for (n, record) in records
            .split(|b| *b == 0)
            .filter(|r| !r.is_empty())
            .enumerate()
        {
            c.check()?;
            if n >= 20000 {
                return fail(LfsFailure::Limit);
            }
            if record.ends_with(b"\t.lfsconfig") {
                return fail(LfsFailure::Configuration);
            }
            let (meta, path) = record
                .split_once_byte(b'\t')
                .ok_or_else(|| Error::parse("lfs-status", "invalid file record"))?;
            let fields: Vec<_> = meta.split(|b| *b == b' ').collect();
            if fields.len() != 3 {
                return fail(LfsFailure::InvalidObject);
            }
            if !matches!(fields[0], b"100644" | b"100755") {
                continue;
            }
            if head.is_none() && fields[2] != b"0" {
                return fail(LfsFailure::Busy);
            }
            let id = parse::oid(
                if head.is_some() { fields[2] } else { fields[1] },
                "lfs-status",
            )?;
            RepoPath::new(path.to_vec())?;
            paths.insert(path.to_vec(), id);
        }
        if paths.is_empty() {
            return Ok(Vec::new());
        }
        // Fetch scans pointers in the whole commit, including pointers without
        // attributes. Use that same scope for its preview so no object is hidden.
        let input = paths
            .values()
            .map(|id| format!("{id}\n"))
            .collect::<String>()
            .into_bytes();
        let checked = self.lfs_read(&args(&["cat-file", "--batch-check"]), input, c)?;
        let headers = checked
            .split(|b| *b == b'\n')
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>();
        if headers.len() != paths.len() {
            return fail(LfsFailure::InvalidObject);
        }
        let mut selected = Vec::new();
        let mut sizes = Vec::new();
        for ((path, id), record) in paths.into_iter().zip(headers) {
            let f: Vec<_> = record.split(|b| *b == b' ').collect();
            if f.len() != 3 || f[0] != id.as_str().as_bytes() || f[1] != b"blob" {
                return fail(LfsFailure::InvalidObject);
            }
            let size = std::str::from_utf8(f[2])
                .ok()
                .and_then(|s| s.parse::<usize>().ok())
                .ok_or(Error::Lfs(LfsFailure::InvalidObject))?;
            if size <= 1024 {
                selected.push((RepoPath::new(path)?, id));
                sizes.push(record);
            }
        }
        if selected.is_empty() {
            return Ok(Vec::new());
        }
        let input = selected
            .iter()
            .map(|(_, id)| format!("{id}\n"))
            .collect::<String>()
            .into_bytes();
        let data = self.lfs_read(&args(&["cat-file", "--batch"]), input, c)?;
        let mut rest = &data[..];
        let mut out = Vec::new();
        for ((path, _), header) in selected.into_iter().zip(sizes) {
            c.check()?;
            let (line, tail) = rest
                .split_once_byte(b'\n')
                .ok_or(Error::Lfs(LfsFailure::InvalidObject))?;
            if line != header {
                return fail(LfsFailure::InvalidObject);
            }
            let size = std::str::from_utf8(header.rsplit(|b| *b == b' ').next().unwrap())
                .unwrap()
                .parse::<usize>()
                .unwrap();
            if tail.len() < size + 1 || tail[size] != b'\n' {
                return fail(LfsFailure::InvalidObject);
            }
            let bytes = tail[..size].to_vec();
            rest = &tail[size + 1..];
            // Git LFS accepts extension lines before the version and trims
            // Unicode whitespace. Detect those too, then refuse unsupported
            // forms explicitly so fetch cannot download an unreviewed object.
            let candidate = String::from_utf8_lossy(&bytes).lines().any(|line| {
                let mut fields = line.trim_start().split_ascii_whitespace();
                fields.next() == Some("version")
                    && fields.next().is_some_and(|s| {
                        s.starts_with("https://git-lfs.github.com/spec/")
                            || s.starts_with("https://hawser.github.com/spec/")
                            || s.starts_with("http://git-media.io/")
                    })
            });
            if !candidate {
                continue;
            }
            let object = LfsPointer::parse(&bytes)?;
            if out.len() >= ENTRY_LIMIT {
                return fail(LfsFailure::Limit);
            }
            let cached = disk::cached(self, &object)?;
            let state = match disk::pointer_bytes(self, &path) {
                Ok(Some(b)) if b == bytes => LfsFileState::Pointer,
                Ok(Some(_)) => LfsFileState::Content,
                Ok(None) => LfsFileState::Missing,
                Err(_) => LfsFileState::Unsupported,
            };
            out.push(LfsEntry {
                path,
                object,
                cached,
                state,
                bytes,
            });
        }
        if !rest.is_empty() {
            return fail(LfsFailure::InvalidObject);
        }
        Ok(out)
    }
    pub fn preview_lfs(
        &self,
        action: LfsAction,
        fixture_root: Option<&Path>,
        c: &CancellationToken,
    ) -> Result<LfsPreview> {
        let scope = fixture_root.map(|root| LfsSourceScope {
            root: root.to_path_buf(),
            network_sources: Vec::new(),
        });
        self.preview_lfs_scoped(action, scope.as_ref(), c)
    }
    pub fn preview_lfs_scoped(
        &self,
        action: LfsAction,
        source_scope: Option<&LfsSourceScope>,
        c: &CancellationToken,
    ) -> Result<LfsPreview> {
        self.runner.step("lfs-preview", "review_lfs", || {
            let context = self.lfs_context(c)?;
            let (helper, _) = helper::locate(self, c)?;
            if !self.operation_state(c)?.is_idle() {
                return fail(LfsFailure::Busy);
            }
            let mut attributes = None;
            let (entries, source, local_source) = match &action {
                LfsAction::Fetch(remote) => {
                    let (source, local) = helper::source(self, remote, source_scope, c)?;
                    let id = head_id(&context.head).ok_or(Error::Lfs(LfsFailure::InvalidObject))?;
                    (
                        self.lfs_entries(Some(id), &context.index, c)?,
                        Some(source),
                        local,
                    )
                }
                LfsAction::Push(remote) => {
                    let (destination, local) = helper::destination(self, remote, source_scope, c)?;
                    let id = head_id(&context.head).ok_or(Error::Lfs(LfsFailure::NoObjects))?;
                    let entries = self.lfs_entries(Some(id), &context.index, c)?;
                    self.runner.step("lfs-preview", "validate_lfs_upload", || {
                        upload::validate(self, &helper, &entries, None, c)
                    })?;
                    (entries, Some(destination), local)
                }
                LfsAction::Checkout(path) => {
                    let entry = self
                        .lfs_entries(None, &context.index, c)?
                        .into_iter()
                        .find(|e| &e.path == path)
                        .ok_or(Error::Lfs(LfsFailure::InvalidObject))?;
                    if context
                        .flags
                        .split(|b| *b == 0)
                        .any(|r| r.len() > 2 && r[2..] == *path.as_bytes() && r[0] != b'H')
                    {
                        return fail(LfsFailure::Layout);
                    }
                    if entry.state != LfsFileState::Pointer {
                        return fail(LfsFailure::Modified);
                    }
                    if !entry.cached {
                        return fail(LfsFailure::MissingObject);
                    }
                    if entry.object.size > LFS_FILE_LIMIT {
                        return fail(LfsFailure::Limit);
                    }
                    disk::verify_cached(self, &helper, &entry.object, c)?;
                    (vec![entry], None, None)
                }
                LfsAction::Track { .. } | LfsAction::Untrack { .. } => {
                    attributes = Some(attributes::preview(self, &context, &action, c)?);
                    (Vec::new(), None, None)
                }
            };
            if entries.iter().any(|e| e.object.size > LFS_FILE_LIMIT)
                || (!matches!(action, LfsAction::Push(_))
                    && entries
                        .iter()
                        .try_fold(0u64, |total, e| total.checked_add(e.object.size))
                        .is_none_or(|n| n > 2 * 1024 * 1024 * 1024))
            {
                return fail(LfsFailure::Limit);
            }
            if self.lfs_context(c)? != context {
                return fail(LfsFailure::Stale);
            }
            Ok(LfsPreview {
                action,
                head: context.head.clone(),
                entries,
                source,
                local_source,
                root: self.info().git_dir.clone(),
                context,
                helper,
                source_scope: source_scope.cloned(),
                attributes,
            })
        })
    }
    fn lfs_recheck(&self, p: &LfsPreview, c: &CancellationToken) -> Result<()> {
        if p.root != self.info().git_dir || self.lfs_context(c)? != p.context {
            return fail(LfsFailure::Stale);
        }
        if !self.operation_state(c)?.is_idle() {
            return fail(LfsFailure::Busy);
        }
        for name in ["index.lock", "HEAD.lock"] {
            if self
                .info()
                .git_dir
                .join(name)
                .try_exists()
                .map_err(|e| Error::io("inspect LFS operation lock", e))?
            {
                return fail(LfsFailure::Busy);
            }
        }
        if let LfsAction::Checkout(path) = &p.action
            && disk::pointer_bytes(self, path)?.as_ref() != Some(&p.entries[0].bytes)
        {
            return fail(LfsFailure::Stale);
        }
        if let Some(edit) = &p.attributes {
            attributes::recheck(self, edit)?;
        }
        c.check()
    }
    pub fn apply_lfs(&self, p: &LfsPreview, c: &CancellationToken) -> Result<LfsWriteReport> {
        self.apply_lfs_with_progress(p, None, c)
    }
    pub fn apply_lfs_with_progress(
        &self,
        p: &LfsPreview,
        progress: Option<&std::sync::mpsc::SyncSender<FetchProgress>>,
        c: &CancellationToken,
    ) -> Result<LfsWriteReport> {
        let _permit = self.acquire_write(c)?;
        let mut recovery = None;
        let mut completed = 0;
        let mut upload_outcome =
            matches!(p.action, LfsAction::Push(_)).then_some(LfsUploadOutcome::NotStarted);
        let result = (|| {
            self.runner
                .step(p.action.label(), "recheck_lfs", || self.lfs_recheck(p, c))?;
            match &p.action {
                LfsAction::Push(remote) => {
                    upload::apply(self, p, remote, progress, c, &mut upload_outcome)?;
                    completed = 1;
                }
                LfsAction::Fetch(remote) => {
                    let (source, local) = helper::source(self, remote, p.source_scope.as_ref(), c)?;
                    if Some(&source) != p.source.as_ref() || local != p.local_source {
                        return fail(LfsFailure::Stale);
                    }
                    let id = head_id(&p.head).ok_or(Error::Lfs(LfsFailure::InvalidObject))?;
                    self.runner.step("lfs-fetch", "download_lfs", || {
                        let _source_guards = if let Some(path) = &local {
                            let source_repo =
                                GitClient::from_runner(self.runner.clone()).open(path, c)?;
                            disk::pin_source(&source_repo, &p.entries, c)?
                        } else {
                            Vec::new()
                        };
                        let _cache_guards = disk::prepare_store(self, &p.entries)?;
                        let config = helper::config(Some((remote, &source)));
                        self.runner
                            .lfs(
                                &p.helper,
                                self.cwd(),
                                &args(&["fetch", "--include=", "--exclude=", remote, id.as_str()]),
                                &config,
                                c,
                            )?
                            .success("lfs-fetch")?;
                        Ok(())
                    })?;
                    completed = 1;
                }
                LfsAction::Checkout(_) => {
                    self.runner.step("lfs-checkout", "publish_lfs", || {
                        disk::checkout(self, p, c, &mut recovery, &mut completed)
                    })?
                }
                LfsAction::Track { .. } | LfsAction::Untrack { .. } => {
                    attributes::apply(self, p, c, &mut recovery, &mut completed)?;
                }
            }
            Ok(())
        })();
        if let (Some(outcome), Some(log)) = (upload_outcome, &self.runner.log) {
            log.lfs_upload_result(outcome, p.object_count(), p.total_bytes());
        }
        let observe = CancellationToken::new();
        let verification = self.runner.step(p.action.label(), "verify_lfs", || {
            if matches!(result, Err(Error::Cancelled | Error::TimedOut))
                && !matches!(p.action, LfsAction::Push(_))
            {
                return fail(LfsFailure::Verification);
            }
            let context = self.lfs_context(&observe)?;
            let unchanged = match p.action {
                LfsAction::Fetch(_) | LfsAction::Push(_) => {
                    p.context.matches_after_fetch(&context, p.source())
                }
                _ => p.context == context,
            };
            if !unchanged {
                return fail(LfsFailure::Verification);
            }
            for e in &p.entries {
                match p.action {
                    LfsAction::Fetch(_) => {
                        disk::verify_cached(self, &p.helper, &e.object, &observe)?
                    }
                    LfsAction::Checkout(_) => disk::verify_working(self, &p.helper, e, &observe)?,
                    _ => (),
                }
            }
            if let Some(edit) = &p.attributes {
                attributes::verify(self, edit)?;
            }
            Ok(())
        });
        let observed = self.runner.step(p.action.label(), "refresh_lfs", || {
            self.lfs_status(&observe)
        });
        Ok(LfsWriteReport {
            result,
            verification,
            observed,
            recovery_directory: recovery,
            completed_steps: completed,
            upload_outcome,
        })
    }
}

trait SplitByte {
    fn split_once_byte(&self, byte: u8) -> Option<(&[u8], &[u8])>;
}
impl SplitByte for [u8] {
    fn split_once_byte(&self, b: u8) -> Option<(&[u8], &[u8])> {
        let n = self.iter().position(|x| *x == b)?;
        Some((&self[..n], &self[n + 1..]))
    }
}

fn head_id(head: &Head) -> Option<&ObjectId> {
    match head {
        Head::Branch { target, .. } | Head::Detached { target } => Some(target),
        Head::Unborn { .. } => None,
    }
}

#[cfg(test)]
mod auth_metadata_tests {
    use super::*;
    fn context(records: &[&str]) -> Context {
        Context {
            head: Head::Unborn {
                branch: b"main".to_vec(),
            },
            index: b"index".to_vec(),
            flags: b"flags".to_vec(),
            config: records.iter().flat_map(|s| s.bytes().chain([0])).collect(),
        }
    }
    #[test]
    fn accepts_only_selected_endpoint_metadata_changes() {
        let source = "https://example.invalid/owner/repo.git";
        let access = "lfs.https://example.invalid/owner/repo.git/info/lfs.access\nbasic";
        let before = context(&["user.name\nFixture"]);
        let after = context(&[
            "user.name\nFixture",
            "lfs.repositoryformatversion\n0",
            access,
        ]);
        assert!(before.matches_after_fetch(&after, Some(source)));
        assert!(after.matches_after_fetch(&after, Some(source)));
        assert!(after.matches_after_fetch(
            &context(&["user.name\nFixture", "lfs.repositoryformatversion\n0"]),
            Some(source)
        ));
        for records in [
            vec!["user.name\nChanged", access],
            vec!["user.name\nFixture", access, access],
            vec![
                "user.name\nFixture",
                "lfs.repositoryformatversion\n1",
                access,
            ],
            vec![
                "user.name\nFixture",
                "lfs.https://other.invalid/repo.git/info/lfs.access\nbasic",
            ],
            vec![
                "user.name\nFixture",
                "lfs.https://example.invalid/owner/repo.git/info/lfs.access\nunknown",
            ],
        ] {
            assert!(
                !before.matches_after_fetch(&context(&records), Some(source)),
                "{records:?}"
            );
        }
        assert!(!before.matches_after_fetch(&after, None));
        let mut changed_index = after.clone();
        changed_index.index.push(1);
        assert!(!before.matches_after_fetch(&changed_index, Some(source)));
    }
    #[test]
    fn standard_ssh_and_https_remotes_have_the_same_auth_endpoint() {
        let expected = Some("lfs.https://example.invalid/owner/repo.git/info/lfs.access".into());
        for source in [
            "https://example.invalid/owner/repo.git",
            "https://example.invalid/owner/repo/",
            "git@example.invalid:owner/repo.git",
            "ssh://git@example.invalid:2222/owner/repo.git",
        ] {
            assert_eq!(helper::access_key(source), expected, "{source}");
        }
        for source in [
            "https://user:secret@example.invalid/repo",
            "https://example.invalid/repo?redirect=1",
            "C:\\local\\repo",
        ] {
            assert!(helper::access_key(source).is_none(), "{source}");
        }
    }
    #[test]
    fn auth_metadata_contains_only_a_recognized_mode() {
        for mode in ["none", "basic", "private", "negotiate", "BASIC"] {
            assert!(
                helper::auth_record_key(
                    format!("lfs.https://example.invalid/repo.git/info/lfs.access\n{mode}")
                        .as_bytes()
                )
                .is_some()
            );
        }
        for record in [
            "lfs.https://example.invalid/repo.git/info/lfs.access\nunknown",
            "lfs.https://user:secret@example.invalid/repo.access\nbasic",
            "lfs.https://example.invalid/repo.url\nbasic",
            "lfs.http://example.invalid/repo.access\nbasic",
            "lfs.https://example.invalid/repo.access\nbasic\nextra",
        ] {
            assert!(
                helper::auth_record_key(record.as_bytes()).is_none(),
                "{record}"
            );
        }
    }
}
