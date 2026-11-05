use crate::{Error, Result};
use std::{borrow::Cow, ffi::OsString, path::PathBuf};

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ObjectId(String);

impl ObjectId {
    pub fn parse(value: &str) -> Result<Self> {
        if !matches!(value.len(), 40 | 64) || !value.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(Error::InvalidInput(
                "expected a full SHA-1 or SHA-256 object ID".into(),
            ));
        }
        Ok(Self(value.to_ascii_lowercase()))
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for ObjectId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// A literal path relative to the repository root. Never use display text as an argument.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct RepoPath(Vec<u8>);

impl RepoPath {
    pub fn new(bytes: impl Into<Vec<u8>>) -> Result<Self> {
        let bytes = bytes.into();
        if bytes.is_empty()
            || bytes.contains(&0)
            || bytes.starts_with(b"/")
            || bytes
                .split(|&b| b == b'/')
                .any(|p| p == b".." || p == b"." || p.is_empty())
        {
            return Err(Error::InvalidInput(
                "expected a nonempty repository-relative path".into(),
            ));
        }
        #[cfg(windows)]
        if bytes.contains(&b'\\') || bytes.contains(&b':') {
            return Err(Error::InvalidInput(
                "Windows repository paths must use '/' and have no drive prefix".into(),
            ));
        }
        Ok(Self(bytes))
    }
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
    pub fn display_lossy(&self) -> Cow<'_, str> {
        String::from_utf8_lossy(&self.0)
    }
    pub(crate) fn argument(&self) -> Result<OsString> {
        bytes_to_os(&self.0)
    }
}

