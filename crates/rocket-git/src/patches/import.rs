//! Reviewed mail imports. Git owns replay; Rocket pins the input and records each
//! paused state so Continue/Skip cannot consume externally edited mail or options.
use super::{files, *};
use crate::{commit_actions::WorkingFile, local_files, parse as git_parse};
use std::{
    collections::BTreeMap,
    fs,
    time::{SystemTime, UNIX_EPOCH},
};
mod display;
pub use display::{MailImportDisplay, visible_path};
const JOURNAL: &str = "rocket-mail-import";
const LIMIT: usize = 64 * 1024 * 1024;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MailImportAction {
    Start,
    Continue,
    Skip,
    Abort,
}
impl MailImportAction {
    pub fn label(self) -> &'static str {
        match self {
            Self::Start => "series-import",
            Self::Continue => "series-continue",
            Self::Skip => "series-skip",
            Self::Abort => "series-abort",
        }
    }
    fn operation(self) -> WriteOperation {
        match self {
            Self::Start => WriteOperation::ImportSeries,
            Self::Continue => WriteOperation::ContinueSeries,
            Self::Skip => WriteOperation::SkipSeries,
            Self::Abort => WriteOperation::AbortSeries,
        }
    }
}
#[derive(Clone)]
pub struct MailImportEntry {
    pub filename: String,
    /// The commit the patch was made from, as its mail names it.
    pub source: String,
    pub subject: String,
    pub author: String,
    pub date: String,
    pub paths: Vec<RepoPath>,
    pub patch: Vec<u8>,
    pub display: MailImportDisplay,
    bytes: Vec<u8>,
    message: Vec<u8>,
    scan_files: Vec<(Vec<u8>, Vec<u8>)>,
}
impl MailImportEntry {
    /// The patch file's size.
    pub fn size(&self) -> usize {
        self.bytes.len()
    }
}
impl fmt::Debug for MailImportEntry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MailImportEntry")
            .field("files", &self.paths.len())
            .finish_non_exhaustive()
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
struct Context {
    status: Status,
    state: OperationState,
    index: Vec<u8>,
    config: Vec<u8>,
    files: Vec<(RepoPath, WorkingFile)>,
    metadata: BTreeMap<String, Vec<u8>>,
}
#[derive(Clone)]
pub struct MailImportPreview {
    repository: PathBuf,
    action: MailImportAction,
    source: Option<PathBuf>,
    context: Context,
    original: ObjectId,
    branch: Vec<u8>,
    branch_display: crate::display_text::DisplayText,
    entries: Vec<MailImportEntry>,
    journal: Option<Vec<u8>>,
    current: usize,
    truncation_acknowledged: bool,
    /// Continue: the staged fix, as a diff against HEAD.
    staged: Vec<u8>,
    /// Abort: the subjects of the commits it takes off, oldest first.
    imported: Vec<String>,
}
impl fmt::Debug for MailImportPreview {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MailImportPreview")
            .field("action", &self.action)
            .field("entries", &self.entries.len())
            .field("current", &self.current)
            .finish_non_exhaustive()
    }
}
impl MailImportPreview {
    pub fn action(&self) -> MailImportAction {
        self.action
    }
    pub fn entries(&self) -> &[MailImportEntry] {
        &self.entries
    }
    pub fn original(&self) -> &ObjectId {
        &self.original
    }
    pub fn branch(&self) -> &[u8] {
        &self.branch
    }
    pub fn branch_display(&self) -> &str {
        &self.branch_display.text
    }
    pub fn requires_truncation_acknowledgement(&self) -> bool {
        if self.action == MailImportAction::Abort {
            return false;
        }
        let first = if self.action == MailImportAction::Skip { self.current } else { self.current - 1 };
        self.branch_display.truncated || self.entries[first..].iter().any(|e| e.display.limited)
    }
    pub fn truncation_acknowledged(&self) -> bool {
        self.truncation_acknowledged
    }
    /// Consent belongs to this immutable source snapshot. A refreshed review starts unaccepted.
    /// Apply still compares every source byte (and recovery state) before writing.
    pub fn acknowledge_truncation(&mut self, accepted: bool) {
        self.truncation_acknowledged = accepted;
    }
    pub fn current(&self) -> usize {
        self.current
    }
    pub fn recovery_reference(&self) -> String {
        reference(&self.original)
    }
    pub fn changed_files(&self) -> usize {
        self.context.files.len()
    }
    /// Continue: the staged fix it saves as the current patch. Skip and Abort: the edits that go.
    pub fn staged(&self) -> &[u8] {
        &self.staged
    }
    /// The commits this import has made so far, which Abort takes off. A patch that was
    /// skipped isn't one of them.
    pub fn imported(&self) -> &[String] {
        &self.imported
    }
    /// Tracked files with changes, each with whether it still has a conflict.
    pub fn changes(&self) -> Vec<(String, bool)> {
        self.context
            .status
            .entries
            .iter()
            .filter(|e| !matches!(e.kind, EntryKind::Untracked | EntryKind::Ignored))
            .map(|e| (e.path.display_lossy().into_owned(), e.kind == EntryKind::Unmerged))
            .collect()
    }
}
#[derive(Debug)]
pub struct MailImportReport {
    pub write: WriteReport,
    pub state: Result<OperationState>,
    pub recovery_reference: String,
    pub backup: Option<PathBuf>,
    pub journal_result: Result<()>,
}
fn bad(code: &str, message: &str) -> Error {
    Error::InvalidInput(format!("series_{code}: {message}"))
}
fn reference(id: &ObjectId) -> String {
    format!("refs/rocket/recovery/mail-import/{id}")
}
fn field(out: &mut Vec<u8>, b: &[u8]) {
    out.extend((b.len() as u32).to_le_bytes());
    out.extend(b);
}
fn take<'a>(input: &mut &'a [u8]) -> Result<&'a [u8]> {
    let (n, rest) = input.split_at_checked(4).ok_or_else(|| {
        bad(
            "journal",
            "Incomplete import recovery record; original tip and Git state are retained.",
        )
    })?;
    let (value, rest) = rest
        .split_at_checked(u32::from_le_bytes(n.try_into().unwrap()) as usize)
        .ok_or_else(|| bad("journal", "Invalid import recovery record."))?;
    *input = rest;
    Ok(value)
}
fn names(folder: &Path, c: &CancellationToken) -> Result<Vec<PathBuf>> {
    let _guard = files::pin_parent(&folder.join("entry"))?;
    let mut list = Vec::new();
    for e in fs::read_dir(folder).map_err(|e| Error::io("read patch series folder", e))? {
        c.check()?;
        let e = e.map_err(|e| Error::io("read patch filename", e))?;
        let path = e.path();
        if path
            .extension()
            .is_some_and(|s| s.eq_ignore_ascii_case("patch"))
        {
            list.push(path);
            if list.len() > 64 {
                return Err(bad(
                    "limit",
                    "Too many patches\nA series holds up to 64 numbered .patch files.",
                ));
            }
        }
    }
    list.sort();
    if list.is_empty() {
        return Err(bad(
            "empty",
            "No patches in this folder\nChoose a folder of numbered .patch files, like one from Export commit series.",
        ));
    }
    for (i, p) in list.iter().enumerate() {
        let name = p
            .file_name()
            .and_then(|s| s.to_str())
            .ok_or_else(|| bad("name", "Patch filenames must be Unicode."))?;
        let n = name.split('-').next().and_then(|s| s.parse::<usize>().ok());
        if n != Some(i + 1) {
            return Err(bad(
                "order",
                "These patches aren't numbered in order\nThey have to run from 0001 with no gaps and no cover letter.",
            ));
        }
    }
    Ok(list)
}
fn encode(p: &MailImportPreview, metadata: &BTreeMap<String, Vec<u8>>) -> Vec<u8> {
    let mut b = b"ROCKET-MAIL-1\0".to_vec();
    field(&mut b, p.original.as_str().as_bytes());
    field(&mut b, &p.branch);
    b.extend((p.entries.len() as u32).to_le_bytes());
    for e in &p.entries {
        field(&mut b, e.filename.as_bytes());
        field(&mut b, &e.bytes);
    }
    b.extend((metadata.len() as u32).to_le_bytes());
    for (k, v) in metadata {
        field(&mut b, k.as_bytes());
        field(&mut b, v);
    }
    b
}
fn count(input: &mut &[u8], max: usize) -> Result<usize> {
    let (n, r) = input
        .split_at_checked(4)
        .ok_or_else(|| bad("journal", "Incomplete recovery record."))?;
    *input = r;
    let n = u32::from_le_bytes(n.try_into().unwrap()) as usize;
    if n > max {
        return Err(bad("journal", "Recovery record exceeds its limit."));
    }
    Ok(n)
}
fn publish_journal(repo: &Repository, expected: Option<&[u8]>, bytes: &[u8]) -> Result<()> {
    let c = CancellationToken::new();
    let path = repo.info().git_dir.join(JOURNAL);
    let _guard = files::pin_parent(&path)?;
    if bytes.len() > 3 * LIMIT {
        return Err(bad("limit", "Recovery metadata exceeds the safety limit."));
    }
    if let Some(expected) = expected {
        if files::read_bounded(&path, 3 * LIMIT, &c)? != expected {
            return Err(bad(
                "journal_changed",
                "The import recovery record changed. Inspect the retained Git state before retrying.",
            ));
        }
        let temporary = path.with_extension(format!(
            "tmp-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        files::write_new(&temporary, bytes, &c)?;
        fs::rename(&temporary, &path).map_err(|e| Error::io("publish import recovery record", e))
    } else {
        files::write_new(&path, bytes, &c)
    }
}
impl Repository {
    fn mail_read(&self, args_: &[&str], c: &CancellationToken) -> Result<Vec<u8>> {
        self.runner
            .run(self.cwd(), &args(args_), Vec::new(), c)?
            .success("series import")
    }
    fn mail_metadata(&self, c: &CancellationToken) -> Result<BTreeMap<String, Vec<u8>>> {
        let directory = self.info().git_dir.join("rebase-apply");
        let mut out = BTreeMap::new();
        if !directory
            .try_exists()
            .map_err(|e| Error::io("read import state", e))?
        {
            return Ok(out);
        }
        let _guard = files::pin_parent(&directory.join("entry"))?;
        let mut total = 0;
        for e in fs::read_dir(directory).map_err(|e| Error::io("read import state", e))? {
            c.check()?;
            let e = e.map_err(|e| Error::io("read import state", e))?;
            let name = e
                .file_name()
                .into_string()
                .map_err(|_| bad("state", "Invalid Git import state filename."))?;
            if out.len() >= 160 {
                return Err(bad("limit", "Import state contains too many files."));
            }
            let bytes = files::read_bounded(&e.path(), LIMIT, c)?;
            total += bytes.len();
            if total > 2 * LIMIT {
                return Err(bad("limit", "Import state exceeds the safety limit."));
            }
            out.insert(name, bytes);
        }
        Ok(out)
    }
    fn mail_context(&self, action: MailImportAction, c: &CancellationToken) -> Result<Context> {
        self.require_worktree(c)?;
        let state = self.operation_state(c)?;
        if action == MailImportAction::Start {
            if !state.is_idle() {
                return Err(bad(
                    "active",
                    "Finish the current operation first\nImport can start once Git has finished what it's doing.",
                ));
            }
        } else if !state.mail_import
            || state.rebase
            || state.cherry_pick
            || state.revert
            || state.bisect
            || !state.merge_heads.is_empty()
        {
            return Err(bad(
                "state",
                "There's no import to continue\nRefresh the repository, then try again.",
            ));
        }
        let status = self.status(c)?;
        let tracked: Vec<_> = status
            .entries
            .iter()
            .filter(|e| !matches!(e.kind, EntryKind::Untracked | EntryKind::Ignored))
            .collect();
        if action == MailImportAction::Start && !tracked.is_empty() {
            let files = if tracked.len() == 1 { "1 file has".to_owned() } else { format!("{} files have", tracked.len()) };
            return Err(bad(
                "dirty",
                &format!("Commit or stash your changes first\n{files} tracked changes. Nothing was imported."),
            ));
        }
        if action == MailImportAction::Continue {
            let name = |e: &&StatusEntry| {
                let path = e.path.display_lossy();
                path.rsplit('/').next().unwrap_or_default().to_owned()
            };
            if let Some(entry) = tracked.iter().find(|e| e.kind == EntryKind::Unmerged) {
                let name = name(entry);
                return Err(bad(
                    "unresolved",
                    &format!("{name} still has a conflict\nFix it in Changes and stage it, then Continue. Nothing has changed."),
                ));
            }
            if let Some(entry) = tracked.iter().find(|e| e.worktree != Change::Unchanged) {
                let name = name(entry);
                return Err(bad(
                    "unresolved",
                    &format!("{name} has changes that aren't staged\nStage them or undo them, then Continue. Nothing has changed."),
                ));
            }
            // Git refuses this too, with "No changes".
            if tracked.is_empty() {
                return Err(bad(
                    "unresolved",
                    "Nothing to save for this patch\nNothing is staged. Stage what it should change, or Skip this patch.",
                ));
            }
        }
        let index = self.mail_read(&["ls-files", "--stage", "-v", "-z"], c)?;
        if index
            .split(|b| *b == 0)
            .filter_map(|e| e.first())
            .any(|b| b.is_ascii_lowercase() || *b == b'S')
        {
            return Err(bad(
                "index",
                "Hidden index flags are unsupported. Existing flags and files were left unchanged.",
            ));
        }
        let files = self.snapshot_changed_files(&status, c)?;
        let config = self.mail_read(&["config", "--null", "--list", "--show-origin"], c)?;
        Ok(Context {
            status,
            state,
            index,
            config,
            files,
            metadata: self.mail_metadata(c)?,
        })
    }
    fn mail_entry(
        &self,
        position: usize,
        filename: String,
        bytes: Vec<u8>,
        scan: bool,
        c: &CancellationToken,
    ) -> Result<MailImportEntry> {
        if !bytes.starts_with(b"From ")
            || bytes
                .split(|b| *b == b'\n')
                .filter(|l| l.starts_with(b"From "))
                .count()
                != 1
        {
            return Err(bad(
                "format",
                "A patch file isn't in the expected format\nEach numbered file has to hold one patch from Export commit series or git format-patch.",
            ));
        }
        let scratch = files::Scratch::new()?;
        let message = scratch.path.join("message");
        let patch = scratch.path.join("patch");
        let mut command = args(&[
            "-c",
            "mailinfo.scissors=false",
            "mailinfo",
            "--encoding=UTF-8",
            "--no-scissors",
        ]);
        command.push(crate::runner::git_path(&message));
        command.push(crate::runner::git_path(&patch));
        let headers = self
            .runner
            .run(self.cwd(), &command, bytes.clone(), c)?
            .success("read commit mail")?;
        let headers = String::from_utf8(headers)
            .map_err(|_| bad("format", "Commit mail headers must decode as UTF-8."))?;
        let value = |key: &str| {
            headers
                .lines()
                .find_map(|line| line.strip_prefix(key))
                .unwrap_or_default()
        };
        let subject = value("Subject: ");
        let author = value("Author: ");
        let date = value("Date: ");
        if subject.is_empty() || author.is_empty() || value("Email: ").is_empty() || date.is_empty()
        {
            return Err(bad(
                "format",
                "A patch has no author, date or subject\nThe file may not be from Export commit series or git format-patch.",
            ));
        }
        let mut message = files::read(&message, c)?;
        message.splice(0..0, format!("{subject}\n\n").bytes());
        let patch = files::read(&patch, c)?;
        // mailinfo leaves the diffstat and format-patch signature around the
        // decoded diff. Reuse the mail-aware parser after MIME decoding.
        let parsed = if patch.is_empty() {
            Vec::new()
        } else {
            let mut mail = bytes
                .split_inclusive(|b| *b == b'\n')
                .next()
                .unwrap()
                .to_vec();
            mail.extend(&patch);
            super::extended::parse(&mail, c)?
        };
        let mut paths = Vec::new();
        let mut content = Vec::new();
        for f in parsed {
            // Binary deltas cannot be safely inspected without their exact preimage.
            // Keep this first mail workflow text-only; ordinary patch import retains its binary support.
            if f.binary {
                return Err(bad(
                    "binary",
                    "This series has binary files\nImport takes text, renames, file modes and empty commits. Use Git for this one. Nothing was imported.",
                ));
            }
            paths.push(f.path.clone());
            if let Some(old) = f.original {
                paths.push(old);
            }
            let bytes = f
                .bytes
                .split_inclusive(|b| *b == b'\n')
                .filter_map(|l| match l.first() {
                    Some(b'+' | b' ') => Some(&l[1..]),
                    _ => None,
                })
                .flatten()
                .copied()
                .collect();
            content.push((f.path.as_bytes().to_vec(), bytes));
        }
        if scan {
            crate::secret_guard::check_import_text(
                &self.runner,
                self.cwd(),
                &message,
                &content,
                c,
            )?;
        }
        scratch.cleanup()?;
        paths.sort_by(|a, b| a.as_bytes().cmp(b.as_bytes()));
        paths.dedup();
        let subject = crate::display_text::bounded(subject, 512);
        let author = crate::display_text::bounded(author, 256);
        let date = crate::display_text::bounded(date, 128);
        let display = MailImportDisplay::new(position, [&subject, &author, &date], &paths, &patch, c)?;
        let source = bytes
            .get(5..)
            .and_then(|rest| rest.split(|b| *b == b' ').next())
            .map(String::from_utf8_lossy)
            .filter(|id| id.len() >= 7 && id.bytes().all(|b| b.is_ascii_hexdigit()))
            .map(|id| id[..7].to_owned())
            .unwrap_or_default();
        Ok(MailImportEntry {
            source,
            filename,
            subject: subject.text,
            author: author.text,
            date: date.text,
            paths,
            patch,
            display,
            bytes,
            message,
            scan_files: content,
        })
    }
    pub fn preview_mail_import(
        &self,
        folder: &Path,
        c: &CancellationToken,
    ) -> Result<MailImportPreview> {
        self.runner.step("series-preview", "inspect_mail_series", || {
            let context = self.mail_context(MailImportAction::Start, c)?;
            if self.info().git_dir.join(JOURNAL).try_exists().map_err(|e| Error::io("read import journal", e))? {
                return Err(bad("journal", "An earlier import record is retained. Inspect it and Git's state before starting another import."));
            }
            let Head::Branch { name, target } = &context.status.head else {
                return Err(bad("branch", "Switch to a local branch first\nImport adds commits to the current branch, which needs at least one commit."));
            };
            let mut entries = Vec::new();
            let mut total = 0;
            for path in names(folder, c)? {
                let bytes = files::read(&path, c)?;
                total += bytes.len();
                if total > LIMIT { return Err(bad("limit", "This series is too big\nA series can be up to 64 MiB.")); }
                entries.push(self.mail_entry(entries.len() + 1, path.file_name().unwrap().to_string_lossy().into_owned(), bytes, true, c)?);
            }
            let p = MailImportPreview {
                repository: self.info().git_dir.clone(), action: MailImportAction::Start,
                source: Some(folder.into()), original: target.clone(), branch: name.clone(),
                entries, journal: None, current: 1,
                branch_display: crate::display_text::bounded(&String::from_utf8_lossy(name), 1024),
                truncation_acknowledged: false, context, staged: Vec::new(), imported: Vec::new(),
            };
            self.mail_validate(&p, c)?;
            Ok(p)
        })
    }
    pub fn preview_mail_recovery(
        &self,
        action: MailImportAction,
        c: &CancellationToken,
    ) -> Result<MailImportPreview> {
        if action == MailImportAction::Start {
            return Err(bad("action", "Choose a patch folder to start an import."));
        }
        self.runner.step("series-preview", "inspect_mail_recovery", || {
            let context = self.mail_context(action, c)?;
            let journal = files::read_bounded(&self.info().git_dir.join(JOURNAL), 3 * LIMIT, c)?;
            let mut data = journal.strip_prefix(b"ROCKET-MAIL-1\0")
                .ok_or_else(|| bad("journal", "This import was not started by Rocket. Use Git to recover it."))?;
            let original = git_parse::oid(take(&mut data)?, "mail import original")?;
            let branch = take(&mut data)?.to_vec();
            let n = count(&mut data, 64)?;
            if n == 0 { return Err(bad("journal", "Empty import recovery record.")); }
            let mut entries = Vec::new();
            for _ in 0..n {
                let filename = String::from_utf8(take(&mut data)?.to_vec())
                    .map_err(|_| bad("journal", "Invalid import record."))?;
                entries.push(self.mail_entry(entries.len() + 1, filename, take(&mut data)?.to_vec(), false, c)?);
            }
            let n = count(&mut data, 160)?;
            let mut saved = BTreeMap::new();
            for _ in 0..n {
                let key = String::from_utf8(take(&mut data)?.to_vec())
                    .map_err(|_| bad("journal", "Invalid import state."))?;
                if saved.insert(key, take(&mut data)?.to_vec()).is_some() {
                    return Err(bad("journal", "Duplicate import state entry."));
                }
            }
            if !data.is_empty() || saved.is_empty() || saved != context.metadata {
                return Err(bad("state_changed", "Git's import state changed outside Rocket or its last snapshot was interrupted. No recovery command ran. Inspect the retained original-tip reference and use Git for this session."));
            }
            let next = saved.get("next").ok_or_else(|| bad("state", "Missing current patch number."))?;
            let current = std::str::from_utf8(next).ok().and_then(|v| v.trim().parse::<usize>().ok())
                .filter(|n| *n > 0 && *n <= entries.len())
                .ok_or_else(|| bad("state", "Invalid current patch number."))?;
            let staged = if action == MailImportAction::Continue {
                self.mail_read(&["diff", "--cached", "--no-ext-diff", "--no-textconv", "--no-color", "HEAD", "--"], c)?
            } else {
                self.mail_read(&["diff", "--no-ext-diff", "--no-textconv", "--no-color", "HEAD", "--"], c)?
            };
            let range = format!("{original}..HEAD");
            let subjects = self.mail_read(&["rev-list", "--reverse", "--no-commit-header", "--format=%s", "--max-count=65", &range, "--"], c)?;
            let imported = String::from_utf8_lossy(&subjects).lines().map(|s| crate::display_text::bounded(s, 512).text).collect();
            let p = MailImportPreview {
                repository: self.info().git_dir.clone(), action, source: None, context,
                branch_display: crate::display_text::bounded(&String::from_utf8_lossy(&branch), 1024),
                original, branch, entries, journal: Some(journal), current,
                truncation_acknowledged: false, staged, imported,
            };
            self.mail_validate(&p, c)?;
            Ok(p)
        })
    }
    fn mail_validate(&self, p: &MailImportPreview, c: &CancellationToken) -> Result<()> {
        if p.repository != self.info().git_dir {
            return Err(bad(
                "repository",
                "This review belongs to another working tree.",
            ));
        }
        if self.mail_context(p.action, c)? != p.context {
            return Err(bad(
                "stale",
                "The checkout, index, configuration or import state changed after review. Review again; local changes were preserved.",
            ));
        }
        if !matches!(&p.context.status.head,Head::Branch{name,..}if *name==p.branch) {
            return Err(bad(
                "branch_moved",
                "The original branch is no longer checked out.",
            ));
        }
        let affected: Vec<_> = p
            .entries
            .iter()
            .flat_map(|e| e.paths.iter().map(|p| p.as_bytes().to_vec()))
            .collect();
        self.protect_local_files(&affected, c)?;
        if let Some(source) = &p.source {
            let current = names(source, c)?;
            if current.len() != p.entries.len() {
                return Err(bad(
                    "source_changed",
                    "The selected patch series changed. Review it again.",
                ));
            }
            for (path, e) in current.iter().zip(&p.entries) {
                if path.file_name().and_then(|s| s.to_str()) != Some(&e.filename)
                    || files::read(path, c)? != e.bytes
                {
                    return Err(bad(
                        "source_changed",
                        "A source patch changed after review. No import was started.",
                    ));
                }
            }
        }
        if let Some(bytes) = &p.journal
            && files::read_bounded(&self.info().git_dir.join(JOURNAL), 3 * LIMIT, c)? != *bytes
        {
            return Err(bad(
                "journal_changed",
                "The recovery record changed after review.",
            ));
        }
        if p.action != MailImportAction::Start {
            let original = self.mail_read(&["rev-parse", "--verify", "ORIG_HEAD"], c)?;
            if git_parse::line(&original) != p.original.as_str().as_bytes() {
                return Err(bad(
                    "original_changed",
                    "ORIG_HEAD changed outside this import. The recovery reference is retained; no recovery command ran.",
                ));
            }
            if p.action == MailImportAction::Abort {
                let safety = p
                    .context
                    .metadata
                    .get("abort-safety")
                    .ok_or_else(|| bad("state", "Git has no abort safety record."))?;
                if !matches!(&p.context.status.head,Head::Branch{target,..}if target.as_str().as_bytes()==git_parse::line(safety))
                {
                    return Err(bad(
                        "head_changed",
                        "New commits were made outside this import. Abort would overwrite them and was refused.",
                    ));
                }
            }
        }
        c.check()
    }
    fn mail_recovery_ref(&self, id: &ObjectId, c: &CancellationToken) -> Result<()> {
        let name = reference(id);
        let out = self.runner.run(
            self.cwd(),
            &args(&["show-ref", "--exists", "--", &name]),
            vec![],
            c,
        )?;
        if out.status.success() {
            let value = self.mail_read(&["show-ref", "--verify", "--hash", "--", &name], c)?;
            if git_parse::line(&value) != id.as_str().as_bytes() {
                return Err(bad(
                    "reference",
                    "The recovery reference already names another commit.",
                ));
            }
            return Ok(());
        }
        if out.status.code() != Some(2) {
            out.success("inspect import recovery tip")?;
        }
        self.runner
            .write(
                self.cwd(),
                &args(&[
                    "update-ref",
                    &name,
                    id.as_str(),
                    &"0".repeat(id.as_str().len()),
                ]),
                vec![],
                c,
            )?
            .success("save import recovery tip")?;
        Ok(())
    }
    fn mail_backup(&self, p: &MailImportPreview, c: &CancellationToken) -> Result<Option<PathBuf>> {
        if p.context.files.is_empty() {
            return Ok(None);
        }
        let directory = self.info().git_dir.join(format!(
            "rocket-mail-backup-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        let _guard = files::pin_parent(&directory)?;
        fs::create_dir(&directory).map_err(|e| Error::io("reserve import recovery backup", e))?;
        let _guard = local_files::pin_directories(&self.info().git_dir, &directory)?;
        let mut manifest = String::from(
            "Rocket mail import recovery copies. Original paths are quoted; missing means the file was absent.\n",
        );
        for (i, (path, value)) in p.context.files.iter().enumerate() {
            let name = format!("working-{i:04}");
            match value {
                WorkingFile::Bytes(bytes) => files::write_new(&directory.join(&name), bytes, c)?,
                WorkingFile::Missing => {}
                WorkingFile::Link(_) => {
                    return Err(bad(
                        "link",
                        "Recovery backup refuses links. Inspect this changed path before continuing.",
                    ));
                }
            };
            manifest.push_str(&format!(
                "{name}\t{:?}\t{}\n",
                path.display_lossy(),
                if matches!(value, WorkingFile::Missing) {
                    "missing"
                } else {
                    "present"
                }
            ));
        }
        files::write_new(&directory.join("manifest.log"), manifest.as_bytes(), c)?;
        let staged = self.mail_read(
            &[
                "diff",
                "--cached",
                "--binary",
                "--full-index",
                "--no-ext-diff",
                "--no-textconv",
                "HEAD",
                "--",
            ],
            c,
        )?;
        files::write_new(&directory.join("staged.patch"), &staged, c)?;
        Ok(Some(directory))
    }
    pub fn apply_mail_import(
        &self,
        p: &MailImportPreview,
        c: &CancellationToken,
    ) -> Result<MailImportReport> {
        c.check()?;
        if p.requires_truncation_acknowledgement() && !p.truncation_acknowledged() {
            return Err(bad("review_incomplete", "The preview omits some mail fields, paths or patch content. Explicitly acknowledge the full-source import in this review before applying."));
        }
        self.require_worktree(c)?;
        let _permit = self.acquire_write(c)?;
        self.runner
            .step(p.action.label(), "revalidate_preview", || {
                self.mail_validate(p, c)
            })?;
        // Abort only restores saved state. Skip scans the remaining mails, since
        // Git automatically continues the series after dropping the current one.
        if p.action != MailImportAction::Abort {
            self.runner
                .step(p.action.label(), "scan_imported_content", || {
                    let first = if p.action == MailImportAction::Skip {
                        p.current
                    } else {
                        p.current - 1
                    };
                    for e in &p.entries[first..] {
                        crate::secret_guard::check_import_text(
                            &self.runner,
                            self.cwd(),
                            &e.message,
                            &e.scan_files,
                            c,
                        )?;
                    }
                    Ok(())
                })?;
        }
        let backup = if matches!(p.action, MailImportAction::Skip | MailImportAction::Abort) {
            self.runner
                .step(p.action.label(), "save_resolution_backup", || {
                    self.mail_backup(p, c)
                })?
        } else {
            None
        };
        self.mail_recovery_ref(&p.original, c)?;
        if let Head::Branch { target, .. } = &p.context.status.head {
            self.mail_recovery_ref(target, c)?;
        }
        self.mail_validate(p, c)?;
        if p.action == MailImportAction::Continue {
            crate::secret_guard::check_import_index(&self.runner, self.cwd(), c)?;
        }
        let expected = if let Some(j) = &p.journal {
            j.clone()
        } else {
            let j = encode(p, &BTreeMap::new());
            publish_journal(self, None, &j)?;
            j
        };
        let mut command = args(&[
            "-c",
            "submodule.recurse=false",
            "-c",
            "rerere.autoupdate=false",
            "-c",
            "am.keepcr=true",
            "am",
        ]);
        let mut input = Vec::new();
        match p.action {
            MailImportAction::Start => {
                command.extend(args(&[
                    "--3way",
                    "--no-rerere-autoupdate",
                    "--no-scissors",
                    "--keep-cr",
                    "--empty=keep",
                    "--patch-format=mbox",
                ]));
                for e in &p.entries {
                    input.extend(&e.bytes);
                    if !input.ends_with(b"\n") {
                        input.push(b'\n');
                    }
                }
            }
            MailImportAction::Continue => command.push("--continue".into()),
            MailImportAction::Skip => command.push("--skip".into()),
            MailImportAction::Abort => command.push("--abort".into()),
        }
        let write = self
            .execute_write(p.action.operation(), &command, input, c)
            .unwrap_or_else(|error| WriteReport {
                operation: p.action.operation(),
                result: Err(error),
                status: self.status(&CancellationToken::new()),
            });
        let reconcile = CancellationToken::new();
        let state = self.operation_state(&reconcile);
        let journal_result = self
            .runner
            .step(p.action.label(), "record_recovery_state", || {
                let metadata = self.mail_metadata(&reconcile)?;
                if metadata.is_empty() {
                    if files::read_bounded(
                        &self.info().git_dir.join(JOURNAL),
                        3 * LIMIT,
                        &reconcile,
                    )? != expected
                    {
                        return Err(bad(
                            "journal_changed",
                            "Recovery record changed; it was retained.",
                        ));
                    }
                    fs::remove_file(self.info().git_dir.join(JOURNAL))
                        .map_err(|e| Error::io("finish import recovery record", e))
                } else {
                    publish_journal(self, Some(&expected), &encode(p, &metadata))
                }
            });
        if let Some(log) = &self.runner.log {
            log.mail_import_result(
                p.action.label(),
                state.as_ref().ok().map(|s| s.mail_import),
                backup.is_some(),
                journal_result.is_ok(),
                write.result.is_ok(),
            );
        }
        Ok(MailImportReport {
            write,
            state,
            recovery_reference: p.recovery_reference(),
            backup,
            journal_result,
        })
    }
}
