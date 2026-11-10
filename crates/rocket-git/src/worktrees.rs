//! Reviewed worktree operations. Creation publishes without replacing a path;
//! removal never forces Git past its clean-worktree or lock checks.
use crate::{
    init, local_files,
    model::bytes_to_os,
    runner::{args, git_path},
    *,
};
use std::{
    fs, io,
    io::Write,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};
static NEXT: AtomicU64 = AtomicU64::new(0);
mod maintenance;
pub use maintenance::WorktreeMaintenance;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorktreeEntry {
    pub path: PathBuf,
    pub head: Option<ObjectId>,
    pub branch: Option<Vec<u8>>,
    pub main: bool,
    pub bare: bool,
    pub locked: Option<Vec<u8>>,
    pub prunable: Option<Vec<u8>>,
    repository: PathBuf,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WorktreeCheckout {
    NewBranch(String),
    ExistingBranch(String),
    /// A new local branch that tracks `upstream`, a remote branch such as
    /// `origin/main`, starting at its reviewed commit.
    TrackingBranch {
        branch: String,
        upstream: String,
    },
}
impl WorktreeCheckout {
    pub fn branch(&self) -> &str {
        match self {
            Self::NewBranch(s)
            | Self::ExistingBranch(s)
            | Self::TrackingBranch { branch: s, .. } => s,
        }
    }
    pub fn new_branch(&self) -> bool {
        !matches!(self, Self::ExistingBranch(_))
    }
    /// The remote branch a new tracking branch follows.
    pub fn upstream(&self) -> Option<&str> {
        match self {
            Self::TrackingBranch { upstream, .. } => Some(upstream),
            _ => None,
        }
    }
}
#[derive(Clone, Debug)]
pub struct WorktreePreview {
    repository: PathBuf,
    path: PathBuf,
    target: ObjectId,
    checkout: Option<WorktreeCheckout>,
    entry: Option<WorktreeEntry>,
    git_dir: Option<PathBuf>,
    maintenance: Option<maintenance::Plan>,
}
impl WorktreePreview {
    pub fn path(&self) -> &Path {
        &self.path
    }
    pub fn target(&self) -> &ObjectId {
        &self.target
    }
    pub fn checkout(&self) -> Option<&WorktreeCheckout> {
        self.checkout.as_ref()
    }
    pub fn entry(&self) -> Option<&WorktreeEntry> {
        self.entry.as_ref()
    }
    pub fn removing(&self) -> bool {
        self.entry.is_some() && self.maintenance.is_none()
    }
    pub fn maintenance(&self) -> Option<WorktreeMaintenance> {
        self.maintenance.as_ref().map(|p| p.kind)
    }
    pub fn label(&self) -> &'static str {
        if let Some(kind) = self.maintenance() {
            kind.label()
        } else if self.removing() {
            "worktree-remove"
        } else {
            "worktree-create"
        }
    }
    pub fn recovery_reference(&self) -> Option<String> {
        (self.removing() || self.maintenance() == Some(WorktreeMaintenance::Prune))
            .then(|| format!("refs/rocket/removed-worktrees/{}", self.target))
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorktreeFailure {
    DestinationExists,
    InvalidDestination,
    InvalidBranch,
    BranchExists,
    BranchUnavailable,
    BranchInUse,
    UpstreamUnavailable,
    TargetUnavailable,
    StalePreview,
    ProtectedWorktree,
    Locked,
    Missing,
    Dirty,
    UnsupportedIndex,
    ActiveOperation,
    ReferenceConflict,
    CommandFailed,
    VerificationFailed,
}
impl WorktreeFailure {
    pub fn code(self) -> &'static str {
        match self {
            Self::DestinationExists => "worktree_destination_exists",
            Self::InvalidDestination => "worktree_invalid_destination",
            Self::InvalidBranch => "worktree_invalid_branch",
            Self::BranchExists => "worktree_branch_exists",
            Self::BranchUnavailable => "worktree_branch_unavailable",
            Self::BranchInUse => "worktree_branch_in_use",
            Self::UpstreamUnavailable => "worktree_upstream_unavailable",
            Self::TargetUnavailable => "worktree_target_unavailable",
            Self::StalePreview => "worktree_stale_preview",
            Self::ProtectedWorktree => "worktree_protected",
            Self::Locked => "worktree_locked",
            Self::Missing => "worktree_missing",
            Self::Dirty => "worktree_dirty",
            Self::UnsupportedIndex => "worktree_unsupported_index",
            Self::ActiveOperation => "worktree_active_operation",
            Self::ReferenceConflict => "worktree_recovery_conflict",
            Self::CommandFailed => "worktree_command_failed",
            Self::VerificationFailed => "worktree_verification_failed",
        }
    }
    pub fn guidance(self) -> &'static str {
        match self {
            Self::DestinationExists => {
                "That destination already exists or is registered to a worktree. Choose a new folder; existing paths are never replaced."
            }
            Self::InvalidDestination => {
                "Choose a new folder on a local drive with an existing parent, outside repositories and Git metadata. Links, junctions and redirected paths are refused."
            }
            Self::InvalidBranch => {
                "Enter a literal local branch name, without a leading dash or revision syntax."
            }
            Self::BranchExists => {
                "That branch already exists. Choose Existing local branch or use a different new name."
            }
            Self::BranchUnavailable => {
                "The selected local branch is missing or indirect. Refresh and choose an existing local branch."
            }
            Self::BranchInUse => {
                "That branch is already checked out in another worktree. Open that worktree, or choose another branch."
            }
            Self::UpstreamUnavailable => {
                "Choose a remote branch such as origin/main. It must exist after the last fetch and must not be a symbolic name such as origin/HEAD."
            }
            Self::TargetUnavailable => {
                "The starting point must resolve to an available commit. Create the first commit or choose another target."
            }
            Self::StalePreview => {
                "The worktree, branch or repository changed after review. Refresh and review again. If an operation had started, inspect its observed state before retrying."
            }
            Self::ProtectedWorktree => {
                "The main or current worktree is protected. Close the target's Rocket tab and manage it from another worktree."
            }
            Self::Locked => {
                "The worktree or its Git metadata is locked. Finish other Git operations and inspect the lock; Rocket does not remove locks."
            }
            Self::Missing => {
                "This worktree's folder or registration is unavailable. Refresh the worktree list, then repair a moved folder or review its missing registration."
            }
            Self::Dirty => {
                "This worktree contains staged, modified, untracked or ignored files. Commit, stash or move them first. Nothing is force-discarded."
            }
            Self::UnsupportedIndex => {
                "This operation refuses submodules, sparse/hidden index flags and redirected files. Inspect these with Git before moving or removing the worktree."
            }
            Self::ActiveOperation => {
                "This worktree has an unfinished Git operation. Continue or abort that operation before maintenance."
            }
            Self::ReferenceConflict => {
                "The committed-history recovery reference has unexpected content. Rocket will not overwrite it."
            }
            Self::CommandFailed => {
                "Git did not confirm completion. Inspect the observed worktrees and retained folder before retrying; cancellation is not rollback."
            }
            Self::VerificationFailed => {
                "The observed folder or worktree registration does not match the reviewed result. Inspect the listed paths before retrying."
            }
        }
    }
}
fn refused<T>(f: WorktreeFailure) -> Result<T> {
    Err(Error::Worktree(f))
}
#[derive(Debug)]
#[must_use = "Inspect command, observation and verification separately; cancellation is not rollback"]
pub struct WorktreeWriteReport {
    pub command: Result<WriteOutput>,
    pub observed: Result<Vec<WorktreeEntry>>,
    pub verification: Result<()>,
    pub destination: PathBuf,
    pub retained_workspace: Option<PathBuf>,
    pub retained_metadata: Option<PathBuf>,
    pub metadata_backup: Option<PathBuf>,
    pub recovery_reference: Option<String>,
    pub failure: Option<WorktreeFailure>,
}