pub(crate) fn bytes_to_os(bytes: &[u8]) -> Result<OsString> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStringExt;
        Ok(OsString::from_vec(bytes.to_vec()))
    }
    #[cfg(not(unix))]
    {
        String::from_utf8(bytes.to_vec())
            .map(OsString::from)
            .map_err(|_| {
                Error::Unsupported(
                    "Git returned a path not representable as UTF-8 on this platform".into(),
                )
            })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ObjectFormat {
    Sha1,
    Sha256,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RepositoryInfo {
    pub worktree: Option<PathBuf>,
    pub git_dir: PathBuf,
    pub common_dir: PathBuf,
    pub object_format: ObjectFormat,
    pub is_shallow: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Head {
    Unborn { branch: Vec<u8> },
    Branch { name: Vec<u8>, target: ObjectId },
    Detached { target: ObjectId },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Change {
    Unchanged,
    Added,
    Modified,
    Deleted,
    Renamed,
    Copied,
    TypeChanged,
    Unmerged,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EntryKind {
    Tracked,
    RenameOrCopy,
    Unmerged,
    Untracked,
    Ignored,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SubmoduleState {
    pub commit_changed: bool,
    pub tracked_changes: bool,
    pub untracked_changes: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StatusEntry {
    pub path: RepoPath,
    pub original_path: Option<RepoPath>,
    pub kind: EntryKind,
    pub index: Change,
    pub worktree: Change,
    pub submodule: Option<SubmoduleState>,
    pub similarity: Option<u8>,
}
impl StatusEntry {
    /// Whether Git can stash this change on its own (`git stash push -- path`). A
    /// staged deletion, including a staged rename's original path, is no longer in
    /// the index, so Git refuses it as a pathspec; conflicts, ignored files and
    /// submodules cannot be stashed either.
    pub fn stashable_alone(&self) -> bool {
        !matches!(self.index, Change::Deleted | Change::Renamed)
            && !matches!(self.kind, EntryKind::Unmerged | EntryKind::Ignored)
            && self.submodule.is_none()
    }
    /// Whether the index holds a change for this path.
    pub fn staged(&self) -> bool {
        !matches!(self.index, Change::Unchanged)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Status {
    pub head: Head,
    pub upstream: Option<Vec<u8>>,
    pub ahead_behind: Option<(u64, u64)>,
    pub entries: Vec<StatusEntry>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BranchKind {
    Local,
    Remote,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Branch {
    /// Full ref name; remote symbolic refs such as origin/HEAD have a symref target.
    pub reference: Vec<u8>,
    pub target: ObjectId,
    pub kind: BranchKind,
    pub is_head: bool,
    pub upstream: Option<Vec<u8>>,
    pub symbolic_target: Option<Vec<u8>>,
    pub checked_out_at: Option<PathBuf>,
    /// When its commit was committed, in Unix seconds; None when Git reports none.
    pub committed: Option<i64>,
    /// Commits ahead of and behind its upstream; None without an upstream, or when
    /// the upstream is gone.
    pub ahead_behind: Option<(u32, u32)>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Signature {
    pub name: Vec<u8>,
    pub email: Vec<u8>,
    pub timestamp: i64,
    pub offset_minutes: i32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Commit {
    pub id: ObjectId,
    pub tree: ObjectId,
    pub parents: Vec<ObjectId>,
    pub author: Signature,
    pub committer: Signature,
    pub encoding: Option<Vec<u8>>,
    /// Raw commit message, including any non-UTF-8 bytes and trailing newlines.
    pub message: Vec<u8>,
}

impl Commit {
    pub fn subject_lossy(&self) -> Cow<'_, str> {
        String::from_utf8_lossy(
            self.message
                .split(|&b| b == b'\n')
                .next()
                .unwrap_or_default(),
        )
    }
}

/// All orders keep parents below their children, including skewed timestamps.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum HistoryOrder {
    #[default]
    Topological,
    CommitDate,
    AuthorDate,
}
impl HistoryOrder {
    pub fn key(self) -> &'static str { match self {
        Self::Topological => "topo", Self::CommitDate => "commit-date", Self::AuthorDate => "author-date",
    }}
    pub fn label(self) -> &'static str { match self {
        Self::Topological => "Topological", Self::CommitDate => "Commit date", Self::AuthorDate => "Author date",
    }}
    pub fn parse(value: &str) -> Option<Self> { match value {
        "topo" => Some(Self::Topological), "commit-date" => Some(Self::CommitDate),
        "author-date" => Some(Self::AuthorDate), _ => None,
    }}
    pub(crate) fn argument(self) -> &'static str { match self {
        Self::Topological => "--topo-order", Self::CommitDate => "--date-order", Self::AuthorDate => "--author-date-order",
    }}
}

/// Pins a traversal to its order and all starting commits even if branches move.
#[derive(Clone, Debug)]
pub struct HistoryCursor {
    pub(crate) repository: PathBuf,
    pub(crate) tips: Vec<ObjectId>,
    pub(crate) offset: usize,
    pub(crate) order: HistoryOrder,
}

#[derive(Clone, Debug)]
pub enum HistoryStart {
    Head,
    /// Local and remote-tracking branches, plus detached HEAD. Excludes tags/stashes.
    AllBranches,
    /// Already observed tips, used to keep a search consistent with its history view.
    Tips(Vec<ObjectId>),
    /// Immutable tips with the presentation order retained for search.
    OrderedTips(Vec<ObjectId>, HistoryOrder),
    /// Traverse an already observed full object ID. Git still verifies its type.
    Commit(ObjectId),
    Revision(String),
    Continue(HistoryCursor),
}

#[derive(Clone, Debug, Default)]
pub struct HistoryPage {
    pub commits: Vec<Commit>,
    pub next: Option<HistoryCursor>,
    pub(crate) tips: Vec<ObjectId>,
    pub order: HistoryOrder,
}

impl HistoryPage {
    pub fn start(&self) -> HistoryStart {
        HistoryStart::OrderedTips(self.tips.clone(), self.order)
    }
    pub fn same_snapshot(&self, other: &Self) -> bool {
        self.tips == other.tips && self.order == other.order
    }
}

/// What a history search compares its query with.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum HistorySearchKind {
    /// Message, author name or email (ignoring case), or a commit ID prefix.
    #[default]
    Text,
    /// A path the commit changed contains the query (ignoring case; `\` matches `/`).
    Path,
    /// The commit adds or removes occurrences of the query text (Git's `-S`,
    /// ignoring case). The query is literal, never a regular expression.
    Content,
}
impl HistorySearchKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Path => "path",
            Self::Content => "content",
        }
    }
}

#[derive(Clone, Debug)]
pub struct HistorySearchCursor {
    pub(crate) history: HistoryCursor,
    pub(crate) query: String,
    pub(crate) kind: HistorySearchKind,
}

#[derive(Clone, Debug)]
pub enum HistorySearchStart {
    Query {
        start: HistoryStart,
        query: String,
        kind: HistorySearchKind,
    },
    Continue(HistorySearchCursor),
}

/// A bounded scan, which may contain no matches while still having a next cursor.
#[derive(Clone, Debug)]
pub struct HistorySearchPage {
    pub commits: Vec<Commit>,
    pub next: Option<HistorySearchCursor>,
    pub scanned: usize,
}

#[derive(Clone, Debug)]
pub enum DiffTarget {
    /// Index versus working tree. Untracked files are not included.
    Unstaged,
    /// HEAD versus index; also works before the first commit.
    Staged,
    /// Two commits, resolved before the diff is run.
    Between { base: String, target: String },
    /// A committed base versus the index or tracked working tree. No untracked files.
    Local { base: String, staged: bool },
    /// One commit against its first parent, or the empty tree for a root commit.
    Commit { id: ObjectId },
}

#[derive(Clone, Debug)]
pub struct DiffOptions {
    pub target: DiffTarget,
    /// Empty means all paths; each path is literal (never a wildcard).
    pub paths: Vec<RepoPath>,
    pub context_lines: u16,
}

impl Default for DiffOptions {
    fn default() -> Self {
        Self {
            target: DiffTarget::Unstaged,
            paths: Vec::new(),
            context_lines: 3,
        }
    }
}

/// Git's patch bytes, with binary markers preserved. No lossy decoding or silent truncation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diff {
    pub patch: Vec<u8>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WriteOperation {
    Stage,
    Unstage,
    Commit,
    CreateBranch,
    SwitchBranch,
    RenameBranch,
    DeleteBranch,
    PushStash,
    ApplyStash,
    DropStash,
    Merge,
    ContinueMerge,
    AbortMerge,
    FinishMerge,
    Amend,
    CherryPick,
    Revert,
    ContinueCherryPick,
    SkipCherryPick,
    AbortCherryPick,
    ContinueRevert,
    SkipRevert,
    AbortRevert,
    Rebase,
    ContinueRebase,
    SkipRebase,
    AbortRebase,
    ImportSeries,
    ContinueSeries,
    SkipSeries,
    AbortSeries,
    ResetSoft,
    ResetMixed,
    ResetHard,
}

/// Output from a confirmed successful Git write, including warnings/hook output.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WriteOutput {
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

/// Returned after an attempted write, even when Git failed or was interrupted.
///
/// Inspect both fields: refresh failure does not undo a successful command, and an
/// interrupted command may already have changed the index or created a commit.
/// The status is an observation after the command, not an atomic snapshot with it.
#[derive(Debug)]
#[must_use = "check the command result and refreshed status; cancellation is not rollback"]
pub struct WriteReport {
    pub operation: WriteOperation,
    pub result: Result<WriteOutput>,
    pub status: Result<Status>,
}

/// A branch mutation followed by status (including HEAD) and branch-list refreshes.
/// Both reads are attempted even if the command failed or was cancelled. Each read
/// can fail independently; neither changes the command's outcome in `write.result`.
/// These observations are not an atomic snapshot across other Git applications.
#[derive(Debug)]
#[must_use = "inspect the command outcome and both refresh results before retrying"]
pub struct BranchWriteReport {
    pub write: WriteReport,
    pub branches: Result<Vec<Branch>>,
}

/// Most branches one batch deletion accepts.
pub const MAX_BRANCH_BATCH: usize = 200;

/// Several branch deletions in the order requested. Each attempted branch has its
/// own report (Git's refusal of an unmerged branch is in its `write.result`).
#[derive(Debug, Default)]
pub struct BranchBatchReport {
    pub results: Vec<(Vec<u8>, Result<BranchWriteReport>)>,
    /// Not attempted because the batch was cancelled first.
    pub skipped: Vec<Vec<u8>>,
}

/// One observation of the shared stash reflog. The index can change at any time;
/// use `id` for apply, and pass the whole entry back for a checked drop.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StashEntry {
    pub index: usize,
    pub id: ObjectId,
    pub subject: Vec<u8>,
    pub(crate) repository: PathBuf,
}

