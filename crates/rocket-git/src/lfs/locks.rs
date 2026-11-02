//! Verified server ownership, reviewed exact-ID writes, and conservative recovery.
use super::*;
use std::{
    collections::BTreeSet,
    fs::File,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};
mod json;
const LIMIT: usize = 1000;
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LfsLock {
    pub id: String,
    pub path: String,
    pub owner: String,
    pub locked_at: String,
    pub ours: bool,
}
#[derive(Clone, Debug)]
pub struct LfsLocks {
    pub remote: String,
    pub source: String,
    pub entries: Vec<LfsLock>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LfsLockAction {
    Lock(String),
    Unlock { lock: LfsLock, force: bool },
}
impl LfsLockAction {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Lock(_) => "lfs-lock",
            Self::Unlock { force: true, .. } => "lfs-force-unlock",
            Self::Unlock { .. } => "lfs-unlock",
        }
    }
    pub fn path(&self) -> &str {
        match self {
            Self::Lock(p) => p,
            Self::Unlock { lock, .. } => &lock.path,
        }
    }
    pub fn force(&self) -> bool {
        matches!(self, Self::Unlock { force: true, .. })
    }
}
#[derive(Clone)]
pub struct LfsLockPreview {
    action: LfsLockAction,
    remote: String,
    source: String,
    root: PathBuf,
    context: Context,
    helper: PathBuf,
    scope: Option<LfsSourceScope>,
    file: Option<LfsPointer>,
    readonly: bool,
}
impl std::fmt::Debug for LfsLockPreview {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LfsLockPreview")
            .field("operation", &self.action.label())
            .finish_non_exhaustive()
    }
}
impl LfsLockPreview {
    pub fn action(&self) -> &LfsLockAction {
        &self.action
    }
    pub fn remote(&self) -> &str {
        &self.remote
    }
    pub fn source(&self) -> &str {
        &self.source
    }
    pub fn makes_writable(&self) -> bool {
        matches!(self.action, LfsLockAction::Lock(_)) && self.readonly
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LfsLockOutcome {
    NotStarted,
    ConfirmedByHelper,
    Unknown,
}
impl LfsLockOutcome {
    pub fn code(self) -> &'static str {
        match self {
            Self::NotStarted => "not_started",
            Self::ConfirmedByHelper => "confirmed_by_helper",
            Self::Unknown => "unknown",
        }
    }
}
pub struct LfsLockReport {
    pub result: Result<()>,
    pub local: Result<()>,
    pub verified: Result<()>,
    pub observed: Result<LfsLocks>,
    pub outcome: LfsLockOutcome,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LfsLockFailure {
    Transport(FetchFailure),
    Unsupported,
    Exists,
    Missing,
    Rejected,
    Other,
}
impl LfsLockFailure {
    pub fn code(self) -> &'static str {
        match self {
            Self::Transport(f) => f.code(),
            Self::Unsupported => "lfs_lock_unsupported",
            Self::Exists => "lfs_already_locked",
            Self::Missing => "lfs_lock_missing",
            Self::Rejected => "lfs_lock_rejected",
            Self::Other => "lfs_lock_failed",
        }
    }
    pub fn guidance(self) -> &'static str {
        match self {
            Self::Transport(FetchFailure::CredentialsUnavailable) => {
                "Complete sign-in using the configured credential helper, then refresh locks and review again. For GitHub with Git Credential Manager, use git credential-manager github login --browser."
            }
            Self::Transport(FetchFailure::ConnectionInterrupted) => {
                "Check the network, proxy and remote availability. A lock request may have completed remotely; refresh server locks before reviewing another attempt."
            }
            Self::Transport(f) => f.guidance(),
            Self::Unsupported => LfsFailure::LockUnsupported.guidance(),
            Self::Exists => LfsFailure::LockExists.guidance(),
            Self::Missing => {
                "The server could not find this lock. Refresh locks before retrying; another user may have removed it."
            }
            Self::Rejected => {
                "The server rejected the lock operation. Check repository permissions and current ownership, then review again. Force unlock never bypasses server permissions."
            }
            Self::Other => {
                "The lock helper failed. Refresh server locks before retrying; a request may have completed remotely. Raw helper output remains an explicit local opt-in."
            }
        }
    }
    pub(crate) fn classify(stdout: &[u8], stderr: &[u8]) -> Self {
        let s = format!(
            "{}\n{}",
            String::from_utf8_lossy(stderr),
            String::from_utf8_lossy(stdout)
        )
        .to_ascii_lowercase();
        if s.contains("locking support")
            || s.contains("not implemented")
            || s.contains("status code: 501")
        {
            return Self::Unsupported;
        }
        if s.contains("already locked")
            || s.contains("lock already exists")
            || s.contains("status code: 409")
        {
            return Self::Exists;
        }
        if s.contains("lock not found") || s.contains("lock does not exist") {
            return Self::Missing;
        }
        if let LfsPushFailure::Transport(f) = LfsPushFailure::classify(stdout, stderr) {
            return Self::Transport(f);
        }
        if s.contains("permission")
            || s.contains("not owner")
            || s.contains("not the owner")
            || s.contains("cannot unlock")
        {
            return Self::Rejected;
        }
        Self::Other
    }
}
/// Exact loopback fixture URL, never accepted by normal production transfers.
pub fn is_lfs_lock_fixture_url(url: &str) -> bool {
    let Some(rest) = url.strip_prefix("http://127.0.0.1:") else {
        return false;
    };
    let Some((port, path)) = rest.split_once('/') else {
        return false;
    };
    let Some(token) = path
        .strip_prefix("rocket-lfs-locks-")
        .and_then(|p| p.strip_suffix(".git"))
    else {
        return false;
    };
    port.parse::<u16>().is_ok_and(|n| n > 0)
        && port.bytes().all(|b| b.is_ascii_digit())
        && token.len() == 32
        && token.bytes().all(|b| b.is_ascii_hexdigit())
}
fn clean(value: &str, max: usize) -> Result<()> {
    if value.len() > max || value.chars().any(char::is_control) {
        fail(LfsFailure::LockResponse)
    } else {
        Ok(())
    }
}
fn valid_path(path: &str) -> Result<RepoPath> {
    clean(path, 4096)?;
    if path.is_empty()
        || path.starts_with('/')
        || path.contains(['\\', ':'])
        || path.split('/').any(|p| {
            p.is_empty()
                || p == "."
                || p == ".."
                || p.eq_ignore_ascii_case(".git")
                || p.ends_with(['.', ' '])
        })
    {
        return fail(LfsFailure::Layout);
    }
    RepoPath::new(path.as_bytes().to_vec())
}
fn parse_lock(v: &json::Value, ours: bool) -> Result<LfsLock> {
    let id = v.field("id")?.string()?.to_owned();
    let path = v.field("path")?.string()?.to_owned();
    let locked_at = v.field("locked_at")?.string()?.to_owned();
    let owner = match v.field("owner") {
        Ok(json::Value::Null) | Err(_) => String::new(),
        Ok(o) => o.field("name")?.string()?.to_owned(),
    };
    clean(&id, 256)?;
    clean(&owner, 1024)?;
    clean(&locked_at, 64)?;
    valid_path(&path)?;
    if id.is_empty()
        || id.starts_with('-')
        || !id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
        || locked_at.is_empty()
    {
        return fail(LfsFailure::LockResponse);
    }
    Ok(LfsLock {
        id,
        path,
        owner,
        locked_at,
        ours,
    })
}
fn parse_list(raw: &[u8]) -> Result<Vec<LfsLock>> {
    let value = json::parse(raw)?;
    let mut out = Vec::new();
    let mut ids = BTreeSet::new();
    let mut paths = BTreeSet::new();
    for (name, ours) in [("ours", true), ("theirs", false)] {
        for v in value.field(name)?.array()? {
            let lock = parse_lock(v, ours)?;
            if !ids.insert(lock.id.clone()) || !paths.insert(lock.path.clone()) {
                return fail(LfsFailure::LockResponse);
            }
            out.push(lock);
            if out.len() > LIMIT {
                return fail(LfsFailure::LockLimit);
            }
        }
    }
    out.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(out)
}
fn text_matches(entries: &[LfsLock], raw: &[u8]) -> bool {
    let Ok(raw) = std::str::from_utf8(raw) else {
        return false;
    };
    let path_len = entries.iter().map(|e| e.path.len()).max().unwrap_or(0);
    let owner_len = entries.iter().map(|e| e.owner.len()).max().unwrap_or(0);
    let expected = entries
        .iter()
        .map(|e| {
            format!(
                "{}{}{}\t{}{}\tID:{}\n",
                if e.ours { "O " } else { "  " },
                e.path,
                " ".repeat(path_len - e.path.len()),
                e.owner,
                " ".repeat(owner_len - e.owner.len()),
                e.id
            )
        })
        .collect::<String>();
    raw.replace("\r\n", "\n") == expected
}
fn config(remote: &str, source: &str) -> Vec<(String, OsString)> {
    let mut cfg = helper::push_config(remote, source);
    // Unlock responses may carry server-provided paths. Never let those paths
    // change local permissions. Lock creation uses only our validated path.
    cfg.push(("lfs.setlockablereadonly".into(), "false".into()));
    cfg.push(("lfs.transfer.maxretries".into(), "0".into()));
    cfg
}
#[derive(Default)]
struct WorkingFile {
    content: Option<LfsPointer>,
    readonly: bool,
    _file: Option<File>,
    _directories: Vec<File>,
}
impl Repository {
    fn lock_command(
        &self,
        helper: &Path,
        command: &[OsString],
        remote: &str,
        source: &str,
        op: &'static str,
        c: &CancellationToken,
    ) -> Result<Vec<u8>> {
        let mut runner = self.runner.clone();
        runner.limits.stdout_bytes = 2 * 1024 * 1024;
        runner.limits.stderr_bytes = 256 * 1024;
        let cache = LockCache::new(self)?;
        let mut cfg = config(remote, source);
        // The helper rewrites its lock database in place. An owned ephemeral
        // store prevents existing cache links/hard links redirecting that write.
        cfg.push(("lfs.storage".into(), git_path(&cache.path)));
        let result = runner
            .lfs(helper, self.cwd(), command, &cfg, c)
            .and_then(|output| output.success(op));
        let cleanup = self
            .runner
            .step(op, "cleanup_lfs_lock_cache", || cache.close());
        match result {
            Ok(bytes) => {
                cleanup?;
                Ok(bytes)
            }
            Err(e) => Err(e),
        }
    }
    fn read_locks(
        &self,
        helper: &Path,
        remote: &str,
        source: &str,
        c: &CancellationToken,
    ) -> Result<LfsLocks> {
        self.runner.step("lfs-locks", "read_lfs_locks", || {
            let mut cmd = args(&[
                "locks",
                "--verify",
                "--json",
                "--limit=1001",
                "--remote",
                remote,
            ]);
            let raw = self.lock_command(helper, &cmd, remote, source, "lfs-locks", c)?;
            let entries = parse_list(&raw)?;
            // Git LFS 3.7.1's JSON path returns before checking search errors.
            // Its text path propagates them. Require a successful second query
            // with the identical IDs, paths, owners and ownership flags.
            cmd.retain(|s| s != "--json");
            let check = self
                .runner
                .step("lfs-locks", "verify_lfs_lock_ownership", || {
                    self.lock_command(helper, &cmd, remote, source, "lfs-locks", c)
                })?;
            if !text_matches(&entries, &check) {
                return fail(LfsFailure::LockResponse);
            }
            Ok(LfsLocks {
                remote: remote.into(),
                source: source.into(),
                entries,
            })
        })
    }
    pub fn lfs_locks(
        &self,
        remote: &str,
        scope: Option<&LfsSourceScope>,
        c: &CancellationToken,
    ) -> Result<LfsLocks> {
        // The helper updates its own cache, so serialize it with repository writes.
        let _permit = self.acquire_write(c)?;
        let before = self.lfs_context(c)?;
        let (helper, _) = helper::locate(self, c)?;
        let source = helper::lock_source(self, remote, scope, c)?;
        let list = self.read_locks(&helper, remote, &source, c)?;
        if !before.matches_after_fetch(&self.lfs_context(c)?, Some(&source)) {
            return fail(LfsFailure::LockStale);
        }
        Ok(list)
    }
    fn lock_file(&self, path: &str, helper: &Path, c: &CancellationToken) -> Result<WorkingFile> {
        let path = valid_path(path)?;
        // Missing selected files are valid when cleaning up server locks.
        let target = self.cwd().join(path.argument()?);
        if !target
            .try_exists()
            .map_err(|e| Error::io("inspect lock file", e))?
        {
            return Ok(WorkingFile::default());
        }
        let target = disk::target(self, &path)?;
        let dirs = local_files::pin_directories(
            self.cwd(),
            target.parent().ok_or(Error::Lfs(LfsFailure::Layout))?,
        )?;
        let file = disk::open(&target, false)?.ok_or(Error::Lfs(LfsFailure::LockStale))?;
        let metadata = file
            .metadata()
            .map_err(|e| Error::io("inspect lock file", e))?;
        if metadata.len() > LFS_FILE_LIMIT {
            return fail(LfsFailure::Limit);
        }
        if metadata.permissions().readonly() && !single_link(&file)? {
            return fail(LfsFailure::Layout);
        }
        let stamp = helper::hash(self, helper, &target, c)?;
        Ok(WorkingFile {
            content: Some(stamp),
            readonly: metadata.permissions().readonly(),
            _file: Some(file),
            _directories: dirs,
        })
    }
    pub fn preview_lfs_lock(
        &self,
        remote: &str,
        action: LfsLockAction,
        scope: Option<&LfsSourceScope>,
        c: &CancellationToken,
    ) -> Result<LfsLockPreview> {
        let _permit = self.acquire_write(c)?;
        self.runner.step("lfs-lock-preview", "review_lfs_lock", || {
            let context = self.lfs_context(c)?;
            if !self.operation_state(c)?.is_idle() {
                return fail(LfsFailure::Busy);
            }
            let (helper, _) = helper::locate(self, c)?;
            let source = helper::lock_source(self, remote, scope, c)?;
            let list = self.read_locks(&helper, remote, &source, c)?;
            validate_action(&action, &list)?;
            let file = self.lock_file(action.path(), &helper, c)?;
            if matches!(action, LfsLockAction::Lock(_)) {
                if file.content.is_none() {
                    return fail(LfsFailure::Modified);
                }
                let path = valid_path(action.path())?;
                if !self
                    .lfs_entries(None, &context.index, c)?
                    .iter()
                    .any(|e| e.path == path)
                {
                    return fail(LfsFailure::InvalidObject);
                }
            }
            let observed = self.lfs_context(c)?;
            if !context.matches_after_fetch(&observed, Some(&source)) {
                return fail(LfsFailure::LockStale);
            }
            Ok(LfsLockPreview {
                action,
                remote: remote.into(),
                source,
                root: self.info().git_dir.clone(),
                context: observed,
                helper,
                scope: scope.cloned(),
                file: file.content,
                readonly: file.readonly,
            })
        })
    }
    pub fn apply_lfs_lock(
        &self,
        p: &LfsLockPreview,
        c: &CancellationToken,
    ) -> Result<LfsLockReport> {
        let _permit = self.acquire_write(c)?;
        let mut outcome = LfsLockOutcome::NotStarted;
        let mut created_id = None;
        let op = p.action.label();
        let result = (|| {
            let _file_guard = self.runner.step(op, "recheck_lfs_lock", || {
                if self.info().git_dir != p.root
                    || self.lfs_context(c)? != p.context
                    || helper::lock_source(self, &p.remote, p.scope.as_ref(), c)? != p.source
                {
                    return fail(LfsFailure::LockStale);
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
                        .map_err(|e| Error::io("inspect repository lock", e))?
                    {
                        return fail(LfsFailure::Busy);
                    }
                }
                let list = self.read_locks(&p.helper, &p.remote, &p.source, c)?;
                validate_action(&p.action, &list)?;
                let file = self.lock_file(p.action.path(), &p.helper, c)?;
                if file.content != p.file
                    || file.readonly != p.readonly
                    || !p
                        .context
                        .matches_after_fetch(&self.lfs_context(c)?, Some(&p.source))
                {
                    return fail(LfsFailure::LockStale);
                }
                Ok(file)
            })?;
            c.check()?;
            let mut cmd = match &p.action {
                LfsLockAction::Lock(path) => {
                    args(&["lock", "--json", "--remote", &p.remote, "--", path])
                }
                LfsLockAction::Unlock { lock, .. } => {
                    args(&["unlock", "--json", "--remote", &p.remote, "--id", &lock.id])
                }
            };
            if p.action.force() {
                cmd.push("--force".into());
            }
            outcome = LfsLockOutcome::Unknown;
            self.runner.step(op, "send_lfs_lock", || {
                let raw = self.lock_command(&p.helper, &cmd, &p.remote, &p.source, op, c)?;
                let values = json::parse(&raw)?;
                let values = values.array()?;
                if values.len() != 1 {
                    return fail(LfsFailure::LockResponse);
                }
                match &p.action {
                    LfsLockAction::Lock(path) => {
                        let created = parse_lock(&values[0], true)?;
                        if created.path != *path {
                            return fail(LfsFailure::LockResponse);
                        }
                        created_id = Some(created.id);
                    }
                    LfsLockAction::Unlock { lock, .. } => {
                        if values[0].field("id")?.string()? != lock.id
                            || !matches!(values[0].field("unlocked")?, json::Value::Bool(true))
                        {
                            return fail(LfsFailure::LockResponse);
                        }
                    }
                }
                Ok(())
            })?;
            outcome = LfsLockOutcome::ConfirmedByHelper;
            Ok(())
        })();
        // Local read-only reconciliation is bounded and runs even after cancellation.
        // Do not send another network request after Cancel: the user refreshes explicitly.
        let observe = CancellationToken::new();
        let local = self.runner.step(op, "verify_lfs_lock_local", || {
            if !p
                .context
                .matches_after_fetch(&self.lfs_context(&observe)?, Some(&p.source))
            {
                return fail(LfsFailure::LockVerification);
            }
            let file = self.lock_file(p.action.path(), &p.helper, &observe)?;
            if file.content != p.file {
                return fail(LfsFailure::LockVerification);
            }
            Ok(())
        });
        let observed = self.runner.step(op, "refresh_lfs_locks", || {
            c.check()?;
            if helper::lock_source(self, &p.remote, p.scope.as_ref(), c)? != p.source {
                return fail(LfsFailure::LockStale);
            }
            self.read_locks(&p.helper, &p.remote, &p.source, c)
        });
        let verified = self.runner.step(op, "verify_lfs_lock_remote", || {
            if observed.as_ref().is_ok_and(|s| match &p.action {
                LfsLockAction::Lock(path) => s
                    .entries
                    .iter()
                    .any(|l| l.path == *path && l.ours && Some(&l.id) == created_id.as_ref()),
                LfsLockAction::Unlock { lock, .. } => !s.entries.iter().any(|l| l.id == lock.id),
            }) {
                Ok(())
            } else {
                fail(LfsFailure::LockVerification)
            }
        });
        if let Some(log) = &self.runner.log {
            log.lfs_lock_result(
                op,
                outcome,
                local.is_ok(),
                observed.is_ok(),
                verified.is_ok(),
            );
        }
        Ok(LfsLockReport {
            result,
            local,
            verified,
            observed,
            outcome,
        })
    }
}
fn validate_action(action: &LfsLockAction, list: &LfsLocks) -> Result<()> {
    valid_path(action.path())?;
    match action {
        LfsLockAction::Lock(path) => {
            if list.entries.iter().any(|l| l.path == *path) {
                return fail(LfsFailure::LockExists);
            }
        }
        LfsLockAction::Unlock { lock, force } => {
            if !list.entries.iter().any(|l| l == lock) {
                return fail(LfsFailure::LockStale);
            }
            if !lock.ours && !force {
                return fail(LfsFailure::LockOwned);
            }
        }
    }
    Ok(())
}
static NEXT_CACHE: AtomicU64 = AtomicU64::new(0);
struct LockCache {
    path: PathBuf,
    root: PathBuf,
    guards: Vec<File>,
    closed: bool,
}
impl LockCache {
    fn new(repo: &Repository) -> Result<Self> {
        let root = repo.info().common_dir.clone();
        local_files::directory(&root)?;
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| Error::Lfs(LfsFailure::Layout))?
            .as_nanos();
        let path = root.join(format!(
            "rocket-lfs-lock-cache-{}-{stamp}-{}",
            std::process::id(),
            NEXT_CACHE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).map_err(|e| Error::io("create isolated LFS lock cache", e))?;
        let guards = local_files::pin_directories(&root, &path)?;
        Ok(Self {
            path,
            root,
            guards,
            closed: false,
        })
    }
    fn close(mut self) -> Result<()> {
        if self.path.parent() != Some(self.root.as_path())
            || self
                .path
                .canonicalize()
                .map_err(|e| Error::io("resolve isolated LFS lock cache", e))?
                != self.path
        {
            self.closed = true;
            return fail(LfsFailure::Layout);
        }
        self.guards.clear();
        self.closed = true;
        fs::remove_dir_all(&self.path).map_err(|e| Error::io("remove isolated LFS lock cache", e))
    }
}
impl Drop for LockCache {
    fn drop(&mut self) {
        // Only the unique child is eligible. Rust removes directory links
        // themselves rather than following their targets during recursive cleanup.
        let safe = self.path.parent() == Some(self.root.as_path())
            && self
                .path
                .file_name()
                .is_some_and(|s| s.to_string_lossy().starts_with("rocket-lfs-lock-cache-"));
        self.guards.clear();
        if safe && !self.closed {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}
#[cfg(windows)]
fn single_link(file: &File) -> Result<bool> {
    use std::os::windows::io::AsRawHandle;
    #[repr(C)]
    struct Info {
        attributes: u32,
        creation: [u32; 2],
        access: [u32; 2],
        write: [u32; 2],
        volume: u32,
        size_high: u32,
        size_low: u32,
        links: u32,
        index_high: u32,
        index_low: u32,
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetFileInformationByHandle(handle: *mut std::ffi::c_void, info: *mut Info) -> i32;
    }
    let mut info = std::mem::MaybeUninit::<Info>::uninit();
    // SAFETY: File owns a live handle; Win32 initializes the repr(C) structure on success.
    if unsafe { GetFileInformationByHandle(file.as_raw_handle(), info.as_mut_ptr()) } == 0 {
        return Err(Error::io(
            "inspect LFS file links",
            std::io::Error::last_os_error(),
        ));
    }
    Ok(unsafe { info.assume_init() }.links == 1)
}
#[cfg(unix)]
fn single_link(file: &File) -> Result<bool> {
    use std::os::unix::fs::MetadataExt;
    Ok(file
        .metadata()
        .map_err(|e| Error::io("inspect LFS file links", e))?
        .nlink()
        == 1)
}
#[cfg(not(any(windows, unix)))]
fn single_link(_: &File) -> Result<bool> {
    Ok(false)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn lock_json_identity_and_ownership() {
        let raw=br#"{"ours":[{"id":"1","path":"\u7d20\u6750/a","owner":{"name":"Alex"},"locked_at":"2026-10-19T10:00:00Z"}],"theirs":[]}"#;
        let list = parse_list(raw).unwrap();
        assert!(list[0].ours);
        assert_eq!(list[0].path, "素材/a");
        assert!(text_matches(&list, "O 素材/a\tAlex\tID:1\n".as_bytes()));
        assert!(!text_matches(&list, b""));
        assert!(!text_matches(&[], b"server error"));
        assert!(parse_list(br#"{"ours":[],"theirs":null}"#).is_err());
        assert!(parse_list(br#"{"ours":[],"theirs":[],"ours":[]}"#).is_err());
    }
    #[test]
    fn exact_lock_recheck_and_force() {
        let lock = LfsLock {
            id: "1".into(),
            path: "a".into(),
            owner: "Someone".into(),
            locked_at: "now".into(),
            ours: false,
        };
        let list = LfsLocks {
            remote: "origin".into(),
            source: "https://example.invalid/r".into(),
            entries: vec![lock.clone()],
        };
        assert!(matches!(
            validate_action(
                &LfsLockAction::Unlock {
                    lock: lock.clone(),
                    force: false
                },
                &list
            ),
            Err(Error::Lfs(LfsFailure::LockOwned))
        ));
        assert!(
            validate_action(
                &LfsLockAction::Unlock {
                    lock: lock.clone(),
                    force: true
                },
                &list
            )
            .is_ok()
        );
        let mut changed = lock;
        changed.id = "2".into();
        assert!(
            validate_action(
                &LfsLockAction::Unlock {
                    lock: changed,
                    force: true
                },
                &list
            )
            .is_err()
        );
        assert!(validate_action(&LfsLockAction::Lock("a".into()), &list).is_err());
    }
    #[test]
    fn lock_fixture_boundary_and_paths() {
        assert!(is_lfs_lock_fixture_url(
            "http://127.0.0.1:1234/rocket-lfs-locks-0123456789abcdef0123456789abcdef.git"
        ));
        for url in [
            "http://localhost:1234/x",
            "http://127.0.0.1:0/rocket-lfs-locks-0123456789abcdef0123456789abcdef.git",
            "http://127.0.0.1:1@evil.invalid/x",
        ] {
            assert!(!is_lfs_lock_fixture_url(url));
        }
        for path in [
            "../a",
            ".git/config",
            "a/../b",
            "/absolute",
            "a\\b",
            "a\nb",
            "a:",
            "a/",
        ] {
            assert!(valid_path(path).is_err());
        }
    }
    #[test]
    fn lock_failures_classify_without_exposing_payloads() {
        for failure in [
            FetchFailure::CredentialsUnavailable,
            FetchFailure::ConnectionInterrupted,
        ] {
            let guidance = LfsLockFailure::Transport(failure).guidance();
            assert!(guidance.contains("locks"));
            assert!(!guidance.contains("fetch") && !guidance.contains("clone"));
        }
        for (text, code) in [
            ("PRIVATE_path already locked", "lfs_already_locked"),
            (
                "PRIVATE_host LFS locking support is not implemented",
                "lfs_lock_unsupported",
            ),
            ("PRIVATE_server lock not found", "lfs_lock_missing"),
            ("PRIVATE_token status code: 403", "access_denied"),
            ("PRIVATE_token status code: 401", "authentication_rejected"),
            ("PRIVATE_path not the owner", "lfs_lock_rejected"),
            ("PRIVATE diagnostic", "lfs_lock_failed"),
        ] {
            let failure = LfsLockFailure::classify(&[], text.as_bytes());
            assert_eq!(failure.code(), code);
            assert!(!failure.guidance().contains("PRIVATE"));
            let error = Error::Git {
                operation: "lfs-unlock",
                code: Some(2),
                stdout: Vec::new(),
                stderr: text.as_bytes().to_vec(),
            };
            assert!(!error.to_string().contains("PRIVATE"));
        }
    }
    #[test]
    fn duplicate_locks_and_excessive_lists_are_not_actionable() {
        let entry = r#"{"id":"one","path":"assets/a","owner":{"name":"A"},"locked_at":"now"}"#;
        assert!(
            parse_list(format!(r#"{{"ours":[{entry}],"theirs":[{entry}]}}"#).as_bytes()).is_err()
        );
        let second = entry.replace("one", "two");
        assert!(
            parse_list(format!(r#"{{"ours":[{entry},{second}],"theirs":[]}}"#).as_bytes()).is_err()
        );
        let entries = (0..=LIMIT)
            .map(|i| format!(r#"{{"id":"{i}","path":"a/{i}","locked_at":"now"}}"#))
            .collect::<Vec<_>>()
            .join(",");
        assert!(matches!(
            parse_list(format!(r#"{{"ours":[{entries}],"theirs":[]}}"#).as_bytes()),
            Err(Error::Lfs(LfsFailure::LockLimit))
        ));
    }
    #[test]
    fn fixture_auth_metadata_is_narrowly_recognized() {
        let url = "http://127.0.0.1:1234/rocket-lfs-locks-0123456789abcdef0123456789abcdef.git";
        let key = format!("lfs.{url}/info/lfs.access");
        assert_eq!(helper::access_key(url), Some(key.clone()));
        assert_eq!(
            helper::auth_record_key(format!("{key}\nbasic").as_bytes()),
            Some(key.as_str())
        );
        assert!(helper::auth_record_key(format!("{key}\nsecret").as_bytes()).is_none());
        assert!(
            helper::auth_record_key(b"lfs.http://example.invalid/repo.git/info/lfs.access\nbasic")
                .is_none()
        );
    }
}