impl Repository {
    pub fn worktrees(&self, cancel: &CancellationToken) -> Result<Vec<WorktreeEntry>> {
        self.runner.step("worktrees", "list_worktrees", || {
            let output = self
                .runner
                .run(
                    self.cwd(),
                    &args(&["worktree", "list", "--porcelain", "-z"]),
                    vec![],
                    cancel,
                )?
                .success("worktree list")?;
            parse_worktrees(&output, &self.info().common_dir)
        })
    }
    pub fn open_worktree(
        &self,
        entry: &WorktreeEntry,
        cancel: &CancellationToken,
    ) -> Result<PathBuf> {
        self.runner
            .step("worktree-open", "validate_worktree_open", || {
                self.check_worktree(entry, cancel)?;
                let repo = self.linked_repository(entry, cancel)?;
                Ok(repo
                    .info()
                    .worktree
                    .clone()
                    .unwrap_or_else(|| repo.info().common_dir.clone()))
            })
    }
    pub fn preview_create_worktree(
        &self,
        path: &Path,
        checkout: WorktreeCheckout,
        start: &str,
        cancel: &CancellationToken,
    ) -> Result<WorktreePreview> {
        self.runner
            .step("worktree-preview", "review_create_worktree", || {
                cancel.check()?;
                let path = self.worktree_destination(path, cancel)?;
                let target = self.check_checkout(&checkout, None, cancel)?.map_or_else(
                    || {
                        self.resolve_commit(start, cancel).map_err(|e| match e {
                            Error::Git { .. } | Error::InvalidInput(_) => {
                                Error::Worktree(WorktreeFailure::TargetUnavailable)
                            }
                            other => other,
                        })
                    },
                    Ok,
                )?;
                Ok(WorktreePreview {
                    repository: self.info().common_dir.clone(),
                    path,
                    target,
                    checkout: Some(checkout),
                    entry: None,
                    git_dir: None,
                    maintenance: None,
                })
            })
    }
    pub fn preview_remove_worktree(
        &self,
        entry: &WorktreeEntry,
        cancel: &CancellationToken,
    ) -> Result<WorktreePreview> {
        self.runner
            .step("worktree-preview", "review_remove_worktree", || {
                let repo = self.removal_context(entry, cancel)?;
                let target = entry
                    .head
                    .clone()
                    .ok_or(Error::Worktree(WorktreeFailure::TargetUnavailable))?;
                Ok(WorktreePreview {
                    repository: self.info().common_dir.clone(),
                    path: entry.path.clone(),
                    target,
                    checkout: None,
                    entry: Some(entry.clone()),
                    git_dir: Some(repo.info().git_dir.clone()),
                    maintenance: None,
                })
            })
    }
    pub fn apply_worktree(
        &self,
        p: &WorktreePreview,
        cancel: &CancellationToken,
    ) -> Result<WorktreeWriteReport> {
        let _permit = self.acquire_write(cancel)?;
        if p.maintenance.is_some() {
            return self.maintain_worktree(p, cancel);
        }
        self.runner.step(p.label(), "revalidate_worktree", || {
            if p.repository != self.info().common_dir {
                return refused(WorktreeFailure::StalePreview);
            }
            if let Some(e) = &p.entry {
                let repo = self.removal_context(e, cancel)?;
                if Some(&repo.info().git_dir) != p.git_dir.as_ref() {
                    return refused(WorktreeFailure::StalePreview);
                }
            } else {
                if self.worktree_destination(&p.path, cancel)? != p.path {
                    return refused(WorktreeFailure::StalePreview);
                }
                self.check_checkout(
                    p.checkout.as_ref().expect("create preview"),
                    Some(&p.target),
                    cancel,
                )?;
            }
            Ok(())
        })?;
        if p.removing() {
            self.remove_reviewed_worktree(p, cancel)
        } else {
            self.create_reviewed_worktree(p, cancel)
        }
    }
    fn worktree_destination(&self, path: &Path, cancel: &CancellationToken) -> Result<PathBuf> {
        // All three platforms publish through local_files::move_path(..., false):
        // MoveFileExW, renameat2(NOREPLACE), or Darwin renamex_np(EXCL). Never
        // fall back to a replacing rename when the filesystem lacks support.
        if !cfg!(any(windows, target_os = "linux", target_os = "macos")) {
            return Err(Error::Unsupported(
                "Safe worktree creation is unavailable on this platform".into(),
            ));
        }
        let destination = init::destination_path(path)
            .map_err(|_| Error::Worktree(WorktreeFailure::InvalidDestination))?;
        match fs::symlink_metadata(&destination) {
            Ok(_) => return refused(WorktreeFailure::DestinationExists),
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(Error::io("inspect worktree destination", e)),
        }
        init::reject_repository(&destination)
            .map_err(|_| Error::Worktree(WorktreeFailure::InvalidDestination))?;
        if destination.starts_with(&self.info().common_dir) {
            return refused(WorktreeFailure::InvalidDestination);
        }
        for e in self.worktrees(cancel)? {
            if equivalent_path(&e.path, &destination) {
                return refused(WorktreeFailure::DestinationExists);
            }
            if destination.starts_with(&e.path) {
                return refused(WorktreeFailure::InvalidDestination);
            }
        }
        Ok(destination)
    }
    fn check_checkout(
        &self,
        c: &WorktreeCheckout,
        pinned: Option<&ObjectId>,
        cancel: &CancellationToken,
    ) -> Result<Option<ObjectId>> {
        self.branch_argument(c.branch().as_bytes(), cancel)
            .map_err(|e| match e {
                Error::InvalidInput(_) => Error::Worktree(WorktreeFailure::InvalidBranch),
                other => other,
            })?;
        let reference = format!("refs/heads/{}", c.branch());
        let branches = self.branches(cancel)?;
        let branch = branches
            .iter()
            .find(|b| b.reference == reference.as_bytes());
        if c.new_branch() {
            if branch.is_some() {
                return refused(WorktreeFailure::BranchExists);
            }
            let Some(upstream) = c.upstream() else {
                return Ok(None);
            };
            // A tracking branch starts at its remote branch, pinned to the reviewed commit.
            let remote = format!("refs/remotes/{upstream}");
            let b = branches
                .iter()
                .find(|b| {
                    b.kind == BranchKind::Remote
                        && b.reference == remote.as_bytes()
                        && b.symbolic_target.is_none()
                })
                .ok_or(Error::Worktree(WorktreeFailure::UpstreamUnavailable))?;
            if pinned.is_some_and(|p| p != &b.target) {
                return refused(WorktreeFailure::StalePreview);
            }
            return Ok(Some(b.target.clone()));
        }
        let b = branch
            .filter(|b| b.symbolic_target.is_none())
            .ok_or(Error::Worktree(WorktreeFailure::BranchUnavailable))?;
        if pinned.is_some_and(|p| p != &b.target) {
            return refused(WorktreeFailure::StalePreview);
        }
        if self
            .worktrees(cancel)?
            .iter()
            .any(|w| w.branch.as_deref() == Some(reference.as_bytes()))
        {
            return refused(WorktreeFailure::BranchInUse);
        }
        Ok(Some(b.target.clone()))
    }
    fn check_worktree(&self, entry: &WorktreeEntry, cancel: &CancellationToken) -> Result<()> {
        if entry.repository != self.info().common_dir || !self.worktrees(cancel)?.contains(entry) {
            return refused(WorktreeFailure::StalePreview);
        }
        if entry.prunable.is_some() || !entry.path.is_dir() {
            return refused(WorktreeFailure::Missing);
        }
        Ok(())
    }
    fn linked_repository(
        &self,
        e: &WorktreeEntry,
        cancel: &CancellationToken,
    ) -> Result<Repository> {
        for path in e.path.ancestors() {
            local_files::directory(path)?;
        }
        let client = GitClient::new(&self.runner.executable, self.runner.limits.clone())?;
        let client = if let Some(log) = &self.runner.log {
            client.with_log(log.clone())
        } else {
            client
        };
        let repo = client.open(&e.path, cancel)?;
        if repo.info().common_dir != self.info().common_dir
            || (!e.bare && repo.info().worktree.as_deref() != Some(e.path.as_path()))
        {
            return refused(WorktreeFailure::StalePreview);
        }
        Ok(repo)
    }
    fn removal_context(&self, e: &WorktreeEntry, cancel: &CancellationToken) -> Result<Repository> {
        self.check_worktree(e, cancel)?;
        if e.main || e.bare || self.info().worktree.as_ref() == Some(&e.path) {
            return refused(WorktreeFailure::ProtectedWorktree);
        }
        if e.locked.is_some() {
            return refused(WorktreeFailure::Locked);
        }
        let repo = self.linked_repository(e, cancel)?;
        if !repo
            .info()
            .git_dir
            .starts_with(self.info().common_dir.join("worktrees"))
        {
            return refused(WorktreeFailure::StalePreview);
        }
        for lock in ["index.lock", "HEAD.lock", "config.worktree.lock", "locked"] {
            if fs::symlink_metadata(repo.info().git_dir.join(lock)).is_ok() {
                return refused(WorktreeFailure::Locked);
            }
        }
        if !repo.operation_state(cancel)?.is_idle() {
            return refused(WorktreeFailure::ActiveOperation);
        }
        let index = repo
            .runner
            .run(
                repo.cwd(),
                &args(&["ls-files", "--stage", "-v", "-z"]),
                vec![],
                cancel,
            )?
            .success("worktree index")?;
        if index.split(|b| *b == 0).any(|r| {
            r.first()
                .is_some_and(|b| b.is_ascii_lowercase() || *b == b'S')
                || r.get(2..8) == Some(b"160000")
        }) {
            return refused(WorktreeFailure::UnsupportedIndex);
        }
        let status = repo
            .runner
            .run(
                repo.cwd(),
                &args(&[
                    "-c",
                    "core.fsmonitor=false",
                    "-c",
                    "core.untrackedCache=false",
                    "status",
                    "--porcelain=v2",
                    "-z",
                    "--untracked-files=all",
                    "--ignored=matching",
                    "--ignore-submodules=none",
                ]),
                vec![],
                cancel,
            )?
            .success("worktree clean check")?;
        if !status.is_empty() {
            return refused(WorktreeFailure::Dirty);
        }
        reject_redirected_tree(&e.path, cancel)?;
        let head = match repo.head(cancel)? {
            Head::Branch { target, .. } | Head::Detached { target } => Some(target),
            Head::Unborn { .. } => None,
        };
        if head != e.head {
            return refused(WorktreeFailure::StalePreview);
        }
        self.check_worktree(e, cancel)?;
        Ok(repo)
    }
    fn create_reviewed_worktree(
        &self,
        p: &WorktreePreview,
        cancel: &CancellationToken,
    ) -> Result<WorktreeWriteReport> {
        let parent = p.path.parent().expect("validated destination");
        let _guard = init::guard_directories(parent)?;
        self.worktree_destination(&p.path, cancel)?;
        let workspace = self
            .runner
            .step(p.label(), "allocate_worktree_workspace", || {
                allocate(parent, cancel)
            })?;
        let checkout = workspace.join("checkout");
        let mode = p.checkout.as_ref().expect("create checkout");
        let mut published = false;
        let mut link = None;
        let command = self.runner.observe("worktree_command", p.label(), || {
            self.runner.step(p.label(), "prepare_worktree", || {
                let mut a = args(&[
                    "-c",
                    "submodule.recurse=false",
                    "-c",
                    "worktree.guessRemote=false",
                    "-c",
                    "worktree.useRelativePaths=false",
                    "worktree",
                    "add",
                    "--no-guess-remote",
                ]);
                match mode {
                    WorktreeCheckout::NewBranch(name) => {
                        a.extend(args(&["--no-track", "-b", name.as_str()]));
                    }
                    WorktreeCheckout::TrackingBranch { branch, .. } => {
                        a.extend(args(&["--track", "-b", branch.as_str()]));
                    }
                    WorktreeCheckout::ExistingBranch(_) => {}
                }
                a.push("--".into());
                a.push(git_path(&checkout));
                a.push(match mode {
                    WorktreeCheckout::NewBranch(_) => p.target.as_str().into(),
                    // Git sets up tracking only from the remote branch's name. It was
                    // checked against the reviewed commit, and the result is verified.
                    WorktreeCheckout::TrackingBranch { upstream, .. } => {
                        format!("refs/remotes/{upstream}").into()
                    }
                    WorktreeCheckout::ExistingBranch(name) => name.into(),
                });
                self.runner
                    .write(self.cwd(), &a, vec![], cancel)?
                    .success("worktree-create")?;
                let admin = self.verify_created(&checkout, p, cancel)?;
                link = Some(PreparedLink::capture(&admin, &checkout)?);
                Ok(())
            })?;
            self.runner.step(p.label(), "publish_worktree", || {
                cancel.check()?;
                self.worktree_destination(&p.path, cancel)?;
                local_files::directory(&workspace)?;
                local_files::directory(&checkout)?;
                local_files::move_path(&checkout, &p.path, false)?;
                published = true;
                Ok(())
            })?;
            // Finish the minimal relink after publication even if cancellation has
            // arrived. A published checkout must not keep a stale registration.
            self.runner.step(p.label(), "relink_worktree", || {
                link.as_ref()
                    .expect("verified checkout link")
                    .publish(&p.path)
            })?;
            cancel.check()?;
            Ok(WriteOutput {
                stdout: vec![],
                stderr: vec![],
            })
        });
        let fresh = CancellationToken::new();
        let observed = self
            .runner
            .step(p.label(), "observe_worktrees", || self.worktrees(&fresh));
        let verification = self.runner.step(p.label(), "verify_worktree_result", || {
            if !published {
                return refused(WorktreeFailure::VerificationFailed);
            }
            self.verify_created(&p.path, p, &fresh).map(|_| ())
        });
        // Remove only our empty outer workspace; never recursively clean a failure.
        let retained = if published && verification.is_ok() && fs::remove_dir(&workspace).is_ok() {
            None
        } else {
            Some(workspace)
        };
        Ok(self.worktree_report(p, command, observed, verification, retained, None))
    }
    fn verify_created(
        &self,
        path: &Path,
        p: &WorktreePreview,
        cancel: &CancellationToken,
    ) -> Result<PathBuf> {
        let canonical = path
            .canonicalize()
            .map_err(|e| Error::io("verify worktree folder", e))?;
        let entry = self
            .worktrees(cancel)?
            .into_iter()
            .find(|w| w.path == canonical)
            .ok_or(Error::Worktree(WorktreeFailure::VerificationFailed))?;
        let checkout = p.checkout.as_ref().expect("create checkout");
        let expected = format!("refs/heads/{}", checkout.branch());
        if entry.main
            || entry.bare
            || entry.locked.is_some()
            || entry.prunable.is_some()
            || entry.head.as_ref() != Some(&p.target)
            || entry.branch.as_deref() != Some(expected.as_bytes())
        {
            return refused(WorktreeFailure::VerificationFailed);
        }
        if let Some(upstream) = checkout.upstream() {
            let tracked = self
                .branches(cancel)?
                .into_iter()
                .find(|b| b.reference == expected.as_bytes())
                .and_then(|b| b.upstream);
            if tracked.as_deref() != Some(format!("refs/remotes/{upstream}").as_bytes()) {
                return refused(WorktreeFailure::VerificationFailed);
            }
        }
        let repo = self.linked_repository(&entry, cancel)?;
        let admin = repo.info().git_dir.clone();
        if admin.parent() != Some(self.info().common_dir.join("worktrees").as_path()) {
            return refused(WorktreeFailure::VerificationFailed);
        }
        Ok(admin)
    }
    fn remove_reviewed_worktree(
        &self,
        p: &WorktreePreview,
        cancel: &CancellationToken,
    ) -> Result<WorktreeWriteReport> {
        let recovery = p.recovery_reference().expect("remove preview");
        let command = self.runner.observe("worktree_command", p.label(), || {
            self.runner.step(p.label(), "retain_worktree_commit", || {
                self.retain_worktree_commit(&recovery, &p.target, cancel)
            })?;
            self.runner.step(p.label(), "remove_worktree", || {
                // Recheck after recovery-ref hooks, immediately before Git's own
                // final clean/lock check. No --force and no automatic retry.
                self.removal_context(p.entry.as_ref().expect("remove entry"), cancel)?;
                let mut a = args(&["worktree", "remove", "--"]);
                a.push(git_path(&p.path));
                let output = self.runner.write(self.cwd(), &a, vec![], cancel)?;
                if !output.status.success() {
                    return Err(Error::Git {
                        operation: p.label(),
                        code: output.status.code(),
                        stdout: output.stdout,
                        stderr: output.stderr,
                    });
                }
                Ok(WriteOutput {
                    stdout: output.stdout,
                    stderr: output.stderr,
                })
            })
        });
        let fresh = CancellationToken::new();
        let observed = self
            .runner
            .step(p.label(), "observe_worktrees", || self.worktrees(&fresh));
        // A cancelled update-ref can already have committed. Report the fresh
        // reference, not whether its interrupted process returned success.
        let retained = self
            .runner
            .step(p.label(), "observe_worktree_recovery", || {
                self.worktree_recovery_ref(&recovery, &fresh)
            });
        let retained =
            matches!(retained, Ok(Some((id, false))) if id == p.target).then_some(recovery);
        let verification = self.runner.step(p.label(), "verify_worktree_result", || {
            if !path_missing(&p.path)
                || !p.git_dir.as_ref().is_some_and(|path| path_missing(path))
                || !matches!(&observed,Ok(v)if !v.iter().any(|e|equivalent_path(&e.path,&p.path)))
            {
                return refused(WorktreeFailure::VerificationFailed);
            }
            if retained.is_none() {
                return refused(WorktreeFailure::VerificationFailed);
            }
            Ok(())
        });
        let mut report = self.worktree_report(p, command, observed, verification, None, retained);
        if path_missing(&p.path) {
            report.retained_metadata = p
                .git_dir
                .as_ref()
                .filter(|path| !path_missing(path))
                .cloned();
        }
        Ok(report)
    }
    fn retain_worktree_commit(
        &self,
        reference: &str,
        id: &ObjectId,
        cancel: &CancellationToken,
    ) -> Result<()> {
        let existing = self.worktree_recovery_ref(reference, cancel)?;
        let mut input = Vec::new();
        if let Some((existing_id, symbolic)) = existing {
            if &existing_id != id || symbolic {
                return refused(WorktreeFailure::ReferenceConflict);
            }
            input.extend_from_slice(b"verify ");
        } else {
            input.extend_from_slice(b"create ");
        }
        input.extend_from_slice(reference.as_bytes());
        input.push(0);
        input.extend_from_slice(id.as_str().as_bytes());
        input.push(0);
        self.runner
            .write(
                self.cwd(),
                &args(&["update-ref", "--no-deref", "--stdin", "-z"]),
                input,
                cancel,
            )?
            .success("retain worktree commit")?;
        Ok(())
    }
    fn worktree_recovery_ref(
        &self,
        reference: &str,
        cancel: &CancellationToken,
    ) -> Result<Option<(ObjectId, bool)>> {
        let refs = self
            .runner
            .run(
                self.cwd(),
                &args(&[
                    "for-each-ref",
                    "--format=%(refname)%00%(objectname)%00%(symref)%00",
                    "--",
                    reference,
                ]),
                vec![],
                cancel,
            )?
            .success("worktree recovery ref")?;
        let existing = refs
            .split(|b| *b == b'\n')
            .find(|r| r.split(|b| *b == 0).next() == Some(reference.as_bytes()));
        if let Some(row) = existing {
            let fields: Vec<_> = row.split(|b| *b == 0).collect();
            if fields.len() != 4 {
                return refused(WorktreeFailure::ReferenceConflict);
            }
            let id = std::str::from_utf8(fields[1])
                .map_err(|_| Error::Worktree(WorktreeFailure::ReferenceConflict))?;
            Ok(Some((ObjectId::parse(id)?, !fields[2].is_empty())))
        } else {
            Ok(None)
        }
    }
    fn worktree_report(
        &self,
        p: &WorktreePreview,
        command: Result<WriteOutput>,
        observed: Result<Vec<WorktreeEntry>>,
        verification: Result<()>,
        retained_workspace: Option<PathBuf>,
        recovery_reference: Option<String>,
    ) -> WorktreeWriteReport {
        let failure = match &command {
            Err(Error::Worktree(f)) => Some(*f),
            Err(_) => Some(WorktreeFailure::CommandFailed),
            Ok(_) if verification.is_err() => Some(WorktreeFailure::VerificationFailed),
            _ => None,
        };
        let _ = self
            .runner
            .observe("worktree_outcome", p.label(), || match failure {
                Some(f) => refused::<()>(f),
                None => Ok(()),
            });
        WorktreeWriteReport {
            command,
            observed,
            verification,
            destination: p.path.clone(),
            retained_workspace,
            retained_metadata: None,
            metadata_backup: None,
            recovery_reference,
            failure,
        }
    }
}