/// Independent views of a pinned stash; tracked includes both saved index and worktree changes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum StashPart {
    #[default]
    Tracked,
    Index,
    Untracked,
}

#[derive(Debug)]
#[must_use = "inspect the command result, status, and refreshed stash list"]
pub struct StashWriteReport {
    pub write: WriteReport,
    pub stashes: Result<Vec<StashEntry>>,
}

/// Worktree-local operation markers, independent of whether conflicts remain.
/// These are observations, not a lock against other Git applications.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OperationState {
    pub bisect: bool,
    pub merge_heads: Vec<ObjectId>,
    /// HEAD is already a merge commit with these incoming parents, but Git has
    /// not cleared MERGE_HEAD (for example, cancellation in a post-merge hook).
    pub merge_committed: bool,
    pub merge_autostash: Option<ObjectId>,
    pub rebase: bool,
    pub mail_import: bool,
    pub cherry_pick: bool,
    pub revert: bool,
    pub sequencer: bool,
    /// Where a rebase or commit import has stopped, as far as Git's state says.
    pub step: Option<OperationStep>,
}

/// What a paused rebase or commit import is waiting for.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum StepKind {
    /// A commit or patch that stopped: it conflicts, or has nothing left to apply.
    #[default]
    Commit,
    /// A planned stop to change this commit.
    Edit,
    /// A planned stop between commits: a break, or a command that stopped.
    Between,
    /// The changes are in, and only saving the reviewed message is left.
    Message,
    /// The branch left the tip this rebase saved, so the review refuses every way on
    /// and only Git can finish it.
    BranchMoved,
}

