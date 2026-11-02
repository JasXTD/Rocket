//! Reviewed root .gitignore editing. Never stages, untracks or deletes selected paths.
use crate::{local_files, runner::args, *};
use std::{
    fmt,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};
const LIMIT: usize = 1024 * 1024;
static NEXT: AtomicU64 = AtomicU64::new(0);
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IgnoreFailure {
    Unsupported,
    Stale,
    Unchanged,
    ReadOnly,
    Selection,
    Verification,
}
impl IgnoreFailure {
    pub fn code(self) -> &'static str {
        match self {
            Self::Unsupported => "ignore_unsupported",
            Self::Stale => "ignore_stale_preview",
            Self::Unchanged => "ignore_unchanged",
            Self::ReadOnly => "ignore_read_only",
            Self::Selection => "ignore_selection",
            Self::Verification => "ignore_verification",
        }
    }
    pub fn guidance(self) -> &'static str {
        match self {
            Self::Unsupported => {
                "Edit a normal UTF-8 root .gitignore up to 1 MiB. Conflicts, redirected files and special index flags must be resolved first."
            }
            Self::Stale => {
                "The .gitignore file or index changed since it was loaded. No reviewed replacement was applied. Reload and review your changes again."
            }
            Self::Unchanged => {
                "The proposed rules are identical to the current file. Nothing needs saving."
            }
            Self::ReadOnly => {
                "The .gitignore file is read-only. Change its permissions yourself before reviewing a save."
            }
            Self::Selection => {
                "Choose repository-relative files or folders without control characters or .git components. Exact rules are anchored at the repository root."
            }
            Self::Verification => {
                "The saved file could not be confirmed. Inspect the current .gitignore and the recovery folder before retrying."
            }
        }
    }
}
fn refusal<T>(f: IgnoreFailure) -> Result<T> {
    Err(Error::Ignore(f))
}
#[derive(Clone)]
pub struct IgnoreSnapshot {
    root: PathBuf,
    git_dir: PathBuf,
    before: Option<Vec<u8>>,
    modified: Option<SystemTime>,
    index: Vec<u8>,
    flags: Vec<u8>,
}
impl fmt::Debug for IgnoreSnapshot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("IgnoreSnapshot")
            .field("present", &self.before.is_some())
            .finish_non_exhaustive()
    }
}
impl IgnoreSnapshot {
    pub fn exists(&self) -> bool {
        self.before.is_some()
    }
    pub fn bytes(&self) -> Option<&[u8]> {
        self.before.as_deref()
    }
    pub fn text(&self) -> &str {
        std::str::from_utf8(self.before.as_deref().unwrap_or_default())
            .unwrap_or_default()
            .strip_prefix('\u{feff}')
            .unwrap_or_else(|| {
                std::str::from_utf8(self.before.as_deref().unwrap_or_default()).unwrap_or_default()
            })
    }
}
#[derive(Clone)]
pub struct IgnorePreview {
    snapshot: IgnoreSnapshot,
    after: Vec<u8>,
}
impl fmt::Debug for IgnorePreview {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("IgnorePreview")
            .field("before_bytes", &self.before().len())
            .field("after_bytes", &self.after.len())
            .finish_non_exhaustive()
    }
}
impl IgnorePreview {
    pub fn before(&self) -> &[u8] {
        self.snapshot.before.as_deref().unwrap_or_default()
    }
    pub fn after(&self) -> &[u8] {
        &self.after
    }
    pub fn existed(&self) -> bool {
        self.snapshot.exists()
    }
}
#[derive(Debug)]
#[must_use = "inspect save result, recovery and refreshed status"]
pub struct IgnoreReport {
    pub result: Result<()>,
    pub recovery: Option<PathBuf>,
    pub published: bool,
    pub restored: bool,
    pub status: Result<Status>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IgnoreTemplate {
    Rust,
    Node,
    Python,
    DotNet,
}
impl IgnoreTemplate {
    pub fn name(self) -> &'static str {
        match self {
            Self::Rust => "Rust",
            Self::Node => "Node.js",
            Self::Python => "Python",
            Self::DotNet => ".NET",
        }
    }
    /// Small, built-in starter rules, not downloaded templates.
    pub fn rules(self) -> &'static [&'static str] {
        match self {
            Self::Rust => &["# Rust build output", "/target/"],
            Self::Node => &[
                "# Node.js dependencies and build output",
                "node_modules/",
                "dist/",
                "npm-debug.log*",
            ],
            Self::Python => &[
                "# Python caches and local environment",
                "__pycache__/",
                "*.py[cod]",
                ".venv/",
            ],
            Self::DotNet => &["# .NET build output", "[Bb]in/", "[Oo]bj/", ".vs/"],
        }
    }
}
pub fn literal_ignore_rule(path: &RepoPath, directory: bool) -> Result<String> {
    let text = std::str::from_utf8(path.as_bytes())
        .map_err(|_| Error::Ignore(IgnoreFailure::Selection))?;
    if text.is_empty()
        || text.len() > 4096
        || text.chars().any(char::is_control)
        || text.contains('\\')
        || text
            .split('/')
            .any(|p| p.is_empty() || matches!(p, "." | "..") || p.eq_ignore_ascii_case(".git"))
    {
        return refusal(IgnoreFailure::Selection);
    }
    let mut out = String::from("/");
    for ch in text.chars() {
        if matches!(ch, '*' | '?' | '[' | ']' | '!' | '#' | ' ') {
            out.push('\\')
        }
        out.push(ch)
    }
    if directory {
        out.push('/')
    }
    Ok(out)
}
pub fn append_ignore_rules(before: &str, rules: &[String]) -> Result<String> {
    if rules.is_empty() {
        return refusal(IgnoreFailure::Selection);
    }
    let newline = if before.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    };
    let mut result = before.to_owned();
    if !result.is_empty() && !result.ends_with('\n') {
        result.push_str(newline)
    }
    for rule in rules {
        if rule.is_empty() || rule.contains(['\r', '\n', '\0']) {
            return refusal(IgnoreFailure::Selection);
        }
        result.push_str(rule);
        result.push_str(newline)
    }
    valid(result.as_bytes())?;
    Ok(result)
}
fn valid(bytes: &[u8]) -> Result<()> {
    if bytes.len() > LIMIT || std::str::from_utf8(bytes).is_err() || bytes.contains(&0) {
        return refusal(IgnoreFailure::Unsupported);
    }
    Ok(())
}
fn open(path: &Path, delete: bool) -> Result<Option<File>> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options
            .share_mode(if delete { 1 | 4 } else { 1 })
            .custom_flags(0x00200000);
    }
    #[cfg(not(windows))]
    let _ = delete;
    let f = match options.open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(Error::io("open ignore rules", e)),
    };
    let meta = f
        .metadata()
        .map_err(|e| Error::io("inspect ignore rules", e))?;
    if !meta.is_file() || local_files::redirected(&meta) {
        return refusal(IgnoreFailure::Unsupported);
    }
    if meta.permissions().readonly() {
        return refusal(IgnoreFailure::ReadOnly);
    }
    Ok(Some(f))
}
fn read(f: &mut File) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    f.take((LIMIT + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|e| Error::io("read ignore rules", e))?;
    valid(&bytes)?;
    Ok(bytes)
}
fn snapshot(root: &Path) -> Result<(Option<Vec<u8>>, Option<SystemTime>)> {
    let _guard = local_files::pin_directories(root, root)?;
    let path = root.join(".gitignore");
    let Some(mut f) = open(&path, false)? else {
        return Ok((None, None));
    };
    crate::review::verify_preview_handle(&f, root, &path)?;
    let modified = f
        .metadata()
        .and_then(|m| m.modified())
        .map_err(|e| Error::io("inspect ignore modification time", e))?;
    Ok((Some(read(&mut f)?), Some(modified)))
}
fn save(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut f = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|e| Error::io("create ignore recovery file", e))?;
    f.write_all(bytes)
        .and_then(|()| f.sync_all())
        .map_err(|e| Error::io("flush ignore recovery file", e))
}
impl Repository {
    pub fn ignore_snapshot(&self, c: &CancellationToken) -> Result<IgnoreSnapshot> {
        self.runner.step("ignore-read", "read_ignore_rules", || {
            self.require_worktree(c)?;
            c.check()?;
            let root = self
                .info()
                .worktree
                .as_ref()
                .ok_or(Error::WorktreeRequired)?
                .clone();
            let (before, modified) = snapshot(&root)?;
            let index = self
                .runner
                .run(self.cwd(), &args(&["ls-files", "--stage", "-z"]), vec![], c)?
                .success("ignore-read")?;
            let flags = self
                .runner
                .run(self.cwd(), &args(&["ls-files", "-v", "-z"]), vec![], c)?
                .success("ignore-read")?;
            for row in index.split(|b| *b == 0) {
                if let Some(tab) = row.iter().position(|b| *b == b'\t')
                    && row[tab + 1..].eq_ignore_ascii_case(b".gitignore")
                    && (!before.is_some()
                        || &row[tab + 1..] != b".gitignore"
                        || !(row.starts_with(b"100644 ") || row.starts_with(b"100755 "))
                        || !row[..tab].ends_with(b" 0"))
                {
                    return refusal(IgnoreFailure::Unsupported);
                }
            }
            if flags
                .split(|b| *b == 0)
                .any(|r| r.len() > 2 && r[2..].eq_ignore_ascii_case(b".gitignore") && r[0] != b'H')
            {
                return refusal(IgnoreFailure::Unsupported);
            }
            let s = IgnoreSnapshot {
                root,
                git_dir: self.info().git_dir.clone(),
                before,
                modified,
                index,
                flags,
            };
            c.check()?;
            Ok(s)
        })
    }
    fn check_ignore(&self, s: &IgnoreSnapshot, c: &CancellationToken) -> Result<()> {
        let now = self.ignore_snapshot(c)?;
        if now.root != s.root
            || now.git_dir != s.git_dir
            || now.before != s.before
            || now.modified != s.modified
            || now.index != s.index
            || now.flags != s.flags
        {
            return refusal(IgnoreFailure::Stale);
        }
        Ok(())
    }
    pub fn preview_ignore(
        &self,
        s: &IgnoreSnapshot,
        text: &str,
        c: &CancellationToken,
    ) -> Result<IgnorePreview> {
        self.runner
            .step("ignore-preview", "review_ignore_rules", || {
                self.check_ignore(s, c)?;
                valid(text.as_bytes())?;
                let mut after = text.as_bytes().to_vec();
                if s.before
                    .as_ref()
                    .is_some_and(|b| b.starts_with(b"\xef\xbb\xbf"))
                    && !after.starts_with(b"\xef\xbb\xbf")
                {
                    after.splice(..0, b"\xef\xbb\xbf".iter().copied());
                }
                valid(&after)?;
                if s.before.as_deref().unwrap_or_default() == after {
                    return refusal(IgnoreFailure::Unchanged);
                }
                Ok(IgnorePreview {
                    snapshot: s.clone(),
                    after,
                })
            })
    }
    pub fn apply_ignore(&self, p: &IgnorePreview, c: &CancellationToken) -> Result<IgnoreReport> {
        let _permit = self.acquire_write(c)?;
        self.apply_ignore_locked(p, c)
    }
    pub(crate) fn apply_ignore_locked(&self, p: &IgnorePreview, c: &CancellationToken) -> Result<IgnoreReport> {
        self.check_ignore(&p.snapshot, c)?;
        let mut recovery = None;
        let mut published = false;
        let mut restored = false;
        let result = (|| {
            let root = &p.snapshot.root;
            let target = root.join(".gitignore");
            let mut guards = local_files::pin_directories(root, root)?;
            guards.extend(local_files::pin_directories(
                &p.snapshot.git_dir,
                &p.snapshot.git_dir,
            )?);
            let base = p.snapshot.git_dir.join("rocket-ignore-backups");
            match fs::create_dir(&base) {
                Ok(()) => (),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => (),
                Err(e) => return Err(Error::io("create ignore recovery root", e)),
            };
            guards.extend(local_files::pin_directories(&p.snapshot.git_dir, &base)?);
            let id = format!(
                "{}-{}-{}",
                std::process::id(),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map_err(|_| Error::Ignore(IgnoreFailure::Verification))?
                    .as_nanos(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            );
            let workspace = base.join(id);
            fs::create_dir(&workspace)
                .map_err(|e| Error::io("create ignore recovery workspace", e))?;
            guards.extend(local_files::pin_directories(
                &p.snapshot.git_dir,
                &workspace,
            )?);
            recovery = Some(workspace.clone());
            let prepared = workspace.join("prepared.gitignore");
            let original = workspace.join("original.gitignore");
            self.runner.step("ignore-save","backup_ignore_rules",||{
                c.check()?;if let Some(bytes)=&p.snapshot.before{save(&workspace.join("before.gitignore"),bytes)?;}
                save(&workspace.join("RECOVERY.md"),b"Rocket .gitignore recovery\n\nBefore/original files contain your prior rules, including uncommitted edits. Prepared is the reviewed replacement. Inspect the current file before restoring; never overwrite later edits. Nothing was staged or untracked.\n")?;save(&prepared,&p.after)
            })?;
            self.runner
                .step("ignore-save", "publish_ignore_rules", || {
                    self.check_ignore(&p.snapshot, c)?;
                    let mut ready =
                        open(&prepared, true)?.ok_or(Error::Ignore(IgnoreFailure::Stale))?;
                    if read(&mut ready)? != p.after {
                        return refusal(IgnoreFailure::Stale);
                    }
                    let mut old = open(&target, true)?;
                    if let Some(f) = &mut old {
                        crate::review::verify_preview_handle(f, root, &target)?;
                        if Some(read(f)?) != p.snapshot.before {
                            return refusal(IgnoreFailure::Stale);
                        }
                        let meta = f
                            .metadata()
                            .map_err(|e| Error::io("inspect ignore permissions", e))?;
                        fs::set_permissions(&prepared, meta.permissions())
                            .map_err(|e| Error::io("preserve ignore permissions", e))?;
                    } else if p.snapshot.before.is_some() {
                        return refusal(IgnoreFailure::Stale);
                    }
                    c.check()?;
                    let moved = old.is_some();
                    if moved {
                        local_files::move_path(&target, &original, false)?;
                    }
                    // Include the moved-file check in rollback handling: a rename
                    // race must leave either the original in place or in recovery.
                    // Never overwrite an external file that appeared meanwhile.
                    let publish = (|| {
                        if moved {
                            let mut actual = open(&original, false)?
                                .ok_or(Error::Ignore(IgnoreFailure::Stale))?;
                            if Some(read(&mut actual)?) != p.snapshot.before {
                                return refusal(IgnoreFailure::Stale);
                            }
                        }
                        c.check()?;
                        local_files::move_path(&prepared, &target, false)
                    })();
                    if let Err(e) = publish {
                        if moved {
                            restored = local_files::move_path(&original, &target, false).is_ok();
                        }
                        return Err(e);
                    }
                    published = true;
                    Ok(())
                })?;
            self.runner.step("ignore-save", "verify_ignore_rules", || {
                if snapshot(root)?.0.as_deref() == Some(p.after()) {
                    Ok(())
                } else {
                    refusal(IgnoreFailure::Verification)
                }
            })
        })();
        let status = self.runner.observe("refresh", "status", || {
            self.status(&CancellationToken::new())
        });
        Ok(IgnoreReport {
            result,
            recovery,
            published,
            restored,
            status,
        })
    }
}