/// Only the newly created checkout's documented absolute backlink needs to move.
/// `git worktree repair <path>` also repairs unrelated registered worktrees; do
/// not invoke it as part of creation. See gitrepository-layout, worktrees/id/gitdir.
struct PreparedLink {
    admin: PathBuf,
    gitfile: Vec<u8>,
    backlink: Vec<u8>,
    _guard: Vec<fs::File>,
}
impl PreparedLink {
    fn capture(admin: &Path, checkout: &Path) -> Result<Self> {
        let guard = init::guard_directories(admin)?;
        let gitfile = local_files::read_config(&checkout.join(".git"))?;
        let backlink = local_files::read_config(&admin.join("gitdir"))?;
        let forward = std::str::from_utf8(&gitfile)
            .ok()
            .and_then(|s| s.trim_end_matches(['\r', '\n']).strip_prefix("gitdir: "))
            .map(Path::new)
            .filter(|p| p.is_absolute());
        let backward = std::str::from_utf8(&backlink)
            .ok()
            .map(|s| Path::new(s.trim_end_matches(['\r', '\n'])))
            .filter(|p| p.is_absolute());
        if forward.and_then(|p| p.canonicalize().ok()).as_deref() != Some(admin)
            || backward.and_then(|p| p.canonicalize().ok())
                != checkout.join(".git").canonicalize().ok()
            || backward.is_none()
        {
            return refused(WorktreeFailure::VerificationFailed);
        }
        Ok(Self {
            admin: admin.to_owned(),
            gitfile,
            backlink,
            _guard: guard,
        })
    }
    fn publish(&self, destination: &Path) -> Result<()> {
        let backlink = self.admin.join("gitdir");
        let lock = self.admin.join("gitdir.lock");
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&lock)
            .map_err(|e| Error::io("lock new worktree backlink", e))?;
        let gitfile = destination.join(".git");
        // Git's backlink reader strips the final '/.git' component using slash
        // spelling even on Windows. Match the format written by Git itself.
        let value = format!(
            "{}\n",
            git_path(&gitfile)
                .to_str()
                .ok_or(Error::Worktree(WorktreeFailure::InvalidDestination))?
                .replace('\\', "/")
        );
        file.write_all(value.as_bytes())
            .and_then(|_| file.sync_all())
            .map_err(|e| Error::io("write new worktree backlink", e))?;
        drop(file);
        if local_files::read_config(&backlink)? != self.backlink
            || local_files::read_config(&gitfile)? != self.gitfile
            || local_files::read_config(&lock)? != value.as_bytes()
        {
            return refused(WorktreeFailure::StalePreview);
        }
        // The destination is already published. Do not interrupt this minimal
        // atomic relink. On failure retain the lock/data for explicit inspection.
        local_files::move_path(&lock, &backlink, true)
    }
}