/// The commit or patch a paused rebase or commit import stopped at.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OperationStep {
    /// Its place in the sequence, from 1.
    pub at: usize,
    pub total: usize,
    pub kind: StepKind,
    /// Its first message line, empty when Git kept none.
    pub subject: String,
    /// The branch under rebase, which Git leaves while it works.
    pub branch: Option<Vec<u8>>,
}

impl OperationState {
    pub fn is_idle(&self) -> bool {
        !self.bisect
            && self.merge_heads.is_empty()
            && self.merge_autostash.is_none()
            && !self.rebase
            && !self.mail_import
            && !self.cherry_pick
            && !self.revert
            && !self.sequencer
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MergeMode {
    FastForwardOnly,
    AllowFastForward,
    NoFastForward,
    /// Prepare ordinary staged changes, without creating a merge commit.
    Squash,
    /// Preserve MERGE_HEAD for an explicit later commit, even for a fast-forward.
    NoCommit,
}

/// Every refresh is attempted independently, while the shared write gate is held.
#[derive(Debug)]
#[must_use = "inspect the command result and refreshed state, especially after conflicts"]
pub struct MergeWriteReport {
    pub write: WriteReport,
    pub state: Result<OperationState>,
    pub branches: Result<Vec<Branch>>,
    pub failure: Option<crate::MergeFailure>,
}

/// Configured values, in configuration order within each list. URLs have not
/// undergone Git's insteadOf/pushInsteadOf rewriting and may contain credentials.
/// An empty push_urls list means Git normally falls back to urls. Fetch uses the
/// first URL. These bytes are application data, never diagnostic log fields.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Remote {
    pub name: Vec<u8>,
    pub urls: Vec<Vec<u8>>,
    pub push_urls: Vec<Vec<u8>>,
    pub fetch_refspecs: Vec<Vec<u8>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RemoteUrlKind {
    Fetch,
    Push,
}

/// One local remote mutation. Command success is separate from fresh observations.
#[derive(Debug)]
#[must_use = "inspect the command and refreshed configuration before retrying"]
pub struct RemoteWriteReport {
    pub result: Result<WriteOutput>,
    pub remotes: Result<Vec<Remote>>,
    pub branches: Result<Vec<Branch>>,
    pub status: Option<Result<Status>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FetchUpdateKind {
    FastForward,
    Forced,
    New,
    UpToDate,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FetchUpdate {
    pub kind: FetchUpdateKind,
    pub old: Option<ObjectId>,
    pub new: ObjectId,
    pub reference: Vec<u8>,
}

/// A confirmed successful fetch. A porcelain parsing error is separate from
/// Git's success, so callers must not mistake it for a failed mutation and retry.
#[derive(Debug)]
pub struct FetchOutput {
    pub output: WriteOutput,
    pub updates: Result<Vec<FetchUpdate>>,
}

/// Returned after any attempted fetch, including spawn failures and cancellation.
/// Fresh, uncancelled reads run while the common-directory write gate is held.
/// Status is None for bare repositories. These reads are independent observations,
/// not an atomic snapshot against other applications. Cancellation is not rollback:
/// objects, FETCH_HEAD, or refs may already have changed. Never delete Git lock files
/// or retry automatically based on an interrupted result.
#[derive(Debug)]
#[must_use = "inspect the fetch outcome and refresh results before retrying"]
pub struct FetchReport {
    pub result: Result<FetchOutput>,
    pub status: Option<Result<Status>>,
    pub branches: Result<Vec<Branch>>,
}

/// The upstream chosen from the current branch's effective Git configuration.
/// Names retain bytes and must never be copied into diagnostic log fields.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PullUpstream {
    pub branch: Vec<u8>,
    pub remote: Vec<u8>,
    pub remote_reference: Vec<u8>,
    pub tracking_reference: Vec<u8>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PullUpdateKind {
    FastForward,
    /// The fetched commit is already contained in the local branch, including
    /// when the local branch is ahead. Local-only commits are kept.
    UpToDate,
}

/// A successful Git merge --ff-only command, separate from verification/refresh.
#[derive(Debug)]
pub struct PullUpdate {
    pub kind: PullUpdateKind,
    pub before: ObjectId,
    pub fetched: ObjectId,
    pub output: WriteOutput,
    /// Advisory error:/fatal: diagnostics despite a successful Git exit. Inspect
    /// retained output locally; false does not certify that all hooks succeeded.
    pub error_diagnostics: bool,
    /// Fresh, uncancelled observation of the expected branch tip. A failure here
    /// does not undo Git's success or certify which process last changed HEAD.
    pub verification: Result<()>,
}

/// Returned once fetch is attempted. Preflight/queue errors are outer errors.
/// `fetch` preserves command success independently of parsing and branch update.
/// `update` is None if fetch failed or its updates could not be parsed; Some(Err)
/// also covers refusal/cancellation before the merge command starts.
/// All three refreshes run independently under the same common-directory write
/// gate with fresh tokens. External Git clients/file edits are not excluded.
/// Cancellation is not rollback; downloaded objects, tracking refs, or the local
/// branch may already have changed. Never automatically retry a failed phase.
#[derive(Debug)]
#[must_use = "inspect fetch, branch update, verification, and all refresh results"]
pub struct PullReport {
    pub upstream: PullUpstream,
    pub fetch: Result<FetchOutput>,
    pub update: Option<Result<PullUpdate>>,
    pub status: Result<Status>,
    pub branches: Result<Vec<Branch>>,
    pub state: Result<OperationState>,
}

/// Explicit source kind avoids interpreting a Windows path as an SSH URL.
/// URLs support HTTPS and ssh://, without passwords/query/fragment credentials.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CloneSource {
    Local(std::path::PathBuf),
    Url(String),
}

/// Returned after a fresh sibling workspace is created, even on cancellation.
/// Preflight failures are outer errors and create no workspace. Command success,
/// verification, non-replacing publication, reopening, and empty-folder cleanup
/// are independent results. None means that phase was not attempted.
///
/// Failed/interrupted clones retain `staging_directory` for explicit inspection;
/// Git itself may remove its incomplete checkout. Rocket never recursively deletes
/// it. If publication succeeded, the destination remains successful even if later
/// opening/cleanup fails. Progress is advisory; this report is authoritative.
#[derive(Debug)]
#[must_use = "inspect clone, verification, publication, reopening, and retained workspace"]
pub struct CloneReport {
    pub destination: std::path::PathBuf,
    pub staging_directory: std::path::PathBuf,
    pub command: Result<WriteOutput>,
    /// Advisory fixed-prefix scan of retained diagnostics; not a phase failure.
    pub error_diagnostics: bool,
    pub verification: Option<Result<()>>,
    pub publication: Option<Result<()>>,
    pub repository: Option<Result<crate::Repository>>,
    pub cleanup: Option<Result<()>>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PushOptions {
    /// Ask Git to configure the selected local branch's upstream on success.
    pub set_upstream: bool,
}

/// Fixed categories from Git's porcelain status, without remote-provided prose.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PushRejection {
    StaleLease,
    NonFastForward,
    FetchFirst,
    RemoteRejected,
    RemoteFailure,
    Other,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PushUpdateKind {
    New,
    FastForward,
    UpToDate,
    Rejected(PushRejection),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PushUpdate {
    pub local_reference: Vec<u8>,
    pub remote_reference: Vec<u8>,
    pub kind: PushUpdateKind,
}

/// A command error is not proof that a remote write was rolled back. In
/// particular, cancellation can happen after the server has updated its ref.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PushRemoteOutcome {
    /// The push process never started. Policy validation or process setup refused it.
    NotAttempted,
    /// Git exited successfully or reported a positive status for the selected ref.
    /// Includes an already-up-to-date destination. Local metadata may still fail.
    Confirmed,
    /// Git reported a client/server rejection for the selected ref.
    Rejected,
    /// No conclusive status was received. Local refreshes cannot resolve this.
    Unknown,
}

impl PushRemoteOutcome {
    pub fn code(self) -> &'static str {
        match self {
            Self::NotAttempted => "not_attempted",
            Self::Confirmed => "confirmed_success",
            Self::Rejected => "rejected",
            Self::Unknown => "unknown",
        }
    }
}

/// Returned once push execution is requested. Selection/queue errors are outer
/// errors; execution-time policy refusals retain a NotAttempted remote outcome.
/// `result` preserves Git's command exit independently of parsing and refreshes.
/// `update` is None when no final stdout was captured; failed pushes can also have
/// a parsed rejection. `upstream` verifies config only when requested and the
/// remote outcome was confirmed; None otherwise. Status is None for bare repos.
/// All local refreshes use fresh tokens under the shared write gate. They do not
/// contact the remote, establish its current state, or undo a completed push.
#[derive(Debug)]
#[must_use = "inspect the remote outcome, command result, and independent refreshes before retrying"]
pub struct PushReport {
    pub result: Result<WriteOutput>,
    pub update: Option<Result<PushUpdate>>,
    pub remote_outcome: PushRemoteOutcome,
    /// Advisory: stderr contained an error:/fatal: line, even if Git exited zero.
    /// This can expose local tracking/config failures after remote success. False
    /// is not proof of clean metadata; unfamiliar or hook diagnostics may differ.
    /// Inspect retained output locally; never send it to diagnostic logs.
    pub error_diagnostics: bool,
    pub upstream: Option<Result<()>>,
    pub status: Option<Result<Status>>,
    pub branches: Result<Vec<Branch>>,
}