fn path_missing(path: &Path) -> bool {
    matches!(fs::symlink_metadata(path), Err(e) if e.kind() == io::ErrorKind::NotFound)
}

fn allocate(parent: &Path, cancel: &CancellationToken) -> Result<PathBuf> {
    cancel.check()?;
    let time = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    for _ in 0..16 {
        let p = parent.join(format!(
            ".rocket-worktree-{}-{time:x}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        match fs::create_dir(&p) {
            Ok(()) => return Ok(p),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(Error::io("allocate worktree workspace", e)),
        }
    }
    Err(Error::Worktree(WorktreeFailure::InvalidDestination))
}
fn equivalent_path(a: &Path, b: &Path) -> bool {
    if a == b {
        return true;
    }
    #[cfg(windows)]
    {
        a.to_string_lossy()
            .trim_start_matches(r"\\?\")
            .replace('\\', "/")
            .eq_ignore_ascii_case(
                &b.to_string_lossy()
                    .trim_start_matches(r"\\?\")
                    .replace('\\', "/"),
            )
    }
    #[cfg(not(windows))]
    {
        false
    }
}
fn reject_redirected_tree(root: &Path, cancel: &CancellationToken) -> Result<()> {
    let mut stack = vec![root.to_owned()];
    let mut count = 0;
    while let Some(path) = stack.pop() {
        cancel.check()?;
        for e in fs::read_dir(path).map_err(|e| Error::io("inspect removable worktree", e))? {
            cancel.check()?;
            let e = e.map_err(|e| Error::io("inspect worktree entry", e))?;
            count += 1;
            if count > 200000 {
                return Err(Error::Unsupported(
                    "Removal inspection exceeds 200,000 filesystem entries; use Git after review"
                        .into(),
                ));
            }
            let meta = fs::symlink_metadata(e.path())
                .map_err(|e| Error::io("inspect worktree file", e))?;
            if local_files::redirected(&meta) {
                return refused(WorktreeFailure::UnsupportedIndex);
            }
            if meta.is_dir() {
                stack.push(e.path())
            }
        }
    }
    Ok(())
}
fn parse_worktrees(bytes: &[u8], repository: &Path) -> Result<Vec<WorktreeEntry>> {
    let mut entries = Vec::new();
    let mut current: Option<WorktreeEntry> = None;
    for field in bytes.split(|b| *b == 0) {
        if field.is_empty() {
            if let Some(e) = current.take() {
                if !e.bare && e.head.is_none() {
                    return Err(Error::parse("worktrees", "missing HEAD"));
                }
                entries.push(e)
            }
            continue;
        }
        if let Some(raw) = field.strip_prefix(b"worktree ") {
            if current.is_some() {
                return Err(Error::parse("worktrees", "missing record boundary"));
            }
            let raw = PathBuf::from(bytes_to_os(raw)?);
            if !raw.is_absolute() {
                return Err(Error::parse("worktrees", "relative worktree path"));
            }
            let path = match raw.canonicalize() {
                Ok(p) => p,
                Err(e) if e.kind() == io::ErrorKind::NotFound => raw,
                Err(e) => return Err(Error::io("resolve worktree listing", e)),
            };
            current = Some(WorktreeEntry {
                path,
                head: None,
                branch: None,
                main: entries.is_empty(),
                bare: false,
                locked: None,
                prunable: None,
                repository: repository.to_owned(),
            });
            continue;
        }
        let e = current
            .as_mut()
            .ok_or_else(|| Error::parse("worktrees", "attribute outside record"))?;
        if let Some(id) = field.strip_prefix(b"HEAD ") {
            e.head = Some(ObjectId::parse(
                std::str::from_utf8(id).map_err(|_| Error::parse("worktrees", "invalid HEAD"))?,
            )?)
        } else if let Some(branch) = field.strip_prefix(b"branch ") {
            if !branch.starts_with(b"refs/heads/") {
                return Err(Error::parse("worktrees", "unexpected branch namespace"));
            }
            e.branch = Some(branch.to_vec())
        } else if field == b"bare" {
            e.bare = true
        } else if field == b"detached" {
        } else if field == b"locked" || field.starts_with(b"locked ") {
            e.locked = Some(field.get(7..).unwrap_or_default().to_vec())
        } else if field == b"prunable" || field.starts_with(b"prunable ") {
            e.prunable = Some(field.get(9..).unwrap_or_default().to_vec())
        } else {
            return Err(Error::parse("worktrees", "unknown worktree attribute"));
        }
    }
    if current.is_some() || entries.is_empty() {
        return Err(Error::parse("worktrees", "incomplete list"));
    }
    Ok(entries)
}
